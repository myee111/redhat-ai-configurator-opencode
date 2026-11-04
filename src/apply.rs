//! The shared plan/apply core used by both the TUI and non-interactive mode.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::catalog::{self, ProviderSpec};
use crate::error::{Fail, Result, code};
use crate::fsutil;
use crate::goose::config::{self, GooseMode, ProviderEntry, RenderInput};
use crate::goose::{providers, secrets};
use crate::platform;

#[derive(Clone, Debug)]
pub struct Selection {
    pub provider_ids: Vec<&'static str>,
    /// provider id -> model name
    pub model_overrides: Vec<(String, String)>,
    pub mode: GooseMode,
    pub secrets_mode: secrets::SecretsMode,
    /// secret name -> key value
    pub keys: Vec<(String, String)>,
    pub keep_existing: bool,
    pub config_dir: PathBuf,
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
    pub secrets_path: PathBuf,
    pub models: Vec<(String, String)>,
    pub secrets_mode: secrets::SecretsMode,
    pub secret_updates: Vec<secrets::Update>,
    pub warnings: Vec<String>,
    pub changed: Vec<PathBuf>,
    pub secrets_changed: bool,
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
    /// Put every backed-up file back where it came from.
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
        let declarative = catalog::resolve_model(spec, model)
            .map(|(d, _)| d.name)
            .unwrap_or(spec.default_provider);
        return (declarative.to_string(), model.clone());
    }
    (spec.default_provider.to_string(), spec.default_model.to_string())
}

pub fn build(sel: &Selection) -> Result<Plan> {
    if sel.provider_ids.is_empty() {
        return Err(Fail::new(code::USAGE, "Choose at least one provider."));
    }

    let mut providers: Vec<&'static ProviderSpec> = Vec::new();
    for id in &sel.provider_ids {
        let spec = catalog::find(id)
            .ok_or_else(|| Fail::new(code::USAGE, format!("Unknown provider: {id}")))?;
        if !providers.iter().any(|p| p.id == spec.id) {
            providers.push(spec);
        }
    }

    let mut warnings = Vec::new();
    let mut secrets_mode = sel.secrets_mode;
    if secrets_mode == secrets::SecretsMode::Keyring && !secrets::keyring_available() {
        warnings.push(
            "The system keyring is not reachable, so keys will be stored in secrets.yaml (0600)."
                .to_string(),
        );
        secrets_mode = secrets::SecretsMode::File;
    }

    let existing_keys = secrets::existing_names(&sel.config_dir);

    let mut secret_updates = Vec::new();
    let mut models = Vec::new();
    let mut configured: Vec<ProviderEntry> = Vec::new();
    let mut active: Option<ProviderEntry> = None;

    for spec in &providers {
        let (declarative, model) = model_for(sel, spec);
        models.push((spec.id.to_string(), model.clone()));
        if !configured.iter().any(|entry| entry.name == declarative) {
            configured.push(ProviderEntry {
                name: declarative.clone(),
                model: model.clone(),
            });
        }
        if active.is_none() {
            active = Some(ProviderEntry {
                name: declarative.clone(),
                model: model.clone(),
            });
        }

        let provided = sel
            .keys
            .iter()
            .find(|(name, value)| name == spec.secret && !value.is_empty())
            .map(|(_, value)| value.clone());

        match provided {
            Some(value) => secret_updates.push(secrets::Update {
                name: spec.secret.to_string(),
                value,
            }),
            None => {
                if !(sel.keep_existing && existing_keys.contains(spec.secret)) {
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

    let active = active.expect("at least one provider");

    let config_path = sel.config_dir.join("config.yaml");
    let existing_config = std::fs::read_to_string(&config_path).unwrap_or_default();
    let config_body = config::render(
        &existing_config,
        &RenderInput {
            active: active.clone(),
            configured: configured.clone(),
            mode: sel.mode,
            disable_keyring: Some(secrets_mode == secrets::SecretsMode::File),
        },
    )?;

    let mut ops = Vec::new();
    let mut written = BTreeSet::new();
    for spec in &providers {
        for declarative in spec.declaratives {
            if !written.insert(declarative.name) {
                continue;
            }
            ops.push(FileOp {
                path: sel
                    .config_dir
                    .join("custom_providers")
                    .join(providers::file_name(declarative)),
                contents: providers::render(declarative, &declarative.base_url()),
                mode: 0o644,
            });
        }
    }

    ops.push(FileOp {
        path: config_path.clone(),
        contents: config_body,
        mode: existing_mode(&config_path).unwrap_or(0o644),
    });

    let mut changed = Vec::new();
    for op in &ops {
        match std::fs::read_to_string(&op.path) {
            Ok(current) if current == op.contents => {}
            _ => changed.push(op.path.clone()),
        }
    }

    let secrets_path = secrets::file_path(&sel.config_dir);
    let mut secrets_changed = false;
    if !secret_updates.is_empty() {
        let current = secrets::read_file(&secrets_path);
        secrets_changed = secret_updates
            .iter()
            .any(|update| current.get(&update.name) != Some(&update.value));
        if secrets_changed && secrets_mode == secrets::SecretsMode::File {
            changed.push(secrets_path.clone());
        }
    }

    Ok(Plan {
        ops,
        config_path,
        secrets_path,
        models,
        secrets_mode,
        secret_updates,
        warnings,
        changed,
        secrets_changed,
        existing_keys,
    })
}

fn existing_mode(path: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o7777)
}

pub fn commit(plan: &Plan, rollback_on_failure: bool) -> Result<Commit> {
    let mut backups: Vec<Backup> = Vec::new();
    let mut warnings = plan.warnings.clone();

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

        if !plan.secret_updates.is_empty() && plan.secrets_changed {
            match plan.secrets_mode {
                secrets::SecretsMode::File => {
                    if plan.secrets_path.exists()
                        && let Some(backup) = fsutil::backup(&plan.secrets_path).map_err(|error| {
                            Fail::new(
                                code::WRITE,
                                format!("Could not back up secrets.yaml: {error}"),
                            )
                        })?
                    {
                        backups.push(Backup {
                            path: backup,
                            original: plan.secrets_path.clone(),
                        });
                    }
                    secrets::apply_file(&plan.secrets_path, &plan.secret_updates)?;
                }
                secrets::SecretsMode::Keyring => {
                    if let Err(error) = secrets::apply_keyring(&plan.secret_updates) {
                        warnings.push(format!(
                            "Could not store the key in the keyring ({error}); falling back to secrets.yaml."
                        ));
                        secrets::apply_file(&plan.secrets_path, &plan.secret_updates)?;
                    }
                }
            }
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

    Ok(Commit {
        backups,
        warnings,
    })
}

pub fn preflight_goose() -> Result<platform::GooseInfo> {
    let info = platform::goose_info().ok_or_else(|| {
        Fail::new(code::GOOSE, "goose is not installed.")
            .hint("sudo subscription-manager repos --enable=rhel-10-for-$(arch)-extensions-rpms")
            .hint("sudo dnf install goose")
    })?;
    if !info.meets_min() {
        return Err(Fail::new(
            code::GOOSE,
            format!(
                "goose {} is older than the required {}.",
                info.version_string(),
                version_text(platform::MIN_GOOSE)
            ),
        )
        .hint("sudo dnf upgrade goose"));
    }
    Ok(info)
}

fn version_text((a, b, c): (u32, u32, u32)) -> String {
    format!("{a}.{b}.{c}")
}
