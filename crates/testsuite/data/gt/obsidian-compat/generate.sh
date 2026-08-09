#!/usr/bin/env bash
# Regenerate the obsidian-compat resolution GT pack from the LIVE Obsidian oracle.
#
#   ./generate.sh
#
# Preconditions: the Obsidian desktop app is open on the `field-notes-sessions`
# vault, the `obsidian` CLI is on PATH, and the `probe-bridge` plugin is present
# (this script enables it). Every answer is produced by the live resolver — this
# pack is DATA, never hand-written (gate-1 ruling 7; contract §4.5, §13.4).
#
# Version-drift posture (contract §13.3-§13.4): an app bump ⇒ re-run this script,
# never assume the old answers carry. The app_version field pins the run.
set -euo pipefail

PACK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_PLIST="/Applications/Obsidian.app/Contents/Info.plist"
APP_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PLIST")"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Enable the resolver bridge (window.__ob === require("obsidian")); idempotent.
obsidian eval code='(typeof window.__ob==="undefined") ? app.plugins.enablePlugin("probe-bridge").then(()=>"enabled") : "already"' >/dev/null
sleep 1

# Run the oracle; the CLI prefixes its result with "=> ".
RAW="$(obsidian eval code="$(cat "$PACK_DIR/oracle.js")")"
PROBES="${RAW#*=> }"

# Corpus checksums pin the exact fixtures the answers correspond to.
WALK_SHA="$(shasum -a 256 "$PACK_DIR/walkvault/walk.md"   | cut -d' ' -f1)"
BLOCKS_SHA="$(shasum -a 256 "$PACK_DIR/walkvault/blocks.md" | cut -d' ' -f1)"

jq -n \
  --arg app_version "$APP_VERSION" \
  --arg generated_at "$GENERATED_AT" \
  --arg walk_sha "$WALK_SHA" \
  --arg blocks_sha "$BLOCKS_SHA" \
  --argjson probes "$PROBES" \
  '{
    pack: "obsidian-compat",
    app_version: $app_version,
    oracle: "live Obsidian resolver via probe-bridge — parseLinktext -> getFirstLinkpathDest (stage 1) -> resolveSubpath (stage 2)",
    generation_command: "obsidian eval code=\"$(cat oracle.js)\"  (driven by ./generate.sh)",
    generated_at: $generated_at,
    corpus: "walkvault/ (walk.md + blocks.md), byte-identical to the gate-3 adversarial-harness walkvault",
    corpus_sha256: { "walkvault/walk.md": $walk_sha, "walkvault/blocks.md": $blocks_sha },
    span_units: "UTF-8 byte offsets (transcoded from the app Loc.offset, which is UTF-16 code units; corpus is NOT ASCII — walk.md carries U+2014 x5 + U+2192 x1)",
    probe_count: { walk_law: 6, underscore: 1, note: "WL-6 realized as the WL-6a/WL-6b divergence pair (2 entries, 1 walk law)" },
    drift_posture: "app bump => re-run generate.sh; answers are never assumed to carry (contract §13.3-§13.4, gate-1 ruling 7)",
    probes: $probes
  }' > "$PACK_DIR/resolution.expected.json"

echo "wrote $PACK_DIR/resolution.expected.json (app $APP_VERSION, $(echo "$PROBES" | jq length) probe entries)"
