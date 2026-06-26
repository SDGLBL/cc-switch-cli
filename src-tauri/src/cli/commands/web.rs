use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;

use crate::cli::ui::{highlight, info, success};
use crate::web;
use crate::{AppError, AppState, AppType};

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
    },
}

pub fn execute(cmd: WebCommand, _app: Option<AppType>) -> Result<(), AppError> {
    match cmd {
        WebCommand::Serve {
            port,
            assets,
            token,
        } => serve_web(port, assets, token),
    }
}

fn serve_web(port: u16, assets: PathBuf, token: Option<String>) -> Result<(), AppError> {
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
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Message(format!("failed to create async runtime: {e}")))?;

    runtime.block_on(async move {
        let web_state = web::WebState::new(Arc::new(state), token.clone());

        // Bind first so we can print the real (possibly ephemeral) port.
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| AppError::Message(format!("failed to bind {addr}: {e}")))?;
        let local = listener
            .local_addr()
            .map_err(|e| AppError::Message(format!("failed to read local address: {e}")))?;
        let url = format!("http://{local}/?token={token}");

        println!(
            "{}",
            highlight(crate::t!("Local Web Dashboard", "本地 Web 控制台"))
        );
        println!("{}", success(&url));
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
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|e| AppError::Message(format!("web server error: {e}")))
    })
}
