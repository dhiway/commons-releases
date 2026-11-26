# The channel manifest, schema 1

One JSON file per channel, `<channel>/manifest.json`, with a detached
signature next to it. `schema/manifest.schema.json` says the same as a JSON
Schema; the verifier (`verifier/src/manifest.rs`) is the reference, and where
the two disagree the verifier is right and the schema file is the bug.

## What is signed

The signature covers **the file's bytes exactly as published**. There is no
canonical re-encoding (no JCS): the verifier checks the signature over the
bytes it read and parses them only after that passes. So the file must reach
clients byte for byte as it was signed. Do not reformat, re-indent or
re-save a signed manifest; `.gitattributes` stops Git from converting its line
endings. Any edit means a new `sequence` and a new signature.

## Fields

Unknown fields are rejected at every level. A new field means a new `schema`
number and a client that reads it.

```text
manifest      = {
  "schema":    1,                        always 1
  "channel":   "dev" | "beta" | "stable",
  "published": TIMESTAMP,                when it was signed; not in the future
  "sequence":  INTEGER >= 0,             one higher than the channel's previous manifest
  "expires":   TIMESTAMP,                later than published; clients refuse it after this
  "sets":      [ set, ... ]              at least one
}
set           = {
  "id":                STRING,           non-empty, unique in the manifest; the set's version
  "format_change":     BOOLEAN,          true when the node's on-disk format changes
  "requires_data_dir": "new" | "same",   "new" exactly when format_change is true
  "components": {
    "cmns": component,                   required
    "node": component,                   required; same version as cmns
    "app":  component                    optional
  },
  "notes_url":     HTTPS_URL,            the release notes
  "build_profile": STRING                starts with "release", e.g.
                                         "release: lto=true, codegen-units=1, strip=true"
}
component     = {
  "version":  VERSION,                   MAJOR.MINOR.PATCH, optional -PRERELEASE
  "commit":   HEX40,                     the source commit the assets were built from
  "assets":   { PLATFORM: asset, ... },  at least one
  "storage_format": INTEGER,             optional; the node's on-disk format number
  "sync_alpn":      STRING,              optional; the node's sync protocol ALPN
  "notarized":      BOOLEAN              optional; macOS assets are notarised
}
asset         = {
  "url":    HTTPS_URL,                   ends in the asset's file name
  "sha256": HEX64,                       lowercase
  "size":   INTEGER > 0                  bytes
}
PLATFORM      = a target triple in [a-z0-9_-], e.g. aarch64-apple-darwin,
                x86_64-unknown-linux-musl, universal-apple-darwin (the App)
TIMESTAMP     = RFC 3339, e.g. 2026-12-01T10:00:00Z
```

## Invariants the verifier enforces

- **One number per set** (owner ruling, 2026-11-26): `components.cmns.version`
  equals `components.node.version` in every set. Reported as the
  `set-version` check.
- A format change requires a new data directory, and only a format change
  does.
- Every asset's file name (the URL's last path segment) is unique in the
  manifest, because a client stores and checks assets by that name.
- Every set is a release-profile build.

## Freshness

`sequence` and `expires` stop an attacker, or a stale mirror, from serving an
older manifest: a client remembers the highest `sequence` it accepted per
channel and refuses a lower one, and refuses any manifest past `expires`. A
channel's manifest must therefore be re-signed, with a higher `sequence`,
before it expires even when nothing new is released. Suggested lifetimes:
30 days for `dev` and `beta`, 90 days for `stable`.

## Examples

`examples/beta/manifest.json` (one set, with the App) and
`examples/stable/manifest.json` (two sets, the second a format change). Their
hashes, sizes and commits are made up. The tests sign each with a throwaway
key and verify it.

## Fields dropped from the research proposal

The research note's proposal (an internal design note of the project, not
published) gave `cmns`
`min_node` and `max_node`. With `cmns` carrying the node's version, the set
already names the node a `cmns` goes with, so schema 1 leaves them out. Its
example also gave `cmns` version 2.1.0 beside node 0.9.40, which the owner's
ruling now forbids.
