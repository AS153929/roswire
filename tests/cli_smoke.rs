use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

#[test]
fn unimplemented_write_command_returns_unsupported_action() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args([
        "ip",
        "address",
        "add",
        "address=192.168.88.2/24",
        "interface=ether1",
        "--json",
    ])
    .assert()
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains(
        "\"error_code\":\"UNSUPPORTED_ACTION\"",
    ))
    .stderr(predicate::str::contains("\"command\":\"ip/address/add\""))
    .stderr(predicate::str::contains("\"path\":[\"ip\",\"address\"]"))
    .stderr(predicate::str::contains("\"action\":\"add\""));
}

#[test]
fn readonly_print_without_connection_config_returns_config_error() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .env_remove("ROS_PORT")
        .env_remove("ROS_PROTOCOL")
        .env_remove("ROS_ROUTEROS_VERSION");
    cmd.args(["interface", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""));
}

#[test]
fn readonly_print_rejects_profile_mac_host_before_network() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    fs::write(
        temp.path().join("config.toml"),
        r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "48:8F:5A:A3:0E:A7"
user = "master"
"#,
    )
    .expect("config should be written");
    #[cfg(unix)]
    {
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
            .expect("home permissions should be set");
        fs::set_permissions(
            temp.path().join("config.toml"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("config permissions should be set");
    }

    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .args(["interface", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("MAC address"))
        .stderr(predicate::str::contains(
            "Layer 2 discovery is not supported",
        ))
        .stderr(predicate::str::contains("\"host\":\"48:8F:5A:A3:0E:A7\""));
}

#[test]
fn unsupported_action_context_redacts_sensitive_args() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args([
        "ip",
        "address",
        "add",
        "address=192.168.88.2/24",
        "password=super-secret",
        "--json",
    ])
    .assert()
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains(
        "\"error_code\":\"UNSUPPORTED_ACTION\"",
    ))
    .stderr(predicate::str::contains("super-secret").not())
    .stderr(predicate::str::contains("***REDACTED***"));
}
