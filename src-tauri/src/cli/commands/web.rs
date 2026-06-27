use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, ValueEnum};

use crate::cli::ui::{error, highlight, info, success};
use crate::web::tunnel::{Tunnel, TunnelMode};
use crate::web::{self};
use crate::{AppError, AppState, AppType};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum TunnelProvider {
    /// Expose via the local `tailscale` CLI (serve = private tailnet, or
    /// funnel = public with --tunnel-public).
    Tailscale,
}

/// `cc-switch web` — start the local web dashboard. The frontend is embedded in
/// the binary, so this is self-contained; it binds loopback and prints how to
/// reach it (headless-friendly).
#[derive(Args, Debug, Clone)]
pub struct WebArgs {
    /// Port to listen on (0 = pick an ephemeral free port)
    #[arg(long, default_value_t = 0)]
    pub port: u16,

    /// Address to bind. Defaults to loopback (127.0.0.1). Use 0.0.0.0 to expose
    /// on the network — DANGER: the dashboard can read/write all your provider
    /// API keys and settings; prefer SSH port-forwarding or --tunnel instead.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Override the embedded frontend with assets from disk (dev only; must
    /// contain index.html). Normally the frontend is embedded in the binary.
    #[arg(long, env = "CC_SWITCH_WEB_ASSETS")]
    pub assets: Option<PathBuf>,

    /// Session token (default: a random token generated per run)
    #[arg(long)]
    pub token: Option<String>,

    /// Expose the dashboard through a reverse tunnel. Bare `--tunnel` defaults
    /// to tailscale (the only provider today; kept as an enum so more can be
    /// added). The server still binds locally; the tunnel forwards to it.
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "tailscale")]
    pub tunnel: Option<TunnelProvider>,

    /// With `--tunnel tailscale`, expose PUBLICLY via Tailscale Funnel instead
    /// of the private tailnet. Anyone with the URL gets full control.
    #[arg(long)]
    pub tunnel_public: bool,
}

pub fn execute(args: WebArgs, _app: Option<AppType>) -> Result<(), AppError> {
    serve_web(args)
}

fn serve_web(args: WebArgs) -> Result<(), AppError> {
    let WebArgs {
        port,
        host,
        assets,
        token,
        tunnel,
        tunnel_public,
    } = args;

    match &assets {
        // `--assets` given (dev override): must contain a built frontend.
        Some(dir) => {
            if !dir.join("index.html").is_file() {
                return Err(AppError::Message(format!(
                    "assets directory '{}' has no index.html — build the frontend first (pnpm build:web)",
                    dir.display()
                )));
            }
        }
        // No override: rely on the frontend embedded at build time.
        None => {
            if !crate::web::assets::embedded_present() {
                return Err(AppError::Message(
                    "this build has no embedded frontend and no --assets was given; \
                     rebuild with CC_SWITCH_WEB_DIST=<path to dist-web>, or pass --assets <dir>"
                        .to_string(),
                ));
            }
        }
    }

    let ip: IpAddr = host.parse().map_err(|_| {
        AppError::Message(format!(
            "invalid --host '{host}': expected an IP address like 127.0.0.1 or 0.0.0.0"
        ))
    })?;

    // Startup recovery already ran in main before dispatch; build the working
    // snapshot the same way other commands do.
    let state = AppState::try_new()?;
    let token = token.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let addr = SocketAddr::new(ip, port);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Message(format!("failed to create async runtime: {e}")))?;

    runtime.block_on(async move {
        let web_state = web::WebState::new(Arc::new(state), token.clone());

        // Bind first so we can target the real (possibly ephemeral) port.
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| AppError::Message(format!("failed to bind {addr}: {e}")))?;
        let local = listener
            .local_addr()
            .map_err(|e| AppError::Message(format!("failed to read local address: {e}")))?;
        let bound_port = local.port();
        let loopback_url = format!("http://127.0.0.1:{bound_port}/?token={token}");

        // Set up the tunnel (if requested). `_tunnel` is held until the end of
        // this scope; its Drop tears the tunnel down after the server stops.
        let (primary_url, _tunnel) = match tunnel {
            Some(TunnelProvider::Tailscale) => {
                let mode = if tunnel_public {
                    print_public_warning();
                    TunnelMode::Funnel
                } else {
                    TunnelMode::Serve
                };
                let (handle, base) = Tunnel::start(mode, bound_port)?;
                (format!("{base}/?token={token}"), Some(handle))
            }
            None => (format!("http://{local}/?token={token}"), None),
        };

        println!(
            "{}",
            highlight(crate::t!("Local Web Dashboard", "本地 Web 控制台"))
        );
        println!("{}", success(&primary_url));

        if tunnel.is_some() {
            // Also surface the loopback URL for on-machine access.
            println!(
                "{}",
                info(&format!("{} {loopback_url}", crate::t!("Local:", "本地：")))
            );
        } else if ip.is_loopback() {
            // Users are typically on a remote, browser-less box. Show how to
            // reach the loopback server from their own machine via SSH
            // port-forwarding (works with or without a public IP), and point at
            // the tunnel for the no-SSH / no-public-IP case.
            println!(
                "{}",
                info(crate::t!(
                    "Headless/remote? Port-forward from your own machine, then open the URL above:",
                    "无头机/远程？在你自己的电脑上做端口转发，然后打开上面的地址："
                ))
            );
            println!("    ssh -L {bound_port}:127.0.0.1:{bound_port} <user>@<this-server>");
            println!(
                "{}",
                info(crate::t!(
                    "No public IP? Try:  cc-switch web --tunnel tailscale",
                    "没有公网？可用：  cc-switch web --tunnel tailscale"
                ))
            );
        } else {
            // Bound to a non-loopback address via --host: reachable over the network.
            print_public_warning();
            println!(
                "{}",
                info(crate::t!(
                    "Bound to a network interface — reach it at this machine's IP on this port.",
                    "已绑定到网络接口 —— 用本机 IP + 该端口访问。"
                ))
            );
        }

        println!(
            "{}",
            info(crate::t!(
                "This URL grants full control of your providers and settings — do not share it.",
                "该地址可完全控制你的供应商与设置，请勿分享。"
            ))
        );
        println!(
            "{}",
            info(crate::t!("Press Ctrl-C to stop.", "按 Ctrl-C 停止。"))
        );

        // Live refresh: push external DB changes (e.g. from the TUI) to browsers.
        web::sync::spawn_db_change_watcher(web_state.clone());

        let app = web::build_router(web_state, assets);
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|e| AppError::Message(format!("web server error: {e}")));

        if tunnel.is_some() {
            println!(
                "{}",
                info(crate::t!("Tearing down tunnel…", "正在拆除隧道…"))
            );
        }
        // `_tunnel` drops here -> tailscale {serve|funnel} ... off.
        result
    })
}

fn print_public_warning() {
    println!(
        "{}",
        error(crate::t!(
            "⚠ PUBLIC exposure: this dashboard will be reachable by ANYONE who can reach \
             this address, and it can read/write your provider API keys. Prefer SSH \
             port-forwarding or a private tailnet unless you really need this.",
            "⚠ 公网暴露：该控制台将对任何能访问此地址的人开放，且能读写你的 provider 密钥。\
             除非确有需要，建议改用 SSH 端口转发或私有 tailnet。"
        ))
    );
}
