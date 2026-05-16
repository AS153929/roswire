use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn commands_json_contains_catalog_entries() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["commands", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.commands.v1\"",
        ))
        .stdout(predicate::str::contains("ip address add"));
}

#[test]
fn help_topic_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "ip", "address", "add", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"ip address add\""));
}

#[test]
fn schema_command_returns_argument_list() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "ip", "address", "add", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"address\""))
        .stdout(predicate::str::contains("\"name\":\"interface\""));
}

#[test]
fn unknown_help_topic_returns_structured_error() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "unknown", "topic", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "\"error_code\":\"HELP_TOPIC_NOT_FOUND\"",
        ));
}

#[test]
fn unknown_schema_topic_returns_structured_error() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "unknown", "topic", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "\"error_code\":\"SCHEMA_UNAVAILABLE\"",
        ));
}

#[test]
fn explain_error_returns_machine_readable_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["explain-error", "ROS_API_FAILURE", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.error.explain.v1\"",
        ))
        .stdout(predicate::str::contains(
            "\"error_code\":\"ROS_API_FAILURE\"",
        ));
}
