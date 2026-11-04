//! Merging the installer's settings into `config.yaml`.
//!
//! goose 1.38 reads the flat `GOOSE_PROVIDER` / `GOOSE_MODEL` keys; newer
//! releases prefer `active_provider` plus a `providers:` block. Both are
//! written so one file works across the RHEL package and current upstream.
//! Unknown keys and extensions are preserved.

use serde_yaml_ng::{Mapping, Value};

use crate::error::{Fail, Result, code};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GooseMode {
    Auto,
    Approve,
    SmartApprove,
    Chat,
}

impl GooseMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GooseMode::Auto => "auto",
            GooseMode::Approve => "approve",
            GooseMode::SmartApprove => "smart_approve",
            GooseMode::Chat => "chat",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GooseMode::Auto => "Auto - runs tools without asking",
            GooseMode::Approve => "Approve - asks before every tool call",
            GooseMode::SmartApprove => "Smart approve - asks before writes and extensions",
            GooseMode::Chat => "Chat - no tools, conversation only",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "auto" => Some(GooseMode::Auto),
            "approve" | "manual" => Some(GooseMode::Approve),
            "smart_approve" | "smart" => Some(GooseMode::SmartApprove),
            "chat" => Some(GooseMode::Chat),
            _ => None,
        }
    }

    pub const ALL: [GooseMode; 4] = [
        GooseMode::Approve,
        GooseMode::SmartApprove,
        GooseMode::Auto,
        GooseMode::Chat,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEntry {
    pub name: String,
    pub model: String,
}

#[derive(Clone, Debug)]
pub struct RenderInput {
    pub active: ProviderEntry,
    pub configured: Vec<ProviderEntry>,
    pub mode: GooseMode,
    /// `Some(true)` forces file-based secrets, `Some(false)` clears the flag
    /// so goose uses the keyring, `None` leaves it alone.
    pub disable_keyring: Option<bool>,
}

pub fn render(existing: &str, input: &RenderInput) -> Result<String> {
    let mut root = if existing.trim().is_empty() {
        Mapping::new()
    } else {
        match serde_yaml_ng::from_str::<Value>(existing) {
            Ok(Value::Mapping(map)) => map,
            Ok(_) => {
                return Err(Fail::new(
                    code::WRITE,
                    "config.yaml does not contain a YAML mapping.",
                )
                .hint("Move the file aside and run the configurator again."));
            }
            Err(error) => {
                return Err(Fail::new(
                    code::WRITE,
                    format!("config.yaml is not valid YAML: {error}"),
                )
                .hint("Fix or move the file aside, then run the configurator again."));
            }
        }
    };

    root.insert(
        Value::String("GOOSE_PROVIDER".into()),
        Value::String(input.active.name.clone()),
    );
    root.insert(
        Value::String("GOOSE_MODEL".into()),
        Value::String(input.active.model.clone()),
    );
    root.insert(
        Value::String("GOOSE_MODE".into()),
        Value::String(input.mode.as_str().into()),
    );

    match input.disable_keyring {
        Some(true) => {
            root.insert(
                Value::String("GOOSE_DISABLE_KEYRING".into()),
                Value::Bool(true),
            );
        }
        Some(false) => {
            root.remove("GOOSE_DISABLE_KEYRING");
        }
        None => {}
    }

    // Structured block for newer goose releases.
    root.insert(
        Value::String("active_provider".into()),
        Value::String(input.active.name.clone()),
    );
    let mut providers = match root.get("providers") {
        Some(Value::Mapping(map)) => map.clone(),
        _ => Mapping::new(),
    };
    for entry in &input.configured {
        let mut block = Mapping::new();
        block.insert(Value::String("enabled".into()), Value::Bool(true));
        block.insert(Value::String("configured".into()), Value::Bool(true));
        block.insert(
            Value::String("model".into()),
            Value::String(entry.model.clone()),
        );
        providers.insert(Value::String(entry.name.clone()), Value::Mapping(block));
    }
    root.insert(
        Value::String("providers".into()),
        Value::Mapping(providers),
    );

    serde_yaml_ng::to_string(&Value::Mapping(root)).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not serialise config.yaml: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> RenderInput {
        RenderInput {
            active: ProviderEntry {
                name: "rhai-maas".into(),
                model: "publishers/prelude-maas/models/glm-53-flash".into(),
            },
            configured: vec![ProviderEntry {
                name: "rhai-maas".into(),
                model: "publishers/prelude-maas/models/glm-53-flash".into(),
            }],
            mode: GooseMode::Approve,
            disable_keyring: Some(true),
        }
    }

    #[test]
    fn writes_flat_and_structured_keys() {
        let out = render("", &input()).unwrap();
        assert!(out.contains("GOOSE_PROVIDER: rhai-maas"));
        assert!(out.contains("GOOSE_MODEL: publishers/prelude-maas/models/glm-53-flash"));
        assert!(out.contains("GOOSE_MODE: approve"));
        assert!(out.contains("GOOSE_DISABLE_KEYRING: true"));
        assert!(out.contains("active_provider: rhai-maas"));
        assert!(out.contains("providers:"));
    }

    #[test]
    fn preserves_unrelated_settings() {
        let existing = "GOOSE_TEMPERATURE: 0.4\nextensions:\n  developer:\n    enabled: true\n    type: builtin\n    name: developer\n";
        let out = render(existing, &input()).unwrap();
        assert!(out.contains("GOOSE_TEMPERATURE: 0.4"));
        assert!(out.contains("developer:"));
        assert!(out.contains("type: builtin"));
    }

    #[test]
    fn keyring_mode_clears_the_flag() {
        let mut with_keyring = input();
        with_keyring.disable_keyring = Some(false);
        let out = render("GOOSE_DISABLE_KEYRING: true\n", &with_keyring).unwrap();
        assert!(!out.contains("GOOSE_DISABLE_KEYRING"));
    }

    #[test]
    fn rejects_non_mapping() {
        let err = render("- a\n- b\n", &input()).unwrap_err();
        assert_eq!(err.code, code::WRITE);
    }
}
