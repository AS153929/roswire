use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use std::fs;

mod common;

use common::predicate;

#[test]
fn file_upload_dry_run_writes_plan_to_stdout_only() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");

    cmd.args([
        "file",
        "upload",
        "/Users/example/private/setup.rsc",
        "flash/setup.rsc",
        "--dry-run",
        "--ssh-host-key",
        "SHA256:test",
        "--allow-from",
        "203.0.113.10/32",
        "--cleanup",
        "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains(
        "\"schema_version\":\"roswire.transfer.plan.v1\"",
    ))
    .stdout(predicate::str::contains("\"operation\":\"file.upload\""))
    .stdout(predicate::str::contains("flash/setup.rsc.roswire.tmp"))
    .stdout(predicate::str::contains("/Users/example").not())
    .stdout(predicate::str::contains("private").not());
}

#[test]
fn file_upload_dry_run_marks_encrypted_key_without_leaking_passphrase() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let passphrase = "SMOKE_KEY_PASSPHRASE_SECRET";
    write_config(
        temp.path(),
        &format!(
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
allow_plain_secrets = true

[profiles.studio.secrets.ssh_key_passphrase]
type = "plain"
value = "{passphrase}"
"#,
        ),
    );

    cmd.env("ROSWIRE_HOME", temp.path())
        .args([
            "file",
            "upload",
            "/Users/example/private/setup.rsc",
            "flash/setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--ssh-key",
            "/Users/example/.ssh/id_ed25519",
            "--allow-from",
            "203.0.113.10/32",
            "--json",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains(
            "\"auth_method\":\"key-encrypted\"",
        ))
        .stdout(predicate::str::contains("\"key_passphrase\":\"provided\""))
        .stdout(predicate::str::contains(
            "\"data_plane\":\"sftp-with-scp-fallback\"",
        ))
        .stdout(predicate::str::contains("***REDACTED***/id_ed25519"))
        .stdout(predicate::str::contains(passphrase).not());
}

fn write_config(home: &std::path::Path, contents: &str) {
    fs::write(home.join("config.toml"), contents).expect("config should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(home, fs::Permissions::from_mode(0o700))
            .expect("home permissions should be set");
        fs::set_permissions(home.join("config.toml"), fs::Permissions::from_mode(0o600))
            .expect("config permissions should be set");
    }
}

#[test]
fn transfer_missing_host_key_writes_structured_error_to_stderr() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");

    cmd.args([
        "file",
        "download",
        "flash/setup.rsc",
        "setup.rsc",
        "--dry-run",
        "--allow-from",
        "203.0.113.10/32",
        "--json",
    ])
    .assert()
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains(
        "\"error_code\":\"SSH_HOST_KEY_REQUIRED\"",
    ))
    .stderr(predicate::str::contains("\"transfer_backend\":\"ssh\""));
}

#[test]
fn transfer_missing_allow_from_writes_structured_error_to_stderr() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");

    cmd.args([
        "file",
        "download",
        "flash/setup.rsc",
        "setup.rsc",
        "--dry-run",
        "--ssh-host-key",
        "SHA256:test",
        "--json",
    ])
    .assert()
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains(
        "\"error_code\":\"SSH_WHITELIST_REQUIRED\"",
    ));
}

#[test]
fn transfer_rejects_unsafe_allow_from() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");

    cmd.args([
        "file",
        "download",
        "flash/setup.rsc",
        "setup.rsc",
        "--dry-run",
        "--ssh-host-key",
        "SHA256:test",
        "--allow-from",
        "0.0.0.0/0",
        "--json",
    ])
    .assert()
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains(
        "\"error_code\":\"SSH_WHITELIST_UNSAFE\"",
    ));
}
