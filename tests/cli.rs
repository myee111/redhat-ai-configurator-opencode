//! End-to-end tests for non-interactive mode and doctor.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::prelude::*;

const FAKE_OPENCODE: &str = r#"#!/bin/sh
set -e
state="$AUTH_STATE"
case "$1" in
  --version)
    echo "opencode v2.2.5"
    ;;
  models)
    echo "ai-bu/publishers/prelude-maas/models/glm-53-flash"
    echo "enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017"
    ;;
  auth)
    case "$2" in
      export)
        if [ -f "$state" ]; then cat "$state"; else echo "[]"; fi
        ;;
      import)
        cp "$3" "$state"
        echo "Imported credentials"
        ;;
      *)
        exit 1
        ;;
    esac
    ;;
  *)
    exit 1
    ;;
esac
"#;

fn command(root: &std::path::Path) -> Command {
    // A stand-in opencode so preflight and verification do not depend on the host.
    let bin = root.join("test-bin");
    fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("opencode");
    if !tool.exists() {
        fs::write(&tool, FAKE_OPENCODE).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut command = Command::cargo_bin("redhat-ai-configurator-opencode").unwrap();
    command
        .env("OPENCODE_CONFIG_DIR", root.join("opencode"))
        .env("HOME", root)
        .env("AUTH_STATE", root.join("auth.json"))
        .env("RHAI_MAAS_BASE_URL", "https://maas.test/v1")
        .env("ENMAAS_BASE_URL", "https://enmaas.test")
        .env("RHAI_MAAS_KEY_PAGE", "https://maas.test/keys")
        .env("ENMAAS_KEY_PAGE", "https://enmaas.test/keys")
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .arg("--allow-root");
    command
}

#[test]
fn writes_providers_and_imports_keys() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let output = command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
            "--format",
            "json",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=sk-test-value\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("sk-test-value"), "key leaked into output");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["providers"][0]["id"], "rhai-maas");
    assert_eq!(value["agent"], "build");

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("opencode/opencode.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["providers"]["ai-bu"]["settings"]["baseURL"],
        "https://maas.test/v1"
    );
    assert_eq!(config["providers"]["ai-bu"]["env"][0], "RHAI_MAAS_API_KEY");
    assert_eq!(
        config["model"],
        "ai-bu/publishers/prelude-maas/models/glm-53-flash"
    );
    assert!(config["providers"]["ai-bu"]["models"].is_object());

    let auth: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth[0]["integrationID"], "ai-bu");
    assert_eq!(auth[0]["value"]["key"], "sk-test-value");
}

#[test]
fn enmaas_is_the_default_when_both_are_configured() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas,enmaas",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=a\nENMAAS_API_KEY=b\n")
        .assert()
        .success();

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("opencode/opencode.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["model"],
        "enmaas/rits/deepseek-ai/DeepSeek-V4.1-Flash-1017"
    );
    assert_eq!(
        config["providers"]["enmaas"]["settings"]["baseURL"],
        "https://enmaas.test/v1"
    );
    assert_eq!(
        config["providers"]["enmaas"]["package"],
        "@opencode/ai/providers/anthropic"
    );
    assert_eq!(
        config["providers"]["enmaas-oai"]["settings"]["baseURL"],
        "https://enmaas.test/v1"
    );
}

#[test]
fn second_run_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let args = [
        "--non-interactive",
        "--provider",
        "rhai-maas,enmaas",
        "--key-file",
        "-",
        "--no-validate",
        "--no-verify",
        "--format",
        "json",
    ];

    let first = command(root)
        .args(args)
        .write_stdin("RHAI_MAAS_API_KEY=a\nENMAAS_API_KEY=b\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(first["changed"], true);

    let second = command(root)
        .args(args)
        .write_stdin("RHAI_MAAS_API_KEY=a\nENMAAS_API_KEY=b\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second: serde_json::Value = serde_json::from_slice(&second).unwrap();
    assert_eq!(second["changed"], false);
    assert_eq!(second["authUpdated"], false);
    assert_eq!(second["backup"], serde_json::Value::Null);
}

#[test]
fn keeps_existing_key_without_resupplying_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=first\n")
        .assert()
        .success();

    command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--keep-existing",
            "--no-validate",
            "--no-verify",
        ])
        .assert()
        .success();
}

#[test]
fn refuses_a_world_readable_key_file() {
    let temp = tempfile::tempdir().unwrap();
    let key_file = temp.path().join("keys.env");
    fs::write(&key_file, "RHAI_MAAS_API_KEY=secret\n").unwrap();
    fs::set_permissions(&key_file, fs::Permissions::from_mode(0o644)).unwrap();

    command(temp.path())
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--key-file",
            key_file.to_str().unwrap(),
            "--no-validate",
            "--no-verify",
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("readable by other users"));
}

#[test]
fn missing_key_exits_four_with_the_key_page() {
    let temp = tempfile::tempdir().unwrap();
    command(temp.path())
        .args([
            "--non-interactive",
            "--provider",
            "enmaas",
            "--no-validate",
            "--no-verify",
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("enmaas.test/keys"));
}

#[test]
fn no_provider_is_a_usage_error() {
    let temp = tempfile::tempdir().unwrap();
    command(temp.path())
        .args(["--non-interactive", "--no-validate", "--no-verify"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("No provider selected"));
}

#[test]
fn dry_run_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--dry-run",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .success();
    assert!(!root.join("opencode/opencode.json").exists());
    assert!(!root.join("auth.json").exists());
}

#[test]
fn placeholder_warning_only_covers_selected_providers() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let output = command(root)
        .env_remove("ENMAAS_BASE_URL")
        .env_remove("ENMAAS_KEY_PAGE")
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--dry-run",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
            "--format",
            "json",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let warnings = value["warnings"].as_array().unwrap();
    assert!(
        !warnings
            .iter()
            .any(|warning| warning.as_str().unwrap_or("").contains("placeholders")),
        "{warnings:?}"
    );
}

#[test]
fn base_url_flags_override_the_environment() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--rhai-maas-base-url",
            " https://cli.test/v1/ ",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .success();

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("opencode/opencode.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["providers"]["ai-bu"]["settings"]["baseURL"],
        "https://cli.test/v1"
    );
}

#[test]
fn base_url_flag_shape_is_validated() {
    let temp = tempfile::tempdir().unwrap();
    command(temp.path())
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--rhai-maas-base-url",
            "maas.test/v1",
            "--no-validate",
            "--no-verify",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("must start with http"));

    command(temp.path())
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--rhai-maas-base-url",
            "https://maas.test",
            "--no-validate",
            "--no-verify",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("must include /v1"));

    command(temp.path())
        .args([
            "--non-interactive",
            "--provider",
            "enmaas",
            "--enmaas-base-url",
            "https://enmaas.test/v1",
            "--no-validate",
            "--no-verify",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("without /v1"));
}

#[test]
fn placeholder_endpoint_is_a_warning_without_require_validation() {
    let temp = tempfile::tempdir().unwrap();
    let output = command(temp.path())
        .env_remove("RHAI_MAAS_BASE_URL")
        .env_remove("ENMAAS_BASE_URL")
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
            "--format",
            "json",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let warnings = serde_json::to_string(&value["warnings"]).unwrap();
    assert!(warnings.contains("placeholders"), "{warnings}");
}

#[test]
fn require_validation_rejects_placeholder_endpoints() {
    let temp = tempfile::tempdir().unwrap();
    command(temp.path())
        .env_remove("RHAI_MAAS_BASE_URL")
        .env_remove("ENMAAS_BASE_URL")
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--key-file",
            "-",
            "--require-validation",
            "--no-verify",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("placeholders"))
        .stderr(predicate::str::contains("--rhai-maas-base-url"));
}

#[test]
fn base_url_flag_clears_the_placeholder_warning() {
    let temp = tempfile::tempdir().unwrap();
    let output = command(temp.path())
        .env_remove("RHAI_MAAS_BASE_URL")
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--rhai-maas-base-url",
            "https://maas.test/v1",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
            "--format",
            "json",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let warnings = serde_json::to_string(&value["warnings"]).unwrap();
    assert!(!warnings.contains("placeholders"), "{warnings}");
}

#[test]
fn agent_flag_sets_default_agent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    command(root)
        .args([
            "--non-interactive",
            "--provider",
            "rhai-maas",
            "--agent",
            "plan",
            "--key-file",
            "-",
            "--no-validate",
            "--no-verify",
        ])
        .write_stdin("RHAI_MAAS_API_KEY=x\n")
        .assert()
        .success();

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("opencode/opencode.json")).unwrap())
            .unwrap();
    assert_eq!(config["default_agent"], "plan");
}

#[test]
fn doctor_reports_missing_configuration() {
    let temp = tempfile::tempdir().unwrap();
    command(temp.path())
        .arg("--doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("agent"))
        .stdout(predicate::str::contains("config"));
}
