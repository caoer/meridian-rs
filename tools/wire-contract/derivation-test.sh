#!/usr/bin/env bash
# Derivation test: docs/wire-contract.md is GENERATED, never hand-edited.
#
#   derivation-test.sh            # check (exit 1 on drift)
#   derivation-test.sh --regen    # regenerate the published copy from the master
#
# The law (DX-04/07 resolution, 2026-08-02): the citation-bearing master lives
# in the wiki domain as the ONE PLACE. The repo carries only the stripped
# derivative. Editing the derivative is editing a build artifact.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

published="$repo/docs/wire-contract.md"

# The master, pinned by wiki path + checksum. The path resolves through the
# wiki-home env var so this is not bound to one machine; the checksum is the
# tripwire — bump it in the same commit that regenerates the derivative.
master_rel="domains/knowledge/meridian-rs/wire-contract-v2.footnoted.md"
master_sha256="4afc6685a6d84e6727ec7d18dfafa79f0e920b8d17d0ac10ff66cea5d1ff59ab"

wiki="${CCC_LLM_WIKI_PATH:-}"
master="${wiki:+${wiki%/}/$master_rel}"

fail=0
note() { echo "  FAIL: $*"; fail=1; }

echo "== derivation: docs/wire-contract.md =="

if [ -z "$master" ] || [ ! -f "$master" ]; then
  # No wiki checkout: gate D (published == strip(master)) is unverifiable.
  # Say so loudly, then run every check that does not need the master.
  echo "  SKIP gate D: master not readable (CCC_LLM_WIKI_PATH unset or"
  echo "               $master_rel absent). The wiki is a private checkout;"
  echo "               this repo does not require it to build."
  echo "  running master-free checks (strip-stability, footnote residue)..."
  a="$("$here/strip-footnotes.sh" "$published")"
  [ "$a" = "$(cat "$published")" ] || note "published is not strip-stable — it carries footnote syntax, so it is not a strip output"
  grep -nE '\[\^[A-Za-z0-9-]+\]' "$published" && note "footnote markers/defs remain in the published copy" || true
  grep -n 'FOOTNOTES' "$published" && note "FOOTNOTES marker remains in the published copy" || true
  [ "$fail" -eq 0 ] && { echo "  OK (partial — gate D skipped)"; exit 0; } || exit 1
fi

# 1. The master is the one we generated from.
actual_sha256="$(shasum -a 256 "$master" | cut -d' ' -f1)"
if [ "$actual_sha256" != "$master_sha256" ]; then
  echo "  FAIL: the master moved under the pin."
  echo "        pinned:  $master_sha256"
  echo "        on disk: $actual_sha256"
  echo "        master:  $master"
  echo
  echo "  The wiki master is law; docs/wire-contract.md is generated from it."
  echo "  The master changed, so the derivative is stale. Fix forward:"
  echo "      tools/wire-contract/derivation-test.sh --regen"
  echo "  then commit the regenerated doc and the bumped pin together."
  exit 1
fi

# 2. published == strip(master), byte for byte.
if [ "${1:-}" = "--regen" ]; then
  "$here/strip-footnotes.sh" "$master" > "$published"
  echo "  regenerated from $master"
  echo "  new sha256: $(shasum -a 256 "$published" | cut -d' ' -f1)"
  echo "  NOTE: if the master's checksum also changed, bump master_sha256 in this script."
  exit 0
fi

if ! diff -u <("$here/strip-footnotes.sh" "$master") "$published" > /tmp/derivation-drift.$$ 2>&1; then
  echo "  FAIL: docs/wire-contract.md != strip(master)."
  echo
  echo "  The wiki master is law; this file is generated. Someone hand-edited"
  echo "  the derivative, which cannot hold — the next regeneration erases it."
  echo "  Move the change into the master, then:"
  echo "      tools/wire-contract/derivation-test.sh --regen"
  echo
  sed 's/^/    /' /tmp/derivation-drift.$$ | head -60
  rm -f /tmp/derivation-drift.$$
  exit 1
fi
rm -f /tmp/derivation-drift.$$

# 3. The full strip-law gates (idempotency, residue, self-containment, identity).
"$here/verify-strip.sh" "$master" "$published" | sed 's/^/  /'

echo "  OK"
