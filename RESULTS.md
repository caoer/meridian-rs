# Team C — epoch-batched honest durability: measured results

**Verdict in one screen.**

- **Honest barriers per commit are constant at 2**, at every destination count
  tested (D = 1, 2, 3, 8) and every corpus layout, against a baseline of
  `6D + 4` (10 / 16 / 22 / 52). Every individual commit paid exactly two —
  `min == max`, not an average.
- **Three independent instruments agree**, on count and on latency: an
  in-process call-site counter, a `DYLD_INSERT_LIBRARIES` interposer, and
  `fs_usage` (the operating system, observing from outside our code).
- **`F_BARRIERFSYNC` is zero** across 113,040 pushes and 10,324 barriers. The
  third durability rung is formally declined and provably absent.
- **The linearized state-owner step is microsecond-scale**: p50 7,500 ns
  against a content-durability p50 of 6,319,875 ns — 843x apart, and it stays
  microsecond-scale as concurrency rises.
- **The assigned risk is real and bounded to the uncontended case.** An epoch
  window costs a lone writer 2x and pays a contended one 3.1x. We did not
  disprove the risk; we located it.
- **No arrival-timing starvation.** A writer arriving at adversarial phase sits
  at the middle of the peer distribution (rank 443–559 per mille, expected 500)
  across six runs.

Everything below traces to raw output kept under `.scratch/`. No number in this
file was produced by any other means.

---

## What was measured, and where

The measured object is **`wire_serve::publish::publish` itself** — the shipped
function, not a model of it. An earlier plan to reconstruct the path at the `fs`
crate seam was rejected: a reconstruction is a second implementation, it can
drift from the engine, and this design's central claim is an *ordering* claim.
A reviewer who found the ordering measured in a copy would be right to discard
the numbers.

Harness: `crates/wire-serve/tests/durability_walltime.rs`, gated behind the
`perf-walltime` feature so `cargo test --workspace` skips it, CI compiles and
lints it, and only a person on the reported hardware runs it.

| | |
|---|---|
| git commit measured | `241fd25226ff4b1c3ff5ea45b2ffa9b96e2f4a4c` plus this file's uncommitted harness |
| hardware | Mac16,5, 16 CPU |
| filesystem | APFS on `/dev/disk3s1`, internal NVMe |
| kernel | Darwin 25.5.0, `xnu-12377.121.10~1`, arm64 |
| load average during runs | 16.6 – 38.3 (one-minute), captured in-harness via `getloadavg` |
| payload | 4096 bytes per destination |
| percentile method | exact nearest-rank, sample `ceil(q·n)` of the sorted vector, no interpolation, no histogram bucketing |

**This host is not quiescent.** It runs a multi-agent fleet; top consumers
during measurement were `ccc-statusd` at 270% CPU, several `nvim`, `git`,
`rustc` and `python3`. Every table carries the load average it was taken under,
captured programmatically at run time rather than eyeballed afterwards.

### Three identifiers, three jobs — do not let one do two

| identifier | what it is | value |
|---|---|---|
| **measured-at sha** | the commit whose tree produced every number here, plus this file and the harness, then uncommitted | `241fd25226ff4b1c3ff5ea45b2ffa9b96e2f4a4c` |
| **landed sha** | the commit that finally carries this work onto the branch | a **descendant** of the measured-at sha — filled in by the team lead at push time |
| **pipeline number** | the Woodpecker run, with its per-step verdict table | reported separately by the team lead from the API |

The measured-at sha will be an **ancestor** of the landed sha. That is normal —
the harness and this file were uncommitted while the numbers were taken — but it
is stated rather than implied, because a reviewer checking a single identifier
against both the tree and the pipeline would find a mismatch we never intended.

**A sha is not a verdict and an exit code is not a verdict.** On this forge a
pipeline cancelled by a newer commit on the same ref exits 0 on a signature that
reads as success, and a watcher has already reported a killed pipeline as green
during this run. Only the API step table settles it.

---

## Criterion 1 — whose flush makes your receipt true?

**Our answer is yes, and here is the defense.**

Team C uses N cheap `fsync(2)` pushes plus **one shared `F_FULLFSYNC`** issued
on the epoch anchor's descriptor. A writer's receipt therefore depends on a
device-level effect of a call issued on another descriptor. That is the shape
the spec asks every team to defend, and it is the shape we ship.

### The ordering argument

1. A writer pushes its own paths with `fsync(2)`, on its own descriptors, in
   parallel with every other writer and outside every exclusion.
2. `data_push_all` returns a `Pushed` token stamped with the instant the last
   push returned. The token is minted nowhere else and is consumed only by
   `EpochBatcher::await_epoch`, so "the member pushed before it joined" is a
   fact about the types rather than a claim in a comment.
3. A writer waits for a seal that **started after its own push returned**. It
   never renames before that seal has itself returned.
4. The seal is a device cache flush. It forces what was already handed to the
   device — which, by step 1, includes this writer's bytes.

### The strongest available evidence that the cheap tier is real

Apple documents `F_BARRIERFSYNC` as performing an `fsync` and then issuing a
**device barrier applying to I/O flushed with `fsync`**, and states it is
implemented on APFS. That contract would be incoherent if APFS could leave
`fsync`-ed I/O sitting in an in-memory checkpoint — the barrier would have
nothing at the device to order. So the third rung we formally declined as a
mechanism is, by its documented existence alone, the strongest evidence that
our cheap tier reaches the device.

### What we measured versus what we rely on — kept apart deliberately

| | |
|---|---|
| **VERIFIED by our instruments** | the syscall *identity* of every call (`fcntl(F_FULLFSYNC)` vs `fsync(2)` vs `fcntl(F_BARRIERFSYNC)`), the *count* of each class per commit, and the *latency* of each |
| **DOCUMENTED, and relied upon** | that `fsync(2)` on APFS reaches the device, and that the barrier covers I/O previously flushed with `fsync` |

The device-reach property is **documented, not verified**. APFS's `VNOP_FSYNC`
is closed and absent from the public XNU tree, so nobody outside Apple can
audit the mechanics. We build on a documented contract and say so; a reviewer
who found us calling a contract a measurement would be entitled to discard the
rest of this file.

**Which step supplies which guarantee.** The cheap push does not claim
power-loss durability. It supplies ordering and hands bytes to the device. Only
the final `F_FULLFSYNC` supplies platter durability, and **our acknowledgement
sits on that barrier**, never on the push. The chain is not uniformly durable
and is not described as such.

### Stated limits of the defense

Every item here counts against our own convenience. None of them is something we
measured; each is something we rely on or something we bound.

1. **No published experiment has ever isolated the cross-descriptor claim.**
   Not ALICE, not CrashMonkey, and not the arXiv APFS paper — which explicitly
   disclaims power-loss testing. No public macOS power-loss rig exists. Our
   mechanism rests on a documented contract plus a mechanism argument, **never
   on an experiment**.
2. **The coverage unit is probably the APFS container, not the device.**
   `F_FULLFSYNC` calls `tx_finish`, which commits the container transaction, so
   two containers on one physical device would break the shared-barrier
   argument. Apple's man page says *same device*; the mechanism says *same
   container*; **we design to the narrower one.** This comes from
   agent-reported disassembly of a closed kext — suggestive, not conclusive.
3. **`F_FULLFSYNC` is best-effort by Apple's own admission.** *Reducing Disk
   Writes* states data can still be lost on sudden power loss, and roughly half
   of consumer NVMe drives tested discard FLUSH-confirmed data — with no such
   study on Apple internal storage. This is orthogonal to our cross-file
   property and fatal to the end goal if firmware lies. It applies to **any**
   design on this platform, not to ours specifically.
4. **Git is not independent confirmation here.** The commit introducing Git's
   durability documentation reasons from a *journal* and names NTFS, HFS+ and
   XFS. APFS is not in that list and has no journal. Citing Git as precedent
   would be citing something making the same inference we are, so we do not.
5. **`ENOTSUP` / `ENOTTY` on `F_FULLFSYNC` is real and undocumented.** It occurs
   on SMB, network mounts and some external devices. We report it as a failure
   mode we **handle**: a failed barrier errors every member of its epoch rather
   than letting them proceed. That is the difference between a durability hole
   and a refusal, and it is a positive property of this design.

6. **`publish_group` is still a sequential map, and we did not close that gap.**
   The tournament spec's landed-tree fact 2 says `publish_group` groups intent,
   not cost, and that closing that gap is this card. We closed part of it and
   not the rest, so here is exactly what is and is not done.

   What Team C delivers is **grouping at the epoch, across independent
   concurrent publishers** — which is where writes actually queue in this
   engine. The evidence is in §1.2: 0.697 barriers per commit at N=16 and 0.485
   at N=64 against a baseline of 10, i.e. well below one barrier per commit.
   The group manifest itself now rides the epoch too, so a group no longer pays
   its own serial pair of honest syncs before its members start.

   What is **not** done: `publish_group` maps its members with
   `members.into_iter().map(|m| publish(...))` — sequentially, end to end.
   Its members therefore **do not share an epoch**, and each pays its own two
   barriers. A caller that wants grouped *cost* from a batch should call
   `publish` concurrently rather than routing the batch through
   `publish_group`, which remains a sequential convenience wrapper for a single
   caller submitting M transactions.

   **Why we did not fix it (deliberate trade, 2026-08-16).** Not difficulty:
   the fix needs no threads — phase 1 for all members, one barrier, then phase 2
   for all. **Risk.** That function carries recovery and `commit_unknown`
   ordering, and we were at close-out with a red branch head. Restructuring the
   ordering of a function whose whole job is to be correct when a crash lands
   mid-sequence is not a change to make against the clock. It is named here
   rather than discovered, and it is one declared trade rather than a silent
   gap.

### The conservative fallback, priced

`SealMode::PerMember` — each writer syncs its own descriptor, no shared barrier,
no rendezvous — is shipped and measured. If a reviewer rejects the shared-barrier
argument, this is the bill:

| mode | writers | content p50 | honest barriers / commit | wall |
|---|---:|---:|---:|---:|
| `AnchorBarrier` | 1 | 8.270 ms | 2.000 | 0.426 s |
| `PerMember` | 1 | 32.833 ms | 9.000 | 1.158 s |
| `AnchorBarrier` | 16 | 38.362 ms | 0.675 | 2.137 s |
| `PerMember` | 16 | 232.989 ms | 9.000 | 8.886 s |

At 16 writers the conservative answer costs **6.1x the latency and 13.3x the
barriers**.

---

## 1. Counts — the load-independent evidence

Counts lead this file because a latency table on this host can be argued away
by load, and correctly so: the same syscall class measured minutes apart has
differed 65x here. **A syscall count cannot be argued away.** It is
deterministic, it reproduces, and it does not move with the fleet.

### 1.1 Pre-registered prediction and measured result

The formula below was **derived from the code and registered with the team lead
before the D=8 cells were measured**. A pre-registered number that comes true is
evidence; the same number reported afterwards is a claim.

Let `D` = destinations in the transaction, `Ddir` = *distinct destination
directories*.

```
phase 1 = 2D inline pushes + |{D tmps, 2D images, 1 image dir,
                               1 intent file, 1 intents dir}|   = 5D + 3
phase 2 = 1  inline push   + |{Ddir dest parents, 1 intent file,
                               1 intents dir}|                  = Ddir + 3

cheap  fsync(2)     per commit = 5D + Ddir + 6
honest F_FULLFSYNC  per commit = 2,  independent of D and Ddir
F_BARRIERFSYNC                 = 0
```

| D | Ddir | layout | **predicted** cheap | **measured** cheap | honest min / max |
|---:|---:|---|---:|---:|---:|
| 1 | 1 | shared | 12 | **12** | 2 / 2 |
| 1 | 1 | per-dest | 12 | **12** | 2 / 2 |
| 2 | 1 | shared | 17 | **17** | 2 / 2 |
| 2 | 2 | per-dest | 18 | **18** | 2 / 2 |
| 3 | 1 | shared | 22 | **22** | 2 / 2 |
| 3 | 3 | per-dest | 24 | **24** | 2 / 2 |
| 8 | 1 | shared | **47** | **47** | 2 / 2 |
| 8 | 8 | per-dest | **54** | **54** | 2 / 2 |

Eight cells, eight exact hits, `cheap_min == cheap_max` in every one —
deterministic, not averaged. Two different layouts at the same `D` yield
different counts and the formula predicted both in advance, which makes this a
test of the derivation rather than a curve fit.

**Why the count depends on layout, and why that is not a skipped push.**
`publish::barrier` calls `seal.sort()` then `seal.dedup()`. `Vec::dedup` after
a sort removes only *consecutive equal* elements — exact duplicates. A
transaction records the same parent directory once per destination, and a
directory pushed once is pushed enough. **No distinct path can lose its push.**
That is provable from the operation and holds at any `D`.

> A note on an argument we withdrew. We first justified the count by observing
> that measured was at or above the old `6D+4` prediction at D = 1, 2, 3. That
> reasoning was wrong: a slope of 5 against a baseline slope of 6 crosses at
> D=3 and inverts after it (at D=10, 57 against 64). The comparison was true
> only of the points we happened to test. The structural dedup argument
> replaced it because it survives at every `D`.

### 1.2 Barriers per commit versus concurrency

Baseline at D=1 is 10 honest barriers per commit.

| writers | commits | honest barriers total | **honest per commit** | reduction vs baseline |
|---:|---:|---:|---:|---:|
| 1 | 12 | 24 | 2.000 | 5.0x |
| 4 | 48 | 82 | 1.708 | 5.9x |
| 16 | 192 | 221 | 1.151 | 8.7x |
| 64 | 768 | 549 | 0.714 | 14.0x |

With a 5 ms window the same figure reaches **0.171 at N=16** and **0.255 at
N=64** (§3), a 58x and 39x reduction.

**Reading `honest_per_commit_min = 0`, which appears from N=4 upward.** That
commit **did not pay for a barrier; it awaited one that started after its own
push returned.** The `Pushed` token is what enforces that ordering. *Zero paid,
never zero awaited.* It is the shared-barrier mechanism visible in raw counts —
not a commit that went un-sealed.

### 1.3 The third rung, absent

`F_BARRIERFSYNC = 0` in every run, at every `D`, at every `N` — across a
192-second matrix that issued 10,324 `F_FULLFSYNC` and 113,040 `fsync(2)`.
Team C declines the ordering-without-durability rung at both the push and the
seal, so a nonzero count here would be a defect we did not intend rather than a
curiosity. This assertion was registered in advance.

---

## 2. Instrument receipt

Gate item 2 requires a runtime receipt, not code reading. `dtrace`'s syscall
provider is blocked by SIP on this host, so three other instruments were used.

**Ground truth.** `.scratch/instrument/dirty_push.rs` issues 4 warmup + 20
measured syncs of one named class, with one fresh 4096-byte write before each.

| witness | `fullfsync` run | `fsync` run |
|---|---|---|
| the program's own known count | 24 | 24 |
| interposer (ours, in-process) | `FULLFSYNC=24 FSYNC=0` | `FULLFSYNC=0 FSYNC=24` |
| `fs_usage` (the OS, not ours) | `<FULLFSYNC>` × 24 | `fsync` × 24 |

`fs_usage`, verbatim, unedited — the trailing number is seconds:

```
$ sudo fs_usage -w -f filesys dirty_push
01:30:08.349808 fcntl F=4 <FULLFSYNC> 0.005573 dirty_push.107466470
01:30:08.355334 fcntl F=4 <FULLFSYNC> 0.004929 dirty_push.107466470
01:30:08.367648 fcntl F=4 <FULLFSYNC> 0.011862 dirty_push.107466470
01:30:08.513472 fsync F=4             0.000078 dirty_push.107466537
01:30:08.513588 fsync F=4             0.000094 dirty_push.107466537
01:30:08.513728 fsync F=4             0.000108 dirty_push.107466537
```

Interposer on the real test binary, tied to the in-process counter by
arithmetic rather than by assertion — 36 commits (4 warmup + 8 measured, three
`D` values):

```
inside counter: honest 2/commit × 36            = 72   → interposer FULLFSYNC=72   MATCH
inside counter: 12×12 + 17×12 + 22×12           = 612  → interposer FSYNC=612      MATCH
                                                         interposer BARRIERFSYNC=0
```

Two witnesses independent of our code (`fs_usage`, and the interposer which
observes the process from outside its own image), agreeing exactly on both
classes. Raw: `.scratch/instrument/three-witness-receipt.txt`,
`fs_usage_raw.txt`, `gate-count-receipt.txt`.

**A host fact found here and published to all three teams.** On macOS, Rust's
`File::sync_all` **and** `File::sync_data` both issue `fcntl(F_FULLFSYNC)`;
neither is the cheap class. A "cheap sync" row measured with `sync_all` reports
the honest class twice under two names. The engine had therefore never called
plain `fsync(2)` on this platform before this card, which means the plan pin's
0.019 ms figure describes a call class the engine did not use — it is not the
engine's former behaviour.

---

## 3. Latency

Every latency figure uses **paired or interleaved sampling**. This is not
fastidiousness. Measured unpaired, raw `fsync` p50 came back at 4.78 ms,
4.83 ms and 0.074 ms from three runs of the same binary seconds apart — a 65x
spread — while `F_FULLFSYNC` drifted 5.0 → 15.5 ms in the same window and the
load average went *down*. Sampling class A at one moment and class B at another
measures *when* you sampled. Paired, the comparison is stable to within 20%.

The writer-count sweep is **round-robin interleaved across 4 rounds** for the
same reason one level up: running N=64 to completion minutes after N=1 would
report device drift as a concurrency effect, and concurrency is the axis this
architecture is claimed on.

**Dirty-page discipline.** One fresh 4096-byte write to that class's own
descriptor immediately before every measured sync; never two syncs on a clean
descriptor. A sync over no dirty pages returns having done nothing — measured
at ~700 ns — and would turn the cheap row into a comparison against a return
statement.

### 3.1 The two sync classes, paired

512 pairs, order alternating so neither class always goes first.

| class | p50 | p90 | p99 | min | max |
|---|---:|---:|---:|---:|---:|
| honest `F_FULLFSYNC` | 5.443 ms | 11.440 ms | 28.633 ms | 3.869 ms | 30.210 ms |
| cheap `fsync(2)` | 0.061 ms | 0.188 ms | 4.450 ms | 0.029 ms | 6.109 ms |

Per-pair ratio (each ratio computed at one moment, before sorting): p50
**91.96x**, p90 188.96x, **min 1.11x**, max 822.24x.

**The minimum matters more than the median.** At the tail a plain push costs as
much as a full barrier — queued behind other I/O. "N cheap pushes" is cheap at
p50 and is *not* uniformly cheap at the tail. The architecture accounts for that
rather than quoting the median past it: pushes run in parallel, outside every
exclusion, and a writer joins its epoch only after its own push returns. **An
unlucky slow push delays that writer into a later epoch — never the epoch and
never its peers.** The baseline pays the same tail into a serial chain, where
every millisecond lands on the transaction.

Against the ruling probe (4.386 ms honest / 0.019 ms cheap): our honest class is
1.2x the pinned figure, our cheap class 3.2x. A cheap operation is
proportionally far more exposed to queueing than an expensive one, which is
what a load average of 16–38 predicts.

### 3.2 The publish path, three figures that are never summed

`PublishReceipt` reports three intervals. **They are three figures, not three
summands** — one interval spanning both durability phases is the aggregate
gate 12 forbids, because it hides which sync class was paid where. The
`publish_call` column is measured independently around the call and is **not**
their sum; it also contains reserve, reverify and finalize, which appear in no
span.

| writers | samples | content durable p50 / p99 | metadata publish p50 / p99 | owner step p50 / p99 | publish call p50 / p99 |
|---:|---:|---:|---:|---:|---:|
| 1 | 512 | 6.320 / 44.511 ms | 6.275 / 57.324 ms | **7,500 / 30,833 ns** | 13.696 / 88.463 ms |
| 4 | 512 | 20.082 / 100.521 ms | 16.318 / 129.656 ms | **7,042 / 46,625 ns** | 41.804 / 200.180 ms |
| 16 | 512 | 48.536 / 212.434 ms | 37.832 / 157.066 ms | **5,458 / 359,084 ns** | 97.372 / 318.805 ms |
| 64 | 1024 | 129.380 / 890.360 ms | 120.128 / 898.146 ms | **4,833 / 1,186,208 ns** | 292.668 / 1040.090 ms |

Throughput moves the other way from per-commit latency, which is the point of
batching: wall time per transaction falls from 23.201 ms at N=1 to 10.390 ms at
N=64 — **2.2x more throughput** while each individual commit waits longer.

### 3.3 The linearized step, shown rather than asserted

The design claims the state-owner step is microsecond-scale and sync-free. It is
reported beside the millisecond-scale sync figures so a reader sees the
separation:

| writers | owner step p50 | content durable p50 | separation |
|---:|---:|---:|---:|
| 1 | 7,500 ns | 6,319,875 ns | **843x** |
| 64 | 4,833 ns | 129,379,708 ns | **26,769x** |

The owner step does not grow with concurrency; it *falls* slightly, while the
sync phases grow with device contention. No sync of any class runs inside it.

---

## 4. The epoch window — a density knob, not a cost

Team C's assigned risk is **added latency for a lone writer that must wait out a
window with nobody to batch with**. That risk is real. It is also bounded to
exactly one regime, and we would have mis-reported it had we swept the window
only where the risk lives.

`content_durable` p50, window × concurrency:

| window | N = 1 | N = 16 | N = 64 |
|---|---:|---:|---:|
| 0 | 5.976 ms | 39.981 ms | 124.729 ms |
| 1 ms | 7.524 ms **(+26%)** | 32.970 ms **(−18%)** | 107.718 ms **(−14%)** |
| 5 ms | 12.066 ms **(+102%)** | 12.871 ms **(−68%)** | 80.053 ms **(−36%)** |

Honest barriers per commit, same cells:

| window | N = 1 | N = 16 | N = 64 |
|---|---:|---:|---:|
| 0 | 2.000 | 0.734 | 0.464 |
| 1 ms | 2.000 | 0.619 | 0.403 |
| 5 ms | 2.000 | **0.171** | **0.255** |

And wall time per transaction at N=16: 8.078 ms → 7.400 ms → **3.555 ms**.

**The finding, stated without softening either half.** The window costs a lone
writer 2x and pays a contended one 3.1x. We did not disprove the assigned risk —
we located it. It is a genuine cost in exactly one regime and a large win in the
others.

With `window = 0` an epoch seals immediately with whoever is present, so batch
density is whatever arrival timing happens to give. The arithmetic that exposed
this: 16 commits at 0.82 barriers is roughly 13 barriers, about 65 ms of device
time, yet writers were waiting 45 ms — spread across many epochs rather than
sharing few. A 5 ms window closes exactly that gap.

**What ships: the knob and this curve, not a controller.** The window is already
a constructor parameter, so this is reportable with no new code. No adaptive
control loop was built: the data supports no single default (5 ms is 3.1x faster
at N=16 and 102% slower at N=1), a contention-sensing controller is scaffolding
nobody asked for, and it would be the change most likely to read as convergence
onto another team's architecture. **The caller chooses**, with measured
guidance: window 0 below about N=4, 5 ms above it. That guidance lives in
`EpochBatcher::new`'s documentation as well as here — a knob whose correct
setting exists only in a results file is a trap for the next maintainer.

---

## 5. Adversarial arrival timing — the cell we predicted to lose

A writer arriving just after a seal begins must wait out that in-flight seal
before its own epoch can seal: a predicted worst case of about two barriers
where a per-writer design pays one. The question is not whether that happens.
It is whether the wait is **bounded** or whether a writer can **starve**.

**First method, and why it was not enough.** One victim publishing alongside 7
noise writers, comparing the first half of its samples against the second. Two
runs disagreed on direction, so six more were run: five of six showed the second
half worse, by 3% to 368%. That looked like degradation. It was not
interpretable — the victim's own median is the wrong reference, because it
cannot distinguish a writer being singled out from a writer paying the same
contention as everyone else.

**The discriminating measurement** is where the victim's median falls inside the
*peer* distribution. ~500 per mille means an ordinary writer; near 1000 means
arrival timing singles it out. Six independent runs:

| run | victim p50 | peer p50 | victim max | peer max | **victim rank in peers** |
|---:|---:|---:|---:|---:|---:|
| 1 | 22.037 ms | 23.512 ms | 46.850 ms | 47.489 ms | **464** |
| 2 | 22.100 ms | 22.704 ms | 42.862 ms | 51.936 ms | **476** |
| 3 | 25.042 ms | 26.200 ms | 601.337 ms | 640.622 ms | **443** |
| 4 | 23.576 ms | 24.159 ms | 53.294 ms | 51.012 ms | **461** |
| 5 | 24.775 ms | 24.302 ms | 63.519 ms | 90.515 ms | **529** |
| 6 | 24.358 ms | 23.260 ms | 54.539 ms | 91.318 ms | **559** |

**Verdict: no arrival-specific penalty, and no starvation.** The victim's rank
clusters at 443–559 against an expected 500. Its median is *faster* than the
peer median in four of six runs, and its maximum is at or below the peer maximum
in five of six — including run 3, where a 601 ms victim tail was matched by a
641 ms peer tail. The fat tail is a property of eight concurrent writers on a
loaded machine, borne equally by every writer, and the first-half/second-half
spread was variance rather than a trend.

The predicted loss is therefore **not observed as a per-writer penalty**. We
report the method that failed to resolve it alongside the one that did, because
the halves comparison would have supported either conclusion depending on which
run was quoted.

---

## 6. Conditions that would change these numbers

- **Machine load.** Every latency figure was taken at load average 16.6–38.3 on
  a host running a multi-agent fleet. Counts are unaffected; latencies would
  fall on a quiet machine, and the *cheap* class would fall proportionally more.
- **Corpus layout.** The cheap push count is `5D + Ddir + 6`. A fixture with a
  different directory spread yields a different count, and the formula predicts
  it. Any count quoted without its layout is not reproducible.
- **Platform.** `F_FULLFSYNC` is macOS-only. On Linux `honest_sync` falls back to
  `sync_all`, a different syscall with different cost, and this target is
  feature-gated so CI can never run it and report the wrong syscall under the
  same heading.
- **Filesystem.** APFS on internal NVMe. An external or network volume changes
  both classes and probably their ratio.
- **Window setting.** Every table except §4 uses `window = 0`. §4 is the sweep.
- **Seal mode.** Every table except the `PerMember` rows uses `AnchorBarrier`.
- **Sample counts.** 512 pooled samples per configuration for the writer sweep
  (1024 at N=64), 512 pairs for the class comparison, 8 commits per cell for
  counts — counts are deterministic, so repetition there buys only confirmation
  that they do not vary.

## 7. Raw output

| file under `.scratch/instrument/` | what it holds |
|---|---|
| `matrix2-raw.txt` | the full extended matrix — every `RESULT`, `COUNT`, `LAYOUTCOUNT`, `WINDOWCELL`, `SEALCOUNT`, `EPOCHRATIO`, `LOAD` line quoted above |
| `matrix-raw.txt` | the earlier matrix, before the window×N and layout sweeps |
| `three-witness-receipt.txt` | the three-instrument agreement on count and latency |
| `fs_usage_raw.txt` | the unfiltered 8,186-line `fs_usage` capture |
| `gate-count-receipt.txt` | the pre-registered count gate, run twice |
| `sync-class-paired.txt` | paired class comparison, four runs |
| `sync-class-control.txt` | the unpaired control that exposed the 65x sampling instability |
| `arrival-peer-control.txt` | item 11, six runs with the peer control |
| `arrival-repeat.txt` | item 11, six runs of the inconclusive halves method |
| `syncprobe.c`, `dirty_push.rs`, `paired_push.rs`, `syncall_probe.rs`, `probe-selftest.c` | instrument sources |
