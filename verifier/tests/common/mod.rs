//! Test helpers: throwaway signing keys, an SSHSIG signer written separately
//! from the verifier (and checked against OpenSSH in `tests/openssh.rs`), and
//! a manifest to change one field at a time.
#![allow(dead_code)]

use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use commons_release_verify::{Options, PublicKey, TrustedKeys};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const NAMESPACE: &str = "commons-release-manifest";
/// The time the tests run "at": after `published`, before `expires`.
pub const NOW: &str = "2026-12-01T00:00:00Z";

pub fn now() -> OffsetDateTime {
    OffsetDateTime::parse(NOW, &Rfc3339).unwrap()
}

/// A throwaway Ed25519 key generated for one test. `hardware` makes it behave
/// as a FIDO2 key would: `sk-ssh-ed25519@openssh.com`, signing over the
/// application hash, a flags byte and a counter.
pub struct TestKey {
    signing: SigningKey,
    hardware: Option<(String, u8)>,
}

impl TestKey {
    fn seed(label: &str) -> [u8; 32] {
        Sha512::digest(format!("commons-releases test key {label}").as_bytes())[..32]
            .try_into()
            .unwrap()
    }

    pub fn file(label: &str) -> Self {
        TestKey {
            signing: SigningKey::from_bytes(&Self::seed(label)),
            hardware: None,
        }
    }

    /// `flags` 0x01 is "user present"; 0x00 is a signature made without a touch.
    pub fn hardware(label: &str, flags: u8) -> Self {
        TestKey {
            signing: SigningKey::from_bytes(&Self::seed(label)),
            hardware: Some(("ssh:commons-releases-test".into(), flags)),
        }
    }

    fn type_name(&self) -> &'static str {
        if self.hardware.is_some() {
            "sk-ssh-ed25519@openssh.com"
        } else {
            "ssh-ed25519"
        }
    }

    fn blob(&self) -> Vec<u8> {
        let mut b = Vec::new();
        string(&mut b, self.type_name().as_bytes());
        string(&mut b, self.signing.verifying_key().as_bytes());
        if let Some((app, _)) = &self.hardware {
            string(&mut b, app.as_bytes());
        }
        b
    }

    /// The `.pub` line `ssh-keygen` would write.
    pub fn openssh_line(&self) -> String {
        format!(
            "{} {} test-key",
            self.type_name(),
            STANDARD.encode(self.blob())
        )
    }

    pub fn public(&self) -> PublicKey {
        PublicKey::from_openssh(&self.openssh_line()).unwrap()
    }

    pub fn trusted(&self) -> TrustedKeys {
        TrustedKeys {
            current: self.public(),
            next: None,
        }
    }

    /// An armored signature as `ssh-keygen -Y sign -n <namespace>` writes it.
    pub fn sign_ns(&self, message: &[u8], namespace: &str) -> String {
        let mut signed = b"SSHSIG".to_vec();
        string(&mut signed, namespace.as_bytes());
        string(&mut signed, b"");
        string(&mut signed, b"sha512");
        string(&mut signed, &Sha512::digest(message));

        let mut sig = Vec::new();
        string(&mut sig, self.type_name().as_bytes());
        match &self.hardware {
            None => string(&mut sig, &self.signing.sign(&signed).to_bytes()),
            Some((app, flags)) => {
                let counter: u32 = 7;
                let mut m = Sha256::digest(app.as_bytes()).to_vec();
                m.push(*flags);
                m.extend_from_slice(&counter.to_be_bytes());
                m.extend_from_slice(&Sha256::digest(&signed));
                string(&mut sig, &self.signing.sign(&m).to_bytes());
                sig.push(*flags);
                sig.extend_from_slice(&counter.to_be_bytes());
            }
        }

        let mut blob = b"SSHSIG".to_vec();
        blob.extend_from_slice(&1u32.to_be_bytes());
        string(&mut blob, &self.blob());
        string(&mut blob, namespace.as_bytes());
        string(&mut blob, b"");
        string(&mut blob, b"sha512");
        string(&mut blob, &sig);
        let b64 = STANDARD.encode(blob);
        let body: Vec<&str> = b64
            .as_bytes()
            .chunks(70)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            body.join("\n")
        )
    }

    pub fn sign(&self, message: &[u8]) -> String {
        self.sign_ns(message, NAMESPACE)
    }
}

fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub const CMNS_ASSET: &[u8] = b"stand-in for cmns-0.9.48-aarch64-apple-darwin.tar.gz";
pub const NODE_ASSET: &[u8] = b"stand-in for cmn-0.9.48-x86_64-unknown-linux-musl.tar.gz";

fn asset(name: &str, bytes: &[u8]) -> Value {
    json!({
        "url": format!("https://github.com/dhiway/commons-releases/releases/download/v0.9.48/{name}"),
        "sha256": sha256_hex(bytes),
        "size": bytes.len(),
    })
}

/// A valid `beta` manifest with one set; tests change one field of it.
pub fn manifest() -> Value {
    json!({
        "schema": 1,
        "channel": "beta",
        "published": "2026-11-26T12:00:00Z",
        "sequence": 7,
        "expires": "2026-12-26T12:00:00Z",
        "sets": [{
            "id": "0.9.48",
            "format_change": false,
            "requires_data_dir": "same",
            "components": {
                "cmns": {
                    "version": "0.9.48",
                    "commit": "1111111111111111111111111111111111111111",
                    "assets": {"aarch64-apple-darwin": asset("cmns-0.9.48-aarch64-apple-darwin.tar.gz", CMNS_ASSET)}
                },
                "node": {
                    "version": "0.9.48",
                    "commit": "2222222222222222222222222222222222222222",
                    "assets": {"x86_64-unknown-linux-musl": asset("cmn-0.9.48-x86_64-unknown-linux-musl.tar.gz", NODE_ASSET)}
                }
            },
            "notes_url": "https://github.com/dhiway/commons-releases/releases/tag/v0.9.48",
            "build_profile": "release: lto=true, codegen-units=1, strip=true"
        }]
    })
}

pub fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap()
}

pub fn options(keys: &TrustedKeys) -> Options<'_> {
    Options {
        keys,
        now: now(),
        channel: Some("beta"),
        last_sequence: Some(7),
        assets_dir: None,
    }
}

/// A fresh directory under Cargo's per-test scratch area.
pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A directory holding both assets of `manifest()`, correct.
pub fn assets_dir(name: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::write(
        dir.join("cmns-0.9.48-aarch64-apple-darwin.tar.gz"),
        CMNS_ASSET,
    )
    .unwrap();
    std::fs::write(
        dir.join("cmn-0.9.48-x86_64-unknown-linux-musl.tar.gz"),
        NODE_ASSET,
    )
    .unwrap();
    dir
}
