//! The command's exit codes and the check name it prints.

mod common;

use std::process::Command;

use common::*;
use serde_json::json;

const BIN: &str = env!("CARGO_BIN_EXE_commons-release-verify");

/// Writes a signed manifest and its key; returns the directory.
fn signed(name: &str, m: &serde_json::Value) -> std::path::PathBuf {
    let dir = scratch(name);
    let key = TestKey::hardware("owner", 0x01);
    let b = bytes(m);
    std::fs::write(dir.join("manifest.json"), &b).unwrap();
    std::fs::write(dir.join("manifest.json.sig"), key.sign(&b)).unwrap();
    std::fs::write(dir.join("stable-1.pub"), key.openssh_line() + "\n").unwrap();
    dir
}

fn run(dir: &std::path::Path, extra: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .arg("--key")
        .arg(dir.join("stable-1.pub"))
        .args(["--now", NOW, "--channel", "beta"])
        .args(extra)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_verified_manifest_exits_0() {
    let dir = signed("cli-ok", &manifest());
    let assets = assets_dir("cli-ok-assets");
    let (code, out, err) = run(
        &dir,
        &["--last-sequence", "7", "--assets", assets.to_str().unwrap()],
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("assets-checked 2 of 2"), "{out}");
}

#[test]
fn a_failed_check_exits_20_and_names_the_check() {
    let mut m = manifest();
    m["sets"][0]["components"]["node"]["version"] = json!("0.9.47");
    let dir = signed("cli-set-version", &m);
    let (code, _, err) = run(&dir, &[]);
    assert_eq!(code, 20);
    assert!(
        err.starts_with("verification failed: set-version: "),
        "{err}"
    );

    let dir = signed("cli-sequence", &manifest());
    let (code, _, err) = run(&dir, &["--last-sequence", "8"]);
    assert_eq!(code, 20);
    assert!(err.starts_with("verification failed: sequence: "), "{err}");
}

#[test]
fn a_missing_file_exits_1() {
    let dir = signed("cli-missing", &manifest());
    std::fs::remove_file(dir.join("manifest.json.sig")).unwrap();
    let (code, _, err) = run(&dir, &[]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: reading "), "{err}");
}
