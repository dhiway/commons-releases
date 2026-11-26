//! The release scripts in `scripts/`, run as their users run them:
//!
//! - `check-release-profile.py` refuses each way a release profile can be
//!   weakened, and `build-cmns.sh` stops on it before building;
//! - `check-stripped.sh` reads the binary, not cargo's output;
//! - `sign-manifest.sh` handles a key rotation's three key files, refuses a
//!   sequence that was not raised and accepts relative paths.
//!
//! They need `python3` (3.11 or later), `git`, `ssh-keygen`, `cc` and `strip`,
//! and on a Mac `otool` and `nm`, on Linux `readelf`. A missing tool fails a
//! test; nothing is skipped.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::*;
use serde_json::{json, Value};

const VERIFY_BIN: &str = env!("CARGO_BIN_EXE_commons-release-verify");

fn scripts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts")
}

fn run(cmd: &mut Command) -> (i32, String) {
    let out: Output = cmd
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("running {cmd:?}: {e}"));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

// ---- check-release-profile.py -------------------------------------------

const GOOD_PROFILE: &str = "\
[workspace]
members = []

[profile.release]
lto = true
codegen-units = 1
strip = true
";

/// Runs the profile check on `dir`, with a Cargo home that holds nothing
/// unless the test puts something there.
fn profile_check(dir: &Path, cargo_home: &Path) -> (i32, String) {
    run(Command::new("python3")
        .arg(scripts().join("check-release-profile.py"))
        .arg(dir)
        .env("CARGO_HOME", cargo_home))
}

/// A workspace with `extra` appended to the good profile's Cargo.toml.
fn workspace(name: &str, extra: &str) -> (PathBuf, PathBuf) {
    let root = scratch(name);
    let ws = root.join("ws");
    write(&ws.join("Cargo.toml"), &format!("{GOOD_PROFILE}{extra}"));
    let home = root.join("cargo-home");
    std::fs::create_dir_all(&home).unwrap();
    (ws, home)
}

fn assert_refused(name: &str, extra: &str, says: &str) {
    let (ws, home) = workspace(name, extra);
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 1, "{name}: {out}");
    assert!(out.contains(says), "{name}: expected {says:?} in: {out}");
}

#[test]
fn the_optimised_release_profile_passes() {
    let (ws, home) = workspace("profile-good", "opt-level = 3\ndebug-assertions = false\n");
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 0, "{out}");
    // The plain profile, as commons-node and commons-app set it, passes too.
    let (ws, home) = workspace("profile-plain", "");
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_profile_missing_lto_codegen_units_or_strip_is_refused() {
    for (key, value) in [
        ("lto", "\"thin\""),
        ("codegen-units", "16"),
        ("strip", "false"),
    ] {
        let toml = GOOD_PROFILE.replace(
            &format!(
                "{key} = {}",
                if key == "codegen-units" { "1" } else { "true" }
            ),
            &format!("{key} = {value}"),
        );
        let root = scratch(&format!("profile-{key}"));
        write(&root.join("ws/Cargo.toml"), &toml);
        std::fs::create_dir_all(root.join("home")).unwrap();
        let (code, out) = profile_check(&root.join("ws"), &root.join("home"));
        assert_eq!(code, 1, "{key}: {out}");
        assert!(out.contains(&format!("does not set {key}")), "{out}");
    }
}

#[test]
fn an_opt_level_other_than_3_is_refused() {
    assert_refused("profile-opt0", "opt-level = 0\n", "opt-level = 0");
    assert_refused("profile-opt-z", "opt-level = \"z\"\n", "opt-level = \"z\"");
}

#[test]
fn debug_assertions_on_are_refused() {
    assert_refused(
        "profile-debug-assertions",
        "debug-assertions = true\n",
        "debug-assertions = true",
    );
}

#[test]
fn overflow_checks_on_are_refused() {
    assert_refused(
        "profile-overflow-checks",
        "overflow-checks = true\n",
        "overflow-checks = true",
    );
}

#[test]
fn a_per_package_override_is_refused() {
    assert_refused(
        "profile-package-star",
        "\n[profile.release.package.\"*\"]\nopt-level = 0\n",
        "per-package override",
    );
    // One crate is enough: an override of any value is refused.
    assert_refused(
        "profile-package-one",
        "\n[profile.release.package.ring]\nopt-level = 3\n",
        "per-package override",
    );
}

#[test]
fn a_release_profile_in_a_cargo_config_file_is_refused() {
    let weak = "[profile.release]\nopt-level = 0\n";
    // In the workspace's own .cargo/, in a parent's, and in the Cargo home.
    let (ws, home) = workspace("config-own", "");
    write(&ws.join(".cargo/config.toml"), weak);
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("sets profile.release"), "{out}");

    let (ws, home) = workspace("config-parent", "");
    write(&ws.parent().unwrap().join(".cargo/config"), weak);
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("sets profile.release"), "{out}");

    let (ws, home) = workspace("config-home", "");
    write(&home.join("config.toml"), weak);
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("sets profile.release"), "{out}");

    // A config file that sets something else is not a profile.
    let (ws, home) = workspace("config-other", "");
    write(&ws.join(".cargo/config.toml"), "[build]\njobs = 4\n");
    let (code, out) = profile_check(&ws, &home);
    assert_eq!(code, 0, "{out}");
}

fn git(dir: &Path, args: &[&str]) {
    let (code, out) = run(Command::new("git")
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir));
    assert_eq!(code, 0, "git {args:?}: {out}");
}

/// `build-cmns.sh` runs the profile check on the commit it builds and stops
/// before cargo runs.
#[test]
fn build_cmns_refuses_a_weakened_profile_before_building() {
    let root = scratch("build-cmns-weak");
    let src = root.join("src");
    write(
        &src.join("Cargo.toml"),
        &format!("{GOOD_PROFILE}debug-assertions = true\n"),
    );
    git(&src, &["init", "-q"]);
    git(&src, &["add", "Cargo.toml"]);
    git(&src, &["commit", "-qm", "weak profile"]);
    let home = root.join("cargo-home");
    std::fs::create_dir_all(&home).unwrap();
    let (code, out) = run(Command::new("bash")
        .arg(scripts().join("build-cmns.sh"))
        .args(["--src", src.to_str().unwrap(), "--commit", "HEAD", "--out"])
        .arg(root.join("out"))
        .env("CARGO_HOME", &home)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS"));
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("debug-assertions = true"), "{out}");
    assert!(
        out.contains("the release profile is not the optimised one"),
        "{out}"
    );
    assert!(!root.join("out/target").exists(), "cargo ran: {out}");
}

// ---- check-stripped.sh ----------------------------------------------------

fn strip_check(binary: &Path) -> (i32, String) {
    run(Command::new("bash")
        .arg(scripts().join("check-stripped.sh"))
        .arg(binary))
}

/// A small C program with a local function, built with debug information,
/// then stripped of debug information only, then fully.
#[test]
fn only_a_fully_stripped_binary_passes_the_strip_check() {
    let dir = scratch("strip-check");
    let c = dir.join("p.c");
    write(
        &c,
        "static int helper(int x) { return x + 1; }\nint main(int c, char **v) { (void)v; return helper(c); }\n",
    );
    let cc = |args: &[&str], out: &str| {
        let (code, text) = run(Command::new("cc")
            .args(args)
            .arg(&c)
            .arg("-o")
            .arg(dir.join(out)));
        assert_eq!(code, 0, "cc: {text}");
        dir.join(out)
    };
    let copy_and_strip = |from: &Path, to: &str, args: &[&str]| {
        let to = dir.join(to);
        std::fs::copy(from, &to).unwrap();
        let (code, text) = run(Command::new("strip").args(args).arg(&to));
        assert_eq!(code, 0, "strip {args:?}: {text}");
        to
    };
    let mac = cfg!(target_os = "macos");

    let debug = cc(&["-g", "-O0"], "p");
    let (code, out) = strip_check(&debug);
    assert_eq!(code, 1, "{out}");
    let expect = if mac { "debug-map (stab)" } else { ".debug_" };
    assert!(out.contains(expect), "{out}");

    // Debug information gone, symbols kept: still not stripped.
    let symbols = copy_and_strip(&debug, "p-debug-stripped", &["-S"]);
    let (code, out) = strip_check(&symbols);
    assert_eq!(code, 1, "{out}");
    let expect = if mac { "local symbols" } else { ".symtab" };
    assert!(out.contains(expect), "{out}");

    let stripped = copy_and_strip(&debug, "p-stripped", &[]);
    let (code, out) = strip_check(&stripped);
    assert_eq!(code, 0, "{out}");

    // An object file carries its debug information in sections.
    let object = cc(&["-g", "-O0", "-c"], "p.o");
    let (code, out) = strip_check(&object);
    assert_eq!(code, 1, "{out}");
    let expect = if mac { "__DWARF" } else { ".debug_" };
    assert!(out.contains(expect), "{out}");

    let (code, out) = strip_check(&c);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("neither a Mach-O nor an ELF"), "{out}");
}

// ---- sign-manifest.sh -----------------------------------------------------

/// A copy of the repository's layout with `dev-<n>` keys for each number:
/// the script, `keys/dev-<n>.pub`, and `dev/manifest.json` at `sequence`.
/// Returns the repository and the directory holding the private keys.
fn signing_repo(name: &str, keys: &[u32], sequence: u64) -> (PathBuf, PathBuf) {
    let root = scratch(name);
    let repo = root.join("repo");
    let private = root.join("private");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::create_dir_all(repo.join("scripts")).unwrap();
    std::fs::copy(
        scripts().join("sign-manifest.sh"),
        repo.join("scripts/sign-manifest.sh"),
    )
    .unwrap();
    std::fs::create_dir_all(repo.join("keys")).unwrap();
    for n in keys {
        let key = private.join(format!("dev-{n}"));
        let (code, out) = run(Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
            .arg(&key));
        assert_eq!(code, 0, "ssh-keygen: {out}");
        std::fs::copy(
            private.join(format!("dev-{n}.pub")),
            repo.join(format!("keys/dev-{n}.pub")),
        )
        .unwrap();
    }
    write_manifest(&repo, sequence);
    git(&repo, &["init", "-q"]);
    (repo, private)
}

/// `manifest()` on the dev channel, fresh by the real clock.
fn write_manifest(repo: &Path, sequence: u64) {
    let mut m: Value = manifest();
    m["channel"] = json!("dev");
    m["published"] = json!("2020-01-01T00:00:00Z");
    m["expires"] = json!("2099-01-01T00:00:00Z");
    m["sequence"] = json!(sequence);
    write(
        &repo.join("dev/manifest.json"),
        &String::from_utf8(bytes(&m)).unwrap(),
    );
}

fn sign(repo: &Path, cwd: &Path, args: &[&str], verify: &Path) -> (i32, String) {
    run(Command::new("bash")
        .arg(repo.join("scripts/sign-manifest.sh"))
        .args(args)
        .current_dir(cwd)
        .env("COMMONS_RELEASE_VERIFY", verify))
}

/// During a rotation keys 1, 2 and 3 exist: older clients carry 1 and 2,
/// newer ones 2 and 3. Only key 2 is trusted by both, so only its signature
/// is accepted, and the script no longer refuses three key files.
#[test]
fn sign_manifest_during_a_rotation_accepts_only_the_key_every_client_trusts() {
    let (repo, private) = signing_repo("sign-rotation", &[1, 2, 3], 7);
    let sig = repo.join("dev/manifest.json.sig");
    let verify = Path::new(VERIFY_BIN);

    let key = |n: u32| private.join(format!("dev-{n}")).display().to_string();
    let (code, out) = sign(&repo, &repo, &["dev", &key(2)], verify);
    assert_eq!(code, 0, "{out}");
    assert!(sig.is_file());

    let (code, out) = sign(&repo, &repo, &["dev", &key(3)], verify);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("keys/dev-{1,2}.pub"), "{out}");
    assert!(!sig.exists(), "the refused signature was left behind");

    let (code, out) = sign(&repo, &repo, &["dev", &key(1)], verify);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("keys/dev-{2,3}.pub"), "{out}");
    assert!(!sig.exists(), "the refused signature was left behind");
}

/// A stand-in for the verifier that records its arguments, then runs it.
fn recording_verifier(dir: &Path) -> (PathBuf, PathBuf) {
    let log = dir.join("verify-args");
    let wrapper = dir.join("verify");
    write(
        &wrapper,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{VERIFY_BIN}' \"$@\"\n",
            log.display()
        ),
    );
    let (code, out) = run(Command::new("chmod").arg("+x").arg(&wrapper));
    assert_eq!(code, 0, "{out}");
    (wrapper, log)
}

#[test]
fn sign_manifest_refuses_a_sequence_that_was_not_raised() {
    let (repo, private) = signing_repo("sign-sequence", &[1], 7);
    let key = private.join("dev-1").display().to_string();
    let (verify, log) = recording_verifier(repo.parent().unwrap());

    // Publish sequence 7: sign it and commit it with its signature.
    let (code, out) = sign(&repo, &repo, &["dev", &key], &verify);
    assert_eq!(code, 0, "{out}");
    git(
        &repo,
        &["add", "dev/manifest.json", "dev/manifest.json.sig"],
    );
    git(&repo, &["commit", "-qm", "dev sequence 7"]);
    let published_sig = std::fs::read(repo.join("dev/manifest.json.sig")).unwrap();

    // Edited but still sequence 7: refused before anything is signed.
    let mut m: Value =
        serde_json::from_slice(&std::fs::read(repo.join("dev/manifest.json")).unwrap()).unwrap();
    m["expires"] = json!("2098-01-01T00:00:00Z");
    write(
        &repo.join("dev/manifest.json"),
        &String::from_utf8(bytes(&m)).unwrap(),
    );
    let (code, out) = sign(&repo, &repo, &["dev", &key], &verify);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("has sequence 7, but HEAD's has 7"), "{out}");
    assert_eq!(
        std::fs::read(repo.join("dev/manifest.json.sig")).unwrap(),
        published_sig,
        "the published signature was touched"
    );

    // Raised to 8: signed, and the verifier was told the floor is 8.
    m["sequence"] = json!(8);
    write(
        &repo.join("dev/manifest.json"),
        &String::from_utf8(bytes(&m)).unwrap(),
    );
    std::fs::remove_file(&log).unwrap();
    let (code, out) = sign(&repo, &repo, &["dev", &key], &verify);
    assert_eq!(code, 0, "{out}");
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(
        args.contains("--last-sequence 8"),
        "verifier ran with: {args}"
    );
}

/// The key and assets directory are named relative to where the owner
/// stands, not to the repository root the script moves to.
#[test]
fn sign_manifest_resolves_relative_paths_from_where_it_is_run() {
    let (repo, _) = signing_repo("sign-relative", &[1], 7);
    let elsewhere = repo.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::rename(assets_dir("sign-relative-assets"), elsewhere.join("assets")).unwrap();
    // The private key moves next to the assets, so both paths are relative.
    std::fs::rename(
        repo.parent().unwrap().join("private/dev-1"),
        elsewhere.join("dev-1"),
    )
    .unwrap();
    let (code, out) = sign(
        &repo,
        &elsewhere,
        &["dev", "dev-1", "assets"],
        Path::new(VERIFY_BIN),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("assets-checked 2 of 2"), "{out}");
}
