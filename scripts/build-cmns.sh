#!/usr/bin/env bash
# Builds the cmns release archives for one commit of commons-cli.
#
#   scripts/build-cmns.sh --src DIR --commit REV --out DIR [--target TRIPLE]... [--offline] [--note TEXT]
#
# --src      a clone of commons-cli; nothing in it is written to
# --commit   the commit to build; its tree is exported with `git archive`, so
#            uncommitted changes in --src never reach the build
# --out      a new or empty directory; it receives the archives, SHA256SUMS,
#            assets.json and the build tree (src/, target/)
# --target   a target triple to build for, native to this host's OS (repeat;
#            default: the host's own release target, see below)
# --offline  pass --offline to cargo (the AWS builder has no network)
# --note     a line recorded as "note" in BUILD-INFO.json, for a build that is
#            not a release (for example, a proof from an unreviewed head);
#            leave it out for a release
#
# For each target it runs `cargo build --locked --release -p cmns --target T`
# and packs `cmns-<version>-<target>.tar.gz` holding `cmns` and BUILD-INFO.json.
# The archive adds nothing that varies between runs: entries are owned by
# 0:0 and dated at the commit time, gzip carries no name or time, and the build
# paths are remapped (--remap-path-prefix) so the binary does not embed --out,
# the Cargo home or the toolchain's directory; the script fails if the binary
# still names --out or $HOME. Whether the binary itself is bit-for-bit reproducible
# across hosts has not been tested. Nothing is signed here: SHA256SUMS and the
# manifest are signed on the owner's machine (docs/signing.md).
#
# It refuses to run when the release profile is weakened: the workspace's
# [profile.release] must set lto = true, codegen-units = 1 and strip = true,
# and no CARGO_PROFILE_RELEASE_*, RUSTFLAGS or CARGO_ENCODED_RUSTFLAGS may be
# set. It fails when cargo reports that stripping failed. Cargo treats that
# only as a warning and leaves the binary unstripped. On aarch64-apple-darwin
# since Rust 1.98, the toolchain's rust-objcopy cannot load libLLVM.dylib
# (rust-lang/rust#164063; the fix, #164134, is in no released stable yet).
# On a Mac this script therefore sets
# DYLD_FALLBACK_LIBRARY_PATH="$(rustc --print sysroot)/lib" itself. The caller
# cannot set it: macOS removes DYLD_* variables when a protected binary starts,
# and this script starts through /usr/bin/env. The App's release script is the
# precedent: commons-app/scripts/release-macos.sh explains the workaround at
# :121-126 and sets it inside its own command at :132. What this script adds is
# the failure when the warning appears anyway.
#
# Scope, as of 2026-11-26:
# - It has produced one aarch64-apple-darwin archive, on the owner's Mac, from
#   commons-cli 667ba2b (an unreviewed head; a proof, not a release).
#   docs/building.md lists the runs, including the two that failed.
# - The linux-musl branch is written but has never run: it needs an x86_64
#   Linux host with `rustup target add x86_64-unknown-linux-musl` and
#   `musl-gcc` (the musl-tools package), because `ring` compiles C.
# - The AWS builder cannot run this script or build a musl target. It runs
#   only fixed cargo commands, so the most it can prove is that the release
#   build compiles there, for the host target x86_64-unknown-linux-gnu
#   (glibc), with no archive:
#
#     python3.13 commons-services/deploy/aws/builder/builder.py run \
#       --task <task id> --owner <agent> --repo <commons-cli worktree> \
#       --commit <sha> --cargo "build --release -p cmns --bin cmns" \
#       --log-dir <new empty dir> --cargo-home <new empty dir> --fetch --wait 1500
#
#   Prerequisite: the commit must pin commons-node-client at its https://
#   URL. The builder never fetches on the instance. --fetch runs
#   `cargo fetch --locked` on the caller's machine and ships that Cargo home.
#   The builder adds --locked --offline itself, and check_credential_free
#   refuses a Cargo home whose git/db/*/config names a URL with a user part,
#   which an ssh://git@ pin leaves behind. docs/building.md has the details.
set -euo pipefail

die() { echo "build-cmns: $*" >&2; exit 1; }

SRC='' COMMIT='' OUT='' NOTE=''
OFFLINE=()
TARGETS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --src) SRC=$2; shift 2 ;;
    --commit) COMMIT=$2; shift 2 ;;
    --out) OUT=$2; shift 2 ;;
    --target) TARGETS+=("$2"); shift 2 ;;
    --offline) OFFLINE=(--offline); shift ;;
    --note) NOTE=$2; shift 2 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d'; exit 0 ;;
    *) die "unknown argument $1" ;;
  esac
done
[ -n "$SRC" ] && [ -n "$COMMIT" ] && [ -n "$OUT" ] || die "--src, --commit and --out are required"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) HOST_FAMILY=darwin DEFAULT=aarch64-apple-darwin ;;
  Linux-x86_64) HOST_FAMILY=linux DEFAULT=x86_64-unknown-linux-musl ;;
  Linux-aarch64) HOST_FAMILY=linux DEFAULT=aarch64-unknown-linux-musl ;;
  *) die "unsupported host $(uname -s)-$(uname -m)" ;;
esac
[ ${#TARGETS[@]} -gt 0 ] || TARGETS=("$DEFAULT")
for t in "${TARGETS[@]}"; do
  case "$t" in
    *-apple-darwin) [ "$HOST_FAMILY" = darwin ] || die "$t builds on a Mac, not on this host" ;;
    *-linux-musl) [ "$HOST_FAMILY" = linux ] || die "$t builds on a Linux host, not on this one" ;;
    *) die "$t is not a release target (darwin or linux-musl)" ;;
  esac
done

# The release profile must be the optimised one (owner ruling 2026-11-20).
for v in RUSTFLAGS CARGO_ENCODED_RUSTFLAGS $(env | sed -n 's/^\(CARGO_PROFILE_RELEASE_[A-Z_]*\)=.*/\1/p'); do
  [ -z "${!v:-}" ] || die "$v is set; it would change the release build"
done

SHA=$(git -C "$SRC" rev-parse --verify "$COMMIT^{commit}") || die "$COMMIT is not a commit in $SRC"
EPOCH=$(git -C "$SRC" show -s --format=%ct "$SHA")
mkdir -p "$OUT"
[ -z "$(ls -A "$OUT")" ] || die "$OUT is not empty"
OUT=$(cd "$OUT" && pwd)
mkdir "$OUT/src"
git -C "$SRC" archive --format=tar "$SHA" | tar -x -C "$OUT/src"

PROFILE=$(awk '/^\[/{s=($0=="[profile.release]")} s&&/^[a-z-]+ *=/' "$OUT/src/Cargo.toml")
echo "$PROFILE" | grep -Eq '^lto *= *(true|"fat")' || die "[profile.release] does not set lto = true"
echo "$PROFILE" | grep -Eq '^codegen-units *= *1$' || die "[profile.release] does not set codegen-units = 1"
echo "$PROFILE" | grep -Eq '^strip *= *(true|"symbols")' || die "[profile.release] does not set strip = true"
PROFILE_LINE="release: $(echo "$PROFILE" | sed 's/ *= */=/' | paste -sd, - | sed 's/,/, /g')"

cd "$OUT/src"
export CARGO_TARGET_DIR="$OUT/target"
# Keep build paths out of the binary. The separator is 0x1f, so paths with
# spaces survive. The sysroot is remapped too: with rust-src installed, the
# standard library's panic locations name the toolchain's directory.
CARGO_ENCODED_RUSTFLAGS=$(printf '%s\037%s\037%s' \
  "--remap-path-prefix=$OUT/src=/build/cmns" \
  "--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo" \
  "--remap-path-prefix=$(rustc --print sysroot)=/rust")
export CARGO_ENCODED_RUSTFLAGS
VERSION=$(cargo metadata --no-deps --format-version 1 ${OFFLINE[@]+"${OFFLINE[@]}"} \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="cmns"))')
RUSTC_V=$(rustc -V)
echo "build-cmns: cmns $VERSION at $SHA, $PROFILE_LINE, $RUSTC_V"
if [ "$HOST_FAMILY" = darwin ]; then
  # rust-objcopy's libLLVM.dylib, see the header; cargo and rustc pass it on.
  DYLD_FALLBACK_LIBRARY_PATH="$(rustc --print sysroot)/lib"
  export DYLD_FALLBACK_LIBRARY_PATH
fi

for t in "${TARGETS[@]}"; do
  CC_VAR="CC_${t//-/_}"
  if [ "$HOST_FAMILY" = linux ] && [ -z "${!CC_VAR:-}" ]; then
    command -v musl-gcc >/dev/null || die "musl-gcc not found; install musl-tools (ring compiles C)"
    export "$CC_VAR=musl-gcc"
  fi
  cargo build --locked ${OFFLINE[@]+"${OFFLINE[@]}"} --release -p cmns --target "$t" 2>&1 | tee "$OUT/cargo-$t.log"
  if grep -q 'stripping debug info with .* failed' "$OUT/cargo-$t.log"; then
    die "cargo could not strip the $t binary (see $OUT/cargo-$t.log); the release profile was not applied"
  fi
  BIN="$OUT/target/$t/release/cmns"
  [ -x "$BIN" ] || die "no binary at $BIN"
  for p in "$OUT/" "$HOME/"; do
    ! LC_ALL=C grep -qaF "$p" "$BIN" || die "the $t binary still names $p (a build path was not remapped)"
  done
  if [ "$t" = "$DEFAULT" ]; then
    # The binary must report the version the archive is named for.
    "$BIN" version | python3 -c 'import json,sys; v=json.load(sys.stdin)["cmns"]; sys.exit(0 if v==sys.argv[1] else "cmns reports "+v+", expected "+sys.argv[1])' "$VERSION"
  fi
  python3 - "$OUT" "$t" "$VERSION" "$SHA" "$EPOCH" "$PROFILE_LINE" "$RUSTC_V" "$BIN" "$NOTE" <<'PY'
import gzip, io, json, sys, tarfile
out, target, version, commit, epoch, profile, rustc, binary, note = sys.argv[1:]
stem = f"cmns-{version}-{target}"
meta = {"component": "cmns", "version": version, "commit": commit, "target": target,
        "build_profile": profile, "rustc": rustc}
if note:
    meta["note"] = note
info = json.dumps(meta, indent=2).encode() + b"\n"
def add(tar, name, data, mode):
    ti = tarfile.TarInfo(f"{stem}/{name}")
    ti.size, ti.mode, ti.mtime = len(data), mode, int(epoch)
    ti.uid = ti.gid = 0
    ti.uname = ti.gname = ""
    tar.addfile(ti, io.BytesIO(data))
raw = io.BytesIO()
with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tar:
    add(tar, "BUILD-INFO.json", info, 0o644)
    add(tar, "cmns", open(binary, "rb").read(), 0o755)
with open(f"{out}/{stem}.tar.gz", "wb") as f, gzip.GzipFile(filename="", mode="wb", fileobj=f, mtime=0) as gz:
    gz.write(raw.getvalue())
PY
done

python3 - "$OUT" "$VERSION" <<'PY'
import hashlib, json, os, sys
out, version = sys.argv[1:]
prefix = f"cmns-{version}-"
names = sorted(n for n in os.listdir(out) if n.startswith(prefix) and n.endswith(".tar.gz"))
sums, assets = [], {}
for n in names:
    data = open(os.path.join(out, n), "rb").read()
    digest = hashlib.sha256(data).hexdigest()
    sums.append(f"{digest}  {n}\n")
    target = n[len(prefix):-len(".tar.gz")]
    assets[target] = {"file": n, "sha256": digest, "size": len(data)}
open(os.path.join(out, "SHA256SUMS"), "w").write("".join(sums))
open(os.path.join(out, "assets.json"), "w").write(json.dumps(assets, indent=2) + "\n")
PY
echo "build-cmns: wrote to $OUT:"
cat "$OUT/SHA256SUMS"
