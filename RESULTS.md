# RESULTS — Team B, honest-durability (dedicated durability actor)

Status: **INTERIM — ladder of record not yet run.** The actor
(`crates/fs/src/durability.rs`) is landing; per the same-harness/same-session
rule, every cross-mode comparison cell below is measured in ONE session after
it lands. Cells marked PENDING are empty by design, not omission. Everything
else on this page (receipts, counts, instrument validation, the premise
probe) is already-measured evidence and cited to raw output on disk.

## 1. What was built

`crates/perfsuite/src/bin/hdb-durability` measures the durability placement:
paired sync-class baseline, synthesized-commit cells (modes `today` / `naive`
/ `actor`), engine-path cells driving the real `wire_serve::publish` at D
destinations per transaction, and `fs_usage` receipt windows for counting
honest syncs at the syscall boundary. Latency percentiles ride the existing
`perfsuite::measure::LatencyRun` (hdrhistogram); per-pair ratios follow the
advisor's reference method (`results/instrument-evidence/paired_push.rs`).
Branch `hd-team-b`, harness commits `577a0944` + `6e65fec9`; the sha of
record is stated beside the ladder when it runs.

**Base freeze:** all three tournament branches are cut from `dca3d09c9`
(advisor ruling; no merging `main` mid-run). The 15 commits `main` has moved
since touch neither `crates/fs` nor `crates/wire-serve`.

**Framing:** `parallel-commits` (`dca3d09c9`) already placed honest sync
outside every exclusion. Team B's contribution is **grouping, metric
separation, and proof** — not sync placement.

## 2. Instrument receipt

**Named instrument: `fs_usage` (kdebug, kernel-level), run as root, filtered
to the pid under test.** It decodes the fcntl subcode symbolically
(`fcntl F=n <FULLFSYNC>`) with per-call latency, so it distinguishes the
three sync tiers. dtrace was tried first and is dead on this host: the
syscall provider is SIP-blocked (`system integrity protection is on … does
not match any probes`). Reading source is not evidence; every count below is
a kernel observation of a live process.

**Three-column discipline (binding):** every receipt reports `F_FULLFSYNC`,
`F_BARRIERFSYNC`, and plain `fsync` as separate columns. `F_BARRIERFSYNC`
orders without claiming durability at return; one appearing where a full
barrier is expected would be a silent honesty downgrade, so its column being
zero is evidence, not decoration.

Validation receipts, raw output beside this repo in `.scratch/`:

| run | expected | FULLFSYNC | BARRIERFSYNC | plain fsync | raw |
|---|---|---|---|---|---|
| C probe, `fcntl(F_FULLFSYNC)` ×50 | 50 | 50 | 0 | 0 | `.scratch/fs_usage_full_test.txt` |
| C probe, `fsync(2)` ×50 (negative control) | 0 | 0 | 0 | 50 | `.scratch/fs_usage_fsync_test.txt` |
| Rust std check: 30 `sync_all` + 30 `sync_data` + 1 dir `sync_all` | 61 | **61** | **0** | **0** | `.scratch/fs_usage_syncall.txt` |
| harness `receipt today` ×25 commits | 50 (25 content + 25 dir) | 50 | 0 | 0 | `.scratch/fs_usage_receipt_shakedown-today.txt` |
| harness `receipt naive` ×25 commits | 50 | 50 | 0 | 0 | `.scratch/fs_usage_receipt_shakedown-naive.txt` |

**Finding (toolchain-pinned):** on rustc/std 1.97.1 (8bab26f4f, 2026-07-14),
aarch64-apple-darwin, `File::sync_all` AND `File::sync_data` both issue
`fcntl(F_FULLFSYNC)` — the count arithmetic closes at exactly 61 FULLFSYNC
with zero BARRIERFSYNC and zero plain fsync, and in-process medians sat at
5.044 / 5.136 ms (the barrier band). Consequence: the engine's live
`write_fsync`/`fsync_dir` (`sync_all`) are ALREADY honest-class on macOS —
the drive-cache defect is real only off-macOS. What survives on this host:
today's path pays two serial barriers per staged-file commit, ungrouped, and
the real publish path pays 6D+4 per transaction. What settles the finding is
the direct zero-BARRIERFSYNC check plus the closing count arithmetic on THIS
host — not a claim about Rust in general.

PENDING (actor-gated): engine content-path receipt through the actor —
`fs_usage` count on `receipt-real`, the in-process `fs::honest_sync` call-site
counter assert, and the per-path cheap-push enumeration from the trace
(pre-registration §2: every distinct `(dev, ino)` durable at a barrier
received exactly one cheap push in the preceding batch). The counter proves
which call site ran; the trace and the latency signature prove which syscall
class ran; together they answer gate item 2.

## 3. Measured tables

**Conditions template for every cell:** host fleet mac Mac16,5 / M4 Max /
internal NVMe `/dev/disk3s1` / APFS; 4 KiB writes; sample count, N writers,
D destinations, and the 1/5/15-minute load averages recorded by the harness
beside each table. Quiet-host check (no cargo/rustc processes, load stated)
runs immediately before the ladder and its output is pasted here.

**Paired discipline (gate item 5 as amended):** classes are sampled inside
one iteration on their own fds, order rotating, per-pair ratio reported
beside absolutes. Unpaired absolute tables are discarded, not reasoned about.
Reference targets: 5.0 ms effective per barrier on this host (advisor
settlement; corroborated by our in-process 5.044/5.136 medians), 52.6 ms
measured single-destination baseline, 6D+4 honest syncs per baseline
transaction. Team B prediction (`results/hdb-team-b-prediction.md`, amended
to 3 before data): **3 barriers per solo commit, CONSTANT IN D** — the
load-bearing claim is constant-in-D, not the number 3.

- PENDING: paired baseline (`hdb-durability baseline`, 1000 iters).
- PENDING: honest syncs per commit, before/after, D=1/2/3 (headline column).
- PENDING: engine-path ladder — D × {1,2,3}, writers × {1,2,4,8,16,32};
  stage sync, durability interval, metadata interval, txn wall as separate
  figures, p50/p99 each; fsync-class and metadata-publish figures never
  summed (gate 12).
- PENDING: Loss A — adversarial arrival, offset swept across the drain
  window, arriving writer's wait reported. Pre-registered: we lose ~2× a
  barrier.
- PENDING: Loss B — mixed-size batch (one high-D writer + several D=1),
  D=1 writers' latency in-batch vs alone; the delta is the head-of-line
  cost. Whole-batch settle p99 reported, never the barrier alone.

Shakedown runs (harness validation under load 64+, absolutes discarded per
protocol): plumbing verified end-to-end — mode `today` ≈ mode `naive` as
expected (syscall-identical on macOS), fsync/publish separation intact,
receipt arithmetic closed exactly both modes.

## 3b. The before numbers — required section, presented rather than superseded

A result that only shows the good run has hidden the most informative thing
that happened. The first counts against the landed actor disagreed with the
pre-registration, and the prediction did not move: the filing holds at **3
barriers per solo commit, constant in D** (`results/hdb-team-b-prediction.md`
— "never explained away by adjusting the expectation").

**Shakedown counts (fs_usage, syscall boundary).** Conditions: 10–50 txns,
single writer, no warmup, tree `dc66998e9` predating both held fix commits,
harness inline stage settle excluded, load >60 (counts are load-immune;
latencies from these runs were discarded). Raw:
`.scratch/fs_usage_real_shakedown-actor-d1*.txt`, `…-d3.txt`.

| cell | actor barriers/txn | filed |
|---|---|---|
| D=1 (fit across 10- and 50-txn runs) | ~4.4 steady-state | 3 |
| D=3 (10 txns) | ~8.4 | 3 |

Count grew with D — the falsifier class named in the filing.

**Hypothesis generated (leader):** eager drain — the actor's blocking recv
returns on the FIRST submission, so a solo writer submitting sequentially
gets many small batches instead of one per phase; signature would be
barriers/txn tracking submits with ~1 object per barrier.

**Discriminator (in-process counters, warmup excluded, 50 txns/cell, tree
`dc66998e9` + held harness commits):**

| cell | barriers/txn | objects/txn | objects/barrier |
|---|---|---|---|
| submitted D=1 | 5.86 | 9.0 | 1.54 |
| submitted D=3 | 6.98 | 16.6 | 2.38 |
| settled D=1 | 5.00 | 8.0 | — |
| settled D=3 | 5.96 | 13.6 | — |

**Strict eager drain is REFUTED** (barriers do not track submits 1:1; there
is real grouping) — but phase batching is not achieved either: groups of
1.5–2.4 instead of phase-sized, 5–7 barriers/txn against the filed 3, still
growing mildly with D. Consistent with drains firing mid-phase and catching
only what arrived before the barrier. Two clean side-findings: `honest_ops`
== `actor_barriers` exactly in submitted mode (5860=5860, 6980=6980 ×1000) —
zero actor bypass; and `F_BARRIERFSYNC` = 0 everywhere.

**After (the ladder of record, post-fix):** PENDING. Either the count comes
down to 3 constant-in-D, or the primary claim is reported FALSIFIED in those
words. It is written here beside the before numbers when it runs.

## 4. Verdict on the named risk

**PENDING the ladder of record.** The question as assigned: does the single
actor become a serialization point in disguise? Pre-registered falsifiers
(filed before data): barriers-per-commit flat as N rises kills the thesis;
p99 at high N worse than naive per-writer honest mode means the actor IS the
serialization point and we say so in those words; head-of-line delay that
does not shrink with N means the single queue is a net loss.

## 5. Premise, falsifier, and the criterion-1 answer

**Does a writer's receipt depend on a device-wide side effect of a call
issued on another writer's fd? Yes — and the actor shape is why that is
sound.**

- No writer ever issues an honest-class sync; only the actor does. There is
  no "another writer's fd" in this design — the barrier is the actor's own
  call on the actor's own handle, the workspace root opened once at start.
- The ack never precedes the durability: a member's bytes reach the device,
  then the actor's barrier returns, then the member is acked.
- **Which step supplies which guarantee:** the cheap `fsync(2)` hands a
  member's bytes to the device; the actor's `F_FULLFSYNC` barrier makes
  everything the device holds survive power loss; the ack sits on the
  barrier's return and never on a cheap push.
- **Documented contract** (not "verified" — APFS `VNOP_FSYNC` is closed
  source and nobody outside Apple can audit the mechanics). Apple `fcntl(2)`,
  read 2026-08-16 (Xcode man mirror; this host ships no man pages — SDK
  header cross-checked at `sys/fcntl.h:259,306`): `F_FULLFSYNC` "asks the
  drive to flush all buffered data to the permanent storage device … As this
  drains the entire queue of the device and acts as a barrier, data that had
  been fsync'd on the same device before is guaranteed to be persisted when
  this call returns." Full extract: `.scratch/fcntl-man-citation.md`.
- **F_BARRIERFSYNC is declined by ruling:** it is documented as ordering
  without claiming durability at return ("no assumption should be made on
  what has been persisted … when this call returns"), so acking on it would
  be criterion-1 dishonest. The zero-BARRIERFSYNC column in every receipt is
  the standing evidence; one appearing in the ladder of record is ours to
  trace by call site.
- **Falsifier:** every batch member must sit on the same physical device as
  the barrier handle; a mount or firmlink inside the workspace root breaks
  the premise. The peer seat's `st_dev` test covers it (cited here when it
  lands).

**Premise probe (corroboration, not verification** — the question was
settled cross-model by the advisor; a confirmation proves nothing new, an
anomaly would have been a finding). Standalone paired probe, n=200 pairs per
payload, alternated, raw at `.scratch/premise_4k.txt` / `premise_1m.txt`.

*What the probe shows:* barrier-after-cheap-push vs
barrier-carrying-writeout on the SAME fd are near-indistinguishable at the
median — per-pair ratio p50 1.039× (4 KiB), 1.072× (1 MiB). This is EXPECTED
and proves nothing either way: `F_FULLFSYNC` on fd X performs `fsync(X)`
internally, so pre-pushing X saves nothing on X's own barrier. The barrier's
cost is dominated by the device flush; writeout rides nearly free at these
sizes.

*What the probe structurally cannot show:* the case our design actually
depends on. The actor's barrier is on its OWN handle (the workspace root)
and the members are OTHER fds; `F_FULLFSYNC(root)` does not fsync the
members. The pushes are load-bearing for CROSS-FD coverage only — and that
case has no latency signal to test here, because an un-pushed member would
simply be silently non-durable. It rests on the documented contract quoted
above, stated as such.

*The edge is the real result, and it is ours to own —
pre-registered-then-confirmed:* `results/hdb-team-b-prediction.md` §4b named
Loss B before any data ("the worst per-pair plain fsync costs about as much
as a full barrier; one slow push delays every waiter in that batch"). The
probe confirms it with a number: a cheap push buys ZERO barrier latency, so
every serialized push on the actor's critical path is pure added latency,
and the push tail reached p99 5.03 ms — barrier-class. Worst-case batch
settle ≈ distinct pushes × worst push, as filed. We said we would lose this
cell, we built the config to find it, and we found it. Load 64–68 during the
probe: ratios are the readout, absolutes are not of record.

## 6. Identified but not landed, with reasons

Named here because seeing an improvement and not landing it is worth more
than pretending we did not see it:

- **Consolidate the 2D recovery images into one per-transaction file** —
  would cut distinct cheap pushes from ~3D+4 to ~D+5, real at high D. Not
  landed: it changes the intent format and recovery, out of budget for this
  card.
- **Parallel pushes inside the actor** — attacks Loss B directly and stays
  inside Team B's shape (pushes are not honest syncs; the barrier stays
  single and owned). Not landed: it adds a thread pool against criterion 7
  and needs its own measurement. If the ladder shows Loss B severe rather
  than merely real, this is the scoped second pass to reconsider.

## 7. What was not measured, and why

- The ladder of record, Loss A, Loss B, the actor receipts, and the paired
  baseline of record: the actor had not landed at the time of writing; the
  same-harness/same-session rule forbids splicing cross-run cells. This
  section shrinks to zero when it lands.
- Power-loss durability itself: asserting it empirically requires cutting
  power mid-write; we rely on the documented contract, stated as such above.
- Non-macOS honest class (`sync_all` elsewhere): CI builds it; latency there
  is out of scope for this darwin-exception card.
- FireWire-class devices that ignore the flush request (documented caveat):
  out of scope — this card's device is internal NVMe.
