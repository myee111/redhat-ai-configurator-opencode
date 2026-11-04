//! The shared plan/apply core used by both the TUI and non-interactive mode.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::agent::Agent;
use crate::catalog::{self, ProviderSpec};
use crate::error::{Fail, Result, code};
use crate::fsutil;
use crate::opencode::auth;
use crate::opencode::config::{self, RenderInput};
use crate::opencode::{install, providers};

#[derive(Clone, Debug)]
pub struct Selection {
    pub provider_ids: Vec<&'static str>,
    /// provider id -> model name
    pub model_overrides: Vec<(String, String)>,
    pub agent: Agent,
    /// secret name -> key value
    pub keys: Vec<(String, String)>,
    pub keep_existing: bool,
    pub config_dir: PathBuf,
    /// OpenCode CLI used for its credential store and verification.
    pub opencode: PathBuf,
}

#[derive(Clone, Debug)]
pub struct FileOp {
    pub path: PathBuf,
    pub contents: String,
    pub mode: u32,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub ops: Vec<FileOp>,
    pub config_path: PathBuf,
    pub models: Vec<(String, String)>,
    pub model_refs: Vec<String>,
    pub agent: Agent,
    pub auth_updates: Vec<auth::Update>,
    pub auth_stored: BTreeMap<String, auth::Credential>,
    pub auth_changed: bool,
    pub warnings: Vec<String>,
    pub changed: Vec<PathBuf>,
    pub existing_keys: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct Backup {
    pub path: PathBuf,
    pub original: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub backups: Vec<Backup>,
    pub warnings: Vec<String>,
}

impl Commit {
    /// Put every backed-up file back where it came from. Accounts already
    /// imported into OpenCode's database stay there.
    pub fn restore(&self) {
        for backup in self.backups.iter().rev() {
            let _ = std::fs::copy(&backup.path, &backup.original);
        }
    }
}

fn model_for(sel: &Selection, spec: &ProviderSpec) -> (String, String) {
    if let Some((_, model)) = sel
        .model_overrides
        .iter()
        .find(|(provider, _)| provider == spec.id)
    {
        let provider = catalog::resolve_model(spec, model)
            .map(|(d, _)| d.name)
            .unwrap_or(spec.default_provider);
        return (provider.to_string(), model.clone());
    }
    (
        spec.default_provider.to_string(),
        spec.default_model.to_string(),
    )
}

pub fn build(sel: &Selection) -> Result<Plan> {
    if sel.provider_ids.is_empty() {
        return Err(Fail::new(code::USAGE, "Choose at least one provider."));
    }

    let mut selected: Vec<&'static ProviderSpec> = Vec::new();
    for id in &sel.provider_ids {
        let spec = catalog::find(id)
            .ok_or_else(|| Fail::new(code::USAGE, format!("Unknown provider: {id}")))?;
        if !selected.iter().any(|p| p.id == spec.id) {
            selected.push(spec);
        }
    }

    let mut warnings = Vec::new();
    let stored = auth::stored(&sel.opencode);
    let existing_keys: BTreeSet<String> = stored
        .iter()
        .filter(|(_, credential)| !credential.key.is_empty())
        .map(|(integration, _)| integration.clone())
        .collect();

    let mut secret_updates = Vec::new();
    let mut models = Vec::new();
    let mut model_refs = Vec::new();
    let mut entries = Vec::new();
    let mut written = BTreeSet::new();

    for spec in &selected {
        let (provider, model) = model_for(sel, spec);
        models.push((spec.id.to_string(), model.clone()));
        model_refs.push(catalog::model_ref(&provider, &model));

        for declarative in spec.declaratives {
            if written.insert(declarative.name) {
                entries.push(providers::entry(declarative));
            }
        }

        let integration = spec.default_provider;
        let provided = sel
            .keys
            .iter()
            .find(|(name, value)| name == spec.secret && !value.is_empty())
            .map(|(_, value)| value.clone());

        match provided {
            Some(value) => secret_updates.push(auth::Update {
                integration: integration.to_string(),
                key: value,
            }),
            None => {
                let already_stored = stored
                    .get(integration)
                    .is_some_and(|credential| !credential.key.is_empty());
                if !(sel.keep_existing && already_stored) {
                    let mut fail = Fail::new(
                        code::KEY,
                        format!("No API key supplied for {}.", spec.display),
                    )
                    .hint(format!("Create one at {}", spec.key_page()))
                    .hint(format!("This step needs: {}", spec.vpn.text()));
                    if let Some(url) = spec.eligibility_url() {
                        fail = fail.hint(format!("New to EnMaaS? Request access at {url}"));
                    }
                    return Err(fail);
                }
            }
        }
    }

    // EnMaaS wins when both are configured, so DeepSeek 4.1 Flash is the
    // default. Ties fall back to the order the user selected.
    let active_spec = selected
        .iter()
        .min_by_key(|spec| spec.priority)
        .copied()
        .expect("at least one provider");
    let (active_provider, active_model) = model_for(sel, active_spec);

    let config_path = config_path(&sel.config_dir);
    let existing_config = std::fs::read_to_string(&config_path).unwrap_or_default();
    let config_body = config::render(
        &existing_config,
        &RenderInput {
            active: (active_provider, active_model),
            entries,
            agent: sel.agent.as_str().to_string(),
        },
    )?;

    let ops = vec![FileOp {
        path: config_path.clone(),
        contents: config_body,
        mode: existing_mode(&config_path).unwrap_or(0o600),
    }];

    let changed: Vec<PathBuf> = ops
        .iter()
        .filter(|op| match std::fs::read_to_string(&op.path) {
            Ok(current) => current != op.contents,
            Err(_) => true,
        })
        .map(|op| op.path.clone())
        .collect();

    if sel.agent == Agent::Plan && selected.iter().all(|spec| spec.id != "enmaas") {
        warnings.push(
            "Plan is read-only; the agent will not edit files with this default.".to_string(),
        );
    }

    let auth_changed = !secret_updates.is_empty() && !auth::matches(&stored, &secret_updates);

    Ok(Plan {
        ops,
        config_path,
        models,
        model_refs,
        agent: sel.agent,
        auth_updates: secret_updates,
        auth_stored: stored,
        auth_changed,
        warnings,
        changed,
        existing_keys,
    })
}

pub fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("opencode.json")
}

fn existing_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o7777)
}

pub fn commit(plan: &Plan, opencode: &Path, rollback_on_failure: bool) -> Result<Commit> {
    let mut backups: Vec<Backup> = Vec::new();
    let warnings = plan.warnings.clone();

    let result = (|| -> Result<()> {
        for op in &plan.ops {
            let current = std::fs::read_to_string(&op.path).ok();
            if current.as_deref() == Some(op.contents.as_str()) {
                continue;
            }
            if current.is_some()
                && let Some(backup) = fsutil::backup(&op.path).map_err(|error| {
                    Fail::new(
                        code::WRITE,
                        format!("Could not back up {}: {error}", op.path.display()),
                    )
                })?
            {
                backups.push(Backup {
                    path: backup,
                    original: op.path.clone(),
                });
            }
            fsutil::write_atomic(&op.path, &op.contents, op.mode).map_err(|error| {
                Fail::new(
                    code::WRITE,
                    format!("Could not write {}: {error}", op.path.display()),
                )
            })?;
        }

        if plan.auth_changed {
            auth::import(opencode, &plan.auth_stored, &plan.auth_updates)?;
        }
        Ok(())
    })();

    if let Err(error) = result {
        if rollback_on_failure {
            for backup in backups.iter().rev() {
                let _ = std::fs::copy(&backup.path, &backup.original);
            }
        }
        return Err(error);
    }

    Ok(Commit { backups, warnings })
}

/// Make sure the CLI is present before the TUI or non-interactive run starts.
pub fn preflight(install_dir: Option<&Path>, allow_install: bool) -> Result<install::Outcome> {
    let outcome = install::ensure(install_dir, allow_install)?;
    if outcome.installed_now {
        eprintln!(
            "Installed OpenCode {} at {}",
            outcome.info.version_string(),
            outcome.info.path.display()
        );
    }
    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(dir: &Path) -> Selection {
        Selection {
            provider_ids: vec!["rhai-maas", "enmaas"],
            model_overrides: Vec::new(),
            agent: Agent::Build,
            keys: vec![
                ("RHAI_MAAS_API_KEY".into(), "one".into()),
                ("ENMAAS_API_KEY".into(), "two".into()),
            ],
            keep_existing: false,
            config_dir: dir.to_path_buf(),
            opencode: PathBuf::from("/bin/true"),
        }
    }

    #[test]
    fn enmaas_is_the_active_provider_when_both_are_selected() {
        let dir = tempfile::tempdir().unwrap();
        let plan = build(&selection(dir.path())).unwrap();
        let model = plan
            .ops
            .iter()
            .find(|op| op.path.ends_with("opencode.json"))
            .map(|op| op.contents.clone())
            .unwrap();
        assert!(model.contains("\"model\": \"enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017\""));
        assert!(model.contains("\"default_agent\": \"build\""));
    }

    #[test]
    fn missing_key_is_a_key_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut sel = selection(dir.path());
        sel.keys.clear();
        let error = build(&sel).unwrap_err();
        assert_eq!(error.code, code::KEY);
    }

    #[test]
    fn key_updates_replace_the_credential_per_integration() {
        let dir = tempfile::tempdir().unwrap();
        let plan = build(&selection(dir.path())).unwrap();
        let integrations: Vec<&str> = plan
            .auth_updates
            .iter()
            .map(|update| update.integration.as_str())
            .collect();
        assert_eq!(integrations, vec!["ai-bu", "enmaas"]);
        assert!(plan.auth_changed);
    }
}
