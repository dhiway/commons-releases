//! One test per check: each builds a manifest that only that check rejects,
//! so removing the check from `verify` turns the test red.

mod common;

use common::*;
use std::path::Path;

use commons_release_verify::{verify, Check, Error, Options, TrustedKeys, Verified};
use serde_json::{json, Value};

fn failed_check(result: Result<Verified, Error>) -> Check {
    match result {
        Err(Error::Failed { check, .. }) => check,
        Err(Error::Io(e)) => panic!("expected a failed check, got an I/O error: {e}"),
        Ok(v) => panic!("expected a failed check, but the manifest verified: {v:?}"),
    }
}

/// Signs `manifest` with a file key and verifies it with the default options.
fn run(manifest: &Value) -> Result<Verified, Error> {
    let key = TestKey::file("ci");
    let b = bytes(manifest);
    verify(&b, &key.sign(&b), &options(&key.trusted()))
}

/// As `run`, also checking the assets in `dir`.
fn run_with_assets(manifest: &Value, dir: &Path) -> Result<Verified, Error> {
    let key = TestKey::file("ci");
    let keys = key.trusted();
    let b = bytes(manifest);
    verify(
        &b,
        &key.sign(&b),
        &Options {
            assets_dir: Some(dir),
            ..options(&keys)
        },
    )
}

#[test]
fn a_good_manifest_verifies_with_its_assets() {
    let v = run_with_assets(&manifest(), &assets_dir("good")).unwrap();
    assert_eq!((v.assets_checked, v.assets_listed), (2, 2));
    assert_eq!(v.signed_by, TestKey::file("ci").public().fingerprint());
    assert!(!v.signed_by_next);
}

#[test]
fn a_manifest_signed_on_a_hardware_key_verifies() {
    let key = TestKey::hardware("owner", 0x01);
    let b = bytes(&manifest());
    verify(&b, &key.sign(&b), &options(&key.trusted())).unwrap();
}

#[test]
fn one_tampered_byte_fails_the_signature() {
    let key = TestKey::file("ci");
    let mut b = bytes(&manifest());
    let sig = key.sign(&b);
    // `"sequence": 7` becomes `"sequence": 9`: still valid JSON and still a
    // valid manifest, so only the signature can catch it.
    let at = b.windows(12).position(|w| w == b"\"sequence\": ").unwrap() + 12;
    assert_eq!(b[at], b'7');
    b[at] = b'9';
    assert_eq!(
        failed_check(verify(&b, &sig, &options(&key.trusted()))),
        Check::Signature
    );
}

#[test]
fn a_signature_by_an_untrusted_key_fails() {
    let signer = TestKey::file("someone else");
    let b = bytes(&manifest());
    let trusted = TestKey::file("ci").trusted();
    assert_eq!(
        failed_check(verify(&b, &signer.sign(&b), &options(&trusted))),
        Check::Signature
    );
}

#[test]
fn a_signature_made_for_another_purpose_fails() {
    let key = TestKey::file("ci");
    let b = bytes(&manifest());
    let sig = key.sign_ns(&b, "git");
    assert_eq!(
        failed_check(verify(&b, &sig, &options(&key.trusted()))),
        Check::Signature
    );
}

#[test]
fn a_hardware_signature_without_a_touch_fails() {
    let key = TestKey::hardware("owner", 0x00);
    let b = bytes(&manifest());
    assert_eq!(
        failed_check(verify(&b, &key.sign(&b), &options(&key.trusted()))),
        Check::Signature
    );
}

#[test]
fn the_second_key_slot_verifies_during_a_rotation() {
    let old = TestKey::hardware("owner 2026", 0x01);
    let new = TestKey::hardware("owner 2027", 0x01);
    let keys = TrustedKeys {
        current: old.public(),
        next: Some(new.public()),
    };
    let b = bytes(&manifest());
    let v = verify(&b, &new.sign(&b), &options(&keys)).unwrap();
    assert!(v.signed_by_next);
    assert_eq!(v.signed_by, new.public().fingerprint());
}

#[test]
fn an_expired_manifest_fails() {
    let mut m = manifest();
    m["expires"] = json!("2026-11-30T00:00:00Z");
    assert_eq!(failed_check(run(&m)), Check::Expires);
    // Expiring exactly now is expired.
    m["expires"] = json!(NOW);
    assert_eq!(failed_check(run(&m)), Check::Expires);
}

#[test]
fn a_manifest_published_more_than_an_hour_ahead_fails() {
    let mut m = manifest();
    m["published"] = json!("2026-12-01T01:00:01Z");
    assert_eq!(failed_check(run(&m)), Check::Published);
}

/// A client whose clock is up to an hour behind the signer's still accepts a
/// manifest signed just now.
#[test]
fn a_manifest_published_up_to_an_hour_ahead_verifies() {
    let mut m = manifest();
    m["published"] = json!("2026-12-01T00:00:01Z");
    run(&m).unwrap();
    m["published"] = json!("2026-12-01T01:00:00Z");
    run(&m).unwrap();
}

#[test]
fn a_rolled_back_sequence_fails() {
    let mut m = manifest();
    m["sequence"] = json!(6);
    assert_eq!(failed_check(run(&m)), Check::Sequence);
    // The same sequence again is not a rollback.
    m["sequence"] = json!(7);
    run(&m).unwrap();
}

#[test]
fn a_manifest_for_another_channel_fails() {
    let mut m = manifest();
    m["channel"] = json!("dev");
    assert_eq!(failed_check(run(&m)), Check::Channel);
}

#[test]
fn a_set_whose_cmns_and_node_versions_differ_fails() {
    let mut m = manifest();
    m["sets"][0]["components"]["cmns"]["version"] = json!("2.1.0");
    assert_eq!(failed_check(run(&m)), Check::SetVersion);
}

#[test]
fn a_wrong_asset_hash_fails() {
    let dir = assets_dir("wrong-hash");
    // Same length, different content: only the hash can tell.
    let mut evil = CMNS_ASSET.to_vec();
    evil[0] ^= 0x20;
    std::fs::write(dir.join("cmns-0.9.48-aarch64-apple-darwin.tar.gz"), evil).unwrap();
    assert_eq!(
        failed_check(run_with_assets(&manifest(), &dir)),
        Check::AssetSha256
    );
}

#[test]
fn a_wrong_asset_size_fails() {
    let dir = assets_dir("wrong-size");
    std::fs::write(
        dir.join("cmn-0.9.48-x86_64-unknown-linux-musl.tar.gz"),
        b"short",
    )
    .unwrap();
    assert_eq!(
        failed_check(run_with_assets(&manifest(), &dir)),
        Check::AssetSize
    );
}

#[test]
fn an_assets_directory_holding_no_listed_asset_fails() {
    let dir = scratch("no-assets");
    assert_eq!(
        failed_check(run_with_assets(&manifest(), &dir)),
        Check::AssetsAbsent
    );
}

#[test]
fn a_format_change_must_require_a_new_data_directory() {
    let mut m = manifest();
    m["sets"][0]["format_change"] = json!(true);
    assert_eq!(failed_check(run(&m)), Check::Schema);
    m["sets"][0]["requires_data_dir"] = json!("new");
    run(&m).unwrap();
}

#[test]
fn a_build_that_is_not_the_release_profile_fails() {
    let mut m = manifest();
    m["sets"][0]["build_profile"] = json!("dev: debug assertions on");
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

/// Only the exact string `build-cmns.sh` writes is accepted: one that starts
/// with "release" but says, or hides, something else is not.
#[test]
fn a_build_profile_that_only_starts_with_release_fails() {
    for profile in [
        "release",
        "release: opt-level=0, debug-assertions=true",
        "release: lto=true, codegen-units=1, strip=true, opt-level=0",
        "release: lto=true, codegen-units=1, strip=false",
    ] {
        let mut m = manifest();
        m["sets"][0]["build_profile"] = json!(profile);
        assert_eq!(failed_check(run(&m)), Check::Schema, "{profile:?}");
    }
    assert_eq!(
        manifest()["sets"][0]["build_profile"],
        commons_release_verify::manifest::RELEASE_PROFILE
    );
}

#[test]
fn a_new_data_directory_without_a_format_change_fails() {
    let mut m = manifest();
    m["sets"][0]["requires_data_dir"] = json!("new");
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

/// Two assets stored under one file name, in different components.
#[test]
fn two_assets_with_one_file_name_fail() {
    let mut m = manifest();
    let cmns_url = m["sets"][0]["components"]["cmns"]["assets"]["aarch64-apple-darwin"]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = cmns_url.replace("/v0.9.48/", "/v0.9.48-copy/");
    m["sets"][0]["components"]["node"]["assets"]["x86_64-unknown-linux-musl"]["url"] = json!(other);
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

#[test]
fn an_asset_url_ending_in_a_dot_segment_fails() {
    for name in [".", ".."] {
        let mut m = manifest();
        m["sets"][0]["components"]["cmns"]["assets"]["aarch64-apple-darwin"]["url"] =
            json!(format!("https://github.com/dhiway/commons-releases/{name}"));
        assert_eq!(failed_check(run(&m)), Check::Schema, "{name:?}");
    }
}

/// `expires` not after `published` is a schema error, whatever the clock:
/// with `expires` equal to `published`, before `now`, the expiry check would
/// also refuse it, so the test names the check that must catch it first.
#[test]
fn expires_not_after_published_fails() {
    let mut m = manifest();
    m["expires"] = m["published"].clone();
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

#[test]
fn a_schema_number_other_than_1_fails() {
    for schema in [0, 2] {
        let mut m = manifest();
        m["schema"] = json!(schema);
        assert_eq!(failed_check(run(&m)), Check::Schema, "schema {schema}");
    }
}

#[test]
fn two_sets_with_one_id_fail() {
    let mut m = manifest();
    let mut second = m["sets"][0].clone();
    // Its own asset file names, so only the set id repeats.
    for (comp, platform) in [
        ("cmns", "aarch64-apple-darwin"),
        ("node", "x86_64-unknown-linux-musl"),
    ] {
        let url = &mut second["components"][comp]["assets"][platform]["url"];
        *url = json!(url.as_str().unwrap().replace("0.9.48-", "0.9.48-b-"));
    }
    m["sets"].as_array_mut().unwrap().push(second);
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

#[test]
fn a_commit_that_is_not_40_lowercase_hex_fails() {
    for commit in [
        "ABCDEF1111111111111111111111111111111111",  // uppercase
        "111111111111111111111111111111111111111",   // 39 digits
        "11111111111111111111111111111111111111111", // 41 digits
        "g111111111111111111111111111111111111111",  // not hex
    ] {
        let mut m = manifest();
        m["sets"][0]["components"]["node"]["commit"] = json!(commit);
        assert_eq!(failed_check(run(&m)), Check::Schema, "{commit:?}");
    }
}

#[test]
fn an_unknown_field_fails() {
    let mut m = manifest();
    m["sets"][0]["components"]["cmns"]["min_node"] = json!("0.9.40");
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

#[test]
fn an_asset_url_that_is_not_https_fails() {
    let mut m = manifest();
    m["sets"][0]["components"]["node"]["assets"]["x86_64-unknown-linux-musl"]["url"] =
        json!("http://example.com/cmn.tar.gz");
    assert_eq!(failed_check(run(&m)), Check::Schema);
}

/// The example manifests in `examples/` are valid schema-1 manifests that
/// verify once signed, at their own publication time.
#[test]
fn the_example_manifests_verify() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    for channel in ["dev", "beta", "stable"] {
        let b = std::fs::read(root.join(channel).join("manifest.json")).unwrap();
        let m: Value = serde_json::from_slice(&b).unwrap();
        let key = TestKey::hardware("example", 0x01);
        let keys = key.trusted();
        let now = time::OffsetDateTime::parse(
            m["published"].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let opts = Options {
            now,
            channel: Some(channel),
            last_sequence: None,
            ..options(&keys)
        };
        let v =
            verify(&b, &key.sign(&b), &opts).unwrap_or_else(|e| panic!("examples/{channel}: {e}"));
        assert_eq!(v.manifest.channel, channel);
    }
}
