use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};

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

#[derive(Subcommand, Debug, Clone)]
pub enum WebCommand {
    /// Start the local web dashboard server (loopback only)
    Serve {
        /// Port to listen on (0 = pick an ephemeral free port)
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Directory containing the built frontend assets (must contain index.html)
        #[arg(long, env = "CC_SWITCH_WEB_ASSETS")]
        assets: PathBuf,

        /// Session token (default: a random token generated per run)
        #[arg(long)]
        token: Option<String>,

        /// Expose the dashboard through a reverse tunnel (currently: tailscale).
        /// The server still binds 127.0.0.1; the tunnel forwards to it locally.
        #[arg(long, value_enum)]
        tunnel: Option<TunnelProvider>,

        /// With `--tunnel tailscale`, expose PUBLICLY via Tailscale Funnel
        /// instead of the private tailnet. Anyone with the URL gets full control
        /// of your providers and settings.
        #[arg(long)]
        tunnel_public: bool,
    },
}

pub fn execute(cmd: WebCommand, _app: Option<AppType>) -> Result<(), AppError> {
    match cmd {
        WebCommand::Serve {
            port,
            assets,
            token,
            tunnel,
            tunnel_public,
        } => serve_web(port, assets, token, tunnel, tunnel_public),
    }
}

fn serve_web(
    port: u16,
    assets: PathBuf,
    token: Option<String>,
    tunnel: Option<TunnelProvider>,
    tunnel_public: bool,
) -> Result<(), AppError> {
    if !assets.join("index.html").is_file() {
        return Err(AppError::Message(format!(
            "assets directory '{}' has no index.html — build the frontend first (pnpm build:web)",
            assets.display()
        )));
    }

    // Startup recovery already ran in main before dispatch; build the working
    // snapshot the same way other commands do.
    let state = AppState::try_new()?;
    let token = token.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Loopback only — never bind a non-local interface for this admin surface.
    // The tunnel (if any) forwards to this loopback port locally.
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

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
        let local_url = format!("http://{local}/?token={token}");

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
                let (handle, base) = Tunnel::start(mode, local.port())?;
                (format!("{base}/?token={token}"), Some(handle))
            }
            None => (local_url.clone(), None),
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
                info(&format!("{} {local_url}", crate::t!("Local:", "本地：")))
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
            "⚠ PUBLIC tunnel: this dashboard will be reachable by ANYONE with the URL, \
             and it can read/write your provider API keys. Prefer the private tailnet \
             (drop --tunnel-public) unless you really need public access.",
            "⚠ 公网隧道：该控制台将对任何拿到 URL 的人开放，且能读写你的 provider 密钥。\
             除非确有需要，建议改用私有 tailnet（去掉 --tunnel-public）。"
        ))
    );
}
