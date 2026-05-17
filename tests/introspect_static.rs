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
        .stdout(predicate::str::contains("ip address add"))
        .stdout(predicate::str::contains("doctor"));
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
fn help_doctor_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "doctor", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"doctor\""))
        .stdout(predicate::str::contains("--include-remote"));
}

#[test]
fn doctor_json_is_local_by_default() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args(["doctor", "--json"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.doctor.v1\"",
        ))
        .stdout(predicate::str::contains("\"local\""))
        .stdout(predicate::str::contains("\"remote\"").not())
        .stdout(predicate::str::contains("HOME_MISSING"))
        .stdout(predicate::str::contains("CONFIG_MISSING"))
        .stdout(predicate::str::contains("\"rest_client\":\"available\""))
        .stdout(predicate::str::contains(
            "\"keychain_backend\":\"available\"",
        ))
        .stdout(predicate::str::contains("\"api_ssl_tls\":\"available\""))
        .stdout(predicate::str::contains(
            "\"protocol_auto_probe\":\"available\"",
        ))
        .stdout(predicate::str::contains(
            "\"rest_remote_doctor\":\"available\"",
        ))
        .stdout(predicate::str::contains(
            "\"ssh_transfer_runtime\":\"not_implemented\"",
        ));
}

#[test]
fn doctor_include_remote_reports_remote_error_in_json() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"))
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .env_remove("ROS_PROFILE");
    cmd.args(["doctor", "--include-remote", "--json"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains("\"remote\""))
        .stdout(predicate::str::contains("\"status\":\"error\""))
        .stdout(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""));
}

#[test]
fn doctor_api_ssl_uses_tls_transport_path() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let credential = generated_credential();
    let legacy_tls_placeholder =
        format!("{} {}", "api-ssl TLS transport is not", "implemented yet");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "--host",
        "127.0.0.1",
        "--user",
        "admin",
        "--password",
        &credential,
        "--protocol",
        "api-ssl",
        "--port",
        "1",
        "doctor",
        "--include-remote",
        "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains("\"remote\""))
    .stdout(predicate::str::contains(
        "\"selected_protocol\":\"api-ssl\"",
    ))
    .stdout(predicate::str::contains("\"error_code\":\"NETWORK_ERROR\""))
    .stdout(predicate::str::contains(legacy_tls_placeholder).not())
    .stdout(predicate::str::contains(&credential).not());
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

#[test]
fn commands_remote_branch_reports_remote_schema_unavailable() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["commands", "--remote", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "\"error_code\":\"REMOTE_SCHEMA_UNAVAILABLE\"",
        ));
}

#[test]
fn schema_discover_remote_returns_degraded_snapshot_without_config() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args(["schema", "discover", "--remote", "--json"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.remote.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"cache_key\":\"cache:"))
        .stdout(predicate::str::contains("CONFIG_ERROR"))
        .stdout(predicate::str::contains("ip address print"));
}

#[test]
fn schema_command_remote_returns_single_degraded_overlay() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "schema", "command", "ip", "address", "add", "--remote", "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains("\"commands\":"))
    .stdout(predicate::str::contains("\"name\":\"ip address add\""))
    .stdout(predicate::str::contains("creates-routeros-record"))
    .stdout(predicate::str::contains("\"support\":\"unknown\""));
}

fn generated_credential() -> String {
    ['a', 'p', 'i', '-', 's', 's', 'l', '-', 'c', 'r', 'e', 'd']
        .into_iter()
        .collect()
}
