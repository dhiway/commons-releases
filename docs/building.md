# Building the `cmns` release archives

`scripts/build-cmns.sh` builds `cmns` from one commit of `commons-cli` with
the release profile. It writes, for each target:

- `cmns-<version>-<target>.tar.gz`, holding `cmns-<version>-<target>/cmns` and
  `BUILD-INFO.json` (version, commit, target, profile, `rustc -V`);
- `SHA256SUMS`, in `sha256sum -c` format;
- `assets.json`, each archive's file name, SHA-256 and size, which is what the
  manifest's `assets` entries need.

It signs nothing. The signer checks `SHA256SUMS` and signs the manifest
([signing.md](signing.md)).

## What it refuses

- A `[profile.release]` that does not set `lto = true`, `codegen-units = 1`
  and `strip = true`, and any `CARGO_PROFILE_RELEASE_*`, `RUSTFLAGS` or
  `CARGO_ENCODED_RUSTFLAGS` in the environment (owner ruling, 2026-11-20:
  every release is an optimised build).
- A build in which cargo reports that stripping failed. Cargo reports this
  only as a warning.
- A target that is not native to the host's operating system. The darwin
  targets build on a Mac, and the `linux-musl` targets on Linux.
- A non-empty `--out`.

It builds the commit's tree from `git archive`, never the working tree, with
`--locked`. It remaps the build paths so that the binary does not embed them.
On the host's own target it runs `cmns version` and checks that the reported
version matches the archive name.

## Running it

On a Mac, for `aarch64-apple-darwin`:

```sh
scripts/build-cmns.sh --src ../commons-cli --commit <sha> --out dist/cmns-<sha>
```

On an x86_64 Linux host, for `x86_64-unknown-linux-musl`:

```sh
rustup target add x86_64-unknown-linux-musl    # once
sudo apt-get install musl-tools                 # once: ring compiles C with musl-gcc
scripts/build-cmns.sh --src ../commons-cli --commit <sha> --out dist/cmns-<sha>
```

Add `--offline` when the host has no network and the Cargo home already holds
every dependency.

### The darwin toolchain cannot strip without help

Since Rust 1.98, the `aarch64-apple-darwin` toolchain's own `rust-objcopy`
(`lib/rustlib/aarch64-apple-darwin/bin/rust-objcopy`) has linked
`@rpath/libLLVM.dylib` with an rpath that misses `<sysroot>/lib`. It aborts,
and cargo reports only a warning (`stripping debug info with rust-objcopy
failed`) and leaves the binary unstripped. The upstream report is
rust-lang/rust#164063. The fix, rust-lang/rust#164134, is in no released
stable yet. This was seen on 2026-11-26 with 1.99.0 on the primary Mac.

The App's release script has the precedent for the workaround:
`commons-app/scripts/release-macos.sh:121-126` explains it and `:132` sets
`DYLD_FALLBACK_LIBRARY_PATH="$(rustc --print sysroot)/lib"` for its
`dx bundle` step. Build `cmns` the same way. What `build-cmns.sh` adds is that
it fails when the warning appears, instead of passing with an unstripped
binary:

```sh
R=$HOME/.rustup/toolchains/1.99.0-aarch64-apple-darwin
PATH=$R/bin:$PATH RUSTC=$R/bin/rustc CARGO_BUILD_JOBS=4 \
DYLD_FALLBACK_LIBRARY_PATH="$($R/bin/rustc --print sysroot)/lib" \
  scripts/build-cmns.sh --src <commons-cli> --commit <sha> --out <dir> --offline
```

Drop the variable when the pinned toolchain carries the fix.

## On the AWS builder: not yet possible

The owner's ruling is that release assets are built on the AWS builders. The
builder (`commons-services/deploy/aws/builder/builder.py`, read at
`c18b1d3`) cannot produce these archives yet:

1. **It does not run scripts.** Its `build` phase runs
   `cargo build <options>` from a fixed list and collects binaries from
   `target/release/`. It cannot run `build-cmns.sh` or package an archive.
2. **It refuses `--target`.** The only extra toolchain targets it installs
   are the wasm32 ones. A `musl` build is therefore impossible. A build there
   produces the builder's host target, `x86_64-unknown-linux-gnu`, which is
   the glibc binary the owner ruled out in favour of musl.
3. **It cannot fetch the CLI's git dependency.** `commons-cli` pins
   `commons-node-client` as a git dependency at
   `ssh://git@github.com/dhiway/commons-node-client.git`. That repository is
   public now, so access is no longer the obstacle. The builder runs every
   cargo command with `--locked --offline`, in a network namespace that holds
   only loopback. A git dependency that the shipped Cargo home does not
   already hold therefore cannot be fetched there, public or not. The
   `ssh://git@` form adds a second obstacle: the builder refuses a Cargo home
   whose `git/db/*/config` names a URL with a user part
   (`check_credential_free`, pattern `://...@`). Pinning the dependency at
   its `https://` URL is the likely fix, but nobody has tried it on the
   builder.
4. **The builders are Linux.** `aarch64-apple-darwin`, and notarisation of
   the macOS node binaries, need a macOS host. That host could be an EC2 Mac
   instance or the build Mac on the local network. The owner has not ruled
   which.

The closest the builder can come today is a release build of the
glibc binary, without an archive. It has not been run:

```sh
B=commons-services/deploy/aws/builder/builder.py
python3.13 $B launch --task <task> --owner <agent>
python3.13 $B build --task <task> --repo <commons-cli worktree> --commit <sha> \
  --build "--release -p cmns --bin cmns" --wait 3600
python3.13 $B collect --task <task> --commit <sha> --out <dir>   # re-downloads and re-hashes
python3.13 $B terminate --task <task>
```

The builder would run this script if it had:

- a `release` phase that runs `scripts/build-cmns.sh --offline` from a
  pinned commit of this repository and collects the files from `--out`;
- `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` in its
  allowed toolchain targets, and `musl-tools` in its packages;
- `commons-node-client` pinned at its public `https://` URL (or vendored), so
  that `--fetch` caches it on the caller's machine and the bundle can carry
  it.

Each change belongs to commons-services or commons-cli, not to this
repository.
