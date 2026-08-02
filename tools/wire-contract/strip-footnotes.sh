#!/usr/bin/env bash
# Mechanical footnote-strip: master (footnoted) -> published (self-contained).
# This is the ONLY transform between the citation-bearing master and the
# in-repo published constitution. Deterministic + idempotent by construction:
# it deletes footnote-definition blocks and inline [^id] markers, nothing else.
#
#   strip-footnotes.sh <master.md> > <published.md>
#
# Guarantees (asserted by verify-strip.sh):
#   - every "<!-- FOOTNOTES ... -->" line and everything a [^id]: block contains
#     is removed;
#   - every inline [^id] reference marker is removed;
#   - no other byte changes;
#   - running it twice == running it once (idempotent).
set -euo pipefail

in="${1:?usage: strip-footnotes.sh <master.md>}"

# 1) Drop the footnote-definition region: from the first "<!-- FOOTNOTES" line
#    to EOF (masters place all [^id]: blocks there, per the worker convention),
#    AND drop any stray standalone "^[^id]: ..." definition line anywhere.
# 2) Strip every remaining inline "[^id]" marker. After step 1 there are no
#    definition heads left (they only ever start a line, always removed above),
#    so an unconditional strip is correct — a marker followed by ':' in prose
#    (e.g. "criterion[^s4-1]:") is an inline reference, not a definition.
awk '
  /<!-- FOOTNOTES/ { infoot=1 }
  infoot { next }
  /^\[\^[A-Za-z0-9-]+\]:[[:space:]]/ { next }   # stray definition lines
  { print }
' "$in" \
| perl -pe 's/\[\^[A-Za-z0-9-]+\]//g'
