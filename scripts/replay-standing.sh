#!/usr/bin/env bash
# Standing corpus-replay run over a REAL workspace (lane b).
#
# Walks the git history of a markdown workspace through `mrd rules replay`,
# synthesizes the ChangeEvent stream, runs a rule set over it, and lands a
# timestamped markdown report in an inspectable output directory. The report
# BODY is deterministic (a pure function of history + rules); only the filename
# carries the run time, so successive reports are diff-able.
#
# This is the in-house lane for the richest real corpus available — point it at
# the field-notes tree (3k+ docs of real history). The mechanism is deliberately
# minimal: a script you schedule. Two ways to make it a STANDING run:
#
#   cron (Linux):
#     0 6 * * *  MERIDIAN_REPLAY_CORPUS=/srv/field-notes \
#                MERIDIAN_REPLAY_RULES=/srv/field-notes/.meridian/rules \
#                /path/to/meridian-rs/scripts/replay-standing.sh >> /var/log/mrd-replay.log 2>&1
#
#   launchd (macOS): wrap this in a LaunchAgent plist with StartCalendarInterval.
#
# Environment:
#   MERIDIAN_REPLAY_CORPUS  the workspace (git repo) to replay   [required]
#   MERIDIAN_REPLAY_RULES   the .star rule set directory         [default: $CORPUS/.meridian/rules]
#   MERIDIAN_REPLAY_OUT     where reports land                   [default: ./replay-reports]
#   MRD_BIN                 an already-built mrd binary          [default: cargo run -p mrd]
set -euo pipefail

corpus="${MERIDIAN_REPLAY_CORPUS:-}"
if [[ -z "$corpus" ]]; then
  echo "error: set MERIDIAN_REPLAY_CORPUS to the workspace to replay" >&2
  echo "       (a git repo of markdown — e.g. the field-notes tree)" >&2
  exit 2
fi
if ! git -C "$corpus" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "error: $corpus is not a git work tree — the git source needs history" >&2
  exit 2
fi

rules="${MERIDIAN_REPLAY_RULES:-$corpus/.meridian/rules}"
if [[ ! -d "$rules" ]]; then
  echo "error: rules dir $rules not found — set MERIDIAN_REPLAY_RULES" >&2
  exit 2
fi

out_dir="${MERIDIAN_REPLAY_OUT:-replay-reports}"
mkdir -p "$out_dir"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
report="$out_dir/replay-$stamp.md"

# Prefer a prebuilt binary; fall back to `cargo run` from the repo root.
if [[ -n "${MRD_BIN:-}" ]]; then
  run_mrd=("$MRD_BIN")
else
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  run_mrd=(cargo run -q --manifest-path "$repo_root/Cargo.toml" -p mrd --)
fi

echo "replaying $corpus (rules: $rules) → $report"
"${run_mrd[@]}" rules replay "$corpus" --rules "$rules" --out "$report"
echo "report: $report"
