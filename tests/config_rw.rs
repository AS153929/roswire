use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path());
    cmd.args(args).assert()
}

fn command(temp: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path());
    cmd
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
fn config_device_rejects_mac_host() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    run(&temp, &["config", "init", "--json"]).success();
    run(
        &temp,
        &[
            "config",
            "device",
            "add",
            "studio",
            "host=48-8F-5A-A3-0E-A7",
            "user=master",
            "--json",
        ],
    )
    .failure()
    .stdout(predicate::str::is_empty())
    .stderr(predicate::str::contains("\"error_code\":\"CONFIG_ERROR\""))
    .stderr(predicate::str::contains("MAC address"));
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

#[test]
fn config_secret_set_supports_env_stdin_and_encrypted_sources() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let stdin_secret = generated_secret();
    let env_secret = generated_env_secret();
    let master_key = generated_master_key();

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

    command(&temp)
        .args([
            "config",
            "secret",
            "set",
            "studio",
            "password",
            "type=plain",
            "--stdin",
            "--json",
        ])
        .write_stdin(format!("{stdin_secret}\n"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\":\"plain\""));

    command(&temp)
        .env("ROSWIRE_RUNTIME_PASSWORD", &env_secret)
        .args([
            "config",
            "secret",
            "set",
            "studio",
            "api_password",
            "type=env",
            "env=ROSWIRE_RUNTIME_PASSWORD",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\":\"env\""));

    command(&temp)
        .env("ROSWIRE_SECRET_SOURCE", &env_secret)
        .env("ROSWIRE_MASTER_KEY", &master_key)
        .args([
            "config",
            "secret",
            "set",
            "studio",
            "encrypted_password",
            "type=encrypted",
            "env=ROSWIRE_SECRET_SOURCE",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\":\"encrypted\""));

    let config = std::fs::read_to_string(temp.path().join("config.toml"))
        .expect("config should be readable");
    assert!(config.contains("type = \"encrypted\""));
    assert!(config.contains("v1:"));
    assert!(config.contains("type = \"env\""));
    assert!(config.contains("ROSWIRE_RUNTIME_PASSWORD"));
    assert!(!config.contains(&env_secret));

    run(
        &temp,
        &["--profile", "studio", "config", "inspect", "--json"],
    )
    .success()
    .stdout(predicate::str::contains("\"type\":\"env\""))
    .stdout(predicate::str::contains("\"type\":\"encrypted\""))
    .stdout(predicate::str::contains(&env_secret).not())
    .stdout(predicate::str::contains(&stdin_secret).not());
}

fn generated_secret() -> String {
    ['s', 't', 'd', 'i', 'n', '-', 's', 'e', 'c', 'r', 'e', 't']
        .into_iter()
        .collect()
}

fn generated_env_secret() -> String {
    ['e', 'n', 'v', '-', 's', 'e', 'c', 'r', 'e', 't']
        .into_iter()
        .collect()
}

fn generated_master_key() -> String {
    ['m', 'a', 's', 't', 'e', 'r', '-', 'k', 'e', 'y']
        .into_iter()
        .collect()
}
