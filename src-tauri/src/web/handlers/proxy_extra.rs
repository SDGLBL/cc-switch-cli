//! Supplementary proxy commands not covered by the main proxy handlers.
//!
//! Follows the [`super::meta`] template.
//!   - `stop_proxy_server` -> [`crate::services::proxy::ProxyService::stop`].
//!   - `get_upstream_proxy_status` -> reads the persisted global proxy URL.
//!   - `test_proxy_url` -> measures a request through the given proxy.
//!   - `scan_local_proxies` -> probes common local proxy-client ports.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::common::{block_on, ok_null, str_arg};
use crate::web::error::WebError;
use crate::AppState;

/// Map a service-layer `String` error into a domain [`WebError`].
fn domain_str(e: String) -> WebError {
    WebError::Domain(crate::AppError::Message(e))
}

pub fn dispatch(state: &AppState, command: &str, args: &Value) -> Option<Result<Value, WebError>> {
    Some(match command {
        // No args -> void. Stops the proxy runtime only (no takeover restore).
        "stop_proxy_server" => match block_on(async { state.proxy_service.stop().await }) {
            Ok(()) => ok_null(),
            Err(e) => Err(domain_str(e)),
        },

        // No args -> { enabled, proxyUrl }. Reads the persisted outbound proxy.
        "get_upstream_proxy_status" => match state.db.get_global_proxy_url() {
            Ok(url) => {
                let url = url.filter(|u| !u.trim().is_empty());
                Ok(json!({ "enabled": url.is_some(), "proxyUrl": url }))
            }
            Err(e) => Err(WebError::Domain(e)),
        },

        // { url } -> ProxyTestResult { success, latencyMs, error }. Never throws
        // on a connection failure — the failure is reported in the result.
        "test_proxy_url" => match str_arg(args, "url") {
            Ok(url) => Ok(test_proxy(url)),
            Err(e) => Err(e),
        },

        // No args -> DetectedProxy[] { url, proxyType, port }. Probes common
        // local proxy-client ports on 127.0.0.1.
        "scan_local_proxies" => Ok(scan_local_proxies()),

        _ => return None,
    })
}

/// Make one request through `proxy_url` and time it. Returns a `ProxyTestResult`.
fn test_proxy(proxy_url: &str) -> Value {
    let outcome = block_on(async {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        client
            .get("https://www.gstatic.com/generate_204")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok::<u64, String>(start.elapsed().as_millis() as u64)
    });
    match outcome {
        Ok(ms) => json!({ "success": true, "latencyMs": ms, "error": Value::Null }),
        Err(e) => json!({ "success": false, "latencyMs": 0, "error": e }),
    }
}

/// Probe common HTTP/SOCKS proxy-client ports on loopback; report the open ones.
fn scan_local_proxies() -> Value {
    // (port, scheme) for the usual local clients (Clash, v2rayN, Verge, ...).
    const COMMON: &[(u16, &str)] = &[
        (7890, "http"),
        (7891, "socks5"),
        (7897, "http"),
        (1080, "socks5"),
        (1087, "http"),
        (8080, "http"),
        (8888, "http"),
        (10808, "socks5"),
        (10809, "http"),
        (2080, "http"),
        (20171, "socks5"),
        (33210, "socks5"),
        (4780, "http"),
    ];
    let mut found = Vec::new();
    for (port, scheme) in COMMON {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *port);
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            found.push(json!({
                "url": format!("{scheme}://127.0.0.1:{port}"),
                "proxyType": scheme,
                "port": port,
            }));
        }
    }
    Value::Array(found)
}
