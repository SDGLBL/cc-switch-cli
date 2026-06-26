use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::AppError;

/// Error type for the web layer.
///
/// Serializes to `{ "error": "<message>" }` so the browser `invoke()` shim can
/// reject its promise with a JS `Error` carrying the message — matching Tauri's
/// `invoke()` reject contract, which the frontend already surfaces in toasts.
#[derive(Debug)]
pub enum WebError {
    /// A domain-level failure from the shared services.
    Domain(AppError),
    /// The command exists in the desktop app but is not wired in the web
    /// prototype yet. Returns HTTP 501 so partial panels degrade gracefully.
    NotImplemented(String),
    /// The request was malformed (missing/!invalid args).
    BadRequest(String),
}

impl From<AppError> for WebError {
    fn from(e: AppError) -> Self {
        WebError::Domain(e)
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            WebError::Domain(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            WebError::NotImplemented(cmd) => (
                StatusCode::NOT_IMPLEMENTED,
                format!("command '{cmd}' is not implemented in the web prototype"),
            ),
            WebError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
