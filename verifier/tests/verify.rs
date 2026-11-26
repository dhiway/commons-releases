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
fn a_manifest_published_in_the_future_fails() {
    let mut m = manifest();
    m["published"] = json!("2026-12-01T00:00:01Z");
    assert_eq!(failed_check(run(&m)), Check::Published);
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
    for channel in ["beta", "stable"] {
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
