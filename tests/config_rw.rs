use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path());
    cmd.args(args).assert()
}

#[test]
fn config_init_creates_home_and_config_file() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    run(&temp, &["config", "init", "--json"])
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.config.init.v1\"",
        ));

    assert!(temp.path().join("config.toml").exists());
    assert!(temp.path().join("logs").exists());
}

#[test]
fn config_device_add_and_inspect_work() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    run(&temp, &["config", "init", "--json"]).success();
    run(
        &temp,
        &[
            "config",
            "device",
            "add",
            "studio",
            "host=10.189.189.1",
            "user=master",
            "protocol=auto",
            "transfer=ssh",
            "--json",
        ],
    )
    .success()
    .stdout(predicate::str::contains(
        "\"schema_version\":\"roswire.config.device.v1\"",
    ));

    run(&temp, &["config", "profiles", "--json"])
        .success()
        .stdout(predicate::str::contains("\"studio\""));

    run(
        &temp,
        &["--profile", "studio", "config", "inspect", "--json"],
    )
    .success()
    .stdout(predicate::str::contains("\"active_profile\":\"studio\""))
    .stdout(predicate::str::contains("\"10.189.189.1\""));
}

#[test]
fn config_secret_set_supports_multiple_types_and_redacts_values() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    run(&temp, &["config", "init", "--json"]).success();
    run(
        &temp,
        &[
            "config",
            "device",
            "add",
            "studio",
            "host=10.189.189.1",
            "user=master",
            "--json",
        ],
    )
    .success();

    run(
        &temp,
        &[
            "config",
            "secret",
            "set",
            "studio",
            "password",
            "type=plain",
            "value=All.007!",
            "--json",
        ],
    )
    .success()
    .stdout(predicate::str::contains(
        "\"schema_version\":\"roswire.config.secret.v1\"",
    ))
    .stdout(predicate::str::contains("\"type\":\"plain\""));

    run(
        &temp,
        &[
            "secret",
            "set",
            "studio",
            "ssh_password",
            "type=same-as",
            "target=password",
            "--json",
        ],
    )
    .success()
    .stdout(predicate::str::contains("\"type\":\"same-as\""));

    run(
        &temp,
        &[
            "config",
            "secret",
            "set",
            "studio",
            "keychain_pass",
            "type=keychain",
            "service=roswire",
            "account=profiles/studio/password",
            "--json",
        ],
    )
    .success()
    .stdout(predicate::str::contains("\"type\":\"keychain\""));

    run(
        &temp,
        &["--profile", "studio", "config", "inspect", "--json"],
    )
    .success()
    .stdout(predicate::str::contains("\"secrets\""))
    .stdout(predicate::str::contains("\"password\""))
    .stdout(predicate::str::contains("\"redacted\":true"))
    .stdout(predicate::str::contains("\"type\":\"plain\""))
    .stdout(predicate::str::contains("\"type\":\"keychain\""))
    .stdout(predicate::str::contains("\"allow_plain_secrets\":true").not())
    .stdout(predicate::str::contains("All.007!").not());
}
