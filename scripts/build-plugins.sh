#!/usr/bin/env bash
# Build the plugins workspace to wasm components and put each .wasm beside its
# manifest, so a plugin directory is loadable exactly where it lies:
#
#   scripts/build-plugins.sh              release build (what an operator runs)
#   scripts/build-plugins.sh --debug      faster builds while developing
#   scripts/build-plugins.sh --watch      rebuild on change; with a backend
#                                         started with --plugins plugins/, that
#                                         is hot reload end to end
#
# Rust ≥1.82's wasm32-wasip2 target emits a proper component directly —
# there is no separate componentize step.

set -euo pipefail
cd "$(dirname "$0")/../plugins"

TARGET=wasm32-wasip2
PROFILE=release
PROFILE_FLAG=--release
WATCH=no

for arg in "$@"; do
    case "$arg" in
        --debug) PROFILE=debug; PROFILE_FLAG="" ;;
        --watch) WATCH=yes ;;
        *) echo "unknown flag: $arg (try --debug, --watch)"; exit 2 ;;
    esac
done

if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
    echo "The $TARGET target is not installed. One command adds it:"
    echo
    echo "    rustup target add $TARGET"
    exit 1
fi

build() {
    # $PROFILE_FLAG stays unquoted on purpose: in a debug build it is empty
    # and must vanish, not become an empty argument.
    cargo build --workspace --target "$TARGET" $PROFILE_FLAG

    # Each cdylib lands beside its manifest under the name the manifest states.
    for dir in */; do
        manifest="$dir/pult-plugin.toml"
        [ -f "$manifest" ] || continue
        wasm=$(sed -n 's/^wasm *= *"\(.*\)"/\1/p' "$manifest" | head -1)
        built="target/$TARGET/$PROFILE/$wasm"
        if [ -f "$built" ]; then
            cp "$built" "$dir/$wasm"
            echo "  $dir$wasm"
        else
            echo "  ! $built not found (expected by $manifest)" >&2
        fi
    done
}

build

if [ "$WATCH" = yes ]; then
    echo "watching for changes (^C stops)…"
    if command -v cargo-watch >/dev/null 2>&1; then
        exec cargo watch -w . -i '*.wasm' -i 'target/*' -s "'$0' $*"
    fi
    # No cargo-watch: a plain loop over mtimes does the job, one second behind.
    last=""
    while sleep 1; do
        now=$(find . -name '*.rs' -o -name 'pult-plugin.toml' -o -name 'config.toml' \
            | grep -v target | xargs stat -f '%m' 2>/dev/null | sort -rn | head -1)
        if [ "$now" != "$last" ]; then
            last="$now"
            build || true
        fi
    done
fi
