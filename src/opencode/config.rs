//! Merging the installer's settings into `opencode.json`.
//!
//! OpenCode allows JSONC, so the file is parsed with comments and trailing
//! commas accepted. The installer owns its provider entries, the default model
//! and the default agent; every other key, provider and model is preserved.

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde_json::{Map, Value, json};

use crate::error::{Fail, Result, code};

pub const SCHEMA_URL: &str = "https://opencode.ai/config.json";

pub struct RenderInput {
    /// `(provider id, model name)` written to the top-level `model`.
    pub active: (String, String),
    /// Provider entries keyed by the provider ID OpenCode uses.
    pub entries: Vec<(String, Value)>,
    /// Primary agent OpenCode starts with.
    pub agent: String,
}

pub fn parse(existing: &str) -> Result<Map<String, Value>> {
    if existing.trim().is_empty() {
        return Ok(Map::new());
    }
    let options = ParseOptions::default();
    match parse_to_serde_value(existing, &options) {
        Ok(Some(Value::Object(map))) => Ok(map),
        Ok(Some(_)) => Err(
            Fail::new(code::WRITE, "opencode.json does not contain a JSON object.")
                .hint("Move the file aside and run the configurator again."),
        ),
        Ok(None) => Ok(Map::new()),
        Err(error) => Err(Fail::new(
            code::WRITE,
            format!("opencode.json is not valid JSON: {error}"),
        )
        .hint("Fix or move the file aside, then run the configurator again.")),
    }
}

pub fn render(existing: &str, input: &RenderInput) -> Result<String> {
    let mut root = parse(existing)?;

    root.entry("$schema").or_insert_with(|| json!(SCHEMA_URL));

    let mut providers = match root.remove("providers") {
        Some(Value::Object(map)) => map,
        Some(other) => {
            return Err(Fail::new(
                code::WRITE,
                format!(
                    "`providers` in opencode.json is {}, not an object.",
                    kind(&other)
                ),
            )
            .hint("Fix or move the file aside, then run the configurator again."));
        }
        None => Map::new(),
    };
    for (id, entry) in &input.entries {
        providers.insert(id.clone(), entry.clone());
    }
    root.insert("providers".into(), Value::Object(providers));

    root.insert(
        "model".into(),
        json!(format!("{}/{}", input.active.0, input.active.1)),
    );
    root.insert("default_agent".into(), json!(input.agent));

    let mut text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not serialise opencode.json: {error}"),
        )
    })?;
    text.push('\n');
    Ok(text)
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    fn entries() -> Vec<(String, Value)> {
        let spec = catalog::find("enmaas").unwrap();
        spec.declaratives
            .iter()
            .map(crate::opencode::providers::entry)
            .collect()
    }

    fn input() -> RenderInput {
        RenderInput {
            active: (
                "enmaas".into(),
                "rits/deepseek-ai/DeepSeek-V4.1-Flash-1017".into(),
            ),
            entries: entries(),
            agent: "build".into(),
        }
    }

    #[test]
    fn writes_providers_model_and_agent() {
        let out = render("", &input()).unwrap();
        let value: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["$schema"], SCHEMA_URL);
        assert_eq!(
            value["model"],
            "enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017"
        );
        assert_eq!(value["default_agent"], "build");
        assert_eq!(
            value["providers"]["enmaas"]["settings"]["baseURL"],
            "https://enmaas.example.com/v1"
        );
        assert!(value["providers"]["enmaas"]["models"]["claude-haiku-4-5"].is_object());
    }

    #[test]
    fn preserves_unrelated_settings_in_a_commented_document() {
        let existing = r#"{
  // keep this comment
  "theme": "keep-me",
  "providers": {
    "other": { "name": "Keep me", "models": { "m": {} } },
    "enmaas": { "name": "Old EnMaaS" }
  }
}"#;
        let out = render(existing, &input()).unwrap();
        let value: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["theme"], "keep-me");
        assert_eq!(value["providers"]["other"]["name"], "Keep me");
        assert_eq!(value["providers"]["enmaas"]["name"], "EnMaaS");
    }

    #[test]
    fn rejects_a_non_object_document() {
        let error = render("[1, 2]", &input()).unwrap_err();
        assert_eq!(error.code, code::WRITE);
    }

    #[test]
    fn rejects_a_non_object_providers_value() {
        let error = render(r#"{"providers": []}"#, &input()).unwrap_err();
        assert_eq!(error.code, code::WRITE);
        assert!(error.message.contains("not an object"));
    }

    #[test]
    fn output_is_stable_for_the_same_input() {
        let first = render("", &input()).unwrap();
        let second = render(&first, &input()).unwrap();
        assert_eq!(first, second);
    }
}
