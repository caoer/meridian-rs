#!/usr/bin/env bash
# Verify a published constitution file against its footnoted master.
#   verify-strip.sh <master.md> <published.md>
# Checks:
#   A. idempotency: strip(master) applied twice == once
#   B. no footnote residue in published (no [^id] markers, no [^id]: defs,
#      no FOOTNOTES markers)
#   C. self-containment grep gate (blessed allowlist for `llm-wiki check`)
#   D. non-citation body identity: stripping the master must not touch any line
#      that carried no footnote marker (guards against a worker mangling prose)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
master="${1:?master}"; pub="${2:?published}"
fail=0; note() { echo "  FAIL: $*"; fail=1; }

echo "== verify $(basename "$pub") =="

# A. idempotency
a="$("$here/strip-footnotes.sh" "$pub")"
[ "$a" = "$(cat "$pub")" ] || note "published is not strip-stable (already-stripped file changed under strip)"

# B. residue
grep -nE '\[\^[A-Za-z0-9-]+\]' "$pub" && note "footnote markers/defs remain" || true
grep -n 'FOOTNOTES' "$pub" && note "FOOTNOTES marker remains" || true

# C. self-containment gate — the genuinely-unresolvable reference classes.
#    NOT `\[\[` (this is a markdown-workspace contract: it discusses wikilink
#    syntax as normative content and carries wikilink fixtures as §0.3 data)
#    and NOT bare "convergence" ("convergence items" is a self-contained design
#    term; the tournament "convergence N" refs are already footnoted).
gate='llm-wiki|home-wiki|18-02-meridian|parser-bench|ZT ruling|ZT \+|review C[0-9]|wiki://|candidate/|team-[a-i]|round-[0-9]|D-C[0-9]|gate-[0-9]|foundation-panel|baseline-digest|KNOBS|/Users/|home-wiki-codespace|8d000853|ca158421'
# allow the frozen op name `llm-wiki check`
hits="$(grep -nE "$gate" "$pub" | grep -vE '`llm-wiki check`' || true)"
[ -z "$hits" ] || { echo "  gate hits:"; echo "$hits" | sed 's/^/    /'; note "self-containment gate tripped"; }

# D. body identity: lines without a marker in master must appear verbatim in published
#    (approximate: every published line must exist in strip(master))
diff <("$here/strip-footnotes.sh" "$master") "$pub" >/dev/null 2>&1 || \
  note "published != strip(master) — republish from master, do not hand-edit"

[ "$fail" -eq 0 ] && echo "  OK" || exit 1
