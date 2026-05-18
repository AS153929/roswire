use assert_cmd::Command;
use predicates::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

const SERVICE: &str = "roswire-ci-smoke";

fn command(temp: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("roswire").expect("binary should compile");
    cmd.env("ROSWIRE_HOME", temp.path());
    cmd
}

fn smoke_account() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    format!(
        "profiles/ci/keychain-smoke/{}/{}",
        std::process::id(),
        nanos
    )
}

fn cleanup_keychain(account: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        let _ = entry.delete_password();
    }
}

#[test]
#[ignore = "requires an unlocked OS keychain / credential store"]
fn native_keychain_roundtrip_redacts_inspect_output() {
    if std::env::var("ROSWIRE_KEYCHAIN_SMOKE").as_deref() != Ok("native") {
        eprintln!("set ROSWIRE_KEYCHAIN_SMOKE=native to run this smoke test");
        return;
    }

    let temp = tempfile::tempdir().expect("temp dir should be created");
    let account = smoke_account();
    let secret = format!("keychain-smoke-secret-{}", std::process::id());

    let result = std::panic::catch_unwind(|| {
        command(&temp)
            .args(["config", "init", "--json"])
            .assert()
            .success();
        command(&temp)
            .args([
                "config",
                "device",
                "add",
                "studio",
                "host=10.189.189.1",
                "user=master",
                "--json",
            ])
            .assert()
            .success();

        let account_arg = format!("account={account}");
        let value_arg = format!("value={secret}");
        command(&temp)
            .args([
                "config",
                "secret",
                "set",
                "studio",
                "password",
                "type=keychain",
                "service=roswire-ci-smoke",
                &account_arg,
                &value_arg,
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"type\":\"keychain\""))
            .stdout(predicate::str::contains(&secret).not());

        let entry = keyring::Entry::new(SERVICE, &account).expect("keychain entry should open");
        let stored = entry
            .get_password()
            .expect("keychain secret should be readable");
        assert_eq!(stored, secret);

        command(&temp)
            .args(["--profile", "studio", "config", "inspect", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"secrets\""))
            .stdout(predicate::str::contains("\"type\":\"keychain\""))
            .stdout(predicate::str::contains("\"redacted\":true"))
            .stdout(predicate::str::contains(&secret).not())
            .stdout(predicate::str::contains(&account).not());
    });

    cleanup_keychain(&account);

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[test]
#[ignore = "validates documented fallback coverage for platforms without CI keychain sessions"]
fn documented_fallbacks_cover_linux_and_windows() {
    if std::env::var("ROSWIRE_KEYCHAIN_SMOKE").as_deref() != Ok("documented-fallback") {
        eprintln!("set ROSWIRE_KEYCHAIN_SMOKE=documented-fallback to run this smoke test");
        return;
    }

    let docs = include_str!("../docs/keychain-smoke.md");

    assert!(docs.contains("Linux"));
    assert!(docs.contains("Secret Service"));
    assert!(docs.contains("Windows"));
    assert!(docs.contains("Credential Manager"));
    assert!(docs.contains("SECRET_BACKEND_UNAVAILABLE"));
    assert!(docs.contains("Documented fallback"));
    assert!(docs.contains("ROSWIRE_KEYCHAIN_SMOKE=native"));
}
