#!/usr/bin/env bash
#
# Build the npm package, and prove the three things about it that are easy to
# get wrong and impossible to notice afterwards:
#
#   1. the version is the crate's version, in every manifest;
#   2. no absolute path from this machine is in the blob;
#   3. pkg/package.json differs from npm/package.json in nothing but nothing.
#
# The package ships one .wasm and two JavaScript glues. The blob is
# byte-identical between wasm-pack's `nodejs` and `web` targets — verified
# below rather than assumed — so a browser build costs about 21 KB of glue
# instead of a second five-megabyte binary.
#
#   ./scripts/build-wasm.sh
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)

# Before the version check, not after: `version.sh` refuses a stale
# pkg/package.json, and the stale one is this script's own previous output.
# Checking a directory we are about to delete would mean the build could only
# ever run twice in a row.
rm -rf pkg pkg-web-tmp

./scripts/version.sh --check

# --- Why RUSTFLAGS and not `trim-paths` -------------------------------------
#
# The 0.1.0 blob carried 234 absolute paths from the machine that built it,
# spelling out a username and a directory layout. `trim-paths = "all"` is the
# tidy fix and cargo 1.97.1 rejects it as unstable; requiring nightly to cut a
# release is a worse trade than the paths. `--remap-path-prefix` has been
# stable for years and does the same job. It is here rather than in a config
# file because the prefixes are absolute and differ per machine.
#
# **Order matters, and it is the opposite of the obvious one.** rustc walks its
# remap rules in reverse and takes the first match, so the *last* rule wins.
# Written specific-first, `$HOME` swallowed everything and the blob came out
# full of `/home/.cargo/registry/...` — no username, but every rule after the
# first one dead. Broadest first, most specific last.
CARGO_SRC="${CARGO_HOME:-$HOME/.cargo}/registry/src"
export RUSTFLAGS="--remap-path-prefix=$HOME=/home --remap-path-prefix=$CARGO_SRC=/cargo --remap-path-prefix=$ROOT=/readany"

echo "==> building nodejs target"
wasm-pack build wasm --target nodejs --release --out-dir ../pkg

echo "==> building web target"
wasm-pack build wasm --target web --release --out-dir ../pkg-web-tmp

# One blob for both. If this ever stops being true the package has silently
# started shipping two five-megabyte binaries, so it is checked rather than
# trusted.
if ! cmp -s pkg/readany_wasm_bg.wasm pkg-web-tmp/readany_wasm_bg.wasm; then
  echo "build-wasm.sh: the nodejs and web targets produced different .wasm files." >&2
  echo "               Shipping one glue against the other's blob would break at" >&2
  echo "               runtime. Ship two blobs deliberately, or find out why." >&2
  exit 1
fi

# The web glue resolves its blob with `new URL('readany_wasm_bg.wasm',
# import.meta.url)`, so it has to sit beside it rather than in a subdirectory.
# Renamed instead of moved, for that reason.
mv pkg-web-tmp/readany_wasm.js pkg/readany_wasm_web.js
mv pkg-web-tmp/readany_wasm.d.ts pkg/readany_wasm_web.d.ts
rm -rf pkg-web-tmp

# The blob statically links about a hundred crates, and MIT, BSD and Apache-2.0
# all require their notices to travel with a binary. `docs/licensing.md` has
# said so since it was written; the package shipped readany's own LICENSE and
# nothing else until 0.2.0. Regenerated here rather than committed stale,
# because the list is a fact about the dependency tree that just built.
echo "==> collecting third-party licences"
./scripts/third-party-licenses.py > THIRD-PARTY-LICENSES.md

echo "==> restoring package metadata"
cp npm/package.json pkg/package.json
cp npm/README.md pkg/README.md
cp LICENSE pkg/LICENSE
cp THIRD-PARTY-LICENSES.md pkg/THIRD-PARTY-LICENSES.md

# npm/package.json is the source and pkg/package.json is generated. They were
# identical by hand and by luck; if they ever drift the published one wins in
# silence, which is the whole class of defect this release is about.
if ! cmp -s npm/package.json pkg/package.json; then
  echo "build-wasm.sh: pkg/package.json differs from npm/package.json" >&2
  exit 1
fi

# The blob must carry no path that says anything about the machine that built
# it: not the user's name, not the home directory, not the layout above the
# project. `$HOME` is checked literally as well as by shape, because a username
# that happens to look like a crate name would slip through a pattern.
echo "==> checking the blob for build-machine paths"
LEAKED=$(strings -a pkg/readany_wasm_bg.wasm \
  | grep -cE "$HOME|/Users/|/home/|$(id -un)" || true)
if [ "$LEAKED" != "0" ]; then
  echo "build-wasm.sh: $LEAKED build-machine paths survived --remap-path-prefix" >&2
  strings -a pkg/readany_wasm_bg.wasm \
    | grep -E "$HOME|/Users/|/home/|$(id -un)" | head -5 >&2
  exit 1
fi
REMAPPED=$(strings -a pkg/readany_wasm_bg.wasm | grep -cE "^/(cargo|readany)/" || true)
echo "    0 build-machine paths, $REMAPPED remapped to /cargo and /readany"

echo "==> sizes"
WASM_BYTES=$(wc -c < pkg/readany_wasm_bg.wasm | tr -d ' ')
echo "    wasm: $WASM_BYTES bytes"
(cd pkg && npm pack --dry-run 2>&1 | grep -E "package size|unpacked size|total files" | sed 's/^npm notice */    /')

echo
echo "pkg/ is ready. Publish with: cd pkg && npm publish"
