//! Optional reverse tunnel for `cc-switch web serve --tunnel`.
//!
//! Shells out to the locally-installed `tailscale` CLI to expose the loopback
//! web server to either the private tailnet (`tailscale serve`, the safe
//! default) or the public internet (`tailscale funnel`, opt-in via
//! `--tunnel-public`). We use `--bg`, so tailscaled owns the proxying and there
//! is no child process to babysit — just a setup call and an `off` teardown,
//! which runs from [`Tunnel`]'s `Drop` so it always fires.
//!
//! The web server still binds 127.0.0.1 only; the tailscale proxy connects to
//! it locally, so the loopback-only + session-token design is unchanged.

use std::process::Command;

use crate::AppError;

/// Tailnet-side HTTPS port. 8443 (not 443) by default so we never clobber an
/// existing `tailscale serve`/`funnel` mapping on the standard HTTPS port — and
/// so our `off` teardown can't remove someone else's service.
const HTTPS_PORT: u16 = 8443;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelMode {
    /// Private: reachable only by devices on your tailnet (`tailscale serve`).
    Serve,
    /// Public: reachable by anyone with the URL (`tailscale funnel`).
    Funnel,
}

impl TunnelMode {
    fn subcommand(self) -> &'static str {
        match self {
            TunnelMode::Serve => "serve",
            TunnelMode::Funnel => "funnel",
        }
    }
}

/// An active tailscale serve/funnel mapping. Tears itself down on drop.
pub struct Tunnel {
    mode: TunnelMode,
    target_port: u16,
}

impl Tunnel {
    /// Set up `tailscale {serve|funnel} --bg` pointing at `localhost:target_port`.
    /// Returns the tunnel handle and the base URL (no token).
    pub fn start(mode: TunnelMode, target_port: u16) -> Result<(Self, String), AppError> {
        ensure_ready(mode)?;

        let target = format!("localhost:{target_port}");
        let status = Command::new("tailscale")
            .args([
                mode.subcommand(),
                "--bg",
                &format!("--https={HTTPS_PORT}"),
                "--yes",
                &target,
            ])
            .output()
            .map_err(|e| AppError::Message(format!("failed to run `tailscale`: {e}")))?;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            return Err(AppError::Message(format!(
                "`tailscale {}` failed: {}",
                mode.subcommand(),
                stderr.trim()
            )));
        }

        let host = dns_name()?;
        let url = format!("https://{host}:{HTTPS_PORT}");
        Ok((Self { mode, target_port }, url))
    }

    fn teardown(&self) {
        let target = format!("localhost:{}", self.target_port);
        let _ = Command::new("tailscale")
            .args([
                self.mode.subcommand(),
                &format!("--https={HTTPS_PORT}"),
                "--yes",
                &target,
                "off",
            ])
            .output();
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// Verify the tailscale CLI is installed and the node is up before we try to
/// set up a tunnel, so failures produce a clear, actionable message.
fn ensure_ready(mode: TunnelMode) -> Result<(), AppError> {
    which::which("tailscale").map_err(|_| {
        AppError::Message(
            "`tailscale` CLI not found. Install Tailscale and run `tailscale up`, \
             then retry with --tunnel tailscale."
                .into(),
        )
    })?;

    let status = Command::new("tailscale")
        .arg("status")
        .output()
        .map_err(|e| AppError::Message(format!("failed to run `tailscale status`: {e}")))?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        return Err(AppError::Message(format!(
            "tailscale is not ready — run `tailscale up` first: {}",
            stderr.trim()
        )));
    }

    if mode == TunnelMode::Funnel {
        // Funnel additionally requires it to be enabled in the tailnet ACLs and
        // the node to carry the funnel attribute; if not, the setup call below
        // returns a descriptive error (with an admin URL) which we surface.
    }
    Ok(())
}

/// The node's MagicDNS name (e.g. `machine.tailnet.ts.net`), trailing dot stripped.
fn dns_name() -> Result<String, AppError> {
    let out = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .map_err(|e| AppError::Message(format!("failed to run `tailscale status --json`: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Message("failed to read tailscale status".into()));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| AppError::Message(format!("invalid `tailscale status` JSON: {e}")))?;
    let dns = json
        .get("Self")
        .and_then(|s| s.get("DNSName"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Message("`tailscale status` did not report this node's DNS name".into())
        })?;
    Ok(dns.trim_end_matches('.').to_string())
}
