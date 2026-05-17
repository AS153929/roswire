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
fn registered_write_command_reaches_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args([
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
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn script_put_dry_run_outputs_plan_without_source_content_or_absolute_path() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let source = temp.path().join("bootstrap.rsc");
    let script = ":put \"SMOKE_SECRET_SCRIPT\"";
    fs::write(&source, script).expect("source should be written");
    let source_arg = format!("@{}", source.display());
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");

    cmd.args([
        "script",
        "put",
        "bootstrap",
        "--source",
        &source_arg,
        "--dry-run",
        "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains(
        "\"schema_version\":\"roswire.workflow.script.put.plan.v1\"",
    ))
    .stdout(predicate::str::contains("\"script_name\":\"bootstrap\""))
    .stdout(predicate::str::contains("***REDACTED***/bootstrap.rsc"))
    .stdout(predicate::str::contains(temp.path().to_string_lossy().as_ref()).not())
    .stdout(predicate::str::contains(script).not());
}

#[test]
fn script_put_reaches_connection_resolution_without_leaking_source() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let source = temp.path().join("bootstrap.rsc");
    let script = ":put \"SMOKE_SECRET_SCRIPT\"";
    fs::write(&source, script).expect("source should be written");
    let source_arg = format!("@{}", source.display());
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");

    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"))
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args([
            "script",
            "put",
            "bootstrap",
            "--source",
            &source_arg,
            "--json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not())
        .stderr(predicate::str::contains(temp.path().to_string_lossy().as_ref()).not())
        .stderr(predicate::str::contains(script).not());
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
fn system_package_print_is_registered_before_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["system", "package", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn user_print_is_registered_before_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["user", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn ip_route_print_is_registered_before_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["ip", "route", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn firewall_print_is_registered_before_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["ip", "firewall", "nat", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn tool_print_is_registered_before_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["tool", "netwatch", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn wireguard_prints_are_registered_before_connection_resolution() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["interface", "wireguard", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());

    let mut peers = Command::cargo_bin("roswire").expect("binary should compile");
    peers
        .env("ROSWIRE_HOME", temp.path())
        .env_remove("ROS_PROFILE")
        .env_remove("ROS_HOST")
        .env_remove("ROS_USER")
        .env_remove("ROS_PASSWORD")
        .args(["interface", "wireguard", "peers", "print", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
        .stderr(predicate::str::contains("UNSUPPORTED_ACTION").not());
}

#[test]
fn unsupported_wireguard_write_redacts_key_material() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args([
        "interface",
        "wireguard",
        "add",
        "private-key=private-secret",
        "preshared-key=shared-secret",
        "--json",
    ])
    .assert()
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains(
        "\"error_code\":\"UNSUPPORTED_ACTION\"",
    ))
    .stderr(predicate::str::contains("private-secret").not())
    .stderr(predicate::str::contains("shared-secret").not())
    .stderr(predicate::str::contains("***REDACTED***"));
}

#[test]
fn unsupported_user_write_redacts_password_argument() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["user", "set", "password=super-secret", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "\"error_code\":\"UNSUPPORTED_ACTION\"",
        ))
        .stderr(predicate::str::contains("super-secret").not())
        .stderr(predicate::str::contains("***REDACTED***"));
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
    cmd.args(["ip", "address", "enable", "password=super-secret", "--json"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "\"error_code\":\"UNSUPPORTED_ACTION\"",
        ))
        .stderr(predicate::str::contains("super-secret").not())
        .stderr(predicate::str::contains("***REDACTED***"));
}
