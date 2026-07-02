use crate::{provider::Provider, proxy::error::ProxyError};
use serde_json::{json, Value};

const MODELHUB_PROVIDER_TYPE: &str = "modelhub_codex";

pub(crate) fn overlay_provider_for_request(
    provider: &Provider,
    body: &Value,
) -> Result<Option<Provider>, ProxyError> {
    if !is_enabled(provider) {
        return Ok(None);
    }

    let model = request_model(body).unwrap_or("");
    let root = modelhub_root_url(provider)?;
    let is_gpt = model.to_ascii_lowercase().contains("gpt");

    let mut overlay = provider.clone();
    let mut meta = overlay.meta.unwrap_or_default();
    meta.is_full_url = Some(true);
    meta.api_format = Some(if is_gpt {
        "openai_responses".to_string()
    } else {
        "openai_chat".to_string()
    });
    overlay.meta = Some(meta);

    let mut settings = overlay
        .settings_config
        .as_object()
        .cloned()
        .unwrap_or_default();
    settings.insert(
        "base_url".to_string(),
        Value::String(if is_gpt {
            format!("{root}/responses")
        } else {
            format!("{root}/v2/crawl")
        }),
    );

    if !model.is_empty() {
        settings.insert("model".to_string(), Value::String(model.to_string()));
        settings.insert(
            "modelCatalog".to_string(),
            json!({
                "models": [
                    { "model": model }
                ]
            }),
        );
    }

    overlay.settings_config = Value::Object(settings);
    Ok(Some(overlay))
}

pub(crate) fn preserve_request_model(source: &Value, target: &mut Value) {
    if let Some(model) = request_model(source) {
        target["model"] = Value::String(model.to_string());
    }
}

fn is_enabled(provider: &Provider) -> bool {
    provider
        .meta
        .as_ref()
        .and_then(|meta| meta.provider_type.as_deref())
        .is_some_and(|provider_type| provider_type == MODELHUB_PROVIDER_TYPE)
}

fn modelhub_root_url(provider: &Provider) -> Result<String, ProxyError> {
    provider
        .meta
        .as_ref()
        .and_then(|meta| meta.modelhub_codex_root_url())
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| {
            ProxyError::ConfigError(
                "ModelHub Codex provider requires meta.modelhubCodex.rootUrl".to_string(),
            )
        })
}

fn request_model(body: &Value) -> Option<&str> {
    body.get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ModelHubCodexMeta, ProviderMeta};

    fn create_provider(settings_config: Value, provider_type: Option<&str>) -> Provider {
        Provider {
            id: "modelhub".to_string(),
            name: "ModelHub Codex".to_string(),
            settings_config,
            website_url: None,
            category: Some("codex".to_string()),
            created_at: None,
            sort_index: None,
            notes: None,
            meta: provider_type.map(|value| ProviderMeta {
                provider_type: Some(value.to_string()),
                ..Default::default()
            }),
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    fn create_modelhub_provider(settings_config: Value, root_url: &str) -> Provider {
        let mut provider = create_provider(settings_config, Some("modelhub_codex"));
        let meta = provider.meta.get_or_insert_with(ProviderMeta::default);
        meta.modelhub_codex = Some(ModelHubCodexMeta {
            root_url: Some(root_url.to_string()),
        });
        provider
    }

    #[test]
    fn modelhub_provider_type_enables_overlay() {
        let provider = create_provider(json!({}), Some("modelhub_codex"));

        assert!(is_enabled(&provider));
    }

    #[test]
    fn modelhub_preset_setting_does_not_enable_overlay() {
        let provider = create_provider(
            json!({
                "codexUpstreamPreset": "modelhub"
            }),
            None,
        );

        assert!(!is_enabled(&provider));
    }

    #[test]
    fn gpt_models_overlay_to_native_responses() {
        let provider = create_modelhub_provider(json!({}), "https://modelhub.example/root");

        let overlay = overlay_provider_for_request(&provider, &json!({ "model": "gpt-5.5-codex" }))
            .expect("valid ModelHub config")
            .expect("modelhub overlay");

        assert_eq!(
            overlay
                .settings_config
                .get("base_url")
                .and_then(Value::as_str),
            Some("https://modelhub.example/root/responses")
        );
        assert_eq!(
            overlay.meta.as_ref().and_then(|meta| meta.is_full_url),
            Some(true)
        );
        assert_eq!(
            overlay
                .meta
                .as_ref()
                .and_then(|meta| meta.api_format.as_deref()),
            Some("openai_responses")
        );
        assert_eq!(overlay.settings_config["model"], "gpt-5.5-codex");
        assert_eq!(
            overlay.settings_config["modelCatalog"]["models"][0]["model"],
            "gpt-5.5-codex"
        );
    }

    #[test]
    fn non_gpt_models_overlay_to_crawl_with_request_model_catalog() {
        let provider = create_modelhub_provider(json!({}), "https://modelhub.example/root");

        let overlay = overlay_provider_for_request(&provider, &json!({ "model": "glm-5.2" }))
            .expect("valid ModelHub config")
            .expect("modelhub overlay");

        assert_eq!(
            overlay
                .settings_config
                .get("base_url")
                .and_then(Value::as_str),
            Some("https://modelhub.example/root/v2/crawl")
        );
        assert_eq!(
            overlay
                .meta
                .as_ref()
                .and_then(|meta| meta.api_format.as_deref()),
            Some("openai_chat")
        );
        assert_eq!(
            overlay.settings_config["modelCatalog"]["models"][0]["model"],
            "glm-5.2"
        );
    }

    #[test]
    fn overlay_uses_custom_modelhub_root_url() {
        let provider = create_modelhub_provider(json!({}), "https://modelhub.example/root/");

        let overlay = overlay_provider_for_request(&provider, &json!({ "model": "kimi-k2" }))
            .expect("valid ModelHub config")
            .expect("modelhub overlay");

        assert_eq!(
            overlay
                .settings_config
                .get("base_url")
                .and_then(Value::as_str),
            Some("https://modelhub.example/root/v2/crawl")
        );
    }

    #[test]
    fn modelhub_root_url_is_required() {
        let provider = create_provider(json!({}), Some("modelhub_codex"));

        let error = overlay_provider_for_request(&provider, &json!({ "model": "gpt-5.5-codex" }))
            .expect_err("missing ModelHub root should fail");

        assert!(error.to_string().contains("meta.modelhubCodex.rootUrl"));
    }

    #[test]
    fn overlay_keeps_auth_settings_but_rebases_model_to_request() {
        let provider = create_modelhub_provider(
            json!({
                "auth": {
                    "OPENAI_API_KEY": "test-key"
                },
                "model": "configured-model"
            }),
            "https://modelhub.example/root",
        );

        let overlay = overlay_provider_for_request(&provider, &json!({ "model": "kimi-k2" }))
            .expect("valid ModelHub config")
            .expect("modelhub overlay");

        assert_eq!(
            overlay.settings_config["auth"]["OPENAI_API_KEY"],
            "test-key"
        );
        assert_eq!(overlay.settings_config["model"], "kimi-k2");
        assert_eq!(
            overlay.settings_config["modelCatalog"]["models"][0]["model"],
            "kimi-k2"
        );
    }

    #[test]
    fn preserve_request_model_restores_original_body_model() {
        let source = json!({ "model": "glm-5.2" });
        let mut target = json!({ "model": "configured-model" });

        preserve_request_model(&source, &mut target);

        assert_eq!(target["model"], "glm-5.2");
    }
}
