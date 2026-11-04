//! Declarative provider files for `~/.config/goose/custom_providers/`.

use serde_json::{Map, Value, json};

use crate::catalog::Declarative;

pub fn render(declarative: &Declarative, base_url: &str) -> String {
    let models: Vec<Value> = declarative
        .models
        .iter()
        .map(|model| {
            let mut entry = Map::new();
            entry.insert("name".into(), json!(model.name));
            entry.insert("context_limit".into(), json!(model.context_limit));
            Value::Object(entry)
        })
        .collect();

    let value = json!({
        "name": declarative.name,
        "engine": declarative.engine.as_str(),
        "display_name": declarative.display_name,
        "description": declarative.description,
        "api_key_env": declarative.api_key_env,
        "base_url": base_url,
        "models": models,
        "supports_streaming": true,
        "requires_auth": true,
    });

    let mut text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into());
    text.push('\n');
    text
}

pub fn file_name(declarative: &Declarative) -> String {
    format!("{}.json", declarative.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    #[test]
    fn openai_provider_uses_chat_completions_root() {
        let spec = catalog::find("rhai-maas").unwrap();
        let declarative = &spec.declaratives[0];
        let text = render(declarative, "https://maas.example.test/v1");
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["engine"], "openai");
        assert_eq!(value["base_url"], "https://maas.example.test/v1");
        assert_eq!(value["api_key_env"], "RHAI_MAAS_API_KEY");
        assert_eq!(value["requires_auth"], true);
    }

    #[test]
    fn anthropic_provider_keeps_bare_host() {
        let spec = catalog::find("enmaas").unwrap();
        let declarative = &spec.declaratives[0];
        let text = render(declarative, "https://enmaas.example.test");
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["engine"], "anthropic");
        // goose joins v1/messages to this root; a /v1 suffix would double it.
        assert_eq!(value["base_url"], "https://enmaas.example.test");
    }

    #[test]
    fn every_model_declares_a_context_limit() {
        // goose 1.45 refuses the whole custom_providers directory if any model
        // omits context_limit, so this must never regress.
        for spec in catalog::all() {
            for declarative in spec.declaratives {
                let value: Value =
                    serde_json::from_str(&render(declarative, "https://example.test")).unwrap();
                for model in value["models"].as_array().unwrap() {
                    assert!(
                        model.get("context_limit").and_then(Value::as_u64).is_some(),
                        "{} model {} has no context_limit",
                        declarative.name,
                        model["name"]
                    );
                }
            }
        }
    }

    #[test]
    fn gemini_provider_shares_the_enmaas_secret() {
        let spec = catalog::find("enmaas").unwrap();
        let declarative = &spec.declaratives[1];
        let value: Value =
            serde_json::from_str(&render(declarative, "https://enmaas.example.test/v1")).unwrap();
        assert_eq!(value["engine"], "openai");
        assert_eq!(value["api_key_env"], "ENMAAS_API_KEY");
        assert_eq!(value["base_url"], "https://enmaas.example.test/v1");
    }
}
