Six log-readability changes to `.woodpecker.yaml`. **No gate logic is touched** — the evidence
gate, `--no-fail-fast`, and the loud sccache stats are exactly as they were.

Spec is ZT-supplied (2026-08-10); the base is `ci-audit/887ce11e-best-practices` at `a54e28da`.

## The changes

1. **test lane** — `cargo test --workspace --locked --no-run -q` is now its own command, so
   compiling and running get separate folds. On green the compile fold is empty; on a compile
   error the diagnostics print there and the suite fold is obviously absent rather than buried.
2. **test lane** — `-q` on the suite invocation. Kills the `Fresh x` restamp *and* puts libtest
   in one-character-per-test mode.
3. **both clippy lanes** — `-q`. A green lane prints nothing, so an empty fold is the verdict; a
   red lane still prints every diagnostic.
4. **bench lane** — `-q` on `cargo bench` and `cargo run`, so the claims table is not preceded by
   a compile scroll.
5. **every compiling lane** — `CARGO_TERM_COLOR: always`. Cargo strips color on a non-TTY;
   Woodpecker's log viewer renders ANSI.

## Deviation from the spec, with its receipt

The spec asked for `-q` **and** `-- --quiet`. That combination is what **pipeline #30 died of**:
`error: Option 'quiet' given more than once` from every test binary, `evidence lines: 0`, and the
gate correctly refused the red as `NO MEASUREMENT — this red is not a test verdict`. Cargo's own
`-q` already forwards `--quiet` to the harness, so the explicit one is a duplicate, not an
addition. **`-q` alone produces exactly the collapsed output the spec wanted** — see the counts.
Fixed in `2835e466`, which also comments the trap in place so it is not re-added.

## Measured, not claimed

Log lines per step, Woodpecker pipeline **#28** at `a54e28da` (before) vs **#31** at `2835e466`
(after). Both green, both on `workstation-nyc-2`.

| step | #28 before | #31 after | delta |
|---|---|---|---|
| test (cargo test --workspace --locked) | 4292 | **1231** | −3061 |
| fmt | 1 | 1 | 0 |
| clippy (workspace lint policy) | 138 | **45** | −93 |
| clippy the perf-gated targets | 454 | **71** | −383 |
| deny (cargo-deny) | 2408 | 2408 | 0 — not a cargo lane, out of this spec's scope |
| bench smoke + claim verdicts | 379 | **127** | −252 |
| **total** | **7672** | **3883** | **−3789 (−49%)** |

Excluding the untouched `deny` lane: **5264 → 1475, −72%**.

## The gate is byte-identical

| | #28 | #31 |
|---|---|---|
| `evidence lines:` printed by the gate | **232** | **232** |
| `^test result` lines in the step log | **232** | **232** |

What actually disappeared: **2705 individual `test <name> ... ok` lines** in #28 → **0** in #31,
while all **232 `running N tests` / `test result: ok` pairs survive**. That is the accepted trade
(ZT, verbatim: *"I'd take that trade"*) — `suite.log` keeps binary-level counts, not the name of
every passing test. **A failing test still prints its name, its panic, and its captured output in
full**, because libtest's quiet mode only collapses the passing path.

Verification: pipeline **#31**, https://ci.0xdao.app/repos/caoer115/meridian-rs/pipeline/31
