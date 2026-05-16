use assert_cmd::Command;
use predicates::prelude::*;

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
