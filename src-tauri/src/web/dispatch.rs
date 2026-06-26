use std::path::PathBuf;
use std::str::FromStr;

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::{config, settings, AppError, AppType, ProviderService};

use super::error::WebError;
use super::state::WebState;

/// `POST /api/invoke/:command` — the bridge for the frontend's Tauri `invoke()`.
///
/// `command` is the snake_case command name; the JSON body is the args object
/// (camelCase keys) the TS code passes as the second `invoke()` argument. The
/// returned JSON value is what the TS promise resolves to.
pub async fn invoke_handler(
    State(state): State<WebState>,
    Path(command): Path<String>,
    Json(args): Json<Value>,
) -> Result<Json<Value>, WebError> {
    let result = dispatch(&state, &command, &args)?;
    Ok(Json(result))
}

/// Extract and parse an [`AppType`] from `args[key]` (e.g. `{ "app": "claude" }`).
fn app_arg(args: &Value, key: &str) -> Result<AppType, WebError> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| WebError::BadRequest(format!("missing '{key}' argument")))?;
    AppType::from_str(raw).map_err(WebError::Domain)
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, WebError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| WebError::BadRequest(format!("missing '{key}' argument")))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, WebError> {
    serde_json::to_value(value)
        .map_err(|e| WebError::Domain(AppError::Message(format!("serialization error: {e}"))))
}

/// The live config directory for an app, used by the directory-settings panel.
/// Exhaustive per-app so each app shows its real directory (e.g. ~/.codex).
fn config_dir_for(app: &AppType) -> PathBuf {
    match app {
        AppType::Claude => config::get_claude_config_dir(),
        AppType::Codex => crate::codex_config::get_codex_config_dir(),
        AppType::Gemini => crate::gemini_config::get_gemini_dir(),
        AppType::OpenCode => crate::opencode_config::get_opencode_dir(),
        AppType::Hermes => crate::hermes_config::get_hermes_dir(),
        AppType::OpenClaw => crate::openclaw_config::get_openclaw_dir(),
    }
}

/// The first-wave command table. Everything else returns 501 NotImplemented so
/// the app boots and the provider-list / switch / settings flows work
/// end-to-end while the remaining ~240 commands degrade gracefully.
fn dispatch(state: &WebState, command: &str, args: &Value) -> Result<Value, WebError> {
    let app = &state.app;
    match command {
        "get_providers" => {
            let app_type = app_arg(args, "app")?;
            to_value(ProviderService::list(app, app_type)?)
        }

        "get_current_provider" => {
            let app_type = app_arg(args, "app")?;
            Ok(Value::String(ProviderService::current(app, app_type)?))
        }

        "switch_provider" => {
            let app_type = app_arg(args, "app")?;
            let id = str_arg(args, "id")?.to_string();
            ProviderService::switch(app, app_type.clone(), &id)?;
            // Notify SSE listeners; the frontend listens for "provider-switched".
            let _ = state.events.send(
                json!({
                    "event": "provider-switched",
                    "payload": { "appType": app_type.as_str(), "providerId": id }
                })
                .to_string(),
            );
            // TS expects SwitchResult = { warnings: string[] }.
            Ok(json!({ "warnings": [] }))
        }

        "get_settings" => to_value(settings::get_settings()),

        "save_settings" => {
            let raw = args
                .get("settings")
                .ok_or_else(|| WebError::BadRequest("missing 'settings' argument".into()))?;
            let parsed = serde_json::from_value(raw.clone())
                .map_err(|e| WebError::BadRequest(format!("invalid settings payload: {e}")))?;
            settings::update_settings(parsed)?;
            Ok(Value::Bool(true))
        }

        "get_config_dir" => {
            let app_type = app_arg(args, "app")?;
            Ok(Value::String(
                config_dir_for(&app_type).to_string_lossy().into_owned(),
            ))
        }

        "get_common_config_snippet" => {
            let app_type = app_arg(args, "appType")?;
            let cfg = app.config.read().map_err(AppError::from)?;
            Ok(match cfg.common_config_snippets.get(&app_type) {
                Some(snippet) => Value::String(snippet.clone()),
                None => Value::Null,
            })
        }

        "import_default_config" => {
            let app_type = app_arg(args, "app")?;
            Ok(Value::Bool(ProviderService::import_default_config(
                app, app_type,
            )?))
        }

        "get_app_config_path" => Ok(Value::String(
            config::get_app_config_path().to_string_lossy().into_owned(),
        )),

        "is_portable_mode" => Ok(Value::Bool(false)),

        // Tray is a desktop-only concept; the switch flow calls this post-switch.
        "update_tray_menu" => Ok(Value::Bool(true)),

        "get_app_version" => Ok(Value::String(env!("CARGO_PKG_VERSION").to_string())),

        other => Err(WebError::NotImplemented(other.to_string())),
    }
}
