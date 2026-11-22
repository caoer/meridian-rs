# ab-bench

Compares prebuilt `mrd` release binaries by running the same CLI commands
against each one and timing them from outside. No bench code is compiled into
any tree, so a commit from before this tool existed is measured the same way as
`HEAD`.

```sh
# one binary per commit, each from its own target dir (or one chain of
# incremental builds, copying target/release/mrd aside after each checkout)
MRD_BUILD_SHA=$sha cargo build --locked --release -p mrd
cargo build --locked --release -p perfsuite --bin corpusgen

tools/ab-bench/ab.py --arm before=/abs/mrd-aaaa --arm after=/abs/mrd-bbbb \
    --corpusgen /abs/corpusgen --root /abs/scratch --out run.jsonl
tools/ab-bench/report.py run.jsonl --json run.json > run.md
```

Linux only (`/proc`). `run-in-image.sh` runs the driver in the CI image, pinned
to a CPU set, for binaries that were built in that image.

## What an arm is

One `--arm name=path` is one binary with its own `HOME`, `XDG_CACHE_HOME`,
`MERIDIAN_CONFIG`, daemon and copy of each fixture under `--root`. Give the arms
in landing order: the report compares each arm with the one before it and the
first with the last.

## Scenarios (`--only`)

| Name | What it times | Fixture |
|---|---|---|
| `read` | one `mrd read`, 1 client and 16 at once | 2,000-file generated vault |
| `write` | one `mrd put`: 10 CHECK rules and a middleware armed, the same on a 45 kB page, no rules | hand-built workspace |
| `links` | `mrd links` on a page that links into a second declared root | 20,000-file second root |
| `bulk` | the first read after 2,000 and after 200 files changed | 20,000-file vault |
| `probe` | `mrd links` with 2,000 links into one 20,000-entry directory; with 50 ignore rules | hand-built |
| `flight` | one edit, then 16 concurrent reads: daemon CPU and time to the last answer | 50,000-file vault |
| `model` | a section read of a 1 MB page, `mrd walk` over 100 pins into it, `mrd links` over 10,000 links to a basename 2,000 pages share | hand-built |
| `cache` | daemon restart with a parse cache on disk and one file edited: first answer, cache re-saved, CPU; default and `MRD_RESIDENCY_HOST_BYTES` modes | 50,000-file vault |
| `mem` | cold start with an empty cache: resident memory once loaded, time, CPU | vaults of `--mem-sizes` files, 20,000 heading-heavy pages |
| `follow` | one `mrd put` with and without a primed `sub` watcher on the workspace: elapsed, daemon CPU, the door's flock hold (`flock.hold.door` from the daemon's `MRD_TIMING` lines), the detect cycles' holds, daemon CPU at rest | vaults of `--follow-sizes` files |

Elapsed time is one CLI process from spawn to exit. Daemon CPU is `utime+stime`
from `/proc/<pid>/stat` (10 ms steps), or `wait4` rusage where the driver starts
the daemon itself (`cache`, `mem`).

## Reading the report

A comparison is `gain` or `loss` only when the two arms' per-round medians do
not overlap and the medians differ by at least 3%; otherwise `none`. The spread
column gives the 10th–90th percentile of all samples and the range of per-round
medians, then n.

## Traps

- The daemon's accept loop decides a floor under every CLI call. A range that
  changes it moves every elapsed-time row; compare neighbouring commits, and
  read the daemon CPU rows beside the elapsed ones.
- Arms never share a cache root: the parse cache's generation differs per build.
- `links` answers `source: ephemeral` when the CLI rebuilt in-process; the
  `links` samples record it.
- The first cold start after a fixture copy is slower on every arm. Use at
  least 3 rounds.
- A scenario that throws writes an `_error` row and the run continues;
  `report.py` lists it.
