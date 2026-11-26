//! The channel manifest, schema 1. `docs/manifest.md` is the written grammar;
//! `schema/manifest.schema.json` says the same as a JSON Schema. Unknown
//! fields are refused: a new field means a new `schema` number.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// The only schema number this verifier accepts.
pub const SCHEMA: u32 = 1;
/// The channels a manifest may name.
pub const CHANNELS: [&str; 3] = ["dev", "beta", "stable"];
/// The only `build_profile` a set may carry. `scripts/build-cmns.sh` writes
/// it only after checking the whole release profile: these three settings,
/// opt-level 3, debug assertions and overflow checks off, no per-package
/// override and no profile from a Cargo config file.
pub const RELEASE_PROFILE: &str = "release: lto=true, codegen-units=1, strip=true";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub channel: String,
    pub published: String,
    pub sequence: u64,
    pub expires: String,
    pub sets: Vec<Set>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    pub id: String,
    pub format_change: bool,
    pub requires_data_dir: String,
    pub components: Components,
    pub notes_url: String,
    pub build_profile: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Components {
    pub cmns: Component,
    pub node: Component,
    #[serde(default)]
    pub app: Option<Component>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub version: String,
    pub commit: String,
    pub assets: BTreeMap<String, Asset>,
    /// The node's on-disk storage format number (node only).
    #[serde(default)]
    pub storage_format: Option<u32>,
    /// The node's sync protocol ALPN (node only).
    #[serde(default)]
    pub sync_alpn: Option<String>,
    /// Whether the macOS assets are notarised (node and App).
    #[serde(default)]
    pub notarized: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl Asset {
    /// The file name a downloaded asset is stored under: the URL's last
    /// path segment. `check_schema` guarantees it is a plain file name.
    pub fn file_name(&self) -> &str {
        self.url.rsplit('/').next().unwrap_or_default()
    }
}

impl Manifest {
    /// Every asset with where it sits: `(set id, component, platform, asset)`.
    pub fn assets(&self) -> impl Iterator<Item = (&str, &'static str, &str, &Asset)> {
        self.sets.iter().flat_map(|set| {
            let c = &set.components;
            [
                ("cmns", Some(&c.cmns)),
                ("node", Some(&c.node)),
                ("app", c.app.as_ref()),
            ]
            .into_iter()
            .filter_map(|(name, comp)| comp.map(|comp| (name, comp)))
            .flat_map(move |(name, comp)| {
                comp.assets
                    .iter()
                    .map(move |(platform, asset)| (set.id.as_str(), name, platform.as_str(), asset))
            })
        })
    }
}

/// Parses an RFC 3339 timestamp.
pub fn timestamp(field: &str, value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|e| format!("{field} {value:?} is not an RFC 3339 timestamp: {e}"))
}

/// The rules a manifest must meet beyond its JSON shape. The set-version rule
/// (`cmns.version == node.version`) is its own check, `check_set_versions`.
pub fn check_schema(m: &Manifest) -> Result<(), String> {
    if m.schema != SCHEMA {
        return Err(format!(
            "schema {} is not supported; this verifier reads schema {SCHEMA}",
            m.schema
        ));
    }
    if !CHANNELS.contains(&m.channel.as_str()) {
        return Err(format!(
            "channel {:?} is not one of {CHANNELS:?}",
            m.channel
        ));
    }
    let published = timestamp("published", &m.published)?;
    let expires = timestamp("expires", &m.expires)?;
    if expires <= published {
        return Err("expires is not later than published".into());
    }
    if m.sets.is_empty() {
        return Err("the manifest lists no set".into());
    }
    let mut ids = BTreeSet::new();
    for set in &m.sets {
        let at = format!("set {:?}", set.id);
        if set.id.is_empty() || !ids.insert(set.id.as_str()) {
            return Err(format!("{at}: set ids must be non-empty and unique"));
        }
        match (set.format_change, set.requires_data_dir.as_str()) {
            (true, "new") | (false, "same") => {}
            (fc, dir) => {
                return Err(format!(
                    "{at}: format_change {fc} with requires_data_dir {dir:?}; a format change requires \"new\", any other set \"same\""
                ))
            }
        }
        https_url(&set.notes_url).map_err(|e| format!("{at}: notes_url {e}"))?;
        if set.build_profile != RELEASE_PROFILE {
            return Err(format!(
                "{at}: build_profile {:?} is not {RELEASE_PROFILE:?}; every release is an optimised build",
                set.build_profile
            ));
        }
        let c = &set.components;
        for (name, comp) in [
            ("cmns", Some(&c.cmns)),
            ("node", Some(&c.node)),
            ("app", c.app.as_ref()),
        ] {
            let Some(comp) = comp else { continue };
            let at = format!("{at} component {name}");
            if !is_version(&comp.version) {
                return Err(format!(
                    "{at}: version {:?} is not MAJOR.MINOR.PATCH[-PRERELEASE]",
                    comp.version
                ));
            }
            if !is_lower_hex(&comp.commit, 40) {
                return Err(format!(
                    "{at}: commit {:?} is not 40 lowercase hex digits",
                    comp.commit
                ));
            }
            if comp.assets.is_empty() {
                return Err(format!("{at}: lists no asset"));
            }
            for (platform, asset) in &comp.assets {
                let at = format!("{at} platform {platform:?}");
                if platform.is_empty()
                    || !platform.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    })
                {
                    return Err(format!("{at}: platform names are lowercase target triples"));
                }
                https_url(&asset.url).map_err(|e| format!("{at}: url {e}"))?;
                if !is_lower_hex(&asset.sha256, 64) {
                    return Err(format!("{at}: sha256 is not 64 lowercase hex digits"));
                }
                if asset.size == 0 {
                    return Err(format!("{at}: size is 0"));
                }
            }
        }
    }
    let mut names = BTreeSet::new();
    for (set, comp, platform, asset) in m.assets() {
        let name = asset.file_name();
        if name.is_empty() || name == "." || name == ".." || name.contains('\\') {
            return Err(format!(
                "set {set:?} {comp} {platform}: the url does not end in a file name"
            ));
        }
        if !names.insert(name) {
            return Err(format!(
                "asset file name {name:?} appears twice; each asset needs its own name"
            ));
        }
    }
    Ok(())
}

/// Owner ruling 3 (2026-11-26): one number identifies a set, so `cmns` and the
/// node of a set carry the same version.
pub fn check_set_versions(m: &Manifest) -> Result<(), String> {
    for set in &m.sets {
        let (cmns, node) = (&set.components.cmns.version, &set.components.node.version);
        if cmns != node {
            return Err(format!(
                "set {:?}: cmns is {cmns} but node is {node}; a set carries one version number",
                set.id
            ));
        }
    }
    Ok(())
}

fn https_url(url: &str) -> Result<(), String> {
    match url.strip_prefix("https://") {
        Some(rest) if !rest.is_empty() && !rest.contains(char::is_whitespace) => Ok(()),
        _ => Err(format!("{url:?} is not an https URL")),
    }
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_version(v: &str) -> bool {
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    };
    let numeric = |p: &str| {
        !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (p == "0" || !p.starts_with('0'))
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| numeric(p))
        && pre.is_none_or(|pre| {
            pre.split('.').all(|id| {
                !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        })
}
