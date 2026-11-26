//! `commons-release-verify`: checks a release manifest offline.
//!
//! Exit codes: 0 every check passed; 20 a check failed (the line on stderr
//! names it); 1 any other error (bad arguments, a file that cannot be read).

use std::path::PathBuf;
use std::process::ExitCode;

use commons_release_verify::{verify, Error, Options, PublicKey, TrustedKeys};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const USAGE: &str = "\
usage: commons-release-verify --manifest FILE --key FILE [--key FILE]
                              [--signature FILE] [--channel NAME]
                              [--last-sequence N] [--assets DIR] [--now TIME]

  --manifest FILE      the manifest exactly as published
  --signature FILE     its OpenSSH signature (default: FILE.sig)
  --key FILE           an OpenSSH public key the channel trusts; give a
                       second --key for the next key during a rotation
  --channel NAME       fail unless the manifest is for this channel
  --last-sequence N    fail if the manifest's sequence is lower than N
  --assets DIR         check the size and sha256 of each listed asset found
                       in DIR under its URL's file name
  --now TIME           check freshness against this RFC 3339 time, not the clock

exit: 0 verified, 20 a check failed (named on stderr), 1 any other error";

struct Args {
    manifest: PathBuf,
    signature: PathBuf,
    keys: Vec<PathBuf>,
    channel: Option<String>,
    last_sequence: Option<u64>,
    assets: Option<PathBuf>,
    now: Option<String>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut manifest = None;
    let mut signature = None;
    let mut keys = Vec::new();
    let mut channel = None;
    let mut last_sequence = None;
    let mut assets = None;
    let mut now = None;
    let mut it = std::env::args_os().skip(1);
    while let Some(flag) = it.next() {
        let flag = flag.to_string_lossy().into_owned();
        if flag == "-h" || flag == "--help" {
            return Ok(None);
        }
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let text = || value.to_string_lossy().into_owned();
        match flag.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(&value)),
            "--signature" => signature = Some(PathBuf::from(&value)),
            "--key" => keys.push(PathBuf::from(&value)),
            "--channel" => channel = Some(text()),
            "--last-sequence" => {
                last_sequence = Some(
                    text()
                        .parse()
                        .map_err(|_| "--last-sequence takes a whole number")?,
                )
            }
            "--assets" => assets = Some(PathBuf::from(&value)),
            "--now" => now = Some(text()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let manifest: PathBuf = manifest.ok_or("--manifest is required")?;
    if keys.is_empty() || keys.len() > 2 {
        return Err("give one --key, or two during a key rotation".into());
    }
    let signature = signature.unwrap_or_else(|| {
        let mut s = manifest.clone().into_os_string();
        s.push(".sig");
        PathBuf::from(s)
    });
    Ok(Some(Args {
        manifest,
        signature,
        keys,
        channel,
        last_sequence,
        assets,
        now,
    }))
}

fn read_key(path: &PathBuf) -> Result<PublicKey, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| format!("{} holds no public key", path.display()))?;
    PublicKey::from_openssh(line).map_err(|e| format!("{}: {e}", path.display()))
}

fn run() -> Result<ExitCode, String> {
    let Some(args) = parse_args().map_err(|e| format!("{e}\n\n{USAGE}"))? else {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    let bytes = std::fs::read(&args.manifest)
        .map_err(|e| format!("reading {}: {e}", args.manifest.display()))?;
    let signature = std::fs::read_to_string(&args.signature)
        .map_err(|e| format!("reading {}: {e}", args.signature.display()))?;
    let current = read_key(&args.keys[0])?;
    let next = args.keys.get(1).map(read_key).transpose()?;
    let keys = TrustedKeys { current, next };
    let now = match &args.now {
        Some(t) => OffsetDateTime::parse(t, &Rfc3339).map_err(|e| format!("--now {t:?}: {e}"))?,
        None => OffsetDateTime::now_utc(),
    };
    let opts = Options {
        keys: &keys,
        now,
        channel: args.channel.as_deref(),
        last_sequence: args.last_sequence,
        assets_dir: args.assets.as_deref(),
    };
    match verify(&bytes, &signature, &opts) {
        Ok(v) => {
            println!(
                "verified: channel {} sequence {} sets {} signed-by {}{} assets-checked {} of {}",
                v.manifest.channel,
                v.manifest.sequence,
                v.manifest.sets.len(),
                v.signed_by,
                if v.signed_by_next { " (next key)" } else { "" },
                v.assets_checked,
                v.assets_listed
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(e @ Error::Failed { .. }) => {
            eprintln!("verification failed: {e}");
            Ok(ExitCode::from(20))
        }
        Err(Error::Io(detail)) => Err(detail),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
