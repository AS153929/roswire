use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn binary_supports_version_flag() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("roswire"));
}

#[test]
fn binary_supports_help_flag() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn binary_without_arguments_returns_structured_usage_error() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"USAGE_ERROR\""));
}

#[test]
fn structured_errors_are_written_to_stderr_only() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.arg("--simulate-error")
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"USAGE_ERROR\""));
}
