//! The library as a client such as `cmns update check` uses it: a key file, a
//! manifest file and its signature on disk, a channel and the last sequence
//! seen, and the set's fields read back after verification.

mod common;

use common::*;

use commons_release_verify::{verify, Check, Error, Options, PublicKey, TrustedKeys};

/// Writes a key file the way `keys/` publishes one: a comment, then the key.
fn key_file(key: &TestKey, name: &str) -> std::path::PathBuf {
    let path = scratch(name).join("beta-ci-1.pub");
    let text = format!(
        "# commons-releases beta, CI key 1\n\n{}\n",
        key.openssh_line()
    );
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_client_verifies_files_and_reads_the_set() {
    let key = TestKey::file("ci");
    let dir = scratch("client-files");
    let manifest_path = dir.join("manifest.json");
    let b = bytes(&manifest());
    std::fs::write(&manifest_path, &b).unwrap();
    std::fs::write(dir.join("manifest.json.sig"), key.sign(&b)).unwrap();

    let keys = TrustedKeys {
        current: PublicKey::read_file(&key_file(&key, "client-key")).unwrap(),
        next: None,
    };
    assert_eq!(keys.current, key.public());
    let assets = assets_dir("client-assets");
    let opts = Options {
        keys: &keys,
        now: now(),
        channel: Some("beta"),
        last_sequence: Some(7),
        assets_dir: Some(&assets),
    };
    let bytes = std::fs::read(&manifest_path).unwrap();
    let sig = std::fs::read_to_string(dir.join("manifest.json.sig")).unwrap();
    let v = verify(&bytes, &sig, &opts).unwrap();

    assert_eq!(v.assets_checked, 2);
    let set = &v.manifest.sets[0];
    assert!(!set.format_change);
    assert_eq!(set.requires_data_dir, "same");
    assert_eq!(set.components.cmns.version, set.components.node.version);
    assert_eq!(
        set.components.cmns.commit,
        "1111111111111111111111111111111111111111"
    );
    let asset = &set.components.cmns.assets["aarch64-apple-darwin"];
    assert_eq!(asset.file_name(), "cmns-0.9.48-aarch64-apple-darwin.tar.gz");
    assert!(asset.url.starts_with("https://"));
    assert_eq!(asset.sha256, sha256_hex(CMNS_ASSET));
    assert_eq!(asset.size, CMNS_ASSET.len() as u64);

    // A client that has seen sequence 8 is being served an older manifest; it
    // can match the failed check rather than parse a message.
    let opts = Options {
        last_sequence: Some(8),
        ..opts
    };
    match verify(&bytes, &sig, &opts) {
        Err(Error::Failed {
            check: Check::Sequence,
            ..
        }) => {}
        other => panic!("expected the sequence check to fail, got {other:?}"),
    }
    assert_eq!(Check::Sequence.as_str(), "sequence");
}

#[test]
fn a_key_file_without_a_key_is_refused() {
    let path = scratch("client-empty-key").join("empty.pub");
    std::fs::write(&path, "# nothing here yet\n\n").unwrap();
    let err = PublicKey::read_file(&path).unwrap_err();
    assert!(err.contains("holds no public key"), "{err}");
}
