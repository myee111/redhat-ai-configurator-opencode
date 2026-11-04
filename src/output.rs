//! Human and machine-readable output.

use serde_json::{Value, json};

use crate::catalog::ProviderSpec;
use crate::error::Fail;

#[derive(Clone, Debug)]
pub struct ProviderReport {
    pub id: String,
    pub display: String,
    pub model: String,
    pub validated: Option<bool>,
    pub key_source: String,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub ok: bool,
    pub dry_run: bool,
    pub goose_path: String,
    pub goose_version: String,
    pub providers: Vec<ProviderReport>,
    pub config: String,
    pub backup: Option<String>,
    pub secrets: String,
    pub secrets_mode: String,
    pub changed: bool,
    pub changed_files: Vec<String>,
    pub verified: Option<bool>,
    pub verify_output: Option<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub error_code: Option<u8>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok,
            "dryRun": self.dry_run,
            "goose": { "path": self.goose_path, "version": self.goose_version },
            "providers": self.providers.iter().map(|p| json!({
                "id": p.id,
                "display": p.display,
                "model": p.model,
                "validated": p.validated,
                "keySource": p.key_source,
            })).collect::<Vec<_>>(),
            "config": self.config,
            "backup": self.backup,
            "secrets": self.secrets,
            "secretsMode": self.secrets_mode,
            "changed": self.changed,
            "changedFiles": self.changed_files,
            "verified": self.verified,
            "verifyOutput": self.verify_output,
            "warnings": self.warnings,
            "error": self.error,
            "errorCode": self.error_code,
        })
    }

    pub fn print_text(&self) {
        if self.dry_run {
            println!("Dry run - nothing was written.");
            println!();
            if self.changed_files.is_empty() {
                println!("Configuration is already up to date.");
            } else {
                println!("Would write:");
                for file in &self.changed_files {
                    println!("  {file}");
                }
            }
            print_providers(self);
            print_warnings(self);
            return;
        }

        println!("Configured goose for {} provider(s).", self.providers.len());
        print_providers(self);
        println!();
        println!(
            "Config    {}",
            self.config
        );
        if let Some(backup) = &self.backup {
            println!("Backup    {backup}");
        }
        println!("Secrets   {} ({})", self.secrets, self.secrets_mode);
        match self.verified {
            Some(true) => println!("Verify    goose info --check: OK"),
            Some(false) => println!("Verify    goose info --check: FAILED"),
            None => {}
        }
        if self.verify_output.as_deref().is_some_and(|o| !o.is_empty()) {
            for line in self.verify_output.as_deref().unwrap_or("").lines() {
                println!("          {line}");
            }
        }
        print_warnings(self);
        println!();
        println!("Next      run `goose` to start a session.");
    }
}

fn print_providers(report: &Report) {
    println!();
    for provider in &report.providers {
        let status = match provider.validated {
            Some(true) => "key validated",
            Some(false) => "key not validated",
            None => "validation skipped",
        };
        println!(
            "  {} ({})",
            provider.display, provider.id
        );
        println!("    model   {}", provider.model);
        println!("    key     {status} [{}]", provider.key_source);
    }
}

fn print_warnings(report: &Report) {
    if report.warnings.is_empty() {
        return;
    }
    println!();
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
}

pub fn print_fail(fail: &Fail) {
    eprintln!("error: {}", fail.message);
    for hint in &fail.hints {
        eprintln!("  hint: {hint}");
    }
}

/// Printed after the alternate screen closes so it stays in scrollback.
pub fn print_recap(specs: &[&ProviderSpec], report: &Report) {
    println!();
    println!("Red Hat AI Configurator");
    for spec in specs {
        println!("  {} - keys: {}", spec.display, spec.key_page());
        println!("    guide: {}", spec.help_url());
        if let Some(url) = spec.eligibility_url() {
            println!("    access request: {url}");
        }
        if let Some(slack) = spec.slack() {
            println!("    help: {slack}");
        }
    }
    println!("  config: {}", report.config);
    if let Some(backup) = &report.backup {
        println!("  backup: {backup}");
    }
    println!("  start the agent with: goose");
}
