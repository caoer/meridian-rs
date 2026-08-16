# RESULTS — honest durability, Team A

Written by the implementing worker (`2b625160`). Numbers measured by the
measurement worker (`701ab8ff`); receipts are named per claim and live in
`.scratch/receipts/`.

Branch `hd-team-a` on `git@git.0xdao.app:caoer115/meridian-rs.git`.
Base `dca3d09c9`, frozen for the tournament so all candidates are measured
against one tree. No merge or rebase of `main` was taken.

Head `8302294e72b5f2b8f741e2597ac0a673c2ce8455`. Pipeline **623, success on
every lane**, bound by the Woodpecker REST API commit field rather than the
banner: `number 623 | status success | event push | branch hd-team-a | ref
refs/heads/hd-team-a | commit 8302294e72b5f2b8f741e2597ac0a673c2ce8455`.

Read from the API step table only. The watch script decides nothing here: its
exit code is 0 on lanes killed by pipeline supersession, and its banner sha is
the operator's LOCAL HEAD rather than the pipeline's commit — proven by
running it against two pipeline numbers and getting one sha. Verdict and
commit identity in this file come from `pipeline ls --output
"table=NUMBER,STATUS,EVENT,COMMIT"` and a direct REST read, never from a
banner.

Machine for every number here: Mac16,5 / M4 Max / 16 cpu / internal NVMe
(Apple Fabric) / APFS `/dev/disk3s1` / macOS 26.5.2 (25F84).

## Results, strongest first

Read in this order. The two strong results are counts and the uncontended
path; the latency result is weak and is placed where its weight puts it.

1. **The count law.** `syncs = D + P + 4` after, against `6D + 4` before —
   measured at the syscall boundary in eight cells across two destination
   layouts, with `F_BARRIERFSYNC` and plain `fsync` zero in every one. At D=2
   that is 16 → 7 sharing a parent, 16 → 8 in split parents. Structural, not
   statistical: no sampling artifact produces the same integer twice.
2. **Grouping amortizes shared work and NOTHING else — measured by target.**
   At N=60 across 240 transactions per arm, the four writer-local barriers are
   identical in both arms (240 each) and only the shared `.meridian/intents`
   directory falls, 480 → 55. That is the CONTENT half of criterion 1 shown by
   attribution rather than argued; the shared directory entry is answered YES
   and defended separately. Net 6.000 → 4.229 barriers per commit, against a
   hard floor of 4 that can never be amortized.
3. **The uncontended writer never waits.** `group_wait` is exactly 0.00 ms at
   N=1 and N=2 with cohort size 1. Contention-triggered rather than
   timer-triggered is visible in that zero.
4. **Latency at N=60: the median is a real effect post-fix, the tail is not.**
   17 of 18 paired repetitions favour grouping at p = 0.000145, about 12.4%
   better per-writer p50, at the same pre-declared config that gave p = 0.24
   before the fixes. p99 remains unseparated (12 of 18, p = 0.238). An earlier
   30% figure from three pairs was retracted before it shipped.
5. **The uncontended regression stands, reported unsoftened, and closed only
   partly.** Grouping cost 12–17% per writer at N=1–2 before the fast-path
   fix; after it, N=2 is closed and N=1 is no longer detectable rather than
   fixed. Both states are in this file.

## The architecture

Every writer issues its own `fcntl(fd, F_FULLFSYNC)` on its own file
descriptor for its own content, on its own thread, before it takes any
reservation. That never changes and is never delegated.

When writers contend they form a bounded queue that does exactly two things:
it amortizes the durability work they genuinely share — the intent-store
directory and the destination parent directories — and it bounds how many
honest syncs are in flight. An uncontended writer syncs immediately and waits
for nobody. No window, no timer, no epoch.

## Criterion 1 — whose flush makes a receipt true?

**The answer is a split, and both halves are stated here rather than one up
top and the other further down: NO for a writer's content bytes — every
acked writer flushed those itself, on its own descriptor. YES for shared
directory entries — one writer's `F_FULLFSYNC` on a directory descriptor is
what makes another writer's rename into that directory durable.**

An earlier draft of this file led with a flat "no". That was wrong. It was
true of content and false of directory entries, and the disclosure sat three
paragraphs below the headline — which is the evasion shape whatever the
intent, because a reviewer reads the bolded sentence as the answer. The
correction is mine and the finding was raised internally before any reviewer
saw it.

### The mechanism, as fact

| | content bytes | shared directory entry |
|---|---|---|
| which call | `fcntl(fd, F_FULLFSYNC)` | `fcntl(dirfd, F_FULLFSYNC)` |
| issued by | the writing thread itself | the round leader — possibly another writer's thread |
| on which descriptor | one this thread opened on its own temp | one opened on the directory both writers renamed into |
| covers | that writer's own bytes | every rename into that directory completed before the call began |
| ack | after every call above returns, on the success path only | same |

The arriving writer's code path is explicit about it: in `grouped_sync_dir`
the coverage branch returns `Ok(())` **without issuing a sync on its own
thread**, because a round that began after it arrived has already completed.
Our own measurement says it more loudly than the code does — **55 shared
directory barriers cover 240 transactions** at N=60.

### Defending the NO half — content

Content durability is `fs::honest_sync_path_capped(&d.tmp)`, called from
`sync_content`, one scoped thread per destination, on a descriptor that thread
opened. It is not shareable by construction: no queue, no owner thread, no
barrier between a writer's content bytes and the platter. Measured rather than
asserted: content barriers are **240 in both arms** at N=60, identical with
grouping on and off, while only the shared directory count moves. A design
resting content on a peer's flush would show that row falling with N.

### Defending the YES half — shared directory entries

Per gate item A10, a "yes" names the guarantee, its scope, where it is
documented, and the failure mode if it does not hold.

- **Guarantee relied on.** `fsync` on a directory descriptor makes that
  directory's already-completed entry changes durable; `F_FULLFSYNC` is the
  variant that additionally forces the drive to flush, which is what makes it
  survive power loss. The shared object and the synced object are the same
  object — this is the documented meaning of the call, not a side effect of
  one issued elsewhere.
- **Scope.** Directory-scoped, and the flush unit beneath it is **probably the
  APFS container rather than the whole device** — static extraction of the
  shipped kext indicates `F_FULLFSYNC` commits the container transaction
  shared by every file in it. Every writer this mechanism groups is inside one
  workspace and therefore one container, so the scope we rely on is narrower
  than the one we need. **Cross-container would break it**, and we design to
  the narrower reading.
- **Documented, not verified.** The directory-fsync semantics are the
  documented contract; APFS itself is closed, and the container-scope finding
  is agent-read disassembly — suggestive, not conclusive. We write
  "documented" where we mean documented.
- **Failure mode if it does not hold.** A crash could leave durable file data
  behind a lost directory entry. For a destination parent that means the
  rename is undone and recovery restores the complete old set — the handled
  path. For `.meridian/intents` it is worse: the intent record becomes
  unreachable and recovery has no account of a transaction whose renames
  partly landed. That is the case this guarantee is load-bearing for, and it
  is why the entry is flushed rather than merely ordered.

Three properties keep this from being the shared-flush trick we declined:

1. **The shared object and the synced object are the same object.** `fsync`
   on a directory making completed renames into that directory durable is the
   documented meaning of that call, not a side effect of one issued
   elsewhere. Nothing about a writer's own file data is claimed by it.
2. **The happens-before is enforced, not assumed.** `honest_sync_dir` returns
   only when a round that BEGAN AFTER this writer arrived has COMPLETED. A
   writer arriving mid-round cannot ride the round in flight, because that
   round may have begun before its rename. That costs the arriving writer one
   extra round, and I paid it deliberately: a design that lets an arrival ride
   the in-flight round would be faster and would be resting on timing that
   nothing checks.
3. **Content durability never depends on it.** A writer's content is already
   platter-durable before it reaches any shared step, so the shared call
   extends the promise to the entry and never carries the bytes.

**A failing covering flush does not ack.** The round records its outcome and
every writer it covered returns that outcome, so a follower of a failed round
receives the error rather than a silent success — `publish` then fails the
transaction. The named evasion pattern here is issuing the honest class but
leaving its result unchecked; the result is checked on both the leader and
the follower path.

The general form of the rejected trick is a cheap per-writer push plus one
shared full barrier. I did not build a cheap tier at all, so there is no
step in this design that could claim a guarantee it does not supply — see
the table below, where every row is the same call. On this host a cheap tier
would also have been an illusion: Rust's `File::sync_all` and `sync_data`
both issue `fcntl(F_FULLFSYNC)` on macOS, so the obvious way to write a cheap
push costs a full barrier anyway.

`F_BARRIERFSYNC` is considered and refused in every position. It orders; it
does not promise durability at return. Content bytes need durability because
a receipt vouches for them; the intent record needs it because a crash that
takes the record while keeping the renames leaves recovery with no account of
a transaction that partly landed; a directory entry needs it because durable
file data behind a lost entry is unreachable. Expected count zero everywhere,
and **measured zero** — see the three-column counts below.

## Which step supplies which guarantee

Every row is `fcntl(fd, F_FULLFSYNC)`. There is no cheap step, so no step can
claim a guarantee it does not supply. The ack sits behind all of them.

| # | step | scope | supplies |
|---|---|---|---|
| 1 | content temp, one per destination, own fd, own thread | outside every exclusion | power-loss durability of that writer's bytes |
| 2 | `O_EXCL` intent record (recovery images inlined) | inside the reservation | durability of the decision, before the first visible rename |
| 3 | `.meridian/intents` directory | inside, grouped | durability of the record's directory entry |
| 4 | destination parent directory, one per DISTINCT parent | inside, grouped | durability of the renames |
| 5 | `finalize` record rewrite | inside | durability of the applied phase, before the ack |
| 6 | `.meridian/intents` directory again | inside, grouped | durability of that rewrite's entry |

That is the whole table: six calls at D=1, and the interposer trace shows
exactly those six in that order (`.scratch/receipts/after-count-d1.log`).

### Exclusion-held work: 5 of the 6, and 3 of those amortize

Only step 1 sits outside the reservation. The team's seam ruling predicted
the exclusion would hold one sync; I found it holds five, reported it, and
the ruling was corrected. Steps 4, 5 and 6 cannot leave: releasing the
reservation before `finalize` lets an overlapping writer rename over these
destinations while this intent is still `Decided`, and recovery then matches
them against neither recorded identity and quarantines the workspace. Steps
3, 4 and 6 go through the grouping, so three of the five amortize under
contention.

## What changed: the count law

Before, measured at the syscall boundary: **`syncs = 6D + 4`** per
`publish()`, deterministic, reproduced at D = 1, 2, 3 and confirmed by three
methods across the tournament. At D=2 that is 16 honest syncs, 96 ms of sync
inside a 115 ms transaction — 85% of wall time — of which only 2 were the
user's content and 14 were bookkeeping, with one directory synced four times.

After: **`syncs = D + P + 4`** for D destinations across P distinct parent
directories, pinned by test and confirmed by trace.

| D (own parent each) | before | after | measured after, per-call ms | receipt |
|---|---|---|---|---|
| 1 | 10 | **6** | 5.24 · 8.84 · 12.78 · 5.05 · 5.34 · 4.37 | `after-count-d1.log` |
| 2 | 16 | **8** | 5.64 ‖ 6.11 · 5.14 · 4.27 · 4.97 · 5.18 · 5.72 · 3.91 | `after-count-d2.log` |
| 3 | 22 | **10** | 5.45 ‖ 5.10 ‖ 4.75 · 4.83 · 6.08 · 10.62 · 5.64 · 5.24 · 4.14 · 5.46 | `after-count-d3.log` |

`‖` marks calls the trace shows on DIFFERENT thread ids — the per-destination
content syncs, running in parallel. At D=2 that is 40.9 ms of device time
summed but ~35 ms on the critical path; before, D=2 paid 96.1 ms. Transaction
wall time fell with it: D=1 60.3 → 45.5 ms, D=2 115.3 → 46.5 / 39.9 ms
(~2.7×), D=3 148.7 → 63.2 ms.

### A fact conflict, settled against the trace rather than averaged

The measurement worker measured **`2D + 4`**; I had reported **`D + P + 4`**.
Settled by reading the D=2 trace, which resolves every path:

```
d1/f1.tmp-pub · d0/f0.tmp-pub                 2 content (distinct thread ids)
.meridian/intents/<txn>.intent                1 record
.meridian/intents                             1 intent dir
d0 · d1                                       2 destination parents — DISTINCT
.meridian/intents/<txn>.tmp-intent            1 finalize
.meridian/intents                             1 intent dir
```

Eight calls, and `D + P + 4 = 2 + 2 + 4 = 8` because the bench gave every
destination its own directory, so **P = D and the two formulas name the same
number**. Neither measurement was wrong; the bench had held layout constant,
which is the one variable that separates the forms.

The measurement worker then settled it properly, by varying that variable —
eight measurements at the syscall boundary, `F_BARRIERFSYNC` and plain `fsync`
zero in all eight (`layout-{sep,same}-d{1,2,3,4}.log`):

| D | separate dirs (P=D) | same dir (P=1) |
|---|---|---|
| 1 | 6 | 6 |
| 2 | 8 | **7** |
| 3 | 10 | 8 |
| 4 | 12 | 9 |

Every cell fits `D + P + 4`: `2D + 4` on the left, `D + 5` on the right. So
`D + P + 4` is the law, `2D + 4` is its separate-directory worst case, and
each distinct parent costs its own barrier after the renames — which is why
layout moves the count at all, by 25% at D=4.

Two independent instruments now agree on the distinguishing case: the trace
above, and
`honest_durability_gates.rs::a_publish_issues_its_enumerated_honest_sync_budget`,
which asserts 6 / 7 / 8 for one dest, two dests sharing a parent, and two
dests in split parents.

**Any single count quoted from this file names its layout**, because the two
layouts differ by a quarter at D=4.

Three columns, never an aggregate, at every N of the saturation run
(`L1-saturation.txt`): `F_FULLFSYNC` 1200/1600/1920/2400, `F_BARRIERFSYNC`
**0**, plain `fsync` **0**. The pre-registered falsifier column stayed at zero
throughout.

Where the reduction came from, in order of size:

- Recovery images now ride **inside** the single checksummed `O_EXCL` record
  instead of four side files with their own syncs — removes 8 calls at D=2,
  and removes a failure mode: the record is atomically complete-and-checksummed
  or absent, where before a crash could leave a partial image set that a
  record referenced.
- Repeated syncs of one directory collapse; destination parents dedupe by
  distinct parent.
- `finalize` stays on the critical path deliberately (below).

## What the grouping actually bought — the A/B control

Both arms in one binary via `fs::set_durability_coordination(bool)`, paired
inside each iteration with the order alternated, so a host whose load swung
40–120 during the window cannot hand either arm a win by sampling luck.
Receipts: `step5-ab-sweep.txt` (N ≤ 16, 3 reps each), `L2-ab-n60.txt` (N=60,
6 reps). Ratio is grouping-on ÷ grouping-off, so **below 1.0 favours
grouping**.

Sweep, 3 pairs per N (`step5-ab-sweep.txt`), medians:

| N | ratio p50 | ratio p99 | `group_wait` p50 | cohort peak | ON tx/s | OFF tx/s |
|---|---|---|---|---|---|---|
| 1 | **1.125** | 0.937 | **0.00 ms** | 1 | 12.0 | 13.6 |
| 2 | **1.169** | 1.176 | **0.00 ms** | 1 | 20.1 | 23.9 |
| 4 | 1.004 | 1.102 | 4.26 ms | 3 | 31.9 | 30.7 |
| 8 | 0.983 | 1.094 | 26.66 ms | 7 | 32.3 | 33.9 |
| 16 | 0.952 | 1.117 | 55.20 ms | 12 | 42.0 | 39.6 |
| 60 | 0.696 ⚠ | 1.325 ⚠ | 100.20 ms | 45 | 44.3 | 40.3 |

⚠ The N=60 row is three pairs and **both its figures were superseded** by the
18-pair run below. It is kept visible because deleting a superseded sample and
showing only the survivor is how a table starts lying.

**What this supports.** Grouping COSTS 12–17% per writer at N=1–2, is neutral
at N=4–8, and pays from N=16 up. The crossover sits between N=4 and N=8. The
low-N regression is real and consistent across three pairs at each N, and
`group_wait` is exactly 0.00 ms there — so the cost was not waiting, it was
the coordination path itself: a registry lookup and a lock per shared
directory sync that a lone writer could never amortize.

**The regression after the fix: closed at N=2, no longer detectable at N=1.**

| N | arm | pairs | median | ON better | sign p | range |
|---|---|---|---|---|---|---|
| 1 | pre-fix | 3 | 1.125 | 1/3 | 1.000 | 0.981–1.170 |
| 1 | **post-fix** | 12 | **1.068** | 4/12 | 0.388 | 0.760–1.221 |
| 2 | pre-fix | 3 | 1.169 | 0/3 | 0.250 | 1.002–1.174 |
| 2 | **post-fix** | 12 | **0.944** | 9/12 | 0.146 | 0.831–1.157 |

N=2 is closed — the 17% is gone. N=1 is **partial**: 12.5% → 6.8%, but at
4/12 and p = 0.388 the residual is no longer measurable in either direction.
That is "no longer detectable", which is not the same claim as "fixed", and it
is written as the former. The comparison also sets a 3-pair pre-fix median
against a 12-pair post-fix one, so the direction is credible and the magnitude
is not precise.

`group_wait` is 0.00 ms at N=1 and N=2 post-fix and **now also at N=4**, where
it was 4.26 ms before — the fast path widened the range in which a writer pays
nothing for coordination it cannot use.

**That regression was a code-versus-filing gap, and it is now fixed.** The
spec assigns this team "a shared queue forms a group commit only when
contention is detected"; a lone writer paying coordination it can never
amortize is unconditional coordination, not that. `honest_sync_dir` now takes
a fast path when no other writer is inside a directory sync, syncing its own
descriptor and skipping the round bookkeeping. The contention signal is one
`AtomicUsize`; a racing pair each sync their own descriptor, which costs
amortization and never durability. Pre-fix numbers stay in this file beside
the post-fix ones — a result showing only the fixed state hides the most
informative thing that happened.

### N=60, settled at 18 paired repetitions — and the tail effect is NULL

The N=60 row above came from three pairs. It was wrong, and the correction is
the most useful number in this file. Pooling 18 paired repetitions at N=60,
identical config, order alternated, load 120 → 30 across the runs
(`L2-ab-n60.txt`, `L2-ab-n60-more.txt`):

| quantity | median ratio ON/OFF | ON better in | range | sign test |
|---|---|---|---|---|
| p99 | 0.968 | **9 of 18** | 0.557–2.201 | p = 1.00 |
| p50 | 0.931 | 12 of 18 | 0.704–1.105 | p = 0.24 |

**p99: no detectable effect.** Nine of eighteen is exactly chance. This is a
settled null result rather than a missing measurement, and the earlier
0.87-versus-1.33 disagreement is what a null looks like when sampled three and
six pairs at a time.

**p50: about 7% better, not separated from zero.** The direction is consistent
across all three subsets (0.914, 0.935, 0.931), but 12 of 18 gives p = 0.24.
Suggestive, not conclusive, and it is written that way.

**A 30% N=60 win was retracted before it shipped.** A three-pair sample at a
different iteration count gave ratio 0.696; eighteen pairs give 0.931. Three
pairs on a host swinging load 30–120 is precisely the sampling luck the paired
method exists to catch, and it caught it — in our own favour, which is the
direction that matters. The 0.696 figure appears nowhere in this file's
conclusions.

### Post-fix, same config: the median becomes a real effect, the tail does not

Re-run at `01142b7e1` after the two fixes, **identical pre-declared config**
(18 pairs, iters=6, order alternated), `postfix-n60-18.txt`:

| set | pairs | p50 median | ON better | sign p | p99 median | ON better | sign p |
|---|---|---|---|---|---|---|---|
| PRE-FIX | 18 | 0.931 | 12/18 | 0.238 | 0.968 | 9/18 | 1.000 |
| **POST-FIX** | 18 | **0.876** | **17/18** | **0.000145** | 0.921 | 12/18 | 0.238 |

**p50 clears noise for the first time in this card**: 17 of 18 pairs favour
grouping at p = 0.000145, about 12.4% better per-writer end-to-end latency at
N=60 — at the same config that previously gave 12/18 and p = 0.24, so this is
not a config hunted for after the fact.

**p99 still does not separate.** 0.921 and 12 of 18 is directionally better
than the pre-fix null but p = 0.238. The honest line: **grouping improves the
median measurably and does not demonstrably change the tail.**

**The caveat that cuts against our own good news, stated by the worker who
produced the number.** Each set is internally paired, so ON-versus-OFF *within*
a set is sound. The PRE-versus-POST axis is **not** paired — the sets were
collected at different times on a host whose load moved (120→30, then 33→49).
So: "post-fix, grouping beats no-grouping at N=60" is strongly defensible;
"the fixes are what changed it" is plausible and consistent with the mechanism,
but rests on an unpaired comparison and is written that way.

**The win in this card is the count, not the scheduling.** Cutting 16 calls to
8 removes about half the device time from every transaction unconditionally.
Grouping redistributes a fixed budget and returns a low-N penalty plus a
high-N gain. That ordering was predicted before the measurement and the
measurement agreed with it.

Two claims the data does support strongly:

- **The uncontended writer waits for nobody.** `group_wait` is exactly 0.00 ms
  at N=1 and N=2 with cohort 1 — measured, not argued. Contention-triggered,
  never timer-triggered, is visible in that zero.
- **Cohorts genuinely form.** At N=60 one round covers up to 45 writers, which
  is the amortization doing what it exists for.

The risk the spec assigned this architecture was behaviour under bursty
concurrency. It is visible, it is bounded, and it is reported: `group_wait`
reaches ~100–230 ms at N=60. It is its own receipt field and is folded into
neither durability figure, because a writer that waited 200 ms for a cohort
did not spend 200 ms on durability.

## The pre-registered falsifiers, scored

Filed before the data in `results/hd-team-a-preregistration.md`. Quoted as
filed; no expectation was re-baselined after seeing a number.

| # | Filed falsifier | Verdict |
|---|---|---|
| 1 | "Measured `F_FULLFSYNC` count on content destinations BELOW `D`" | **NOT TRIGGERED.** Exactly `D`, traced by path and thread id: 1 at D=1, 2 at D=2, 3 at D=3, each on its own thread |
| 2 | "Any `F_BARRIERFSYNC > 0` on our path" | **NOT TRIGGERED.** Zero in all eight layout cells and at every N of the saturation run |
| 3 | "Any plain `fsync` appearing on our durability path" | **NOT TRIGGERED.** Zero in the same three-column receipts |
| 4 | "Overhead grows with D — measured count minus `D` minus `P` is not the constant 4" | **NOT TRIGGERED.** `count − D − P = 4` in all eight cells |
| 5 | "Count grows with N" | **NOT TRIGGERED, both arms, both code states.** Pre-change 10.00 barriers/commit at N = 8–60; post-change the control arm holds exactly 6.000 across the same range while the shipped arm FALLS to 4.229. My filed prediction of "flat" was half wrong — see below |
| 6 | "Grouping-ON shows no per-pair improvement over grouping-OFF at N ≥ 4" | **AMBIGUOUS on pre-fix code** — reported as ambiguous, not resolved in our favour. **CLEARED at N=60 on post-fix code** (17/18, p = 0.000145). The rest of the declared range, N = 4–16 post-fix, is **not yet measured**, so no claim is made there |
| 7 | "Bounded-at-16 shows worse p99 than unbounded at N=60" | **NOT TRIGGERED.** 18 pairs give no detectable p99 difference in either direction (9 of 18, p = 1.00) |

### Falsifier 6 is ambiguous, and that is the finding

Per-pair p50 ratios at N ≥ 4: **1.004** (N=4), **0.983** (N=8), **0.952**
(N=16), **0.931** (N=60, 18 pairs, 12 of 18 favouring ON, sign test p = 0.24).

Three of the four sit below 1.0 and the direction is consistent across every
subset, so improvement is not absent. None of them separates from zero at the
power we have, so improvement is not established either. **This falsifier is
neither cleanly cleared nor cleanly triggered, and it is reported that way.**

Weak improvement and absent improvement are different findings, and the
difference is exactly what a team is tempted to collapse in its own favour. We
do not collapse it. A reviewer should score falsifier 6 as ambiguous.

### Falsifier 5, closed by measurement — and my prediction was half wrong

I filed this prediction, and it is kept verbatim because a prediction that is
quietly rewritten after the data is worth nothing:

> Grouping changes which writer issues a shared sync, never how many a
> transaction needs, so the count should stay flat in N.

**Flat is what the CONTROL arm does. The SHIPPED arm falls.** Measured at
`01142b7e1`, D=1, load 28.9–30.3, three columns every run with
`F_BARRIERFSYNC` and plain `fsync` zero throughout
(`postfix-count-vs-n.txt`, `postfix-count-vs-n-OFF.txt`):

| barriers per commit, D=1 | N=8 | N=16 | N=32 | N=60 |
|---|---|---|---|---|
| coordination OFF (control) | 6.000 | 6.000 | 6.000 | 6.000 |
| coordination ON (shipped) | 5.375 | 4.818 | 4.442 | **4.229** |

Falsifier 5 is **refuted on both arms** — the count never grows with N on
either. The control arm sitting at exactly 6.000 across a 7.5× range of N is
also the cleanest confirmation that `D + P + 4` is per-transaction and
concurrency-independent: D=1, P=1, `1 + 1 + 4 = 6`, four times, no drift.

Where my prediction went wrong is instructive rather than embarrassing:
grouping does not change how many syncs a transaction *needs*, but it does
change how many a transaction *issues*, because the shared one is paid by one
writer on behalf of the cohort. Averaged per commit, that is a fall.

### The criterion-1 answer, proven by target attribution

This is the strongest evidence in this file, and it is a measurement rather
than an argument. At N=60, 240 transactions per arm, barriers broken down by
the target they were issued against:

| target | OFF (control) | ON (shipped) |
|---|---|---|
| per-txn CONTENT | 240 | **240** |
| per-txn destination directory | 240 | **240** |
| per-txn intent record | 240 | **240** |
| per-txn finalize tmp | 240 | **240** |
| SHARED `.meridian/intents` | 480 | **55** |
| total | 1440 | 1015 |

**The four writer-local barriers are identical in both arms.** Grouping does
not touch one byte of a writer's own durability. The entire difference is the
one genuinely shared directory, amortized 8.7×. Totals reconcile exactly:
240×4 + 480 = 1440, and 240×4 + 55 = 1015.

That is the CONTENT half of the criterion-1 answer demonstrated by target
rather than asserted by design intent: a design resting content bytes on a
peer's flush would show the content row falling with N, and ours does not
move. It says nothing about the shared directory entry, which is the half
answered YES — that row is exactly the one that falls, by design and by
disclosure.

The destination-directory row staying at 240 in both arms is the same property
seen from the other side: in this workload each writer owns its own
destination directory, so there is nothing to share and grouping correctly
amortizes nothing. Only what is actually shared is amortized.

Net at N=60: 6.000 → 4.229 barriers per commit, a 29.5% reduction in device
work **on top of** the count-law reduction from `6D + 4`. The floor is 4 —
the four writer-local barriers, which by construction can never be amortized —
so at 4.229 the shared work is nearly fully amortized already.

**Attributable by control, not by assumption.** The measurement worker added a
switch so a count sweep can run on either arm. Without the OFF arm this fall
would have been a grouping win by assumption; with it, the attribution is
measured.

### `cohort_size` 45 against a cap of 16 — two different quantities

Asked by the measurement worker, answered from the code rather than from
interpretation:

- `HONEST_SYNC_SLOTS = 16` bounds honest syncs **in flight**. It is enforced
  in `honest_sync_capped`, reached only through `honest_sync_path_capped`,
  which is called only from `sync_content` — the pre-reservation content
  temps.
- `cohort_size` counts **writers covered by one shared directory round**:
  `members = state.waiting + 1` in `honest_sync_dir`, the writers parked on
  that directory plus the round's leader.
- `honest_sync_dir` is deliberately NOT capped — a slot wait taken while
  holding a reservation, against a writer holding a slot and waiting for that
  reservation, is a deadlock.

So a cohort of 45 is 45 writers waiting on ONE sync, not 45 syncs running. The
cap cannot bound it and should not: a larger cohort is better amortization,
and the two numbers cannot contradict each other because they count different
objects. **Not a defect.**

### The cap is enforced AND binding — measured, with a control

This was an open gap in an earlier draft of this file: the constant's
justification was the device sweep, but its enforcement was code-read only,
and `cohort_size` cannot stand in for it because it counts the other object.
It is now measured, by an external receipt that required no instrumentation
inside `crates/fs`.

Method: each interposer line carries a start timestamp, which turns every
barrier from a point into an interval; peak concurrent occupancy is then
reconstructed offline by sweeping interval endpoints, with releases ordered
before acquires at a tie so a touching pair is never miscounted as an overlap
— a choice that makes the number conservative rather than inflated.
Requalified against the positive control first (`qualify-interval.log`).

N=60 writers, D=1, at `01142b7e1` (`occupancy-n60.log`, `occupancy-n60-OFF.log`):

| arm | peak concurrent, all barriers | peak concurrent, CONTENT | calls |
|---|---|---|---|
| ON (shipped, capped) | **15** | **10** | 1525 |
| OFF (control, uncapped) | 28 | 28 | 2160 |

**Enforced:** peak 15 across all barrier classes and 10 for content, both under
16, across 1525 barriers at 60 concurrent writers. No sample exceeded the cap.

**Binding, which is the half that needed the control:** the uncapped arm
reaches 28 concurrent content syncs on the identical workload. Without the OFF
arm, "peak 15, under 16" would have been equally consistent with a cap that
does nothing against low demand. And 28 lands inside exactly the region the
bound was ruled to avoid — the device sweep saturates at N≈16–24 with the tail
multiplying past it.

**Stated limit:** the interposer writes a line per barrier, so this is peak
occupancy *under instrumentation*, not of an unobserved run. The perturbation
is microseconds against 5–15 ms barriers, and it would take a far larger one
to turn 28 into 15.

## Why the queue is bounded at 16

Raw device, no engine: N threads, each its own file and descriptor, `pwrite`
4 KiB + `F_FULLFSYNC`, released from one barrier. Two to three sweeps, one in
reverse N order against thermal and ordering drift
(`fffscale-sweep.log`, `fffscale-sweep-reverse.log`, `fffscale-sweep-n60.log`).
At N=1 the driver reproduces the probe that ruled this card: p50 4.98–5.01 ms
and p99 10.37–10.53 ms against the probe's 4.386 / 10.352 ms.

| N | syncs/s | vs N=1 | p50 ms | p99 ms |
|---|---|---|---|---|
| 1 | 179–185 | 1.0× | 4.98–5.01 | 10.4–10.5 |
| 2 | 191–213 | 1.1× | 8.81–9.59 | 20.8–25.1 |
| 4 | 258–292 | 1.5× | 12.2–13.7 | 42.0–44.5 |
| 8 | 439–507 | 2.6× | 13.5–14.7 | 51.5–68.3 |
| 16 | 562–786 | 3.6× | 15.6–20.0 | 64.5–146.8 |
| 24 | 815 | 4.5× | 22.0 | 94.3 |
| 32 | 739–767 | 4.1× | 27.6–30.5 | 135.7–260.2 |
| 60 | 899–909 | 4.9× | 45.2–45.3 | 241.5–259.4 |

**Method caveat, up front: this table is unpaired** and the host was not
quiescent (load 28–52 during the window), so absolute levels carry more
uncertainty than their precision suggests. The reverse-order sweep controls
ordering and drift but is not pairing. The two claims it is load-bearing for —
a ceiling near N=16–24, and a tail that grows past it — are re-derived under
paired sampling in `paired-sweep.tsv`.

Concurrent `F_FULLFSYNC` neither serializes nor fully coalesces: it partially
coalesces to about 900 syncs/s at N≈16–24, then stops, while p99 keeps
climbing 24× from N=1 to N=60. So writer-local parallel sync is worth having
(full serialization would be flat at ~180/s) and unbounded concurrency past
the knee buys nothing. 16 captures 3.6× of the 4.9× ceiling at roughly a third
of N=60's tail.

**The bound's price, measured and paired** (`L3-arrival-cap16.tsv`, 8 reps,
alternating order): a writer released exactly when 16 barriers are in flight
waits a median 4.0× its own barrier time (p50 ~5.2 ms quiet, ~21.0 ms
saturated; p99 ratio median 6.4×). Anyone quoting the throughput gain without
this is quoting half the trade. The filed falsifier was "the wait exceeds
roughly one barrier time WITH NO throughput gain": the first clause triggered
and the second did not, so the bound survives with a real price. That price is
the device floor measured with no engine in the path — any design capping at
16 on this hardware pays it.

## The card's premise, corrected on this platform

The card states the defect as drive-cache-class durability. **On macOS that
premise does not hold**, and the correction is ruled, not ours to argue:
`decisions/2026-08-16-sync-class-platform-scope.md` scopes merged-plan §4.7 by
platform. On rustc/std 1.97.1 (aarch64-apple-darwin) `File::sync_all` and
`File::sync_data` both issue `fcntl(F_FULLFSYNC)`, verified by direct
zero-`F_BARRIERFSYNC` checks across three traces
(`results/instrument-evidence/fs_usage_syncall.txt`). So the engine's
`sync_all` call sites were always honest class here, and the ruling probe's
0.019 ms plain-`fsync` figure measured a class this engine never used.

What that changes is what this work corrected. Not "make the sync honest" — it
was honest — but **stop paying for honesty more times than a transaction
needs**. Every number above is reported against that corrected baseline.
Nothing here was measured on another platform and nothing here claims
anything about one.

## Criterion 6 — internal audit, and what it does not settle

A non-opus seat audited this diff specifically for invented permission layers,
unrequested guard scaffolding and capability plumbing — the standing landing
gate for an opus-implemented engine diff. Card `tasks/hd-a-crit6-audit.md`.

Verdict: **none found**, with the search shown and the enumeration empty. It
classified the three constructs a reviewer is most likely to stop on — the
in-flight slot semaphore, the `TallyDrain` scope guard, and the test lock
guards — as ordinary concurrency control rather than permission machinery.

This is reported as an internal audit result, not as a gate outcome. **The
gate reviewers score criterion 6 themselves**, and an internal pass is
evidence for them to weigh rather than a conclusion they inherit.

## Scope: what these numbers describe

`publish()` has no production caller yet — it appears only in its own tests
and in `publish_group`. The live splice door is still `apply_batch`, and the
merged plan rewires it at step 7, the cutover; this card is step 6. So these
figures describe the path as it will run after the cutover, measured on the
real code, not a projection. Counts are reported publish-only; no production
staging caller was invented to make a bigger number available.

## Instruments and their blind spots

1. **In-process counter** — `AtomicU64` in `crates/fs`, incremented in
   `honest_sync`. The gate-2 receipt and the gate-3 proof. **Blind spot:** it
   cannot see `write_fsync` / `fsync_dir`, which reach the device through
   `sync_all`, so it under-reports whole-transaction cost by construction and
   is not used for cost numbers.
2. **`DYLD_INSERT_LIBRARIES` interposer** (`.scratch/fffspy.c`) at the syscall
   boundary — sees every `fcntl`/`fsync` whatever Rust API reached it, with
   resolved path, thread id and per-call latency. The cost instrument, and the
   one that shows content syncs on separate threads.
3. **`fs_usage`**, kernel-level and not ours — the independent cross-check,
   because a count confirmed by a tool we did not write is harder to dispute
   than one from our own dylib.

Every count is reported in three columns — `F_FULLFSYNC`, `F_BARRIERFSYNC`,
plain `fsync` — never merged. A barrier-fsync standing where a full fsync is
expected is a silent honesty downgrade, and only a per-class column shows it.
Zero is a legitimate value.

## Platform statement

`F_FULLFSYNC` is macOS-only and the 4.386 ms baseline that ruled this card is
a mac number, so every figure here was produced on the mac named above.
Woodpecker CI (Linux, `workstation-nyc-2`) is the compile and test gate for
this branch and cannot produce these figures — on Linux the honest class is
`sync_all` and the `F_FULLFSYNC` arm compiles but never runs. A number without
its platform is not a number.

## Gate milestone checklist

| # | Gate | State |
|---|---|---|
| 1 | Compiles, branch pipeline green, bound by API commit fields | **MET** — pipeline 623 success on every lane, commit field `8302294e7…` read from the API step table by two readers independently. A later commit adds the two fixes below; its pipeline is the one this gate is finally claimed on |
| 2 | Instrument-verified honest sync on the content path | **MET** — interposer trace shows `F_FULLFSYNC` on `d0/f0.tmp-pub` with resolved path and thread id (`after-count-d*.log`); in-process counter pins the per-publish budget by test |
| 3 | No sync inside the linearized state-owner step | **MET** — `honest_durability_gates.rs::the_linearized_state_owner_step_issues_no_honest_sync` reads the counter either side of `owner.apply`, asserts zero issued and asserts the step ran (`seq == 1`) |
| 4 | Gate-12 separation visible in recorded numbers | **MET, with one gap named** — `content_fsync`, `metadata_publish`, `group_wait`, `cohort_size` are separate receipt fields read straight off the receipt by the harness, and `group_wait` / `cohort_size` appear as their own columns in the sweep tables. The published sweep tables do not yet print `content_fsync` and `metadata_publish` as columns; that split is available per call in the interposer trace by resolved path |
| 5 | p50 and p99, one writer and N concurrent, N stated | **MET** — N = 1, 2, 4, 8, 16, 60, paired A/B both arms, `step5-ab-sweep.txt` and `L2-ab-n60.txt` |

### CI notes carried as receipts

- The perf-gated clippy lane went green on 623, which settles by CI — not by
  file reading — an earlier lint report of mine that described the working
  tree rather than the commit. A local gate result in a multi-agent working
  tree names an instant; only a sha is reviewable.
- Pipeline 623 carries the same `perfsuite` dependency graph and lockfile that
  pipeline 620 was killed on, with no change to either. A dependency-graph or
  lockfile defect cannot pass in one run and kill another; the kill was
  supersession — 620 was cancelled two seconds before 623 cloned, by our own
  next push.

## One saving found and declined

Moving `intent::finalize` off the critical path removes 2 syncs and looks
free. It is not. `publish()` calls `finalize` before returning `Ok`, so every
acked transaction is `Phase::Applied` on disk. A `Decided` intent whose
destinations are all-new — exactly what an acked-but-unfinalized transaction
looks like after a crash — is recovered through the recorded policy, and
`Policy::Restore` restores the complete old set. Moving finalize after the ack
therefore opens a window where a crash rolls back a transaction the caller was
already told had committed: a durable ack preceding real durability, which is
the object this card exists to remove rather than reintroduce.

We also did not "fix" it by making recovery forward on all-new regardless of
policy, because that rewrites a recovery contract ruled deliberately in the
previous card.
