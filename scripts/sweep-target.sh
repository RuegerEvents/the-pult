#!/usr/bin/env bash
# Removes build output nothing has touched for a while.
#
# Cargo never deletes anything from `target/`: a feature set, a test target or a
# toolchain it has stopped building leaves its artifacts and its incremental session
# behind for good, and that reached 313 GB here. A build reading a file updates its
# access time, but APFS does not record those reliably, so the rule is modification
# time: anything not *rebuilt* in `--days` (default 7). Deleting something still in use
# costs a rebuild of that crate and nothing else, because cargo sees the output missing.
#
#   scripts/sweep-target.sh             # everything under target/ older than a week
#   scripts/sweep-target.sh --days 2
#   scripts/sweep-target.sh --dry-run   # say how much, delete nothing
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$here/target}"
days=7
dry=0

while [ $# -gt 0 ]; do
    case "$1" in
        --days) days="$2"; shift 2 ;;
        --dry-run) dry=1; shift ;;
        -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 1 ;;
    esac
done

[ -d "$target" ] || { echo "no $target"; exit 0; }

before=$(du -sk "$target" | cut -f1)

# Every profile directory, including the ones a CARGO_TARGET_DIR subdirectory made
# (target/<name>/debug). An incremental session is a directory whose files are
# replaced as a set, so it goes whole or not at all.
stale() {
    while IFS= read -r profile; do
        for sub in deps build .fingerprint examples; do
            if [ -d "$profile/$sub" ]; then
                find "$profile/$sub" -mindepth 1 -maxdepth 1 -mtime +"$days" -print0
            fi
        done
        if [ -d "$profile/incremental" ]; then
            find "$profile/incremental" -mindepth 2 -maxdepth 2 -type d -mtime +"$days" -print0
        fi
    done < <(find "$target" -maxdepth 3 -type d -name .fingerprint -exec dirname {} \;)
}

if [ "$dry" = 1 ]; then
    gone=$(stale | xargs -0 du -sk 2>/dev/null | awk '{s+=$1} END {print s+0}')
    echo "$target: $((before / 1048576)) GB, $((gone / 1048576)) GB of it not rebuilt in $days days"
else
    stale | xargs -0 rm -rf
    find "$target" -maxdepth 4 -path '*/incremental/*' -type d -empty -delete
    after=$(du -sk "$target" | cut -f1)
    echo "$target: $((before / 1048576)) GB → $((after / 1048576)) GB"
fi
