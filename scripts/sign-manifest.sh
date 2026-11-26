#!/usr/bin/env bash
# Signs a channel's manifest and verifies the result with the channel's keys.
#
#   scripts/sign-manifest.sh CHANNEL KEY [ASSETS_DIR]
#
# CHANNEL     dev, beta or stable; signs CHANNEL/manifest.json
# KEY         the private key file ssh-keygen signs with: for stable, the
#             handle file of the owner's hardware key (~/.ssh/commons-releases-stable-1)
# ASSETS_DIR  optional: a directory holding the downloaded assets, which the
#             verification then checks against the manifest too
#
# KEY and ASSETS_DIR may be relative to the directory it is run from.
#
# SSH_KEYGEN names the ssh-keygen to use (default: the one on PATH). A hardware
# key needs one built with FIDO support, such as Homebrew's
# /opt/homebrew/bin/ssh-keygen; Apple's /usr/bin/ssh-keygen has none.
# COMMONS_RELEASE_VERIFY names a built commons-release-verify to run (default:
# build and run this repository's, with cargo run --release).
#
# Before signing, it refuses a manifest whose sequence is not higher than the
# one committed at HEAD, if HEAD has one: the last published manifest of the
# channel. Then it runs, from the repository root:
#   ssh-keygen -Y sign -f KEY -n commons-release-manifest CHANNEL/manifest.json
# which writes CHANNEL/manifest.json.sig, and runs commons-release-verify with
# --last-sequence set one above the committed sequence.
#
# Keys: keys/CHANNEL-<n>.pub, numbered from 1. A client carries two
# consecutive keys (current and next), so during a rotation three files can
# exist: clients still carrying n and n+1, newer ones carrying n+1 and n+2
# (docs/signing.md, "Rotating a key"). The signature must verify for every
# such pair, so every client still served accepts it. With one key file it is
# verified against that key alone.
#
# If any verification fails the new signature is removed and the script
# exits non-zero. It runs under the bash 3.2 that macOS ships.
set -euo pipefail

die() { echo "sign-manifest: $*" >&2; exit 1; }
abs() { case "$1" in /*) printf '%s\n' "$1" ;; *) printf '%s\n' "$PWD/$1" ;; esac; }
[ $# -ge 2 ] && [ $# -le 3 ] || die "usage: $0 CHANNEL KEY [ASSETS_DIR]"
CHANNEL=$1
KEY=$(abs "$2")
ASSETS=''
[ -z "${3:-}" ] || ASSETS=$(abs "$3")
case "$CHANNEL" in dev|beta|stable) ;; *) die "channel must be dev, beta or stable" ;; esac
[ -f "$KEY" ] || die "key file $KEY does not exist"
[ -z "$ASSETS" ] || [ -d "$ASSETS" ] || die "assets directory $ASSETS does not exist"
cd "$(dirname "$0")/.."

MANIFEST="$CHANNEL/manifest.json"
[ -f "$MANIFEST" ] || die "$MANIFEST does not exist"

# The channel's key numbers, in order.
FOUND=''
for k in keys/"$CHANNEL"-*.pub; do
  [ -e "$k" ] || continue
  n=${k#keys/"$CHANNEL"-}
  n=${n%.pub}
  case "$n" in ''|*[!0-9]*) die "$k is not named keys/$CHANNEL-<number>.pub" ;; esac
  FOUND="$FOUND$n"$'\n'
done
NUMBERS=()
while read -r n; do [ -z "$n" ] || NUMBERS+=("$n"); done < <(printf '%s' "$FOUND" | sort -n)
[ ${#NUMBERS[@]} -gt 0 ] || die "no keys/$CHANNEL-*.pub: commit the channel's public key first"
# One entry per key pair a client can carry: "a b", or "a" for a single key.
SLOTS=("${NUMBERS[0]}")
if [ ${#NUMBERS[@]} -gt 1 ]; then
  SLOTS=()
  for ((i = 0; i + 1 < ${#NUMBERS[@]}; i++)); do
    SLOTS+=("${NUMBERS[i]} ${NUMBERS[i+1]}")
  done
fi

# The last published sequence: the manifest committed at HEAD, if any.
git rev-parse --git-dir >/dev/null 2>&1 ||
  die "$(pwd) is not a Git checkout; the committed manifest gives the last sequence"
LAST=()
if git cat-file -e "HEAD:./$MANIFEST" 2>/dev/null; then
  seq_of() { python3 -c 'import json,sys; print(int(json.load(sys.stdin)["sequence"]))'; }
  committed=$(git show "HEAD:./$MANIFEST" | seq_of) || die "cannot read the sequence of $MANIFEST at HEAD"
  current=$(seq_of <"$MANIFEST") || die "cannot read the sequence of $MANIFEST"
  [ "$current" -gt "$committed" ] ||
    die "$MANIFEST has sequence $current, but HEAD's has $committed; raise it to $((committed + 1)) or more"
  LAST=(--last-sequence "$((committed + 1))")
fi

if [ -n "${COMMONS_RELEASE_VERIFY:-}" ]; then
  VERIFY=("$COMMONS_RELEASE_VERIFY")
else
  VERIFY=(cargo run --quiet --locked --release -p commons-release-verify --)
fi

rm -f "$MANIFEST.sig"
"${SSH_KEYGEN:-ssh-keygen}" -Y sign -f "$KEY" -n commons-release-manifest "$MANIFEST"

for slot in "${SLOTS[@]}"; do
  ARGS=(--manifest "$MANIFEST" --channel "$CHANNEL")
  [ ${#LAST[@]} -eq 0 ] || ARGS+=("${LAST[@]}")
  for n in $slot; do ARGS+=(--key "keys/$CHANNEL-$n.pub"); done
  [ -z "$ASSETS" ] || ARGS+=(--assets "$ASSETS")
  if ! "${VERIFY[@]}" "${ARGS[@]}"; then
    rm -f "$MANIFEST.sig"
    die "the signed manifest did not verify with keys/$CHANNEL-{${slot// /,}}.pub; the signature was removed"
  fi
done
echo "sign-manifest: signed and verified $MANIFEST"
