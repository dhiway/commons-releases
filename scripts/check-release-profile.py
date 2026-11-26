#!/usr/bin/env python3
"""Refuses a release profile that is not the optimised one.

    scripts/check-release-profile.py DIR

DIR is the root of the workspace cargo will build from (build-cmns.sh passes
its exported tree). Exits 0 when the profile is the one every release must
use (owner ruling 2026-11-20), and 1 naming the first problem otherwise.

The workspace's Cargo.toml must have a [profile.release] that sets
lto = true, codegen-units = 1 and strip = true, and that does not weaken
anything else cargo would apply:

- opt-level, if set, is 3;
- debug-assertions and overflow-checks, if set, are false;
- there is no [profile.release.package.<name>] override, which would let any
  crate, or all of them with "*", be built differently.

No Cargo config file that cargo would read from DIR may set anything under
profile.release: .cargo/config.toml or .cargo/config in DIR and in each of
its parents, and config.toml or config in the Cargo home ($CARGO_HOME, or
~/.cargo). Cargo merges those over Cargo.toml, so a profile there would
change the build without changing the source. The environment
(CARGO_PROFILE_RELEASE_*, RUSTFLAGS) is checked by build-cmns.sh itself.

TOML is parsed with tomllib (Python 3.11 or later), so dotted keys such as
`profile.release.opt-level = 0` are seen as well as tables.
"""

import json
import os
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:  # Python < 3.11
    sys.exit("check-release-profile: needs Python 3.11 or later (tomllib)")


def load(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        sys.exit(f"check-release-profile: cannot read {path}: {e}")


def show(value):
    """A value as TOML writes it: true, 0, "z"."""
    return json.dumps(value)


def problems(root):
    manifest = root / "Cargo.toml"
    if not manifest.is_file():
        yield f"{manifest} does not exist"
        return
    release = load(manifest).get("profile", {}).get("release")
    if not isinstance(release, dict):
        yield f"{manifest} has no [profile.release]"
        return
    where = f"{manifest} [profile.release]"
    if release.get("lto") not in (True, "fat"):
        yield f"{where} does not set lto = true"
    if release.get("codegen-units") != 1:
        yield f"{where} does not set codegen-units = 1"
    if release.get("strip") not in (True, "symbols"):
        yield f"{where} does not set strip = true"
    if "opt-level" in release and release["opt-level"] not in (3, "3"):
        yield f"{where} sets opt-level = {show(release['opt-level'])}; a release is opt-level 3"
    for key in ("debug-assertions", "overflow-checks"):
        if release.get(key, False) is not False:
            yield f"{where} sets {key} = {show(release[key])}; a release turns it off"
    for name in release.get("package", {}):
        yield f"{where} has a per-package override, [profile.release.package.{show(name)}]"

    cargo_home = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")
    configs = [d / ".cargo" / n for d in (root, *root.parents) for n in ("config.toml", "config")]
    configs += [cargo_home / "config.toml", cargo_home / "config"]
    for path in configs:
        if path.is_file() and "release" in load(path).get("profile", {}):
            yield f"{path} sets profile.release; cargo would apply it over Cargo.toml"


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: check-release-profile.py DIR")
    found = list(problems(Path(sys.argv[1]).resolve()))
    for p in found:
        print(f"check-release-profile: {p}", file=sys.stderr)
    sys.exit(1 if found else 0)


if __name__ == "__main__":
    main()
