Step 2 of ZT's ruling (2026-08-10, verbatim: *"do A , then ci-audit merge into main"*). Step 1 —
the six log-readability changes — landed into this branch as #1 (`6e192eff`).

`main` today carries the **23-line** `.woodpecker.yaml`: one `cargo test --workspace` step, no
clippy lanes, no bench lane, no `cargo-deny`, no evidence gate. This branch carries the redesign
that has been running green on `workstation-nyc-2` for the last several pipelines.

## What this branch's own work touches

Only CI. Nothing under `crates/`:

    .woodpecker.yaml  | 286 +++++++++++++++++++++++++++++++++++-
    Dockerfile.ci     |  35 ++++
    justfile          |   6 ++
    3 files changed, 312 insertions(+), 15 deletions(-)

## What it brings

- **A DAG, not a serial pipeline.** Every step declares `depends_on: []`, so no lane's verdict can
  mask another's. The GitHub lane's failure mode — a clippy nit turning the test verdict into
  `SKIPPED` for eighteen hours — is removed structurally, not by ordering.
- **An evidence gate on the test lane.** `--no-fail-fast` plus a `^test result` line count that
  runs in the *same command* as the suite, so it cannot be skipped by the abort it exists for. It
  already earned its keep: it caught pipeline #30 and named it *"NO MEASUREMENT — this red is not
  a test verdict; the build died before any test ran"*, which is exactly what it was.
- **A baked CI image** (`Dockerfile.ci`) with the toolchain, `mold`, `sccache`, `cargo-deny`, and
  `python3-yaml` — pipelines start at the verdict instead of at `apt-get`.
- **Per-lane sccache shards and `mold`** — a disk cache is single-server and the lanes run
  concurrently; sharding keeps cross-pipeline reuse without cross-lane collision.
- **`cargo-deny` and a bench-smoke + claim-verdicts lane**, at parity with what the GitHub
  workflow used to run before it was disabled.
- **Logs a human can read** — 7672 → 3883 log lines across the pipeline, −72% outside the
  untouched `deny` lane, with the evidence count byte-identical at 232 lines before and after.
  Full measurement in #1.

## Verification

- Pipeline **#31** at `2835e466` — this branch's content, green.
- Pipeline **#32** — the #1 merge result, green.
- This PR's own run is the one that matters: **this branch has never been built against `main`'s
  current tip** (`dd6b4efa`, eleven commits of `crates/mrd` work it does not contain). The merge
  result is the first time the redesigned pipeline meets that code. Do not merge on #31's green —
  merge on this PR's.
