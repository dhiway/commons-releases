#!/usr/bin/env bash
# Fails unless a binary is stripped, by reading the binary itself.
#
#   scripts/check-stripped.sh BINARY
#
# Cargo's strip = true can fail with only a warning and leave the binary as it
# was (docs/building.md), so build-cmns.sh checks the artifact, not cargo's
# output. "Stripped" means, by format:
#
# - Mach-O (macOS), with otool and nm: no __DWARF segment and no __debug_*
#   section; no debug-map (stab) entry, which is how a Mac binary points at
#   its debug information; no local symbol. The external symbols dyld binds
#   stay, as they do after strip.
# - ELF (Linux), with readelf: no .debug_* section and no .symtab. Stripping
#   symbols removes .symtab whole; .dynsym, which the loader needs, stays.
#
# It exits 0 when the binary is stripped, 1 naming what remains otherwise, and
# 1 when the tool it needs is missing.
set -euo pipefail

die() { echo "check-stripped: $*" >&2; exit 1; }
[ $# -eq 1 ] || die "usage: $0 BINARY"
BIN=$1
[ -f "$BIN" ] || die "$BIN does not exist"

need() { command -v "$1" >/dev/null || die "$1 not found; it is needed to check $BIN"; }

case "$(head -c 4 "$BIN" | od -An -tx1 | tr -d ' \n')" in
  cffaedfe|cefaedfe|cafebabe)
    need otool
    need nm
    # Captured first: with pipefail, grep -q stopping early would fail the pipe.
    loads=$(otool -l "$BIN")
    if grep -Eq '^ *(segname __DWARF|sectname __debug_)' <<<"$loads"; then
      die "$BIN has a __DWARF segment or __debug_ section: it carries debug information"
    fi
    # nm -a lists stab entries with type "-"; a lowercase type is a local symbol.
    # Apple's strip itself adds one stab, "OPT radr://5614542", which marks a
    # stripped binary and points at nothing, so that one is not counted.
    stabs=$(nm -a "$BIN" | awk '$2 == "-" && $5 != "OPT"' | wc -l | tr -d ' ')
    [ "$stabs" -eq 0 ] || die "$BIN has $stabs debug-map (stab) entries: it was not stripped"
    locals=$(nm "$BIN" | awk 'NF >= 3 && $2 ~ /^[a-z]$/' | wc -l | tr -d ' ')
    [ "$locals" -eq 0 ] || die "$BIN has $locals local symbols: it was not stripped"
    ;;
  7f454c46)
    need readelf
    sections=$(readelf -SW "$BIN")
    if grep -q ' \.debug_' <<<"$sections"; then
      die "$BIN has .debug_ sections: it carries debug information"
    fi
    if grep -q ' \.symtab ' <<<"$sections"; then
      die "$BIN has a .symtab: its symbols were not stripped"
    fi
    ;;
  *) die "$BIN is neither a Mach-O nor an ELF binary" ;;
esac
echo "check-stripped: $BIN is stripped"
