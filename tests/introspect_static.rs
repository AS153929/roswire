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
        .stdout(predicate::str::contains("ip firewall address-list print"))
        .stdout(predicate::str::contains("ip firewall filter print"))
        .stdout(predicate::str::contains("ip firewall nat print"))
        .stdout(predicate::str::contains("ip route print"))
        .stdout(predicate::str::contains("interface wireguard print"))
        .stdout(predicate::str::contains("interface wireguard peers print"))
        .stdout(predicate::str::contains("system package print"))
        .stdout(predicate::str::contains("script put"))
        .stdout(predicate::str::contains("raw"))
        .stdout(predicate::str::contains("tool mac-server print"))
        .stdout(predicate::str::contains("tool netwatch print"))
        .stdout(predicate::str::contains("user print"))
        .stdout(predicate::str::contains("doctor"));
}

#[test]
fn help_index_lists_refresh_option() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--refresh"));
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
fn help_system_package_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "system", "package", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains(
            "\"name\":\"system package print\"",
        ))
        .stdout(predicate::str::contains(
            "Print installed RouterOS packages.",
        ));
}

#[test]
fn help_script_put_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "script", "put", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"script put\""))
        .stdout(predicate::str::contains("--source"))
        .stdout(predicate::str::contains("without creating a RouterOS file"));
}

#[test]
fn help_raw_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "raw", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"raw\""))
        .stdout(predicate::str::contains("--allow-write"))
        .stdout(predicate::str::contains("classic API path"));
}

#[test]
fn help_user_print_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "user", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"user print\""))
        .stdout(predicate::str::contains("without exposing password"));
}

#[test]
fn help_ip_route_print_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "ip", "route", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"ip route print\""))
        .stdout(predicate::str::contains("v6/v7 route table fields"));
}

#[test]
fn help_firewall_print_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "ip", "firewall", "filter", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains(
            "\"name\":\"ip firewall filter print\"",
        ))
        .stdout(predicate::str::contains("without changing packet handling"));
}

#[test]
fn help_tool_print_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "tool", "netwatch", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains("\"name\":\"tool netwatch print\""))
        .stdout(predicate::str::contains("does not run ad-hoc probes"));
}

#[test]
fn help_wireguard_print_returns_command_details() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["help", "interface", "wireguard", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.help.v1\"",
        ))
        .stdout(predicate::str::contains(
            "\"name\":\"interface wireguard print\"",
        ))
        .stdout(predicate::str::contains("without exposing private keys"));

    let mut peers = Command::cargo_bin("roswire").expect("binary should compile");
    peers
        .args(["help", "interface", "wireguard", "peers", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"name\":\"interface wireguard peers print\"",
        ))
        .stdout(predicate::str::contains("without exposing preshared keys"));
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
fn schema_system_package_print_is_registered() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "system", "package", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.schema.v1\"",
        ))
        .stdout(predicate::str::contains(
            "\"command\":\"system package print\"",
        ))
        .stdout(predicate::str::contains("\"arguments\":[]"));
}

#[test]
fn schema_script_put_is_registered() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "script", "put", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"command\":\"script put\""))
        .stdout(predicate::str::contains("\"name\":\"--source\""));
}

#[test]
fn schema_raw_is_registered() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "raw", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"command\":\"raw\""))
        .stdout(predicate::str::contains("routeros-path"))
        .stdout(predicate::str::contains("--allow-write"));
}

#[test]
fn schema_user_print_is_registered() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "user", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"command\":\"user print\""))
        .stdout(predicate::str::contains("\"arguments\":[]"));
}

#[test]
fn schema_ip_route_print_is_registered() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args(["schema", "command", "ip", "route", "print", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.command.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"command\":\"ip route print\""))
        .stdout(predicate::str::contains("\"arguments\":[]"));
}

#[test]
fn schema_firewall_prints_are_registered() {
    for topic in [
        ["ip", "firewall", "address-list", "print"],
        ["ip", "firewall", "filter", "print"],
        ["ip", "firewall", "nat", "print"],
    ] {
        let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
        cmd.args(["schema", "command"])
            .args(topic)
            .arg("--json")
            .assert()
            .success()
            .stdout(predicate::str::contains("\"arguments\":[]"));
    }
}

#[test]
fn schema_tool_prints_are_registered() {
    for topic in [
        ["tool", "mac-server", "print"],
        ["tool", "netwatch", "print"],
    ] {
        let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
        cmd.args(["schema", "command"])
            .args(topic)
            .arg("--json")
            .assert()
            .success()
            .stdout(predicate::str::contains("\"arguments\":[]"));
    }
}

#[test]
fn schema_wireguard_prints_are_registered() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.args([
        "schema",
        "command",
        "interface",
        "wireguard",
        "print",
        "--json",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "\"command\":\"interface wireguard print\"",
    ))
    .stdout(predicate::str::contains("\"arguments\":[]"));

    let mut peers = Command::cargo_bin("roswire").expect("binary should compile");
    peers
        .args([
            "schema",
            "command",
            "interface",
            "wireguard",
            "peers",
            "print",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"command\":\"interface wireguard peers print\"",
        ))
        .stdout(predicate::str::contains("\"arguments\":[]"));
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

    let mut restore = Command::cargo_bin("roswire").expect("binary should compile");
    restore
        .args(["explain-error", "SSH_RESTORE_FAILED", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"error_code\":\"SSH_RESTORE_FAILED\"",
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
        .stdout(predicate::str::contains("\"status\":\"miss\""))
        .stdout(predicate::str::contains("CONFIG_ERROR"))
        .stdout(predicate::str::contains("ip address print"));
}

#[test]
fn schema_discover_remote_refresh_marks_cache_status() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args(["schema", "discover", "--remote", "--refresh", "--json"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains(
            "\"schema_version\":\"roswire.remote.schema.v1\"",
        ))
        .stdout(predicate::str::contains("\"status\":\"refresh\""))
        .stdout(predicate::str::contains("\"runtime_value_hints\""))
        .stdout(predicate::str::contains("not_exhaustive"));
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

#[test]
fn schema_command_remote_system_package_has_static_fields() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "schema", "command", "system", "package", "print", "--remote", "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains(
        "\"name\":\"system package print\"",
    ))
    .stdout(predicate::str::contains("\"output_fields_observed\""))
    .stdout(predicate::str::contains("\"version\""))
    .stdout(predicate::str::contains("\"support\":\"unknown\""));
}

#[test]
fn schema_command_remote_user_has_static_fields() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args(["schema", "command", "user", "print", "--remote", "--json"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains("\"name\":\"user print\""))
        .stdout(predicate::str::contains("\"group\""))
        .stdout(predicate::str::contains("\"last-logged-in\""))
        .stdout(predicate::str::contains("\"support\":\"unknown\""));
}

#[test]
fn schema_command_remote_ip_route_has_static_fields() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "schema", "command", "ip", "route", "print", "--remote", "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains("\"name\":\"ip route print\""))
    .stdout(predicate::str::contains("\"dst-address\""))
    .stdout(predicate::str::contains("\"routing-table\""))
    .stdout(predicate::str::contains("\"support\":\"unknown\""));
}

#[test]
fn schema_command_remote_firewall_has_static_fields() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "schema", "command", "ip", "firewall", "nat", "print", "--remote", "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains(
        "\"name\":\"ip firewall nat print\"",
    ))
    .stdout(predicate::str::contains("\"chain\""))
    .stdout(predicate::str::contains("\"to-addresses\""))
    .stdout(predicate::str::contains("static_catalog_hint"));
}

#[test]
fn schema_command_remote_tool_has_static_fields() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "schema", "command", "tool", "netwatch", "print", "--remote", "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains("\"name\":\"tool netwatch print\""))
    .stdout(predicate::str::contains("\"status\""))
    .stdout(predicate::str::contains("\"support\":\"unknown\""));
}

#[test]
fn schema_command_remote_wireguard_has_static_fields_without_private_material() {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    let temp = tempfile::tempdir().expect("temp dir should be created");
    cmd.env("ROSWIRE_HOME", temp.path().join("missing-home"));
    cmd.args([
        "schema",
        "command",
        "interface",
        "wireguard",
        "peers",
        "print",
        "--remote",
        "--json",
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty())
    .stdout(predicate::str::contains(
        "\"name\":\"interface wireguard peers print\"",
    ))
    .stdout(predicate::str::contains("\"public-key\""))
    .stdout(predicate::str::contains("\"preshared-key\"").not())
    .stdout(predicate::str::contains("\"private-key\"").not());
}

fn generated_credential() -> String {
    ['a', 'p', 'i', '-', 's', 's', 'l', '-', 'c', 'r', 'e', 'd']
        .into_iter()
        .collect()
}
