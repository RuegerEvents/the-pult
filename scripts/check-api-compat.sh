#!/usr/bin/env bash
# Does a plugin built against an older contract still run on this station?
#
# The plugin API's whole compatibility story is one claim: a component built
# against `pult:plugin@1.0.0` instantiates against a host offering `1.1.0`, so
# adding an interface does not strand the plugins already in people's showfiles.
# That claim is not self-evident — a component's imports carry the package
# version they were built against, and they link only because wasmtime resolves
# them semver-compatibly. It is also why the package cannot live at `0.x`, where
# a minor bump is breaking by definition and every import fails to resolve.
#
# So this script checks it rather than trusting it:
#
#   1. build the reference plugins against the contract as it stands
#   2. put one aside — that is the "old" plugin
#   3. bump the contract's minor, and the version the station claims with it
#   4. run the station against the plugin from step 2
#   5. put the contract back, whatever happened
#
# Nothing is checked in: the fixture is a build output, made and thrown away.
#
#   scripts/check-api-compat.sh
#
# Needs the wasm32-wasip2 target, like any plugin build.

set -euo pipefail
cd "$(dirname "$0")/.."

WIT=wit/pult-plugin.wit
MANIFEST=crates/pult-backend/src/infra/plugins/manifest.rs
WORK=$(mktemp -d)

# The guard comes first, and the trap is installed only after it passes.
#
# Order matters more than it looks: `restore` throws away whatever is in these
# two files, so installing the trap before this check would mean the guard's own
# `exit 1` destroyed exactly the uncommitted work it exists to protect.
if ! git diff --quiet -- "$WIT" "$MANIFEST"; then
    echo "! $WIT or $MANIFEST has uncommitted changes."
    echo "  This script rewrites both and restores them with git checkout, which"
    echo "  would take your edits with it. Commit or stash them first."
    rm -rf "$WORK"
    exit 1
fi

# Put the contract back even on a failure or a Ctrl-C: leaving the tree on a
# bumped version would be a worse outcome than the check not running.
restore() {
    git checkout -- "$WIT" "$MANIFEST" 2>/dev/null || true
    rm -rf "$WORK"
}
trap restore EXIT

version=$(sed -n 's/^package pult:plugin@\([0-9]*\)\.\([0-9]*\)\.\([0-9]*\);/\1 \2 \3/p' "$WIT")
read -r major minor patch <<<"$version"
if [ -z "${minor:-}" ]; then
    echo "! could not read the package version out of $WIT"
    exit 1
fi
next=$((minor + 1))

echo "==> building the reference plugins against $major.$minor.$patch"
scripts/build-plugins.sh >/dev/null

echo "==> setting one aside as the old plugin"
mkdir -p "$WORK/command-line"
cp plugins/command-line/command_line.wasm "$WORK/command-line/"
cp plugins/command-line/pult-plugin.toml "$WORK/command-line/"
cp -r plugins/command-line/assets "$WORK/command-line/" 2>/dev/null || true
imports=$(strings "$WORK/command-line/command_line.wasm" | grep -o "pult:plugin/data@[0-9.]*" | head -1)
echo "    it imports $imports"

echo "==> bumping the contract to $major.$next.0"
sed -i.bak "s/^package pult:plugin@$major\.$minor\.$patch;/package pult:plugin@$major.$next.0;/" "$WIT"
sed -i.bak "s/ApiVersion { major: $major, minor: $minor }/ApiVersion { major: $major, minor: $next }/" "$MANIFEST"
rm -f "$WIT.bak" "$MANIFEST.bak"

echo "==> running the $major.$minor plugin against a $major.$next station"
if PULT_OLD_API_PLUGINS="$WORK/command-line" \
    cargo test -p pult-backend --test plugins -- --ignored --nocapture \
    a_plugin_built_against_an_earlier_api_still_runs; then
    echo
    echo "OK — a $major.$minor plugin runs on a $major.$next station."
    echo "     Adding an interface is safe for the plugins already out there."
else
    echo
    echo "FAILED — a $major.$minor plugin does NOT run on a $major.$next station."
    echo "         Adding an interface would strand every plugin in every showfile."
    echo "         Do not ship a minor bump until this passes."
    exit 1
fi
