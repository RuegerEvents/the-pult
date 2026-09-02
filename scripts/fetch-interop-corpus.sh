#!/usr/bin/env bash
# Other people's GDTF and MVR files, and the schemas they are supposed to match.
#
# Everything this downloads lands in testdata/corpus/, which is gitignored: these are
# other people's files under other people's licences, and a corpus checked into the
# repository would be both large and not ours to redistribute. What *is* checked in
# is testdata/gdtf/ — small fixtures written here, which the default test suite reads.
#
# The tests that read the corpus are all `#[ignore]`, so a clone that has never run
# this script has a passing suite. Run it, then:
#
#     cargo test -p pult-gdtf -p pult-mvr -- --ignored
#
# GDTF Share files need credentials, since the Share requires a login for downloads:
#
#     GDTF_SHARE_USER=you@example.com GDTF_SHARE_PASSWORD=... scripts/fetch-interop-corpus.sh
#
# Without them the script still fetches the schemas and the openly-hosted MVR
# examples, and says which part it skipped.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
corpus="$root/testdata/corpus"
mkdir -p "$corpus/gdtf" "$corpus/mvr" "$corpus/schema"

say() { printf '\033[1m%s\033[0m\n' "$*"; }
warn() { printf '\033[33m%s\033[0m\n' "$*" >&2; }

# ── The schemas ──────────────────────────────────────────────────────
# mvrdevelopment/spec is where DIN SPEC 15800 and 15801 publish their XSDs.

say "schemas"
for name in gdtf mvr; do
  url="https://raw.githubusercontent.com/mvrdevelopment/spec/main/${name}.xsd"
  if curl -fsSL "$url" -o "$corpus/schema/${name}.xsd"; then
    echo "  ${name}.xsd"
  else
    warn "  could not fetch ${name}.xsd from $url"
    rm -f "$corpus/schema/${name}.xsd"
  fi
done

# ── MVR examples ─────────────────────────────────────────────────────
# Openly hosted, no login. Each entry is a URL and the name to save it under.

say "MVR examples"
mvr_examples=(
  "https://raw.githubusercontent.com/mvrdevelopment/spec/main/examples/demo_show.mvr demo_show.mvr"
)
for entry in "${mvr_examples[@]}"; do
  read -r url name <<<"$entry"
  if curl -fsSL "$url" -o "$corpus/mvr/$name"; then
    echo "  $name"
  else
    warn "  could not fetch $name"
    rm -f "$corpus/mvr/$name"
  fi
done

# ── GDTF Share ───────────────────────────────────────────────────────
# The Share's API is a login that sets a cookie, then a download by revision id.
# The rids below are a spread on purpose: a simple LED par, a moving head with
# 16-bit position, a multi-cell bar and a media server, which between them cover
# the corners this reader gets wrong.

say "GDTF Share"
if [[ -z "${GDTF_SHARE_USER:-}" || -z "${GDTF_SHARE_PASSWORD:-}" ]]; then
  warn "  skipped: set GDTF_SHARE_USER and GDTF_SHARE_PASSWORD to fetch Share files"
else
  jar="$(mktemp)"
  trap 'rm -f "$jar"' EXIT
  base="https://gdtf-share.com/apis/public"

  # The login endpoint answers 200 with an HTML page when the credentials are
  # wrong, so success is decided by the body, never by the status.
  body="$(curl -fsSL -c "$jar" \
    --data-urlencode "user=$GDTF_SHARE_USER" \
    --data-urlencode "password=$GDTF_SHARE_PASSWORD" \
    "$base/login.php" || true)"
  if [[ "$body" != *'"result":true'* ]]; then
    warn "  login failed; the Share answers 200 with an error body, so check the credentials"
  else
    for rid in ${GDTF_SHARE_RIDS:-13483 13024 12886 10585}; do
      out="$corpus/gdtf/share-$rid.gdtf"
      if curl -fsSL -b "$jar" "$base/downloadFile.php?rid=$rid" -o "$out" \
         && head -c 2 "$out" | grep -q PK; then
        echo "  share-$rid.gdtf"
      else
        warn "  could not fetch rid $rid"
        rm -f "$out"
      fi
    done
  fi
fi

say "corpus in $corpus"
find "$corpus" -type f | sed "s|$corpus/|  |" | sort
