#!/usr/bin/env bash
# Discover the same feature-gated targets Woodpecker compiles, then run each
# in its own process. --no-run prepares every executable before measurement.
set -euo pipefail
cd "$(dirname "$0")/.."

targets=$(mktemp)
trap 'rm -f "$targets"' EXIT
python3 - <<'PY' > "$targets"
from pathlib import Path
import tomllib

for manifest in sorted(Path("crates").glob("*/Cargo.toml")):
    package = tomllib.loads(manifest.read_text())
    for target in package.get("test", []):
        if "perf-walltime" in target.get("required-features", []):
            print(package["package"]["name"], target["name"])
PY
if [ ! -s "$targets" ]; then
    echo "No perf-walltime targets discovered" >&2
    exit 1
fi

perf_status=0
while read -r package target; do
    printf '\nPerformance target: %s/%s\n' "$package" "$target"
    if ! tools/test.sh --locked -p "$package" --features perf-walltime --test "$target" "$@"; then
        perf_status=1
    fi
done < "$targets"
exit "$perf_status"
