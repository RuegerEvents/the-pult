#!/usr/bin/env bash
# The console's evaluator, built for a browser.
#
# One implementation, compiled twice: the station links `pult-render` natively, and
# this puts the same crate in the page. `wasm32-unknown-unknown` with `wasm-bindgen`,
# which is *not* the toolchain the plugins use — those are `wasm32-wasip2` components
# run by wasmtime on the host, and a component is the wrong thing for code that has to
# run inside a tab.
#
# The output lands beside the TypeScript `pult-codegen` writes and is gitignored for
# the same reason: it is generated, and a checked-in copy is a copy that can be stale.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$here/frontend/src/lib/evaluator"

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo "wasm-pack is not installed. cargo install wasm-pack" >&2
    exit 1
fi

# `--target web` rather than `bundler`: the page loads it itself with an explicit
# `init()`, which is what lets a caller decide whether the evaluator is worth loading
# at all rather than having it pulled in by whatever imported a type from it.
#
# `--no-pack` so no `package.json` is written: this is not a package, it is generated
# source inside one.
wasm-pack build "$here/crates/pult-render-wasm" \
    --target web \
    --no-pack \
    --out-dir "$out" \
    "${@:---release}"

echo "✓ evaluator → ${out#"$here"/}"
