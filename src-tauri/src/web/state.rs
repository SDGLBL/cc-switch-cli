use std::sync::Arc;

use tokio::sync::broadcast;

use crate::AppState;

/// Shared state handed to every axum handler.
///
/// Holds `Arc`s because [`AppState`] is intentionally not `Clone` (it owns the
/// database handle and the in-memory config snapshot). The `events` channel
/// fans Tauri-style events out to all connected SSE clients.
#[derive(Clone)]
pub struct WebState {
    /// The single, process-wide application state, shared across requests.
    pub app: Arc<AppState>,
    /// Per-run session token required on every `/api/*` request.
    pub token: Arc<String>,
    /// Broadcast channel for server-sent events (frontend `listen()` bridge).
    pub events: broadcast::Sender<String>,
}

impl WebState {
    pub fn new(app: Arc<AppState>, token: String) -> Self {
        let (events, _rx) = broadcast::channel(64);
        Self {
            app,
            token: Arc::new(token),
            events,
        }
    }
}
