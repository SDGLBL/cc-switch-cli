//! Local web dashboard server.
//!
//! Serves the cc-switch React frontend over HTTP on loopback and bridges the
//! frontend's Tauri `invoke()` / `listen()` calls to the same domain services
//! the CLI uses. The frontend is built from the cc-switch desktop source with a
//! Vite alias that replaces `@tauri-apps/api/core` (and friends) with a thin
//! HTTP/SSE shim, so the React code runs unchanged in a browser.
//!
//! Entry point: `cc-switch web serve` in `cli/commands/web.rs`.

pub mod assets;
pub mod dispatch;
pub mod error;
pub mod events;
pub mod server;
pub mod state;

pub use server::build_router;
pub use state::WebState;
