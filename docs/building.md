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

### This Mac's toolchain cannot strip

On 2026-11-26 the stock 1.99.0 toolchain on the primary Mac failed every strip:
`rust-objcopy` aborts because it cannot load `@rpath/libLLVM.dylib`. The
library is in the toolchain's `lib/`, but `rust-objcopy` looks for it under
`lib/rustlib/aarch64-apple-darwin/lib/`. Cargo prints a warning and leaves
the binary unstripped. Any release build made with that toolchain on that
machine is therefore unstripped, unless someone reads the warning.
`build-cmns.sh` fails on that warning. Until the toolchain is reinstalled,
build with the library on the fallback path:

```sh
R=$HOME/.rustup/toolchains/1.99.0-aarch64-apple-darwin
PATH=$R/bin:$PATH RUSTC=$R/bin/rustc CARGO_BUILD_JOBS=4 \
DYLD_FALLBACK_LIBRARY_PATH=$R/lib \
  scripts/build-cmns.sh --src <commons-cli> --commit <sha> --out <dir> --offline
```

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
3. **The CLI's dependency is pinned over `ssh://`.** `commons-cli` pins
   `commons-node-client` over `ssh://git@github.com/...`, and that
   dependency is fetched on the instance and fails there (the coordination
   repository's notes on the AWS builder's command).
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
- `commons-node-client` fetched over `https` or vendored, so the CLI builds
  without `ssh`.

Each change belongs to commons-services or commons-cli, not to this
repository.
