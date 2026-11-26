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

- **`dev`**: the only channel until the owner declares the first release. It
  lives while this repository is private, so nothing is published early.
- **`beta`**: every public release is a `beta` until everything is released.
- **`stable`**: does not exist before then.

Every release on every channel is built with the optimised release profile
(`lto`, one codegen unit, stripped); the manifest records it in each set's
`build_profile`, and the verifier rejects a set whose profile is not a
release one. Assets are built on the AWS builders, reproducibly, and the
manifest that lists them is signed on the owner's machine. Building the macOS
assets needs a macOS host, which the Linux builders are not; see
[docs/building.md](docs/building.md).

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
