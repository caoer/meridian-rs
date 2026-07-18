#!/usr/bin/env bash
# Generate a small SYNTHETIC md fixture for the root-stamp demo. Synthetic (not
# copied corpus) so no wiki content is baked into this repo, while still
# exercising frontmatter, headings, and wikilinks. Idempotent.
set -uo pipefail
cd "$(dirname "$0")/.."
FIX=data/fixture
rm -rf "$FIX"; mkdir -p "$FIX"
N="${1:-40}"
for i in $(seq 1 "$N"); do
  pad=$(printf '%03d' "$i")
  nxt=$(printf '%03d' "$(( (i % N) + 1 ))")
  far=$(printf '%03d' "$(( (i * 7 % N) + 1 ))")
  grp=$(( i % 5 ))
  if [ $(( i % 3 )) -eq 0 ]; then st=done; else st=pending; fi
  cat > "$FIX/page-$pad.md" <<EOF
---
type: note
status: $st
tags: [topic/synthetic, group/$grp]
created: 2026-07-18
---

# Page $i

Body text for synthetic page $i. It links to [[page-$nxt]]
and references [[page-001]] as a hub.

## Section A

Some content with a [[page-$far]] link.

## Section B

Trailing content, no links.
EOF
done
echo "generated $N synthetic md files in $FIX"
