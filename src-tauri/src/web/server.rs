use std::path::PathBuf;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use super::{assets, dispatch, events, state::WebState};

/// Body-size cap for `/api/invoke` (settings payloads are small).
const MAX_BODY: usize = 16 * 1024 * 1024;

/// Build the full router: token-gated `/api/*` plus the static frontend.
///
/// Security posture:
/// - Static assets are public (just the SPA shell); all data lives behind
///   `/api/*`, which is gated by the per-run session token.
/// - The frontend is served same-origin as the API, so no CORS is configured
///   (no cross-origin access is intended). This is deliberately stricter than
///   the proxy server's permissive `Any` CORS.
pub fn build_router(state: WebState, assets_dir: PathBuf) -> Router {
    // Token-protected endpoints (invoke bridge + event stream).
    let protected = Router::new()
        .route("/invoke/:command", post(dispatch::invoke_handler))
        .route("/events", get(events::sse_handler))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    // `/api/health` is public for liveness probing.
    let api = Router::new().route("/health", get(health)).merge(protected);

    Router::new()
        .nest("/api", api)
        .fallback_service(assets::serve_dir(&assets_dir))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

/// Token gate for `/api/invoke` and `/api/events`.
///
/// Accepts the token from the `X-CC-Switch-Token` header (set by the invoke
/// shim) or a `token` query param (used by the `EventSource` shim, which cannot
/// set custom headers). Comparison is constant-time.
async fn require_token(
    State(state): State<WebState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided = req
        .headers()
        .get("x-cc-switch-token")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| token_from_query(req.uri().query()));

    match provided {
        Some(t) if constant_time_eq(t.as_bytes(), state.token.as_bytes()) => {
            Ok(next.run(req).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

fn token_from_query(query: Option<&str>) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            return Some(percent_decode(value));
        }
    }
    None
}

/// Minimal percent-decoder symmetric with the browser's `encodeURIComponent`,
/// used to recover the session token from the `?token=` query the EventSource
/// shim sends (EventSource cannot set headers). Dependency-free: `%XX` -> byte,
/// other bytes pass through. Without this, a custom `--token` containing any
/// character `encodeURIComponent` escapes would 401 the SSE stream while the
/// header-based invoke path still works — a confusing asymmetry.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_recovers_encoded_token() {
        // encodeURIComponent("a b/c:1") => "a%20b%2Fc%3A1"
        assert_eq!(percent_decode("a%20b%2Fc%3A1"), "a b/c:1");
        // Lowercase hex and a literal that needs no decoding.
        assert_eq!(percent_decode("%2bx"), "+x");
        assert_eq!(percent_decode("550e8400-e29b"), "550e8400-e29b");
        // Trailing stray '%' is passed through, not panicked on.
        assert_eq!(percent_decode("ab%"), "ab%");
    }

    #[test]
    fn token_from_query_decodes_value() {
        assert_eq!(
            token_from_query(Some("foo=1&token=a%20b&bar=2")).as_deref(),
            Some("a b")
        );
        assert_eq!(token_from_query(Some("no=token")), None);
        assert_eq!(token_from_query(None), None);
    }

    #[test]
    fn constant_time_eq_matches_only_identical() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
