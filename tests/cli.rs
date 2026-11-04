//! End-to-end tests for non-interactive mode and doctor.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::prelude::*;

fn command(root: &std::path::Path) -> Command {
    // A stand-in goose so preflight checks do not depend on the host.
    let bin = root.join("test-bin");
    fs::create_dir_all(&bin).unwrap();
    let goose = bin.join("goose");
    if !goose.exists() {
        fs::write(
            &goose,
            "#!/bin/sh\nif [ \"$1\" = info ]; then echo 'Provider: rhai-maas'; exit 0; fi\necho 'goose 1.45.0'\n",
        )
        .unwrap();
        fs::set_permissions(&goose, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut command = Command::cargo_bin("redhat-ai-configurator").unwrap();
    command
        .env("GOOSE_PATH_ROOT", root)
        .env("HOME", root)
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
fn writes_config_providers_and_private_secrets() {
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

    let config_dir = root.join("config");
    let config = fs::read_to_string(config_dir.join("config.yaml")).unwrap();
    assert!(config.contains("GOOSE_PROVIDER: rhai-maas"));
    assert!(
        config.contains("GOOSE_MODEL: publishers/prelude-maas/models/glm-53-flash"),
        "{config}"
    );
    assert!(config.contains("GOOSE_DISABLE_KEYRING: true"));

    let provider: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(config_dir.join("custom_providers/rhai-maas.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(provider["engine"], "openai");
    assert_eq!(
        provider["base_url"],
        "https://maas.test/v1"
    );

    let secrets = fs::read_to_string(config_dir.join("secrets.yaml")).unwrap();
    assert!(secrets.contains("RHAI_MAAS_API_KEY: sk-test-value"));
    let mode = fs::metadata(config_dir.join("secrets.yaml"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
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
    assert!(!root.join("config/config.yaml").exists());
    assert!(!root.join("config/secrets.yaml").exists());
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

    let provider: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("config/custom_providers/rhai-maas.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(provider["base_url"], "https://cli.test/v1");
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
fn doctor_reports_missing_configuration() {
    let temp = tempfile::tempdir().unwrap();
    command(temp.path())
        .arg("--doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("provider"))
        .stdout(predicate::str::contains("config"));
}
