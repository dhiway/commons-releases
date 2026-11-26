# commons-releases

This repository is where Commons publishes its downloadable builds: the node
(`cmn`, with `cmn-http`), the command-line tool `cmns` and the App. It holds no
source code. It holds, per channel, a signed manifest that says which builds
exist and how to check them; the builds themselves are release assets.

Clients (`cmns update`, later the App) fetch a channel's manifest, verify it
with the public keys they carry, and only then download anything. The
verifier in `verifier/` is the code that does that check; any client calls it,
and it never uses the network.

## Layout

| Path | What it is |
|---|---|
| `<channel>/manifest.json` | The channel's manifest, schema 1 ([docs/manifest.md](docs/manifest.md)). |
| `<channel>/manifest.json.sig` | Its detached OpenSSH signature ([docs/signing.md](docs/signing.md)). |
| `keys/<channel>-<n>.pub` | The public keys clients trust for a channel ([keys/README.md](keys/README.md)). |
| `examples/` | One example manifest per channel; not live, never signed. |
| `schema/manifest.schema.json` | The manifest's JSON Schema. |
| `verifier/` | `commons-release-verify`: the offline verifier, a Rust library and command. |
| `scripts/build-cmns.sh` | The release build of `cmns` archives ([docs/building.md](docs/building.md)). |
| `scripts/sign-manifest.sh` | Signs a channel's manifest and verifies the result. |

No channel directory exists yet: nothing has been released.

## Channel policy

- **`dev`**: the only channel until the owner declares the first release.
- **`beta`**: every public release is a `beta` until everything is released.
- **`stable`**: does not exist before then.

Every release on every channel is built with the optimised release profile
(`lto`, one codegen unit, stripped); the manifest records it in each set's
`build_profile`, and the verifier rejects a set whose profile is not a
release one. Assets are built on the AWS builders, reproducibly, and the
manifest that lists them is signed on the owner's machine. Building the macOS
assets needs a macOS host, which the Linux builders are not.

Where that stands today (2026-11-26): `build-cmns.sh` has run only on the
owner's Mac, for `aarch64-apple-darwin`. It has produced one kept archive:
stripped, with no build-host paths. That archive came from commons-cli
`667ba2b`, an **unreviewed** work-branch head, so it proves the script and is
not a release. Its `BUILD-INFO.json` says so. Two earlier attempts were
refused or discarded. At `6e94254` cargo could not strip the binary, and the
script refused it. A first `667ba2b` archive was stripped, but it named the
build host's home directory in standard-library paths, which the script now
remaps and checks for. On the AWS builder, the release build of `cmns`
compiles offline from the same commit. But the builder cannot run this
script, makes no archive, and builds a glibc binary, not the musl one.
[docs/building.md](docs/building.md) says what has run where and what the
builder still lacks.

## The set rule

A manifest lists *sets*: a node, a `cmns` and optionally an App that are known
to work together. One version number identifies a set: `cmns` carries the
node's version, so in every set `cmns.version` equals `node.version`, and the
verifier rejects a manifest where they differ.

A set with `format_change: true` changes the node's on-disk format. Node and
`cmns` then move together as one set or not at all, the node needs a new data
directory (`requires_data_dir: "new"`), and nothing is migrated or
downgraded. A set without a format change keeps the data directory
(`requires_data_dir: "same"`).

## The signing rule

- `stable` is signed by the owner, on the owner's machine, with an Ed25519 key
  held on a hardware token.
- `dev` and `beta` may be signed by CI, in a protected environment that holds
  the key.
- Nothing is ever signed on a build machine. The builders produce archives and
  `SHA256SUMS`; the signer checks them and signs the manifest that lists them.
- Each channel has two key slots, the key in use and the next one, so a key can
  be rotated or replaced without stranding clients.

The exact commands are in [docs/signing.md](docs/signing.md).

## Verifying a manifest

```sh
cargo run --release -p commons-release-verify -- \
  --manifest beta/manifest.json --key keys/beta-1.pub --channel beta \
  --last-sequence 41 --assets ~/Downloads/commons
```

It exits 0 when every check passes, 20 when one fails (stderr names it:
`verification failed: expires: the manifest expired at ...`), and 1 on any
other error. The checks, in order: `signature`, `schema`, `channel`,
`published`, `expires`, `sequence`, `set-version`, then `asset-size` and
`asset-sha256` for each asset found in `--assets` (and `assets-absent` if that
directory holds none of them).

## The verifier is a pinned interface

`cmns` does not repeat these checks. It depends on the `commons-release-verify`
library from this repository, over `https://` at a full commit, so a change to
the library's public interface breaks `cmns` at its next pin. That interface is
listed in the crate documentation (`verifier/src/lib.rs`) and exercised the
way a client uses it by `verifier/tests/client.rs`:

- `PublicKey::read_file` and `PublicKey::from_key_text` read a key;
  `TrustedKeys { current, next }` holds a channel's two slots.
- `verify(bytes, signature, &Options { keys, now, channel, last_sequence,
  assets_dir })` returns `Verified`, or `Error::Failed { check, detail }`
  with a `Check` to match on (`Check::as_str` is the name printed above), or
  `Error::Io`.
- `Verified::manifest.sets` gives each set's `format_change`,
  `requires_data_dir` and `components.cmns`, `.node` and `.app`, each with
  `version`, `commit` and `assets` keyed by target triple (`url`, `sha256`,
  `size`).
