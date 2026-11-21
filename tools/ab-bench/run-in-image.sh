#!/bin/bash
# Host side: run the driver inside the image the binaries were built in, pinned
# to a CPU set, as uid 1000. The work folder is mounted at /work.
#   BENCH_WORK=/abs/work BENCH_IMAGE=<image@digest> \
#     run-in-image.sh <container-name> <cpuset> <ab.py args...>
# /work must hold bench/ (this directory), the binaries the --arm flags name,
# and be writable by uid 1000.
set -eu
name=$1; cpus=$2; shift 2
: "${BENCH_WORK:?set BENCH_WORK to the work folder}" "${BENCH_IMAGE:?set BENCH_IMAGE to the CI image}"
mkdir -p "$BENCH_WORK/run" "$BENCH_WORK/out" "$BENCH_WORK/home/.config"
chown 1000:100 "$BENCH_WORK/run" "$BENCH_WORK/out" "$BENCH_WORK/home" "$BENCH_WORK/home/.config"
docker rm -f "$name" >/dev/null 2>&1 || true
exec docker run -d --name "$name" --user 1000:100 --cpuset-cpus="$cpus" \
  --ulimit nofile=65536:65536 -v "$BENCH_WORK":/work -w /work -e HOME=/work/home \
  "$BENCH_IMAGE" python3 -u /work/bench/ab.py --root /work/run "$@"
