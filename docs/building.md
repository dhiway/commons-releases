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
`DYLD_FALLBACK_LIBRARY_PATH="$(rustc --print sysroot)/lib"` inside its own
`dx bundle` command. `build-cmns.sh` sets the same variable itself on a Mac,
and fails if the strip warning appears anyway, instead of passing with an
unstripped binary.

Setting the variable in the caller's shell does not work. macOS removes every
`DYLD_*` variable when a protected system binary starts, and the script
starts through `#!/usr/bin/env bash`. A run on 2026-11-26 with the variable
exported by the caller still failed to strip. The script refused the
binary, as designed. To build on the primary Mac:

```sh
R=$HOME/.rustup/toolchains/1.99.0-aarch64-apple-darwin
PATH=$R/bin:$PATH RUSTC=$R/bin/rustc CARGO_BUILD_JOBS=4 \
  scripts/build-cmns.sh --src <commons-cli> --commit <sha> --out <dir> --offline
```

Remove the variable from the script when the pinned toolchain carries the
fix.

## On the AWS builder: what it can prove, and what it cannot yet

The owner's ruling is that release assets are built on the AWS builders. The
builder is `commons-services/deploy/aws/builder/builder.py`, read at
`c18b1d3`.

### What has run where (2026-11-26)

On this Mac, for `aarch64-apple-darwin`, with the stock 1.99.0 toolchain:

1. **commons-cli `6e94254`** (main at the time), with
   `DYLD_FALLBACK_LIBRARY_PATH` exported by the caller. The release build
   compiled, cargo printed the strip warning, and the script refused the
   binary (exit 1). The variable had not reached cargo, because of
   `/usr/bin/env` (above). That binary was not stripped, and no archive was
   made.
2. **commons-cli `667ba2b`**, script at `2138e82`, which sets the variable
   itself. The archive was made and the binary was stripped: no `__DWARF`
   segment, no `__debug_*` section, 183 symbols, none of them local. But the
   binary still held 28 standard-library source paths under
   `~/.rustup/toolchains/1.99.0-aarch64-apple-darwin/lib/rustlib/src/`, which
   name the build host's home directory. Script `3bdd58c` therefore remaps the
   toolchain's sysroot as well, and fails if the binary still names `$HOME` or
   `--out`. That first archive was discarded.
3. **commons-cli `667ba2b`, script `3bdd58c`**: the archive
   `cmns-0.1.0-aarch64-apple-darwin.tar.gz` (4,014,811 bytes, SHA-256
   `b452fd8876dee08606f02b39649aee3f55efe45fee04beb90e90efaf25cb6933`).
   The binary is 8,042,672 bytes. It has no strip warning, no `__DWARF`
   segment and no `__debug_*` section, and 183 symbols, none of them local.
   It names neither `$HOME` nor `--out`: the standard library's paths read
   `/rust/lib/rustlib/src/...`. `cmns version` reports 0.1.0, client
   revision `2460a97`. This is the first archive the script has produced
   and kept.

`667ba2b` is the head of a commons-cli work branch. It was
**unreviewed** when these archives were built, so they are proofs of the
script, not releases. Each archive's `BUILD-INFO.json` says so in `note`
(`build-cmns.sh --note`).

On the AWS builder, the proof command below ran twice at `667ba2b` on
2026-11-26. Each time `--cargo-home` named a fresh Cargo home that had fetched
only the `https://` form of the pin.

- The first run's build record says `ran`, with no error, at 10:24 UTC. Its
  logs were in a scratch directory that was cleared before anyone read them.
- The second run (`20261126T161629Z-be785968`, an `m6a.4xlarge`) has its logs
  kept. `cargo build --locked --offline --release -p cmns --bin cmns` exited 0
  after 228 s. The log has 524 `Compiling` lines, no warning and no error,
  and ends `Finished release profile [optimized] target(s) in 3m 48s`. The
  run's record shows the build user had no network: egress refused, no DNS,
  the metadata service refused. The builder was terminated, and `status`
  then listed no builder.

So the release build of `cmns` compiles offline on the builder from the
committed tree, for `x86_64-unknown-linux-gnu`. The run is a build, not a
test, so it has no test counts.

### The prerequisite: an `https://` pin

commons-cli `6e94254` pins `commons-node-client` at
`ssh://git@github.com/dhiway/commons-node-client.git`. Both repositories are
public, so access is not the obstacle. The obstacle is how the builder gets
dependencies:

- It never fetches on the instance. Every cargo command runs with
  `--locked --offline` (the builder adds both; passing them yourself is
  refused), in a network namespace that holds only loopback.
- Every dependency comes from a Cargo home built on the caller's machine:
  `--fetch` runs `cargo fetch --locked` there and ships the result.
- It refuses a Cargo home whose `git/db/*/config` names a URL with a user
  part (`check_credential_free`, pattern `://...@`). The cached checkout of
  an `ssh://git@` pin trips that check.

So the tree to build must pin `commons-node-client` at
`https://github.com/dhiway/commons-node-client.git`, as `667ba2b` does. Its
checkout then lands in the shipped Cargo home without credential material,
and the offline build finds it. The builder also needs exactly one Cargo git
database per dependency, and a Mac that has fetched the same repository over
both `ssh://` and `https://` has two. So pass `--cargo-home` with a new, empty
directory, which `--fetch` fills with the `https://` form only.

### The proof run

With `<the builder's AWS profile>` (the `PROFILE` that builder.py names)
valid (`aws sts get-caller-identity` names `<the builder's IAM identity>`):

```sh
B=<commons-services>/deploy/aws/builder/builder.py
CLI=<a commons-cli worktree whose committed head pins commons-node-client over https://>
LOGS=<a new, empty directory that outlives the session>
HOME_HTTPS=<a new, empty directory>
python3.13 $B run --task <task id> --owner <agent> \
  --repo $CLI --commit $(git -C $CLI rev-parse HEAD) \
  --cargo "build --release -p cmns --bin cmns" \
  --log-dir $LOGS --cargo-home $HOME_HTTPS --fetch --wait 1500
# exit 3 means still running: repeat the `next` command it prints (wait-run)
python3.13 $B status --task <task id>    # must list no builder at the end
```

That proves the release-profile `cmns` builds on the builder from the
committed tree, offline. `$LOGS/cargo-1.log` holds cargo's output and
`$LOGS/manifest.json` the toolchain and isolation record. If it fails, record
the exact command and error.

### What the run does not prove

`run` executes cargo commands only, so it cannot run this script. The run
does not produce the release archive:

1. **No script, no archive.** The builder cannot run `build-cmns.sh`, so
   it makes no `.tar.gz`, no `SHA256SUMS` and no `assets.json`. Its `build`
   and `collect` subcommands can bring the bare `cmns` binary back, re-hashed.
2. **No musl.** `--target` is refused, and the only extra toolchain targets
   it installs are the wasm32 ones. What it builds is the host target,
   `x86_64-unknown-linux-gnu`: the glibc binary the owner ruled out in
   favour of musl.
3. **No macOS.** The builders are Linux. `aarch64-apple-darwin`, and
   notarisation of the macOS node binaries, need a macOS host. That host
   could be an EC2 Mac instance or the build Mac on the local network. The
   owner has not ruled which.

For the builder to produce the release archives themselves, it needs:

- a `release` phase that runs `scripts/build-cmns.sh --offline` from a
  pinned commit of this repository and collects the files from `--out`;
- `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` in its
  allowed toolchain targets, and `musl-tools` in its packages.

Both changes belong to commons-services, not to this repository.
