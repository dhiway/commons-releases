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
# SSH_KEYGEN names the ssh-keygen to use (default: the one on PATH). A hardware
# key needs one built with FIDO support, such as Homebrew's
# /opt/homebrew/bin/ssh-keygen; Apple's /usr/bin/ssh-keygen has none.
#
# It runs, from the repository root:
#   ssh-keygen -Y sign -f KEY -n commons-release-manifest CHANNEL/manifest.json
# which writes CHANNEL/manifest.json.sig, then runs commons-release-verify
# against keys/CHANNEL-*.pub. If verification fails the new signature is
# removed and the script exits non-zero.
set -euo pipefail

die() { echo "sign-manifest: $*" >&2; exit 1; }
[ $# -ge 2 ] && [ $# -le 3 ] || die "usage: $0 CHANNEL KEY [ASSETS_DIR]"
CHANNEL=$1 KEY=$2 ASSETS=${3:-}
case "$CHANNEL" in dev|beta|stable) ;; *) die "channel must be dev, beta or stable" ;; esac
cd "$(dirname "$0")/.."

MANIFEST="$CHANNEL/manifest.json"
[ -f "$MANIFEST" ] || die "$MANIFEST does not exist"
[ -f "$KEY" ] || die "key file $KEY does not exist"
KEYS=()
for k in keys/"$CHANNEL"-*.pub; do [ -f "$k" ] && KEYS+=(--key "$k"); done
[ ${#KEYS[@]} -gt 0 ] || die "no keys/$CHANNEL-*.pub: commit the channel's public key first"
[ ${#KEYS[@]} -le 4 ] || die "more than two keys/$CHANNEL-*.pub; a channel has two key slots"

rm -f "$MANIFEST.sig"
"${SSH_KEYGEN:-ssh-keygen}" -Y sign -f "$KEY" -n commons-release-manifest "$MANIFEST"

ARGS=(--manifest "$MANIFEST" --channel "$CHANNEL" "${KEYS[@]}")
[ -z "$ASSETS" ] || ARGS+=(--assets "$ASSETS")
if ! cargo run --quiet --locked --release -p commons-release-verify -- "${ARGS[@]}"; then
  rm -f "$MANIFEST.sig"
  die "the signed manifest did not verify; the signature was removed"
fi
echo "sign-manifest: signed and verified $MANIFEST"
