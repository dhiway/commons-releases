//! Offline verification of a Commons release manifest.
//!
//! [`verify`] takes the manifest's bytes exactly as published, its detached
//! OpenSSH signature and the trusted keys, and runs these checks in order,
//! stopping at the first that fails:
//!
//! 1. [`Check::Signature`]: an Ed25519 signature by a trusted key, in the
//!    `commons-release-manifest` namespace, over the file's bytes. Nothing is
//!    parsed before this passes.
//! 2. [`Check::Schema`]: the JSON is a schema-1 manifest (`docs/manifest.md`).
//! 3. [`Check::Channel`]: the manifest names the channel the caller asked for.
//! 4. [`Check::Published`]: `published` is not in the future.
//! 5. [`Check::Expires`]: `expires` is in the future.
//! 6. [`Check::Sequence`]: `sequence` is not lower than the last one seen.
//! 7. [`Check::SetVersion`]: in every set, `cmns` and the node have one version.
//! 8. [`Check::AssetSize`] and [`Check::AssetSha256`]: each listed asset found
//!    in the assets directory has the size and SHA-256 the manifest gives;
//!    [`Check::AssetsAbsent`] if a directory was given and holds none of them.
//!
//! No check touches the network.
//!
//! # Using it from a client
//!
//! `cmns` pins this crate over `https://` at a full commit, so what follows is
//! its interface; `tests/client.rs` exercises it the same way.
//!
//! - Keys: [`PublicKey::read_file`] reads a `keys/*.pub` file, and
//!   [`PublicKey::from_key_text`] parses one compiled into the client. Put
//!   them in [`TrustedKeys`] (`current`, and `next` during a rotation).
//! - Checks: [`verify`] takes the manifest's bytes, the signature file's text
//!   and [`Options`] (`channel`, `last_sequence`, `assets_dir` for downloaded
//!   assets, and `now`). A failure is [`Error::Failed`] carrying a [`Check`] to
//!   match on, with [`Check::as_str`] as its printable name; [`Error::Io`]
//!   means an asset could not be read.
//! - The result: [`Verified::manifest`] is a [`Manifest`] whose
//!   [`sets`](Manifest::sets) give each [`manifest::Set`]'s `format_change`,
//!   `requires_data_dir` and `components` (`cmns`, `node`, optional `app`),
//!   each a [`manifest::Component`] with `version`, `commit` and `assets`
//!   keyed by target triple, each a [`manifest::Asset`] with `url`, `sha256`
//!   and `size`.

pub mod manifest;
pub mod sshsig;

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;

pub use manifest::Manifest;
pub use sshsig::{PublicKey, NAMESPACE};

/// The check that failed. [`Check::as_str`] is the stable name the command
/// prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Signature,
    Schema,
    Channel,
    Published,
    Expires,
    Sequence,
    SetVersion,
    AssetsAbsent,
    AssetSize,
    AssetSha256,
}

impl Check {
    pub fn as_str(self) -> &'static str {
        match self {
            Check::Signature => "signature",
            Check::Schema => "schema",
            Check::Channel => "channel",
            Check::Published => "published",
            Check::Expires => "expires",
            Check::Sequence => "sequence",
            Check::SetVersion => "set-version",
            Check::AssetsAbsent => "assets-absent",
            Check::AssetSize => "asset-size",
            Check::AssetSha256 => "asset-sha256",
        }
    }
}

/// Why verification did not succeed.
#[derive(Debug)]
pub enum Error {
    /// A check failed: the manifest, its signature or an asset is not what it
    /// must be. The command exits 20.
    Failed { check: Check, detail: String },
    /// Verification could not run (an asset could not be read). Exits 1.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Failed { check, detail } => write!(f, "{}: {detail}", check.as_str()),
            Error::Io(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for Error {}

fn fail(check: Check) -> impl FnOnce(String) -> Error {
    move |detail| Error::Failed { check, detail }
}

/// The keys a channel trusts: the key in use and, during a rotation, the key
/// that replaces it. A client compiles both slots in.
#[derive(Clone, Debug)]
pub struct TrustedKeys {
    pub current: PublicKey,
    pub next: Option<PublicKey>,
}

impl TrustedKeys {
    fn slots(&self) -> Vec<&PublicKey> {
        std::iter::once(&self.current)
            .chain(self.next.as_ref())
            .collect()
    }
}

/// What the caller knows and wants checked.
#[derive(Clone, Debug)]
pub struct Options<'a> {
    pub keys: &'a TrustedKeys,
    /// The time to check `published` and `expires` against.
    pub now: OffsetDateTime,
    /// The channel the manifest was fetched for; `None` accepts any.
    pub channel: Option<&'a str>,
    /// The highest `sequence` this client has accepted on this channel.
    pub last_sequence: Option<u64>,
    /// A directory holding downloaded assets under their URL's file name.
    pub assets_dir: Option<&'a Path>,
}

/// A manifest that passed every check.
#[derive(Clone, Debug)]
pub struct Verified {
    pub manifest: Manifest,
    /// Fingerprint of the key that signed it.
    pub signed_by: String,
    /// Whether it was the `next` slot's key.
    pub signed_by_next: bool,
    /// Assets found in the assets directory and checked.
    pub assets_checked: usize,
    pub assets_listed: usize,
}

/// Runs every check on a manifest; see the crate documentation for the order.
pub fn verify(bytes: &[u8], signature: &str, opts: &Options<'_>) -> Result<Verified, Error> {
    let slot =
        sshsig::verify(bytes, signature, &opts.keys.slots()).map_err(fail(Check::Signature))?;
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|e| fail(Check::Schema)(e.to_string()))?;
    manifest::check_schema(&manifest).map_err(fail(Check::Schema))?;
    check_channel(&manifest, opts.channel).map_err(fail(Check::Channel))?;
    check_published(&manifest, opts.now).map_err(fail(Check::Published))?;
    check_expires(&manifest, opts.now).map_err(fail(Check::Expires))?;
    check_sequence(&manifest, opts.last_sequence).map_err(fail(Check::Sequence))?;
    manifest::check_set_versions(&manifest).map_err(fail(Check::SetVersion))?;
    let assets_checked = match opts.assets_dir {
        Some(dir) => check_assets(&manifest, dir)?,
        None => 0,
    };
    Ok(Verified {
        signed_by: opts.keys.slots()[slot].fingerprint(),
        signed_by_next: slot == 1,
        assets_checked,
        assets_listed: manifest.assets().count(),
        manifest,
    })
}

fn check_channel(m: &Manifest, expected: Option<&str>) -> Result<(), String> {
    match expected {
        Some(want) if want != m.channel => Err(format!(
            "the manifest is for channel {:?}, but channel {want:?} was asked for",
            m.channel
        )),
        _ => Ok(()),
    }
}

fn check_published(m: &Manifest, now: OffsetDateTime) -> Result<(), String> {
    if manifest::timestamp("published", &m.published)? > now {
        return Err(format!("published {} is in the future", m.published));
    }
    Ok(())
}

fn check_expires(m: &Manifest, now: OffsetDateTime) -> Result<(), String> {
    if manifest::timestamp("expires", &m.expires)? <= now {
        return Err(format!("the manifest expired at {}", m.expires));
    }
    Ok(())
}

fn check_sequence(m: &Manifest, last: Option<u64>) -> Result<(), String> {
    match last {
        Some(last) if m.sequence < last => Err(format!(
            "sequence {} is lower than {last}, the last one accepted: an older manifest is being served",
            m.sequence
        )),
        _ => Ok(()),
    }
}

fn check_assets(m: &Manifest, dir: &Path) -> Result<usize, Error> {
    let mut checked = 0;
    for (set, comp, platform, asset) in m.assets() {
        let path = dir.join(asset.file_name());
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::Io(format!("reading {}: {e}", path.display()))),
        };
        let at = format!("set {set:?} {comp} {platform} ({})", asset.file_name());
        if !meta.is_file() {
            return Err(Error::Io(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        if meta.len() != asset.size {
            return Err(fail(Check::AssetSize)(format!(
                "{at}: the file is {} bytes, the manifest says {}",
                meta.len(),
                asset.size
            )));
        }
        let digest = sha256_file(&path)
            .map_err(|e| Error::Io(format!("reading {}: {e}", path.display())))?;
        if digest != asset.sha256 {
            return Err(fail(Check::AssetSha256)(format!(
                "{at}: the file's sha256 is {digest}, the manifest says {}",
                asset.sha256
            )));
        }
        checked += 1;
    }
    if checked == 0 {
        return Err(fail(Check::AssetsAbsent)(format!(
            "{} holds none of the manifest's {} assets",
            dir.display(),
            m.assets().count()
        )));
    }
    Ok(checked)
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
