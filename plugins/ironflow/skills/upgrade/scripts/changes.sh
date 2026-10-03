#!/usr/bin/env bash
# Ironflow crates that moved between an older Cargo.lock and the current one,
# with the CHANGELOG entries in between, read from the new crates in the local
# cargo cache. Run it after the bump. Modifies nothing.
#
# Usage: changes.sh [<git-rev>]    older lock = <git-rev>:Cargo.lock (default HEAD)
set -euo pipefail
export LC_ALL=C
REV="${1:-HEAD}"
CACHE="${CARGO_HOME:-$HOME/.cargo}/registry/cache"

# "name version" for every ironflow crate of a lock file resolved from a registry.
locked() {
  awk '/^\[\[package\]\]/ {n = ""} /^name = "ironflow-/ {n = $3} /^version = / {v = $3}
       /^source = "registry/ && n != "" {print n, v}' | tr -d '"' | sort -u
}

# Entries of <crate>-<new>/CHANGELOG.md newer than <old>, as "version [group] text".
slice() {
  local archives=("$CACHE"/*/"$1-$3.crate")
  [ -f "${archives[0]}" ] || { echo "  (no $1-$3.crate in $CACHE)"; return; }
  tar -xzOf "${archives[0]}" "$1-$3/CHANGELOG.md" 2>/dev/null | awk -v old="$2" '
    function newer(a, b,  x, y, i) {
      split(a, x, "."); split(b, y, ".")
      for (i = 1; i <= 3; i++) if (x[i] + 0 != y[i] + 0) return x[i] + 0 > y[i] + 0
      return 0
    }
    /^## \[[0-9]/ { split($0, h, /[][]/); if (!newer(h[2], old)) exit; v = h[2]; g = "Other"; next }
    /^### / { g = substr($0, 5); next }
    /^- / && v { print "  " v " [" g "] " substr($0, 3) }'
}

old=$(git show "$REV:Cargo.lock" 2>/dev/null | locked) || true
[ -n "$old" ] || { echo "no ironflow crate in $REV:Cargo.lock" >&2; exit 1; }
new=$(locked < Cargo.lock)
cargo fetch --quiet

printf '%-26s %10s  %10s\n' crate before after
join -a 2 -e - -o 0,1.2,2.2 <(echo "$old") <(echo "$new") | while read -r name before after; do
  printf '%-26s %10s  %10s\n' "$name" "$before" "$after"
done
join <(echo "$old") <(echo "$new") | while read -r name before after; do
  [ "$before" = "$after" ] && continue
  echo; echo "== $name $before -> $after"
  slice "$name" "$before" "$after"
done | awk '{k = $0; sub(/^  [0-9.]+ \[[^]]*\] /, "", k)} !/^  [0-9]/ || !seen[k]++'
