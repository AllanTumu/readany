#!/usr/bin/env bash
#
# One version number for this project.
#
# There were four: Cargo.toml, wasm/Cargo.toml, npm/package.json and the
# generated pkg/package.json, each carrying a copy of the same fact, in a
# release whose whole point is coherence. Three of them could drift silently
# and the published one would win.
#
# The source is Cargo.toml. This script derives the rest:
#
#   - wasm/Cargo.toml is *asserted* equal, not rewritten, because
#     wasm/src/lib.rs already refuses to compile when it disagrees. Rewriting
#     it here would defeat that check by making the two agree after the fact.
#   - npm/package.json is *stamped* from the source.
#   - pkg/package.json is generated from npm/package.json at build time by
#     build-wasm.sh, which asserts the two are otherwise identical.
#
#   ./scripts/version.sh            stamp npm/package.json from Cargo.toml
#   ./scripts/version.sh --check    fail on any disagreement, change nothing
#   ./scripts/version.sh --print    print the crate version and nothing else
#
set -euo pipefail

cd "$(dirname "$0")/.."

CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

# The first `version = "..."` inside [package]. `sed -n '/^\[package\]/,...'`
# rather than a bare grep, so a dependency pinned to a version string can never
# be mistaken for the package's own.
read_cargo_version() {
  sed -n '/^\[package\]/,/^\[/p' "$1" \
    | grep -m1 '^version *= *"' \
    | sed 's/.*"\(.*\)".*/\1/'
}

CRATE=$(read_cargo_version Cargo.toml)

# `--print` exists so the release workflow has one parser to trust rather than
# a second `grep version` of its own. It runs before the node checks below,
# because a machine asking "what version is this" should not need node
# installed to find out.
if [ "${1:-}" = "--print" ]; then
  printf '%s\n' "$CRATE"
  exit 0
fi

WASM=$(read_cargo_version wasm/Cargo.toml)
NPM=$(node -p "require('./npm/package.json').version")

if [ -z "$CRATE" ]; then
  echo "version.sh: could not read a version from Cargo.toml" >&2
  exit 1
fi

fail=0

if [ "$WASM" != "$CRATE" ]; then
  echo "version.sh: wasm/Cargo.toml is $WASM, Cargo.toml is $CRATE" >&2
  echo "            Edit wasm/Cargo.toml by hand — it is asserted, not stamped," >&2
  echo "            because wasm/src/lib.rs checks it at compile time." >&2
  fail=1
fi

if [ "$NPM" != "$CRATE" ]; then
  if [ "$CHECK" = "1" ]; then
    echo "version.sh: npm/package.json is $NPM, Cargo.toml is $CRATE" >&2
    echo "            Run ./scripts/version.sh to stamp it." >&2
    fail=1
  else
    node -e '
      const fs = require("fs");
      const path = "npm/package.json";
      const raw = fs.readFileSync(path, "utf8");
      // A targeted replacement rather than JSON.parse + stringify, so the file
      // keeps the hand-written key order and one-line arrays it was written
      // with. A reformatting diff would hide the one line that changed.
      const next = raw.replace(
        /("version"\s*:\s*)"[^"]*"/,
        `$1"${process.argv[1]}"`
      );
      if (next === raw) {
        console.error("version.sh: no version field in npm/package.json");
        process.exit(1);
      }
      fs.writeFileSync(path, next);
    ' "$CRATE"
    echo "version.sh: npm/package.json stamped $NPM -> $CRATE"
    NPM="$CRATE"
  fi
fi

# pkg/ is generated and gitignored, so it is only checked when it exists.
if [ -f pkg/package.json ]; then
  PKG=$(node -p "require('./pkg/package.json').version")
  if [ "$PKG" != "$CRATE" ]; then
    echo "version.sh: pkg/package.json is $PKG, Cargo.toml is $CRATE" >&2
    echo "            pkg/ is stale. Run ./scripts/build-wasm.sh." >&2
    fail=1
  fi
fi

[ "$fail" = "1" ] && exit 1

echo "version.sh: $CRATE (crate, wasm, npm${PKG:+, pkg})"
