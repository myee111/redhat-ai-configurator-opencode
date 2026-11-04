//! OpenCode provider entries for `providers.<id>` in `opencode.json`.

use serde_json::{Map, Value, json};

use crate::catalog::{self, Declarative};

/// One `providers.<id>` object: display name, runtime package, credential
/// variable, endpoint, and the model inventory.
pub fn entry(declarative: &Declarative) -> (String, Value) {
    let mut models = Map::new();
    for model in declarative.models {
        models.insert(
            model.name.into(),
            json!({
                "name": model.display,
                "limit": {
                    "context": model.context_limit,
                    "output": catalog::OUTPUT_LIMIT,
                },
            }),
        );
    }

    let value = json!({
        "name": declarative.display_name,
        "package": declarative.engine.package(),
        "env": [declarative.api_key_env],
        "settings": { "baseURL": declarative.base_url() },
        "models": models,
    });

    (declarative.name.to_string(), value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_provider_uses_the_compatible_package() {
        let spec = catalog::find("rhai-maas").unwrap();
        let (id, value) = entry(&spec.declaratives[0]);
        assert_eq!(id, "ai-bu");
        assert_eq!(value["package"], "@opencode/ai/providers/openai-compatible");
        assert_eq!(value["env"][0], "RHAI_MAAS_API_KEY");
        assert_eq!(value["settings"]["baseURL"], "https://maas.example.com/v1");
    }

    #[test]
    fn anthropic_provider_keeps_v1_in_the_base_url() {
        let spec = catalog::find("enmaas").unwrap();
        let (id, value) = entry(&spec.declaratives[0]);
        assert_eq!(id, "enmaas");
        assert_eq!(value["package"], "@opencode/ai/providers/anthropic");
        // The runtime joins /messages to this base.
        assert_eq!(
            value["settings"]["baseURL"],
            "https://enmaas.example.com/v1"
        );
    }

    #[test]
    fn every_model_declares_limits() {
        for spec in catalog::all() {
            for declarative in spec.declaratives {
                let (_, value) = entry(declarative);
                for (name, model) in value["models"].as_object().unwrap() {
                    assert!(
                        model["limit"]["context"].as_u64().is_some(),
                        "{} model {name} has no context limit",
                        declarative.name
                    );
                    assert!(model["limit"]["output"].as_u64().is_some());
                    assert!(model["name"].as_str().is_some_and(|n| !n.is_empty()));
                }
            }
        }
    }

    #[test]
    fn gemini_provider_uses_the_openai_shape_and_enmaas_key() {
        let spec = catalog::find("enmaas").unwrap();
        let (id, value) = entry(&spec.declaratives[1]);
        assert_eq!(id, "enmaas-oai");
        assert_eq!(value["package"], "@opencode/ai/providers/openai-compatible");
        assert_eq!(value["env"][0], "ENMAAS_API_KEY");
        assert_eq!(
            value["settings"]["baseURL"],
            "https://enmaas.example.com/v1"
        );
    }
}
