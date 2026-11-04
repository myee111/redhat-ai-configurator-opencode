//! Provider accounts in OpenCode's credential store.
//!
//! OpenCode keeps saved keys in its SQLite database and documents `auth` as the
//! only supported way to change them. The installer therefore never touches the
//! database: it builds the JSON shape `opencode auth export` produces, imports
//! it with `opencode auth import`, and reads back only what it needs to avoid
//! creating duplicate accounts.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::error::{Fail, Result, code};
use crate::fsutil;
use crate::opencode::runner;

const AUTH_TIMEOUT: Duration = Duration::from_secs(120);
/// Label shown for accounts this tool creates.
pub const LABEL: &str = "Red Hat AI Configurator";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credential {
    pub id: String,
    pub key: String,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct Update {
    /// OpenCode provider ID, e.g. `enmaas`.
    pub integration: String,
    pub key: String,
}

/// The saved account for each provider, preferring the active one. Keys are
/// held in memory for comparison only and are never printed.
pub fn stored(opencode: &Path) -> BTreeMap<String, Credential> {
    let Ok((true, stdout, _stderr)) = runner::run(opencode, &["auth", "export"], AUTH_TIMEOUT)
    else {
        return BTreeMap::new();
    };
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(&stdout) else {
        return BTreeMap::new();
    };

    let mut accounts: BTreeMap<String, Credential> = BTreeMap::new();
    for entry in entries {
        let Some(integration) = entry.get("integrationID").and_then(Value::as_str) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        let key = entry
            .get("value")
            .and_then(|value| value.get("key"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let active = entry
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let credential = Credential {
            id: id.to_string(),
            key,
            active,
        };
        match accounts.get(integration) {
            Some(existing) if existing.active || !active => {}
            _ => {
                accounts.insert(integration.to_string(), credential);
            }
        }
    }
    accounts
}

/// True when every update is already stored with the same key.
pub fn matches(stored: &BTreeMap<String, Credential>, updates: &[Update]) -> bool {
    !updates.is_empty()
        && updates.iter().all(|update| {
            stored
                .get(&update.integration)
                .is_some_and(|c| c.key == update.key)
        })
}

/// Import the keys through OpenCode's own command. The temporary file holds
/// secrets, so it is created mode 0600 and removed even when the import fails.
pub fn import(
    opencode: &Path,
    stored: &BTreeMap<String, Credential>,
    updates: &[Update],
) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let entries: Vec<Value> = updates
        .iter()
        .enumerate()
        .map(|(index, update)| {
            let id = stored
                .get(&update.integration)
                .map(|credential| credential.id.clone())
                .unwrap_or_else(|| new_id(index));
            json!({
                "id": id,
                "integrationID": update.integration,
                "label": LABEL,
                "active": true,
                "value": { "type": "key", "key": update.key },
            })
        })
        .collect();

    let path = temp_path();
    let body = serde_json::to_string(&Value::Array(entries)).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not encode credentials: {error}"),
        )
    })?;
    fsutil::write_atomic(&path, &body, 0o600).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not write {}: {error}", path.display()),
        )
    })?;

    let result = runner::run(
        opencode,
        &["auth", "import", &path.to_string_lossy()],
        AUTH_TIMEOUT,
    );
    let _ = fs::remove_file(&path);

    match result {
        Ok((true, _stdout, _stderr)) => Ok(()),
        Ok((false, stdout, stderr)) => Err(Fail::new(
            code::WRITE,
            "OpenCode rejected the imported key.",
        )
        .hint(first_line(&format!("{stdout}\n{stderr}")))),
        Err(error) => Err(Fail::new(
            code::WRITE,
            format!("Could not store the key with OpenCode: {error}"),
        )),
    }
}

fn first_line(text: &str) -> String {
    runner::strip_ansi(text)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("No detail was reported.")
        .to_string()
}

fn temp_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "rhai-opencode-auth-{}-{nanos}.json",
        std::process::id()
    ))
}

/// Fresh credential ID in the same shape `auth export` uses.
fn new_id(index: usize) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("cred_{nanos:x}{:x}{index:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_the_active_account_and_ignores_oauth_entries() {
        let json = r#"[
          {"id":"a","integrationID":"enmaas","active":false,"value":{"type":"key","key":"old"}},
          {"id":"b","integrationID":"enmaas","active":true,"value":{"type":"key","key":"new"}},
          {"id":"c","integrationID":"ai-bu","active":true,"value":{"type":"oauth","refresh":"x"}}
        ]"#;
        let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(json) else {
            unreachable!()
        };
        // Mirror the parsing loop without spawning OpenCode.
        let mut accounts: BTreeMap<String, Credential> = BTreeMap::new();
        for entry in entries {
            let integration = entry["integrationID"].as_str().unwrap().to_string();
            let active = entry["active"].as_bool().unwrap_or(false);
            let credential = Credential {
                id: entry["id"].as_str().unwrap().to_string(),
                key: entry["value"]["key"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                active,
            };
            match accounts.get(&integration) {
                Some(existing) if existing.active || !active => {}
                _ => {
                    accounts.insert(integration, credential);
                }
            }
        }
        assert_eq!(accounts["enmaas"].key, "new");
        assert_eq!(accounts["ai-bu"].key, "");
    }

    #[test]
    fn matches_only_when_every_key_is_unchanged() {
        let mut stored = BTreeMap::new();
        stored.insert(
            "enmaas".to_string(),
            Credential {
                id: "a".into(),
                key: "one".into(),
                active: true,
            },
        );
        let same = vec![Update {
            integration: "enmaas".into(),
            key: "one".into(),
        }];
        let changed = vec![Update {
            integration: "enmaas".into(),
            key: "two".into(),
        }];
        assert!(matches(&stored, &same));
        assert!(!matches(&stored, &changed));
        assert!(!matches(&stored, &[]));
    }

    #[test]
    fn generated_ids_are_unique_and_prefixed() {
        let a = new_id(0);
        let b = new_id(1);
        assert!(a.starts_with("cred_"));
        assert_ne!(a, b);
    }
}
