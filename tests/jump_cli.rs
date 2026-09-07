use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use std::fs;

mod common;

use common::predicate;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn command(home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", home);
    cmd
}

fn write_config(home: &std::path::Path, contents: &str) {
    fs::write(home.join("config.toml"), contents).expect("config should be written");
    #[cfg(unix)]
    {
        fs::set_permissions(home, fs::Permissions::from_mode(0o700))
            .expect("home permissions should be set");
        fs::set_permissions(home.join("config.toml"), fs::Permissions::from_mode(0o600))
            .expect("config permissions should be set");
    }
}

#[test]
fn config_device_add_jump_fields_round_trip_through_inspect() {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();
    command(temp.path())
        .args([
            "config",
            "device",
            "add",
            "lab",
            "host=192.168.88.1",
            "user=admin",
            "jump_host=bastion.example",
            "jump_port=2222",
            "jump_user=ops",
            "jump_host_key=SHA256:bastion",
            "jump_key=/Users/example/.ssh/id_ed25519",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("jump_host"));

    command(temp.path())
        .args(["--profile", "lab", "config", "inspect", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bastion.example"))
        .stdout(predicate::str::contains("\"2222\""))
        .stdout(predicate::str::contains("ops"))
        .stdout(predicate::str::contains("SHA256:bastion"))
        .stdout(predicate::str::contains("***REDACTED***/id_ed25519"))
        .stdout(predicate::str::contains("/Users/example/.ssh").not())
        .stdout(predicate::str::contains("\"local_bind\"").not());
}

#[test]
fn dry_run_with_jump_includes_via_and_does_not_connect() {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();

    command(temp.path())
        .args([
            "--host",
            "192.168.88.1",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--jump-host-key",
            "SHA256:test",
            "--dry-run",
            "--json",
            "ip",
            "address",
            "print",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains("\"kind\": \"jump\""))
        .stdout(predicate::str::contains("bastion.example"))
        .stdout(predicate::str::contains("\"local_bind\": false"))
        .stdout(predicate::str::contains("\"teardown\": \"process-exit\""))
        .stdout(predicate::str::contains("\"will_connect\": false"));
}

#[test]
fn missing_jump_host_key_fails_before_network() {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();

    command(temp.path())
        .args([
            "--host",
            "192.0.2.1",
            "--user",
            "admin",
            "--password",
            "secret",
            "--protocol",
            "api",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--json",
            "ip",
            "address",
            "print",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("JUMP_HOST_KEY_REQUIRED"));
}

#[test]
fn doctor_reports_configured_jump_without_local_bind() {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();
    write_config(
        temp.path(),
        r#"
version = 1
default_profile = "lab"

[profiles.lab]
host = "192.168.88.1"
user = "admin"

[[profiles.lab.jump]]
host = "bastion.example"
port = 22
user = "ops"
host_key = "SHA256:bastion"
"#,
    );

    command(temp.path())
        .args(["doctor", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"configured\": true"))
        .stdout(predicate::str::contains("\"local_bind\": false"))
        .stdout(predicate::str::contains("bastion.example"))
        .stdout(predicate::str::contains("process-exit"));
}

#[test]
fn doctor_include_remote_reports_jump_legs_when_bastion_is_unreachable() {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();
    write_config(
        temp.path(),
        r#"
version = 1
default_profile = "lab"

[profiles.lab]
host = "192.0.2.1"
user = "admin"
protocol = "api"
allow_plain_secrets = true

[[profiles.lab.jump]]
host = "192.0.2.1"
port = 1
user = "ops"
host_key = "SHA256:bastion"

[profiles.lab.secrets.password]
type = "plain"
value = "secret"
[profiles.lab.secrets.jump_password]
type = "plain"
value = "jump-secret"
"#,
    );

    command(temp.path())
        .args(["doctor", "--include-remote", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"role\": \"bastion\""))
        .stdout(predicate::str::contains("\"role\": \"target\""))
        .stdout(predicate::str::contains("NETWORK_ERROR"));
}

fn command_via_unreachable_jump_fails_with_network_error(protocol: &str) {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();
    command(temp.path())
        .args([
            "--host",
            "192.0.2.1",
            "--user",
            "admin",
            "--password",
            "secret",
            "--protocol",
            protocol,
            "--jump-host",
            "192.0.2.1",
            "--jump-port",
            "1",
            "--jump-user",
            "ops",
            "--jump-host-key",
            "SHA256:test",
            "--jump-password",
            "jump-secret",
            "--connect-timeout-seconds",
            "1",
            "--json",
            "ip",
            "address",
            "print",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("NETWORK_ERROR"));
}

#[test]
fn api_command_via_unreachable_jump_returns_network_error() {
    command_via_unreachable_jump_fails_with_network_error("api");
}

#[test]
fn rest_command_via_unreachable_jump_returns_network_error() {
    command_via_unreachable_jump_fails_with_network_error("rest");
}

#[test]
fn api_ssl_command_via_unreachable_jump_returns_network_error() {
    command_via_unreachable_jump_fails_with_network_error("api-ssl");
}

#[test]
fn auto_command_via_unreachable_jump_returns_network_error() {
    command_via_unreachable_jump_fails_with_network_error("auto");
}

#[test]
fn transfer_upload_via_unreachable_jump_returns_network_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    let local = temp.path().join("setup.rsc");
    fs::write(&local, b"/system identity print\n").expect("file");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();
    command(temp.path())
        .args([
            "file",
            "upload",
            local.to_str().expect("path"),
            "flash/setup.rsc",
            "--host",
            "192.0.2.1",
            "--user",
            "admin",
            "--password",
            "secret",
            "--ssh-host-key",
            "SHA256:router",
            "--jump-host",
            "192.0.2.1",
            "--jump-port",
            "1",
            "--jump-user",
            "ops",
            "--jump-host-key",
            "SHA256:bastion",
            "--jump-password",
            "jump-secret",
            "--allow-from",
            "203.0.113.10/32",
            "--connect-timeout-seconds",
            "1",
            "--json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("NETWORK_ERROR"));
}

#[test]
fn transfer_dry_run_includes_jump_via() {
    let temp = tempfile::tempdir().expect("temp dir");
    command(temp.path())
        .args(["config", "init", "--json"])
        .assert()
        .success();

    command(temp.path())
        .args([
            "file",
            "upload",
            "/Users/example/private/setup.rsc",
            "flash/setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:router",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--jump-host-key",
            "SHA256:bastion",
            "--allow-from",
            "203.0.113.10/32",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"jump\""))
        .stdout(predicate::str::contains("\"local_bind\": false"))
        .stdout(predicate::str::contains("/Users/example").not());
}
