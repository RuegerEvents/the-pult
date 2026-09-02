#!/usr/bin/env bash
# Other people's GDTF and MVR files, and the schemas they are supposed to match.
#
# Everything this gathers lands in testdata/corpus/, which is gitignored: these are
# other people's files under other people's licences, and a corpus checked into the
# repository would be both large and not ours to redistribute. What *is* checked in
# is testdata/gdtf/ — small fixtures written here, which the default test suite reads.
#
# Worth knowing why this exists at all: every part of the GDTF reader was written
# strictly, passed against those checked-in fixtures, and failed on the first real file
# it was pointed at. Hand-written test material proves the arithmetic and nothing about
# the shapes real files take.
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

# ── MVR files ────────────────────────────────────────────────────────
#
# There is no public collection of MVR files this script can rely on. The spec
# repository (github.com/mvrdevelopment/spec) publishes the standard as Markdown
# and no XSD and no samples; MVR-Stash and the vendor example files move around.
# So this takes them from wherever you have them, and says so when you have not
# said:
#
#     PULT_MVR_SAMPLES=~/Downloads/rigs scripts/fetch-interop-corpus.sh
#     PULT_MVR_SAMPLES="https://example.com/a.mvr https://example.com/b.mvr" ...
#
# An earlier version of this script fetched two URLs at that repository that do
# not exist. It reported the 404s and carried on, which is the failure mode this
# rewrite exists to not have: a corpus job that looks like it ran.

say "MVR files"
if [[ -z "${PULT_MVR_SAMPLES:-}" ]]; then
  warn "  none: set PULT_MVR_SAMPLES to a directory of .mvr files, or to URLs"
else
  for source in $PULT_MVR_SAMPLES; do
    if [[ -d "$source" ]]; then
      found=0
      while IFS= read -r -d '' file; do
        cp "$file" "$corpus/mvr/$(basename "$file")"
        echo "  $(basename "$file")"
        found=1
      done < <(find "$source" -maxdepth 2 -iname '*.mvr' -print0)
      [[ $found -eq 1 ]] || warn "  no .mvr files under $source"
    elif [[ "$source" == http* ]]; then
      name="$(basename "${source%%\?*}")"
      if curl -fsSL "$source" -o "$corpus/mvr/$name" && head -c 2 "$corpus/mvr/$name" | grep -q PK; then
        echo "  $name"
      else
        warn "  could not fetch $source"
        rm -f "$corpus/mvr/$name"
      fi
    else
      warn "  $source is neither a directory nor a URL"
    fi
  done
fi

# ── GDTF Share ───────────────────────────────────────────────────────
# The Share's API is a login that sets a cookie, then a download by revision id.
# The rids below are a spread on purpose, and each is here for a corner:
#
#   38193   flashPRO LED PAR RGBWAUV — six emitters, so colour mixing has more than
#           the three every hand-written fixture has
#   138392  Robe Robin MegaPointe — 39 channels, CMY as subtractive flags, 16-bit
#           position, two gobo wheels. The first real file this reader was pointed
#           at, and it failed on three separate things
#   62993   Astera AX2-50 PixelBar — a hundred and seventy modes and cells behind
#           geometry references, which is where a footprint stops being a number
#   117897  Martin MAC Aura XIP — five modes from 20 to 93 channels
#   136960  Showtec Photon Sunstrip — a strip, so the multi-cell case twice over
#
# Verified against the live Share on 2026-09-02. If one 404s the Share has moved
# it; pick another with the console's own search and set GDTF_SHARE_RIDS.

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
    for rid in ${GDTF_SHARE_RIDS:-38193 138392 62993 117897 136960}; do
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
