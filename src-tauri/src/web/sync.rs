//! Live cross-process refresh.
//!
//! A background task watches the shared SQLite database for **external** changes
//! (via `PRAGMA data_version`, which only advances when another connection —
//! e.g. the TUI, or another `cc-switch` process — commits). On such a change it
//! refreshes this server's in-memory config snapshot and pushes a `db-changed`
//! SSE event so connected browsers refetch. The server's own mutations never
//! bump `data_version`, so this never fires for changes the web made itself
//! (those already update the snapshot write-through and emit their own events).

use std::time::Duration;

use serde_json::json;

use super::state::WebState;

/// Poll cadence. `PRAGMA data_version` is a cheap cached read, so 1s gives
/// near-immediate sync at negligible cost.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Spawn the watcher on the current Tokio runtime. The task ends when the
/// runtime shuts down (after the server stops), so no explicit handle is kept.
pub fn spawn_db_change_watcher(state: WebState) {
    tokio::spawn(async move {
        let mut last = state.app.db.data_version().unwrap_or(0);
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        // If the loop ever stalls, don't burst-catch-up — one check is enough.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            let current = match state.app.db.data_version() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if current == last {
                continue;
            }
            last = current;
            // Another connection committed: refresh our snapshot so subsequent
            // reads are fresh, then nudge browsers to refetch.
            //
            // `refresh_config_from_db()` only rebuilds the DB-derived config
            // snapshot. The current provider is resolved via
            // `get_effective_current_provider`, which lets the process-global
            // `settings_store` cache win over the DB — so also reload settings
            // and make DB current authoritative, otherwise `get_current_provider`
            // keeps returning this process's stale cached current after a switch
            // made by the TUI/another process.
            let _ = state.app.refresh_config_from_db();
            let _ = crate::settings::reload_settings();
            let _ = crate::settings::sync_current_providers_from_db(&state.app.db);
            let _ = state
                .events
                .send(json!({ "event": "db-changed" }).to_string());
        }
    });
}
