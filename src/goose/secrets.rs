//! Secret storage: `secrets.yaml` (mode 0600) or goose's Secret Service
//! entry through `secret-tool`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::error::{Fail, Result, code};
use crate::fsutil;

const KEYRING_SERVICE: &str = "goose";
const KEYRING_USERNAME: &str = "secrets";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SecretsMode {
    File,
    Keyring,
}

impl SecretsMode {
    pub fn label(self) -> &'static str {
        match self {
            SecretsMode::File => "secrets.yaml (0600)",
            SecretsMode::Keyring => "system keyring",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Update {
    pub name: String,
    pub value: String,
}

pub fn file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("secrets.yaml")
}

/// Read the names and values from secrets.yaml. Only string values are used.
pub fn read_file(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(map) =
        serde_yaml_ng::from_str::<BTreeMap<String, serde_yaml_ng::Value>>(&text)
    else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter_map(|(key, value)| match value {
            serde_yaml_ng::Value::String(text) => Some((key, text)),
            _ => None,
        })
        .collect()
}

pub fn apply_file(path: &Path, updates: &[Update]) -> Result<()> {
    let mut values: serde_yaml_ng::Mapping = match fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => match serde_yaml_ng::from_str(&text) {
            Ok(serde_yaml_ng::Value::Mapping(map)) => map,
            _ => {
                return Err(Fail::new(
                    code::WRITE,
                    format!("{} is not a YAML mapping.", path.display()),
                )
                .hint("Move the file aside so the configurator can recreate it."));
            }
        },
        _ => serde_yaml_ng::Mapping::new(),
    };

    for update in updates {
        values.insert(
            serde_yaml_ng::Value::String(update.name.clone()),
            serde_yaml_ng::Value::String(update.value.clone()),
        );
    }

    let body = serde_yaml_ng::to_string(&serde_yaml_ng::Value::Mapping(values))
        .map_err(|error| Fail::new(code::WRITE, format!("Could not write secrets: {error}")))?;

    fsutil::ensure_private_dir(path.parent().unwrap_or_else(|| Path::new("."))).map_err(|e| {
        Fail::new(
            code::WRITE,
            format!("Could not create {}: {e}", path.display()),
        )
    })?;
    fsutil::write_atomic(path, &body, 0o600).map_err(|e| {
        Fail::new(
            code::WRITE,
            format!("Could not write {}: {e}", path.display()),
        )
    })
}

fn secret_tool() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("secret-tool"))
            .find(|candidate| candidate.is_file())
    })
}

/// goose stores all secrets as one JSON object in the keyring.
fn keyring_read_json(tool: &Path) -> std::result::Result<serde_json::Map<String, Value>, String> {
    let output = Command::new(tool)
        .args([
            "lookup",
            "service",
            KEYRING_SERVICE,
            "username",
            KEYRING_USERNAME,
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Ok(serde_json::Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Ok(serde_json::Map::new()),
    }
}

pub fn keyring_available() -> bool {
    let Some(tool) = secret_tool() else {
        return false;
    };
    keyring_read_json(&tool).is_ok()
}

pub fn keyring_names() -> BTreeSet<String> {
    let Some(tool) = secret_tool() else {
        return BTreeSet::new();
    };
    keyring_read_json(&tool)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

pub fn apply_keyring(updates: &[Update]) -> Result<()> {
    let tool = secret_tool().ok_or_else(|| {
        Fail::new(
            code::WRITE,
            "secret-tool is not installed, so the keyring is unavailable.",
        )
        .hint("Install libsecret-tools, or store keys in secrets.yaml instead.")
    })?;

    let mut values = keyring_read_json(&tool).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not read the system keyring: {error}"),
        )
    })?;
    for update in updates {
        values.insert(update.name.clone(), Value::String(update.value.clone()));
    }
    let payload = serde_json::to_string(&Value::Object(values)).map_err(|error| {
        Fail::new(code::WRITE, format!("Could not encode secrets: {error}"))
    })?;

    let mut child = Command::new(&tool)
        .args([
            "store",
            "--label=goose",
            "service",
            KEYRING_SERVICE,
            "username",
            KEYRING_USERNAME,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Fail::new(code::WRITE, format!("Could not run secret-tool: {error}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload.as_bytes())
            .map_err(|error| Fail::new(code::WRITE, format!("Could not write to keyring: {error}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| Fail::new(code::WRITE, format!("secret-tool failed: {error}")))?;
    if !output.status.success() {
        return Err(Fail::new(
            code::WRITE,
            format!(
                "The system keyring rejected the key: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(())
}

/// Names that already hold a value, from the file and from the keyring.
pub fn existing_names(config_dir: &Path) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = read_file(&file_path(config_dir)).into_keys().collect();
    names.extend(keyring_names());
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_private_secrets_and_merges() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_path(dir.path());
        apply_file(
            &path,
            &[Update {
                name: "RHAI_MAAS_API_KEY".into(),
                value: "one".into(),
            }],
        )
        .unwrap();
        apply_file(
            &path,
            &[Update {
                name: "ENMAAS_API_KEY".into(),
                value: "two".into(),
            }],
        )
        .unwrap();

        let values = read_file(&path);
        assert_eq!(values.get("RHAI_MAAS_API_KEY").map(String::as_str), Some("one"));
        assert_eq!(values.get("ENMAAS_API_KEY").map(String::as_str), Some("two"));

        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
