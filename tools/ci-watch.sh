#!/usr/bin/env bash
# Block until a Woodpecker pipeline finishes, then print one line per step and
# the full log of every step that did not succeed.
#
# The point is that watching CI costs ONE command, not a poll loop in an
# agent's context: this script does the polling, and prints a verdict.
#
#   tools/ci-watch.sh              # the newest pipeline for HEAD's sha
#   tools/ci-watch.sh 122          # that pipeline number
#   tools/ci-watch.sh --sha d863335f
#   tools/ci-watch.sh --start      # trigger a manual pipeline on main, then watch it
#
# Exit code IS the verdict: 0 green, 1 red, 2 could not resolve a pipeline.
# Token: $WOODPECKER_TOKEN, else decrypted from the sops store.
set -euo pipefail

REPO=${WOODPECKER_REPO_ID:-12}
export WOODPECKER_SERVER=${WOODPECKER_SERVER:-https://ci.0xdao.app}
export WOODPECKER_DISABLE_UPDATE_CHECK=1
SECRETS=${CCC_SECRETS_COMMON:-/Users/Shared/repos/osfiles/secrets/svc/common.sops.yaml}
INTERVAL=${CI_WATCH_INTERVAL:-20}

if [ -z "${WOODPECKER_TOKEN:-}" ]; then
  WOODPECKER_TOKEN=$(sops -d --extract '["woodpecker_token"]' "$SECRETS")
  export WOODPECKER_TOKEN
fi
wp() { woodpecker-cli "$@"; }

sha=$(git rev-parse HEAD)
pipeline=""
case "${1:-}" in
  --start)
    pipeline=$(wp pipeline create --branch "${2:-main}" "$REPO" | awk 'NR>1 {print $1; exit}')
    ;;
  --sha)
    sha=$2
    ;;
  "")
    ;;
  *)
    pipeline=$1
    ;;
esac

# Newest pipeline whose commit is $sha. `pipeline ls` prints a table; the
# commit is not in it, so ask the API for the mapping.
if [ -z "$pipeline" ]; then
  pipeline=$(curl -fsS -H "Authorization: Bearer $WOODPECKER_TOKEN" \
    "$WOODPECKER_SERVER/api/repos/$REPO/pipelines?perPage=50" |
    python3 -c '
import json,sys
sha=sys.argv[1]
for p in json.load(sys.stdin):
    if p["commit"].startswith(sha) or sha.startswith(p["commit"]):
        print(p["number"]); break
' "$sha")
fi
[ -n "$pipeline" ] || { echo "no pipeline found for $sha" >&2; exit 2; }

echo "watching $WOODPECKER_SERVER/repos/$REPO/pipeline/$pipeline (sha ${sha:0:8})"
while :; do
  states=$(wp pipeline ps "$REPO" "$pipeline" --format '{{.step.PID}}|{{.step.Name}}|{{.step.State}}')
  # `pending`/`running` anywhere means the pipeline is not done yet.
  printf '%s\n' "$states" | grep -qE '\|(pending|running)$' || break
  sleep "$INTERVAL"
done

rc=0
while IFS='|' read -r pid name state; do
  [ -n "$pid" ] || continue
  printf '%-8s %-52s %s\n' "$pid" "$name" "$state"
  case "$state" in success|skipped) ;; *) rc=1 ;; esac
done <<<"$states"

if [ "$rc" -ne 0 ]; then
  while IFS='|' read -r pid name state; do
    case "$state" in success|skipped|"") continue ;; esac
    echo; echo "=== log: $name ($state) ==="
    wp pipeline log show "$REPO" "$pipeline" "$pid" || true
  done <<<"$states"
fi
exit "$rc"
