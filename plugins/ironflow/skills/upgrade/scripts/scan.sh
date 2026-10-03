#!/usr/bin/env bash
# Places in a project that an upgrade catalogue entry points at. Every entry of
# references/*.md has a "- kind:" line and zero or more "- detect: `<ERE>`"
# lines; their patterns are grepped over the project. Modifies nothing.
#
# Usage: scan.sh [<project-dir>]
set -euo pipefail
REFS="$(cd "$(dirname "$0")/../references" && pwd)"
ROOT="${1:-.}"
MAX=15 # hits printed per entry

# One "file<TAB>id<TAB>kind<TAB>(p1)|(p2)" line per entry that has patterns,
# most severe kind first.
entries() {
  awk 'BEGIN { split("breaking deprecated behavior idiom adopt", k, " "); for (i in k) rank[k[i]] = i }
       function flush() { if (pat != "") print rank[kind] + 0 "\t" file "\t" id "\t" kind "\t" pat; pat = "" }
       FNR == 1 { flush(); file = FILENAME; sub(/.*\//, "", file) }
       /^## / { flush(); id = $2; kind = "" }
       /^- kind: / { kind = $3 }
       /^- detect: `/ { p = $0; sub(/^- detect: `/, "", p); sub(/`$/, "", p)
                        pat = pat (pat == "" ? "" : "|") "(" p ")" }
       END { flush() }' "$REFS"/*.md | sort -s -n -k1,1 | cut -f2-
}

cd "$ROOT"
found=0
while IFS=$'\t' read -r file id kind pattern; do
  hits=$(grep -rnE -e "$pattern" . \
    --include='*.rs' --include='*.toml' --include='.env*' --include='*.sh' \
    --include='*.yml' --include='*.yaml' --include='Dockerfile*' \
    --exclude-dir=target --exclude-dir=.git --exclude-dir=node_modules 2>/dev/null || true)
  [ -n "$hits" ] || continue
  found=1
  echo "[$kind] $id (references/$file#$id) - $(printf '%s\n' "$hits" | wc -l | tr -d ' ') hit(s)"
  printf '%s\n' "$hits" | head -n "$MAX" | cut -c1-180 | sed 's/^/  /'
done < <(entries)
[ "$found" = 1 ] || echo "No catalogue pattern matches."
