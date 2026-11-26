//! Interoperability with the real OpenSSH `ssh-keygen`, the signing tool the
//! procedure names. These tests fail, not skip, when `ssh-keygen` is missing.
//!
//! - A signature `ssh-keygen -Y sign` makes with a fresh file key verifies,
//!   and a tampered copy does not: the CI path, end to end.
//! - The hardware-key signatures the other tests make are accepted by
//!   `ssh-keygen -Y verify`: the owner's path, checked against OpenSSH's own
//!   verifier, since no hardware key is available to the tests.

mod common;

use std::process::{Command, Stdio};

use common::*;
use commons_release_verify::{verify, Check, Error, PublicKey, TrustedKeys};

fn ssh_keygen(args: &[&str]) -> std::process::Output {
    Command::new("ssh-keygen")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("ssh-keygen must be installed to run these tests")
}

#[test]
fn a_signature_made_by_ssh_keygen_verifies() {
    let dir = scratch("ssh-keygen-sign");
    let key = dir.join("ci_key");
    let out = ssh_keygen(&[
        "-q",
        "-t",
        "ed25519",
        "-N",
        "",
        "-C",
        "throwaway",
        "-f",
        key.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "ssh-keygen -t ed25519: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let path = dir.join("manifest.json");
    let b = bytes(&manifest());
    std::fs::write(&path, &b).unwrap();
    let out = ssh_keygen(&[
        "-Y",
        "sign",
        "-f",
        key.to_str().unwrap(),
        "-n",
        NAMESPACE,
        path.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "ssh-keygen -Y sign: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let sig = std::fs::read_to_string(dir.join("manifest.json.sig")).unwrap();
    let public = std::fs::read_to_string(dir.join("ci_key.pub")).unwrap();
    let keys = TrustedKeys {
        current: PublicKey::from_openssh(&public).unwrap(),
        next: None,
    };
    verify(&b, &sig, &options(&keys)).unwrap();

    let mut tampered = b.clone();
    let last = tampered.len() - 2;
    tampered[last] = b' ';
    match verify(&tampered, &sig, &options(&keys)) {
        Err(Error::Failed {
            check: Check::Signature,
            ..
        }) => {}
        other => panic!("a tampered manifest must fail the signature check, got {other:?}"),
    }
}

/// Runs `ssh-keygen -Y verify` on a signature the test helper made.
fn openssh_accepts(key: &TestKey, name: &str) -> bool {
    let dir = scratch(name);
    let b = bytes(&manifest());
    let path = dir.join("manifest.json");
    std::fs::write(&path, &b).unwrap();
    std::fs::write(dir.join("manifest.json.sig"), key.sign(&b)).unwrap();
    std::fs::write(
        dir.join("allowed_signers"),
        format!(
            "release@test namespaces=\"{NAMESPACE}\" {}\n",
            key.openssh_line()
        ),
    )
    .unwrap();
    let out = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(dir.join("allowed_signers"))
        .args(["-I", "release@test", "-n", NAMESPACE, "-s"])
        .arg(dir.join("manifest.json.sig"))
        .stdin(std::fs::File::open(&path).unwrap())
        .output()
        .expect("ssh-keygen must be installed to run these tests");
    out.status.success()
}

#[test]
fn openssh_accepts_the_test_signatures() {
    assert!(openssh_accepts(&TestKey::file("ci"), "openssh-file"));
    assert!(openssh_accepts(
        &TestKey::hardware("owner", 0x01),
        "openssh-sk"
    ));
}

/// `ssh-keygen -Y verify` does not require the user-presence flag on a file
/// signature; this verifier does. The signature is well formed (OpenSSH
/// accepts it), so the verifier's refusal is the touch rule and nothing else.
#[test]
fn a_well_formed_signature_without_a_touch_is_refused_only_by_this_verifier() {
    let key = TestKey::hardware("owner", 0x00);
    assert!(openssh_accepts(&key, "openssh-sk-no-touch"));
    let b = bytes(&manifest());
    match verify(&b, &key.sign(&b), &options(&key.trusted())) {
        Err(Error::Failed {
            check: Check::Signature,
            detail,
        }) => {
            assert!(detail.contains("user-presence"), "{detail}")
        }
        other => panic!("expected the touch rule to refuse it, got {other:?}"),
    }
}
