//! Verification of OpenSSH signatures, the files `ssh-keygen -Y sign` writes.
//!
//! The format is OpenSSH's `PROTOCOL.sshsig`. Only Ed25519 keys are accepted,
//! in the two forms OpenSSH has:
//!
//! - `ssh-ed25519`: a key held in a file. CI uses one for `dev` and `beta`.
//! - `sk-ssh-ed25519@openssh.com`: an Ed25519 key held on a FIDO2 hardware
//!   token (`PROTOCOL.u2f`). The owner signs `stable` with one. The token
//!   signs a digest that also carries the key's application string, a flags
//!   byte and a counter; the user-presence flag must be set.
//!
//! What is signed is the SSHSIG "signed data" structure: the magic `SSHSIG`,
//! the namespace, a reserved string, the hash algorithm name and the hash of
//! the message. The namespace binds a signature to its purpose, so a signature
//! the same key made for something else (a Git commit, a file) does not verify
//! as a manifest signature.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256, Sha512};

/// The namespace every manifest signature is made in: `ssh-keygen -Y sign -n`.
pub const NAMESPACE: &str = "commons-release-manifest";

const MAGIC: &[u8; 6] = b"SSHSIG";
const ARMOR_BEGIN: &str = "-----BEGIN SSH SIGNATURE-----";
const ARMOR_END: &str = "-----END SSH SIGNATURE-----";
const ED25519: &str = "ssh-ed25519";
const SK_ED25519: &str = "sk-ssh-ed25519@openssh.com";
/// `PROTOCOL.u2f`: bit 0 of the flags byte, the user touched the token.
const SK_USER_PRESENT: u8 = 0x01;

/// A public key a manifest may be signed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicKey {
    /// `ssh-ed25519`: a key in a file.
    Ed25519([u8; 32]),
    /// `sk-ssh-ed25519@openssh.com`: a key on a FIDO2 token, with the
    /// application string it was enrolled under (for example `ssh:`).
    SkEd25519 { key: [u8; 32], application: String },
}

impl PublicKey {
    /// Parses one line of an OpenSSH `.pub` file: `<type> <base64> [comment]`.
    pub fn from_openssh(line: &str) -> Result<Self, String> {
        let mut fields = line.split_whitespace();
        let kind = fields.next().ok_or("the public key line is empty")?;
        let b64 = fields
            .next()
            .ok_or("the public key line has no key data after its type")?;
        let blob = STANDARD
            .decode(b64)
            .map_err(|e| format!("the public key's base64 does not decode: {e}"))?;
        let key = Self::from_blob(&blob)?;
        if key.type_name() != kind {
            return Err(format!(
                "the public key line says {kind} but its data is {}",
                key.type_name()
            ));
        }
        Ok(key)
    }

    fn from_blob(blob: &[u8]) -> Result<Self, String> {
        let mut r = Reader::new(blob);
        let kind = r.string()?;
        let key = match kind {
            k if k == ED25519.as_bytes() => PublicKey::Ed25519(r.key32()?),
            k if k == SK_ED25519.as_bytes() => {
                let key = r.key32()?;
                let application = String::from_utf8(r.string()?.to_vec())
                    .map_err(|_| "the key's application string is not UTF-8")?;
                PublicKey::SkEd25519 { key, application }
            }
            other => {
                return Err(format!(
                    "key type {} is not accepted; only {ED25519} and {SK_ED25519} are",
                    String::from_utf8_lossy(other)
                ))
            }
        };
        r.finish("public key")?;
        Ok(key)
    }

    /// The OpenSSH wire encoding of the key.
    fn blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_string(&mut out, self.type_name().as_bytes());
        match self {
            PublicKey::Ed25519(key) => put_string(&mut out, key),
            PublicKey::SkEd25519 { key, application } => {
                put_string(&mut out, key);
                put_string(&mut out, application.as_bytes());
            }
        }
        out
    }

    fn type_name(&self) -> &'static str {
        match self {
            PublicKey::Ed25519(_) => ED25519,
            PublicKey::SkEd25519 { .. } => SK_ED25519,
        }
    }

    fn raw(&self) -> &[u8; 32] {
        match self {
            PublicKey::Ed25519(key) | PublicKey::SkEd25519 { key, .. } => key,
        }
    }

    /// The fingerprint `ssh-keygen -l` prints: `SHA256:` and unpadded base64.
    pub fn fingerprint(&self) -> String {
        format!(
            "SHA256:{}",
            STANDARD_NO_PAD.encode(Sha256::digest(self.blob()))
        )
    }
}

/// Verifies an armored SSHSIG `signature` over `message` and returns the index
/// in `trusted` of the key that made it. The error says what was wrong.
pub fn verify(message: &[u8], signature: &str, trusted: &[&PublicKey]) -> Result<usize, String> {
    let blob = dearmor(signature)?;
    let mut r = Reader::new(&blob);
    if r.bytes(MAGIC.len())? != MAGIC {
        return Err("the signature does not start with the SSHSIG magic".into());
    }
    let version = r.u32()?;
    if version != 1 {
        return Err(format!("signature format version {version}, expected 1"));
    }
    let key = PublicKey::from_blob(r.string()?)?;
    let namespace = r.string()?;
    let reserved = r.string()?;
    let hash_alg = r.string()?;
    let sig_blob = r.string()?;
    r.finish("signature")?;

    if namespace != NAMESPACE.as_bytes() {
        return Err(format!(
            "the signature was made in namespace {:?}, not {NAMESPACE:?}",
            String::from_utf8_lossy(namespace)
        ));
    }
    let index = trusted.iter().position(|t| **t == key).ok_or_else(|| {
        format!(
            "the signature was made by key {}, which is not a trusted key",
            key.fingerprint()
        )
    })?;

    let digest = match hash_alg {
        b"sha512" => Sha512::digest(message).to_vec(),
        b"sha256" => Sha256::digest(message).to_vec(),
        other => {
            return Err(format!(
                "hash algorithm {:?} is not accepted; sha512 or sha256 is",
                String::from_utf8_lossy(other)
            ))
        }
    };
    let mut signed = MAGIC.to_vec();
    put_string(&mut signed, namespace);
    put_string(&mut signed, reserved);
    put_string(&mut signed, hash_alg);
    put_string(&mut signed, &digest);

    let mut s = Reader::new(sig_blob);
    if s.string()? != key.type_name().as_bytes() {
        return Err("the signature's type does not match its key's type".into());
    }
    let sig_bytes: [u8; 64] = s
        .string()?
        .try_into()
        .map_err(|_| "the Ed25519 signature is not 64 bytes")?;
    let to_verify = match &key {
        PublicKey::Ed25519(_) => signed,
        PublicKey::SkEd25519 { application, .. } => {
            let flags = s.u8()?;
            let counter = s.u32()?;
            if flags & SK_USER_PRESENT == 0 {
                return Err("the hardware key signed without the user-presence flag".into());
            }
            let mut m = Sha256::digest(application.as_bytes()).to_vec();
            m.push(flags);
            m.extend_from_slice(&counter.to_be_bytes());
            m.extend_from_slice(&Sha256::digest(&signed));
            m
        }
    };
    s.finish("signature value")?;

    let vk = VerifyingKey::from_bytes(key.raw())
        .map_err(|_| "the public key is not a valid Ed25519 point")?;
    vk.verify_strict(&to_verify, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| {
            format!(
                "the Ed25519 signature by {} does not verify",
                key.fingerprint()
            )
        })?;
    Ok(index)
}

fn dearmor(text: &str) -> Result<Vec<u8>, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some(ARMOR_BEGIN) {
        return Err(format!(
            "the signature file does not start with {ARMOR_BEGIN}"
        ));
    }
    let mut b64 = String::new();
    for line in lines.by_ref() {
        if line == ARMOR_END {
            if lines.next().is_some() {
                return Err(format!("the signature file has text after {ARMOR_END}"));
            }
            return STANDARD
                .decode(b64)
                .map_err(|e| format!("the signature's base64 does not decode: {e}"));
        }
        b64.push_str(line);
    }
    Err(format!("the signature file has no {ARMOR_END} line"))
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Reads the OpenSSH wire types (`byte`, `uint32`, `string`) from a buffer.
struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.buf.len() < n {
            return Err("the encoding is truncated".into());
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Result<&'a [u8], String> {
        let len = self.u32()? as usize;
        self.bytes(len)
    }

    fn key32(&mut self) -> Result<[u8; 32], String> {
        self.string()?
            .try_into()
            .map_err(|_| "the Ed25519 public key is not 32 bytes".into())
    }

    fn finish(&self, what: &str) -> Result<(), String> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(format!("the {what} has trailing bytes"))
        }
    }
}
