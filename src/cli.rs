//! Command-line surface and the three run modes: TUI, non-interactive and
//! doctor.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};
use serde_json::{Value, json};

use crate::agent::Agent;
use crate::apply::{self, Selection};
use crate::catalog::{self, ProviderSpec};
use crate::error::{Fail, Result, code};
use crate::keyfile;
use crate::netcheck;
use crate::opencode::auth;
use crate::opencode::runner;
use crate::output::{ProviderReport, Report};
use crate::platform;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Format {
    Text,
    Json,
}

#[derive(Parser, Debug)]
#[command(
    name = "redhat-ai-configurator-opencode",
    version,
    about = "Install and configure OpenCode for Red Hat AI MaaS (rhai-maas) and EnMaaS",
    long_about = "A guided installer that installs the OpenCode CLI when it is missing and \
configures it with your own Red Hat AI MaaS and EnMaaS API keys. Run without arguments for \
the TUI, or with --non-interactive for automation."
)]
pub struct Args {
    /// Run without prompts. Implied by --yes, --dry-run and a non-TTY.
    #[arg(long)]
    pub non_interactive: bool,

    /// Accept defaults without prompting (implies non-interactive).
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Providers to configure: rhai-maas, enmaas.
    #[arg(long, value_name = "ID", value_delimiter = ',')]
    pub provider: Vec<String>,

    /// Base URL for rhai-maas, including /v1. Overrides the environment and
    /// endpoints.conf.
    #[arg(long, value_name = "URL")]
    pub rhai_maas_base_url: Option<String>,

    /// Base URL for enmaas, the host root without /v1. Overrides the
    /// environment and endpoints.conf.
    #[arg(long, value_name = "URL")]
    pub enmaas_base_url: Option<String>,

    /// Read keys from NAME=value file, or - for stdin.
    #[arg(long, value_name = "PATH")]
    pub key_file: Option<String>,

    /// Reuse a key that is already stored.
    #[arg(long)]
    pub keep_existing: bool,

    /// Pick the default model, e.g. enmaas=rits/zai-org/glm-5-3.
    #[arg(long, value_name = "PROVIDER=MODEL")]
    pub model: Vec<String>,

    /// Primary agent OpenCode starts with: build or plan.
    #[arg(long, value_name = "AGENT")]
    pub agent: Option<String>,

    /// Install OpenCode into this directory when it is missing.
    #[arg(long, value_name = "DIR")]
    pub install_dir: Option<PathBuf>,

    /// Never download OpenCode; fail when it is not installed.
    #[arg(long)]
    pub no_install: bool,

    /// Do not call the services to check keys.
    #[arg(long)]
    pub no_validate: bool,

    /// Fail when a key could not be validated (for example on the wrong VPN).
    /// Also refuses to write a configuration whose selected provider still has
    /// a placeholder endpoint.
    #[arg(long)]
    pub require_validation: bool,

    /// Do not run `opencode models` afterwards.
    #[arg(long)]
    pub no_verify: bool,

    /// Restore the previous config if verification fails.
    #[arg(long)]
    pub rollback_on_failure: bool,

    /// Show what would change without writing anything.
    #[arg(long)]
    pub dry_run: bool,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// OpenCode configuration directory (default: ~/.config/opencode).
    #[arg(long, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Check the current configuration without changing it.
    #[arg(long)]
    pub doctor: bool,

    /// Print the models the service offers, then exit.
    #[arg(long)]
    pub list_models: bool,

    /// Permit running as root (containers and image builds only).
    #[arg(long, hide = true)]
    pub allow_root: bool,
}

pub fn run(args: Args) -> Result<u8> {
    crate::settings::set_overrides(endpoint_overrides(&args)?);

    let config_dir = platform::config_dir(args.config_dir.as_deref());

    if platform::is_root() && !args.allow_root {
        return Err(Fail::new(
            code::USAGE,
            "Run this as the user who will use OpenCode, not as root.",
        )
        .hint("All OpenCode configuration lives in that user's home directory."));
    }

    if args.doctor {
        return run_doctor(&config_dir, args.format);
    }
    if args.list_models {
        return run_list_models(&args);
    }

    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let has_mode_flag = args.non_interactive || args.yes || args.dry_run;

    if !tty && !has_mode_flag {
        return Err(Fail::new(
            code::USAGE,
            "No terminal available for the interactive installer.",
        )
        .hint("Add --non-interactive (or --yes) when running from automation."));
    }

    let tool = apply::preflight(args.install_dir.as_deref(), !args.no_install)?;

    if has_mode_flag || !tty {
        return run_non_interactive(&args, &config_dir, &tool.info);
    }

    crate::tui::run(config_dir, tool.info)
}

/// Validate the base-URL flags and turn them into endpoint overrides. Flags are
/// strict because the user typed them for this run, while values from the
/// environment or `endpoints.conf` only produce a warning so a shared file
/// cannot break every invocation.
fn endpoint_overrides(args: &Args) -> Result<BTreeMap<String, String>> {
    let mut overrides = BTreeMap::new();
    let flags = [
        (
            "--rhai-maas-base-url",
            "RHAI_MAAS_BASE_URL",
            true,
            args.rhai_maas_base_url.as_deref(),
        ),
        (
            "--enmaas-base-url",
            "ENMAAS_BASE_URL",
            false,
            args.enmaas_base_url.as_deref(),
        ),
    ];

    for (flag, key, needs_v1, raw) in flags {
        let Some(raw) = raw else { continue };
        let value = raw.trim();
        if value.is_empty() {
            return Err(Fail::new(code::USAGE, format!("{flag} cannot be empty."))
                .hint("Omit the flag to use the environment, endpoints.conf, or the default."));
        }
        if !(value.starts_with("http://") || value.starts_with("https://")) {
            return Err(Fail::new(
                code::USAGE,
                format!("{flag} must start with http:// or https://."),
            )
            .hint(format!("Got: {value}")));
        }
        let value = value.trim_end_matches('/');
        if needs_v1 && !value.ends_with("/v1") {
            return Err(Fail::new(code::USAGE, format!("{flag} must include /v1."))
                .hint("OpenCode appends /chat/completions to an OpenAI-compatible base."));
        }
        if !needs_v1 && value.ends_with("/v1") {
            return Err(Fail::new(
                code::USAGE,
                format!("{flag} must be the host root, without /v1."),
            )
            .hint("This installer appends /v1 for EnMaaS; including it would double it."));
        }
        overrides.insert(key.to_string(), value.to_string());
    }

    Ok(overrides)
}

fn parse_providers(raw: &[String]) -> Result<Vec<&'static ProviderSpec>> {
    let mut out: Vec<&'static ProviderSpec> = Vec::new();
    for item in raw {
        for part in item.split(',') {
            if part.trim().is_empty() {
                continue;
            }
            let spec = catalog::find(part).ok_or_else(|| {
                Fail::new(code::USAGE, format!("Unknown provider: {part}"))
                    .hint("Use rhai-maas, enmaas, or both, comma separated.")
            })?;
            if !out.iter().any(|p| p.id == spec.id) {
                out.push(spec);
            }
        }
    }
    if out.is_empty() {
        return Err(
            Fail::new(code::USAGE, "No provider selected.").hint("Use --provider rhai-maas,enmaas")
        );
    }
    Ok(out)
}

fn parse_models(raw: &[String]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for item in raw {
        for part in item.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((provider, model)) = part.split_once('=') else {
                return Err(Fail::new(
                    code::USAGE,
                    format!("Model '{part}' must be PROVIDER=MODEL."),
                ));
            };
            let id = catalog::normalize_id(provider)
                .or_else(|| {
                    catalog::all().iter().find_map(|spec| {
                        spec.declaratives
                            .iter()
                            .any(|d| d.name == provider.trim())
                            .then_some(spec.id)
                    })
                })
                .ok_or_else(|| {
                    Fail::new(
                        code::USAGE,
                        format!("Unknown provider in model: {provider}"),
                    )
                })?;
            out.push((id.to_string(), model.trim().to_string()));
        }
    }
    Ok(out)
}

type KeyInputs = (Vec<(String, String)>, BTreeMap<String, String>, Vec<String>);

fn key_inputs(args: &Args, providers: &[&'static ProviderSpec]) -> Result<KeyInputs> {
    let mut warnings = Vec::new();
    let entries = match &args.key_file {
        Some(path) => keyfile::read(path)?,
        None => Vec::new(),
    };
    let file_label = match args.key_file.as_deref() {
        Some("-") => "stdin",
        Some(_) => "file",
        None => "env",
    };

    let mut keys = Vec::new();
    let mut sources = BTreeMap::new();

    for spec in providers {
        let from_env = std::env::var(spec.secret)
            .ok()
            .filter(|v| !v.trim().is_empty());
        if let Some(value) = from_env {
            keys.push((spec.secret.to_string(), value));
            sources.insert(spec.id.to_string(), "env".to_string());
            continue;
        }
        if let Some(entry) = entries.iter().find(|e| e.name == spec.secret) {
            keys.push((spec.secret.to_string(), entry.value.clone()));
            sources.insert(spec.id.to_string(), file_label.to_string());
        }
    }

    let known: Vec<&str> = catalog::all().iter().map(|s| s.secret).collect();
    for entry in &entries {
        if !known.contains(&entry.name.as_str()) {
            warnings.push(format!(
                "Ignoring {} in the key file; it is not a provider key.",
                entry.name
            ));
        }
    }

    Ok((keys, sources, warnings))
}

fn run_non_interactive(args: &Args, config_dir: &Path, tool: &platform::ToolInfo) -> Result<u8> {
    let providers = parse_providers(&args.provider)?;
    let (keys, sources, mut warnings) = key_inputs(args, &providers)?;

    let endpoints = crate::settings::endpoints();
    let placeholders: Vec<&str> = providers
        .iter()
        .map(|spec| spec.id)
        .filter(|id| endpoints.uses_placeholders_for([*id]))
        .collect();
    if !placeholders.is_empty() {
        let which = placeholders.join(", ");
        let message =
            format!("Service endpoints for {which} are still placeholders (example.com).");
        if args.require_validation {
            return Err(Fail::new(code::USAGE, message)
                .hint(
                    "Set --rhai-maas-base-url and --enmaas-base-url, the matching environment \
                     variables, or /etc/redhat-ai-configurator-opencode/endpoints.conf.",
                )
                .hint("Drop --require-validation to write the configuration anyway."));
        }
        warnings.push(format!(
            "{message} Set --rhai-maas-base-url / --enmaas-base-url, the matching environment \
             variables, or /etc/redhat-ai-configurator-opencode/endpoints.conf."
        ));
    }
    for spec in &providers {
        if let Some(warning) = endpoints.shape_warning(spec.id) {
            warnings.push(warning);
        }
    }

    let mut validated: BTreeMap<String, Option<bool>> = BTreeMap::new();
    if !args.no_validate {
        for spec in &providers {
            let key = keys
                .iter()
                .find(|(name, _)| name == spec.secret)
                .map(|(_, value)| value.clone());
            let Some(key) = key else {
                validated.insert(spec.id.to_string(), None);
                continue;
            };
            let result = netcheck::validate(&spec.validate_url(), &key);
            if result.ok {
                validated.insert(spec.id.to_string(), Some(true));
            } else if result.server_error() {
                warnings.push(format!(
                    "{}: {} The key was not verified; the service is failing, so you can continue.",
                    spec.display, result.message
                ));
                validated.insert(spec.id.to_string(), Some(false));
                if args.require_validation {
                    return Err(Fail::new(
                        code::NETWORK,
                        format!("{}: {}", spec.display, result.message),
                    )
                    .hint("The service returned a server error, not a key rejection."));
                }
            } else if result.status.is_none() {
                warnings.push(format!(
                    "{}: {} This is expected if the VPN is in the wrong state.",
                    spec.display, result.message
                ));
                validated.insert(spec.id.to_string(), Some(false));
                if args.require_validation {
                    return Err(Fail::new(
                        code::NETWORK,
                        format!("{}: {}", spec.display, result.message),
                    )
                    .hint(format!("This step needs: {}", spec.vpn.text())));
                }
            } else {
                return Err(
                    Fail::new(code::KEY, format!("{}: {}", spec.display, result.message))
                        .hint(format!("Create a new key at {}", spec.key_page()))
                        .hint(format!("This step needs: {}", spec.vpn.text())),
                );
            }
        }
    } else {
        for spec in &providers {
            validated.insert(spec.id.to_string(), None);
        }
    }

    let agent = match &args.agent {
        Some(raw) => Agent::parse(raw).ok_or_else(|| {
            Fail::new(code::USAGE, format!("Unknown agent: {raw}")).hint("Use build or plan.")
        })?,
        None => Agent::Build,
    };

    let selection = Selection {
        provider_ids: providers.iter().map(|p| p.id).collect(),
        model_overrides: parse_models(&args.model)?,
        agent,
        keys: keys.clone(),
        keep_existing: args.keep_existing,
        config_dir: config_dir.to_path_buf(),
        opencode: tool.path.clone(),
    };

    let plan = apply::build(&selection)?;
    warnings.extend(plan.warnings.clone());

    let provider_reports: Vec<ProviderReport> = providers
        .iter()
        .map(|spec| ProviderReport {
            id: spec.id.to_string(),
            display: spec.display.to_string(),
            model: plan
                .models
                .iter()
                .find(|(id, _)| id == spec.id)
                .map(|(_, model)| model.clone())
                .unwrap_or_default(),
            validated: validated.get(spec.id).copied().flatten(),
            key_source: sources.get(spec.id).cloned().unwrap_or_else(|| {
                if plan.existing_keys.contains(spec.default_provider) {
                    "existing".to_string()
                } else {
                    "none".to_string()
                }
            }),
        })
        .collect();

    let mut report = Report {
        ok: true,
        dry_run: args.dry_run,
        binary_path: tool.path.display().to_string(),
        binary_version: tool.version_string(),
        providers: provider_reports,
        agent: plan.agent.as_str().to_string(),
        config: plan.config_path.display().to_string(),
        backup: None,
        auth: "OpenCode credential store".to_string(),
        auth_updated: plan.auth_changed,
        changed: !plan.changed.is_empty() || plan.auth_changed,
        changed_files: plan
            .changed
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        verified: None,
        verify_output: None,
        warnings: Vec::new(),
        error: None,
        error_code: None,
    };
    report.warnings = dedup(warnings.clone());

    if args.dry_run {
        emit(&report, args.format);
        return Ok(0);
    }

    let commit = match apply::commit(&plan, &tool.path, args.rollback_on_failure) {
        Ok(commit) => commit,
        Err(error) => {
            report.ok = false;
            report.error = Some(error.message.clone());
            report.error_code = Some(error.code);
            emit(&report, args.format);
            return Err(error);
        }
    };
    report.backup = commit
        .backups
        .first()
        .map(|backup| backup.path.display().to_string());
    warnings.extend(commit.warnings.clone());
    report.warnings = dedup(warnings);

    if !args.no_verify {
        let check = runner::models_check(&tool.path, &plan.model_refs);
        report.verified = Some(check.ok);
        report.verify_output = Some(check.output.clone());
        if !check.ok {
            if args.rollback_on_failure {
                commit.restore();
                report
                    .warnings
                    .push("Restored the previous configuration.".to_string());
            }
            report.ok = false;
            report.error = Some("opencode models failed after configuration.".to_string());
            report.error_code = Some(code::VERIFY);
            emit(&report, args.format);
            return Ok(code::VERIFY);
        }
    }

    emit(&report, args.format);
    Ok(0)
}

fn emit(report: &Report, format: Format) {
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&report.to_json()).unwrap_or_default()
        ),
        Format::Text => report.print_text(),
    }
}

fn dedup(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

fn run_list_models(args: &Args) -> Result<u8> {
    let providers = parse_providers(&args.provider)?;
    let (keys, _, _) = key_inputs(args, &providers)?;
    let mut all = Vec::new();
    for spec in &providers {
        let Some((_, key)) = keys.iter().find(|(name, _)| name == spec.secret) else {
            return Err(Fail::new(
                code::KEY,
                format!("No API key supplied for {}.", spec.display),
            )
            .hint(format!("Create one at {}", spec.key_page())));
        };
        let result = netcheck::validate(&spec.validate_url(), key);
        if !result.ok {
            let code = if result.status.is_none() || result.server_error() {
                code::NETWORK
            } else {
                code::KEY
            };
            return Err(
                Fail::new(code, format!("{}: {}", spec.display, result.message))
                    .hint(format!("This step needs: {}", spec.vpn.text())),
            );
        }
        all.push(json!({ "provider": spec.id, "models": result.models }));
    }
    if matches!(args.format, Format::Json) {
        let value = if all.len() == 1 {
            all.remove(0)
        } else {
            Value::Array(all)
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        for entry in &all {
            println!(
                "{}:",
                entry.get("provider").and_then(Value::as_str).unwrap_or("")
            );
            if let Some(models) = entry.get("models").and_then(Value::as_array) {
                for model in models {
                    println!("  {}", model.as_str().unwrap_or(""));
                }
            }
        }
    }
    Ok(0)
}

/// Providers an OpenCode config actually references: the default model's
/// provider plus every provider entry this catalog knows. IDs such as
/// `enmaas-oai` map back to their parent provider.
fn configured_provider_ids(config_text: Option<&str>) -> Vec<&'static str> {
    let Some(text) = config_text else {
        return Vec::new();
    };
    let Ok(root) = crate::opencode::config::parse(text) else {
        return Vec::new();
    };

    let mut names: Vec<String> = Vec::new();
    if let Some(model) = root.get("model").and_then(Value::as_str)
        && let Some((provider, _)) = model.split_once('/')
    {
        names.push(provider.to_string());
    }
    if let Some(providers) = root.get("providers").and_then(Value::as_object) {
        names.extend(providers.keys().cloned());
    }

    let mut ids: Vec<&'static str> = Vec::new();
    for name in names {
        let id = catalog::normalize_id(&name).or_else(|| {
            catalog::all().iter().find_map(|spec| {
                spec.declaratives
                    .iter()
                    .any(|declarative| declarative.name == name)
                    .then_some(spec.id)
            })
        });
        if let Some(id) = id
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids
}

struct CheckItem {
    name: String,
    ok: bool,
    detail: String,
}

fn run_doctor(config_dir: &Path, format: Format) -> Result<u8> {
    let mut checks: Vec<CheckItem> = Vec::new();

    checks.push(CheckItem {
        name: "user".into(),
        ok: !platform::is_root(),
        detail: if platform::is_root() {
            "running as root; OpenCode config is per-user".into()
        } else {
            "non-root".into()
        },
    });

    let tool = platform::opencode_info();
    checks.push(CheckItem {
        name: "opencode".into(),
        ok: tool.as_ref().is_some_and(|t| t.meets_min()),
        detail: match &tool {
            Some(t) => format!("{} at {}", t.version_string(), t.path.display()),
            None => "not found on PATH".into(),
        },
    });

    let config_path = apply::config_path(config_dir);
    let config_text = std::fs::read_to_string(&config_path).ok();
    checks.push(CheckItem {
        name: "config".into(),
        ok: config_text.is_some(),
        detail: if config_text.is_some() {
            config_path.display().to_string()
        } else {
            format!("{} is missing", config_path.display())
        },
    });

    let root = config_text
        .as_deref()
        .and_then(|text| crate::opencode::config::parse(text).ok())
        .unwrap_or_default();
    let model = root
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let agent = root
        .get("default_agent")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    checks.push(CheckItem {
        name: "model".into(),
        ok: !model.is_empty(),
        detail: if model.is_empty() {
            "no default model".into()
        } else {
            model.clone()
        },
    });
    checks.push(CheckItem {
        name: "agent".into(),
        ok: !agent.is_empty(),
        detail: if agent.is_empty() {
            "no default agent".into()
        } else {
            agent
        },
    });

    let configured = configured_provider_ids(config_text.as_deref());
    let checked: Vec<&'static ProviderSpec> = if configured.is_empty() {
        catalog::all().iter().collect()
    } else {
        configured
            .iter()
            .filter_map(|id| catalog::find(id))
            .collect()
    };

    checks.push(CheckItem {
        name: "providers".into(),
        ok: !configured.is_empty(),
        detail: if configured.is_empty() {
            "no provider entries".into()
        } else {
            configured.join(", ")
        },
    });

    let endpoints = crate::settings::endpoints();
    checks.push(CheckItem {
        name: "endpoints".into(),
        ok: !endpoints.uses_placeholders_for(checked.iter().map(|spec| spec.id)),
        detail: checked
            .iter()
            .map(|spec| endpoints.provider_base(spec.id))
            .collect::<Vec<_>>()
            .join(" / "),
    });

    if let Some(tool) = &tool {
        let stored = auth::stored(&tool.path);
        for spec in &checked {
            checks.push(CheckItem {
                name: format!("key {}", spec.id),
                ok: stored
                    .get(spec.default_provider)
                    .is_some_and(|credential| !credential.key.is_empty()),
                detail: if stored
                    .get(spec.default_provider)
                    .is_some_and(|credential| !credential.key.is_empty())
                {
                    "stored".into()
                } else {
                    "not stored".into()
                },
            });
        }
    }

    for spec in &checked {
        let ok = netcheck::reachable(&spec.validate_url());
        checks.push(CheckItem {
            name: format!("reachable {}", spec.id),
            ok,
            detail: if ok {
                "reachable".to_string()
            } else {
                format!("unreachable; this step needs {}", spec.vpn.text())
            },
        });
    }

    let ok = checks.iter().all(|check| check.ok);
    if matches!(format, Format::Json) {
        let value = json!({
            "ok": ok,
            "configDir": config_dir.display().to_string(),
            "checks": checks.iter().map(|c| json!({
                "name": c.name, "ok": c.ok, "detail": c.detail
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        println!("OpenCode configuration in {}", config_dir.display());
        println!();
        for check in &checks {
            println!(
                "  {} {:<24} {}",
                if check.ok { "ok  " } else { "FAIL" },
                check.name,
                check.detail
            );
        }
        println!();
        println!(
            "{}",
            if ok {
                "All checks passed."
            } else {
                "Some checks failed."
            }
        );
    }
    Ok(if ok { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_ids_come_from_model_and_providers() {
        let text = r#"{
          "model": "enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017",
          "providers": { "ai-bu": {}, "enmaas": {}, "enmaas-oai": {} }
        }"#;
        let ids = configured_provider_ids(Some(text));
        assert!(ids.contains(&"rhai-maas"));
        assert!(ids.contains(&"enmaas"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn configured_ids_map_opencode_names_to_providers() {
        let ids = configured_provider_ids(Some(r#"{"providers":{"enmaas-oai":{}}}"#));
        assert_eq!(ids, vec!["enmaas"]);
    }

    #[test]
    fn configured_ids_empty_without_a_provider() {
        assert!(configured_provider_ids(None).is_empty());
        assert!(configured_provider_ids(Some(r#"{"model": "x"}"#)).is_empty());
        assert!(configured_provider_ids(Some("{ unclosed")).is_empty());
    }
}
