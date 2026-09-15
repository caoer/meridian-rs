---
type: reference
id: status
status: standing
description: What the binary exposes and verifies today, reproducible from the commands shown. Also the home of R12, the armed-plane exit reading.
owns: [what the binary exposes today, R12 — the armed-plane exit reading]
---

# Status

A **descriptive** snapshot of the shipped operator surface: what the `mrd`
binary exposes and verifies today. Numbers are reproducible from the commands
shown — prefer running them to trusting this prose.

This page describes behaviour as shipped; it does not rule it. Wire **design
law** lives only in `wire-contract.md`, the one standing contract, and design
wins on conflict.

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

## Build

- Toolchain: Rust edition 2024, `rust-version = 1.96`.
- `cargo build` builds the twenty-seven default members (engine planes,
  `workspace`/`cache`/`registry`/`mrd` CLI foundation, attestation's `git`
  plumbing leaf); `perfsuite` is outside default-members (28 members total
  under `crates/`) and needs `cargo build -p perfsuite`.
- Fork: `pulldown-cmark` is rev-pinned via `[patch.crates-io]` (`obsidian`
  branch); see the workspace `Cargo.toml`.

## Wire surface

What the wire offers, in two readings: the design law, then what the binaries
ship today.

**Design law:** the content-hash noun is **`fingerprint`** (the workspace
content hash); to **mint** is to issue one of these tokens. Mint addresses are
**segments only**. Ops: `hello`, `toc`, `cat`, `extract`, `read`, `resolve`,
`links`, `splice` (only write), `fingerprint`, `diff`, `sub`; standing
additives `plan_edits`, `pin`, `create`, `hello.identity`, …

**As shipped** (may lag design; gaps are debt, not law):

- The daemon answers protocol 1 as `meridian-daemon/1.1.0` (derived:
  `concat!("meridian-daemon/", env!("CARGO_PKG_VERSION"))`); its socket is
  the only host — no stdio sidecar (wire-contract §3.3).
- Binaries still carry a dual negotiation path and some legacy
  `root`/`if_root` spellings in code and caps tables; standing emission and
  agent teaching use `fingerprint`/`if_fingerprint`/segments.

Capabilities agents should assume (design):

```
hello (+ identity.build)
toc cat extract read
resolve links links.require_fingerprint
splice splice.if_node_rev splice.if_fingerprint splice.dry
splice.receipt splice.verdicts splice.plan_edits splice.pin
fingerprint diff sub create
```

Also standing:

- `meta.duration_us` on dispatched responses; composed-read authz facts
  (`span`/`content_span`/`anchors[]`);
- a `fingerprint` on each served section row of sections-mode reads
  (pin-proof token, wire-contract § A.3); reads mint nothing and carry no
  `actor`;
- pin/error codes: `wire-contract.md` § A.

Some host fields still emit a **joined display string**. That is leftover debt,
not address law.

## Workspace CLI

`mrd` (`crates/mrd`) is the operator CLI over the workspace foundation. Four
house words recur below. A **door** is a place where a request enters the
engine. A **face** is one rendering of a verb's answer, human or `--json`. A
**leg** is one branch of that answer, served or refused. A **drawer** is one
workspace's own on-disk cache directory. The verbs:

```
mrd init [PATH] [--name NAME]
 declare the root (PATH's own MERIDIAN.md,
 `type: meridian-root`), register its drawer, reconcile
 shadowed descendant drawers
mrd unregister [PATH] drop the daemon entry (if a daemon answers) + the drawer.
 A PATH whose directory is already gone is matched as
 given — `Registry::unregister`'s own fallback key, and
 the stale-entry class a sweep leaves behind. A vanished
 path keyed by nothing refuses (exit 2) rather than
 reporting the never-registered clean no-op — and that
 refusal names only what the run checked: the registry
 only when a daemon answered, the drawer only when the
 cache root resolved and the drawer directory could be
 probed. An unchecked half is reported as unchecked —
 "the registry was NOT checked", "the drawer could not
 be examined (…)" — and the unknown stays open, instead
 of asserting an absence nobody looked for. `--json`
 spells that null: `drawer_removed: null` with the
 reason under `drawer_unexamined`
mrd resolve [PATH] report how a path resolves — the tier that answered and
 the root it named (read-only; writes nothing). PATH
 also takes the agent-plane `root:path` spelling: a
 rooted ref answers WHERE THE REF LANDS — the physical path, the
 rooted lane, the bound root and its workspace, and
 the canonical `root:path` spelling — resolved through
 the read door's seam with no existence check and no
 daemon contact. A typo'd or unbound root refuses as a
 root problem (exit 1, the seam's refusal family, the
 bound names enumerated), never a literal-path lookup
 of the colon-bearing string. A `#` fragment refuses:
 resolve answers at path grain
mrd links [PATH] the corpus edge map (whole corpus, or one file),
 answered by the daemon (auto-spawned) or in-process
mrd read <PATH>[#FRAG] [--section SEL]
 the composed read: addressing + content + render at
 ONE engine snapshot (daemon or in-process; human
 output is the rendered text verbatim). PATH takes the
 agent-plane `[root:]path` spelling (address-grammar
 §4.1 colon law): a rooted ref binds to the named
 root's workspace from the machine mount table
 (~/MERIDIAN.md, read fresh per call — the same
 name→workspace binding a wire client and the engine's
 pin-cross-root lane resolve), then reads the rel half
 inside that root, warm and degrade alike. The root
 reading wins unconditionally — a head colon is never
 a literal path, so a typo'd or unbound root refuses
 with the bound names enumerated (the pin door's
 refusal family, exit 1, {workspace,error} under
 --json) instead of degrading into an ambient lookup.
 The rooted lane spans EVERY page-taking door
 (address-grammar §4.6): read,
 fingerprint, resolve, links, walk, repair, realise,
 run, rules, put (TARGET and --scope),
 rm, pin (PAGE; TARGET was already cross-root), and
 script --files — one resolution seam, the page's tree
 governing conventions and receipts. The preset lane
 (unfold/reconcile/new) refuses rooted refs with a
 teaching until it rides the daemon (§4.6, the stated
 exception); non-page arguments (arm --at, sql,
 test --history, status --cwd) stay as they are. SEL is a
 heading chain, a `^id`, or a dewey ordinal — the
 chain joins on `/` and that delimiter is dead for
 heading text (§ the joined selector coat). A section
 read is bounded: at most 20000 words served and 64
 distinct selectors per call, refused never truncated,
 with repeated selectors collapsed and the collapse
 stated. The section map is never word-bounded — it
 prices every section (`words`) before you ask for one,
 and it is the way back in from a refusal. When the
 daemon's hello serves the `scoped-guards` cap
 (wire-contract §5.4 family), a `--json` read also
 captures the read file's scoped token through the
 §4.7 mint arm — one extra `fingerprint {scope}`
 exchange on the same connection — and carries the
 mint body as the frame's `mint` key beside `read`;
 the wire `read` body's own `fingerprint` stays the
 ambient world token (§5.1, unchanged). No cap, the
 human face, or the in-process degrade: no mint call,
 frame byte-identical to before. A mint the daemon
 refuses after advertising the family is a loud
 refusal, never a silent omission. The standalone door
 is `mrd fingerprint` — folders and raw-byte names
 mint there, no read required
mrd fingerprint [PATH | --scope-bytes B64] [--json]
 the standalone §4.7 mint door: bare, the §5.1 world
 token (v2-identical, no cap needed); PATH, the named
 node's scoped token — workspace root, folder, or file
 leaf; `--scope-bytes B64` (base64url over the raw
 path bytes), a node whose name the UTF-8 `Path` noun
 cannot carry. PATH takes the agent-plane `[root:]path`
 spelling (address-grammar §4.1 colon law, the read
 door's own rooted lane, one resolution seam): a
 rooted ref binds to the named root's workspace from
 the machine mount table and mints the rel half's node
 THERE — the token that root's own workspace mints,
 never an `absent` minted against the ambient
 workspace's literal string. The root reading wins
 unconditionally: a typo'd or unbound root refuses as
 a root problem with the bound names enumerated (exit
 1, {workspace,error} under --json), never `absent` —
 an `absent` for a misspelled root is a permanently
 true premise whose guard can never fire. A genuinely
 missing path inside a BOUND root still mints `absent`
 (§5.6 lawful absence). A `#` fragment on PATH refuses
 at path grain, rooted and ambient alike (exit 1,
 {workspace,error} under --json — the resolve door's
 posture): a mint binds a node, never a section, so
 splitting the fragment off would mint the WHOLE FILE
 under a section echo — a §4.7 desync frozen into the
 receipt — while the ambient literal would miss and
 mint a permanently-true `absent`. A name carrying a
 literal `#` mints through `--scope-bytes`. On a
 rooted mint the scope
 echo is the caller's rooted spelling and {workspace}
 names the root's bound workspace — token and address
 stay paired in the caller's own frame (the §4.7
 desync guard); the wire carries the rel half only.
 The echo is copy-pasteable as minted: `mrd put
 --scope` accepts the rooted spelling for the
 workspace it writes, so token and echoed scope paste
 from one mint into one put verbatim.
 `--scope-bytes` stays ambient — raw bytes carry no
 root head. Exactly one spelling — a mint names ONE
 node (§4.7); both refuse at parse, exit 2. Scoped
 arms ride only when the daemon's hello serves
 `scoped-guards` (taught refusal at exit 2 otherwise,
 nothing sent). The answer is {fingerprint, seq,
 scope|scope_bytes} with the request's spelling echoed
 beside the token (the §4.7 desync guard); a lawful
 path with no node answers the reserved token `absent`
 (§5.6). `--json` answers {workspace, mint} on the
 served leg and {workspace, error} on a refusal. Exit
 triad: 0 minted / 1 engine refusal / 2 bad invocation
mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
 [--if-fingerprint FP] [--scope PATH | --scope-bytes B64]
 [--receipt PATH#ANCHOR] [--field K=V]... [--json]
 the batch write: the edits ride stdin as a BARE JSON
 array — the VALUE of the wire §4.4 `edits` field, not
 the request object around it (id / op / path are
 argv's here) — as a wire `splice` to the running
 daemon (authenticated IPC; no direct-publication
 fallback). The daemon must come up (`mrd daemon`, or
 the next call auto-spawns it). A
 guardless put is a wire client: fingerprint-or-force
 applies (`--force` or `if_node_rev`). `--scope PATH`
 narrows the `--if-fingerprint` premise to the named
 node (wire-contract §5.4): FP is then that node's
 scoped token from the §4.7 mint arm, not the world
 value — a disjoint sibling's birth no longer refuses
 the put. The scope takes the agent-plane `[root:]path`
 spelling (address-grammar §4.1 colon law, the mint
 door's own seam): a rooted scope is accepted exactly
 when the named root binds the workspace this put
 writes — the spelling a rooted §4.7 mint echoes
 pastes beside its token verbatim — and the wire
 carries the rel half only (the §5.4 `scope` field
 stays workspace-relative on the wire). The root
 reading wins on a head colon, never a literal node
 name: a bound root whose workspace is NOT the one
 this put writes refuses naming both workspaces, and a
 typo'd or unbound root refuses as a root problem with
 the bound names enumerated (exit 1, {workspace,error}
 under --json, the seam's refusal family) — a rooted
 scope never surfaces as the §5.5 "no premise covers"
 coverage refusal. The write TARGET takes the rooted
 spelling too (address-grammar §4.6): a rooted put resolves to
 the named root's workspace and writes there under
 that tree's law; the wire still carries the rel half
 only. The pair law is
 the CLI's own wall: `--scope` without
 `--if-fingerprint` is half a premise, exit 2.
 The §1 path law is the same wall: a `--scope` spelling
 the law refuses (absolute, `.`/`..`/empty segment, or
 empty — the unquoted-shell-variable mistake) refuses
 at exit 2 before any dial, teaching the law, naming
 the flag, the respell when the spelling lies inside
 the workspace, and the recoveries: a scoped token
 binds the exact spelling the §4.7 mint echoed
 (`mint.scope`), and without `--scope` the premise is
 the §5.1 world fingerprint. Under `--json` this wall
 answers the `{workspace, error}` envelope on stdout.
 `--scope-bytes B64` is the same premise for a node
 whose name the UTF-8 `Path` noun cannot carry: B64 is
 base64url over the raw path bytes (§5.4), FP is the
 token the §4.7 `fingerprint {scope_bytes}` mint
 echoed, and the pair rides the wire as one `guards[]`
 entry — `scope_bytes` is a top-level field on NO
 write door (the §5.4 field matrix). Exactly one of
 `--scope`/`--scope-bytes`: two spellings of one
 premise refuse at parse, exit 2. The pair law and the
 cap wall hold for it exactly as for `--scope`; the §1
 path-law wall does not apply (raw bytes are the names
 that law's noun cannot spell), so the face refuses
 only the empty spelling, and an undecodable base64url
 is the engine's taught refusal at exit 1.
 Cap-aware: when the connected daemon's hello does not
 serve `scoped-guards`, a scoped put refuses with a
 teaching at exit 2 before any engine write — the
 daemon cannot check the premise, so nothing is sent.
 The face teaches
 the grammar itself: `--help` states the target
 shapes ({"hpath":[…]} / {"anchor":"…"} / {"fm_key":"…"})
 and the nested edit shapes ({"match":{"old","new"}} /
 {"put":{"at","text"}}) with a working batch, and a
 malformed-stdin refusal repeats that working shape
 beside the decoder's own words. `--json` is the machine
 face on BOTH legs: a commit answers {workspace, put};
 an engine refusal answers {workspace, error} on stdout
 (the engine's §8 error body, v3 vocabulary — never
 empty stdout) beside the human stderr line, exit triad
 unchanged. The human commit face prints one line per
 FIRED intent beside the fingerprint —
 `fired: <rule_id> <action> → <target> (receipt <addr>)`,
 target omitted when the intent carries none, the
 receipt address VERBATIM (the pairing key delivery
 faces echo as `correlation`) — what the write armed,
 never that anything was delivered; a workspace with
 no armed hooks prints exactly what it always did
mrd rm <PAGE> --rev <FILE_REV> [--if-fingerprint FP] [--dry] [--actor A]
 [--now T] [--json]
 guarded file death (wire-contract § A.3 remove door):
 the write model's third mutation beside `new` (birth)
 and `put` (edit), through the daemon `remove` door over IPC
 (remove-what-you-read CAS + referential check + armed
 gate; no direct-write fallback). `--rev`
 is the page's whole-file rev from a prior read,
 REQUIRED — the engine demands it from every origin and
 there is no `--force` on this door. A page with inbound
 wikilinks, embeds, or ambient meridian-lock pins
 refuses `remove_refused` naming every referring file,
 its edge kind, and its count — unlink those edges,
 then rerun. `--json` answers both legs: a removal
 {workspace, rm}; an engine refusal {workspace, error}.
 Exit triad: 0 removed|dry / 1 refused / 2 bad
 invocation
mrd pin <PAGE> <TARGET>#<SELECTOR> [--fingerprint TOKEN] [--vibe] [--dry] [--json]
 mint a meridian-lock pin: PAGE records the claim,
 TARGET#SELECTOR is the content being attested
 (heading path / `^id` / dewey — see § mrd pin).
 `--fingerprint` supplies the § A.3 proof token —
 optional on this door, verified whenever supplied
mrd repair [PAGE] [--dry] [--json]
 lost-pin repair: walk the repository's own history for
 the content of pins whose evidence is gone (both planes
 dark — the live target no longer verifies the
 fingerprint AND git no longer holds the recorded blob),
 and repoint each recovered pin's hash at the durable
 blob carrying it. No match anywhere in history is a TRUE
 LOSS, reported and never auto-fixed
mrd retire <report|mark> [--id ID] [--dry-run] [--expect-root ROOT]
 the type-2 retirement DSL: report labels measured vs
 declared; mark sweeps the `~~term~~ replacer (retired: ID)`
 markers over meridian-retire blocks (idempotent; REQUIRES
 --expect-root unless --dry-run — quiesce other writers and
 commit the vault first)
mrd walk <PAGE> [--down] [--depth N]
 the context-assembly listing over the pin graph;
 every answer cites the revs it read
mrd rules [PATH] [--workspace | --user] [--json]
 the effective-rules print verb: what governs at PATH
 after id-based override resolution — winner first, the
 pages it shadows beneath it, plus a separate armed
 column read from the attested armed set (read-only)
mrd arm <ID> --mode <off|warn|block|armed> --rev <16HEX> [--at DIR] [--json]
 the ARM act: resolve ID at the arm root (--at, a
 workspace-relative directory, default `.`), admit the
 attestation only if the live page rev equals --rev,
 and pin the winner into meridian/armed-rules.md. A
 check arms off|warn|block, a hook off|armed
mrd config the MERIDIAN.md config plane: resolve the bootstrap
 (MERIDIAN_CONFIG, then $HOME/MERIDIAN.md) and print path,
 state, origin, rev/fingerprint, the BOUND mount table, and
 declared tools — this verb PUBLISHES the mount table.
 It resolves in THIS process from THIS process's env, and
 says so on BOTH faces: the human line "answered by: this
 process", `--json` at the `answered_by` key. A serving
 daemon reads ITS OWN env on EVERY mount-addressed path —
 `mounts`, `walk`, `sql`, cross-root read and write — so a
 wire client can be served a different table on any of
 them, not just on discovery; MERIDIAN_CONFIG set for this
 CLI never reaches that daemon. The refusal path says the
 same on stderr: a refused `mrd config` is THIS process's
 chain and says nothing about the table a daemon binds.
 What the daemon is NOT frozen on is the file's CONTENTS —
 it re-derives per call on a blake3 of the bytes
 (wire-contract § A.5), so editing the table the daemon
 already reads rebinds with no restart
mrd config get [KEY] [--json]
 the config plane's VALUE face: the same bootstrap chain,
 then the `^config` block in that file — a `starlark` fence
 whose `config()` returns this machine's config. Evaluated
 in the sealed kernel (standard globals only, `load`
 disabled, `EvalLimits::default()`); bare prints the whole
 returned value, KEY one member by dot-path
 (`repos_root.coscene-wiki`), resolved exact-key-first at
 every level so a member really named `a.b` stays reachable
 (§6a.3). The value is ARBITRARY — the engine declares no
 schema and reads no key of it (`meridian-md-schema.md` §6a).
 It prints the
 value and nothing else, so `r=$(mrd config get repos_root)`
 is the intended use: a string prints bare, any other shape
 prints as JSON, `--json` prints JSON for every shape. This
 verb never binds roots — an unbound root refuses `mrd
 config` and leaves this one answering. Exits: 0 value
 printed / 1 the chain, the block, the eval, or the key
 refused / 2 bad invocation
mrd check [--core] [--staged] [--commit-gate [--require-pins]] [--json]
 the pure READ validity verb: claim drift + the pin
 plane (pin verdicts + blob anchoring); writes nothing,
 mints no receipt. WRITE HISTORY is NOT assessed (NOT
 CHECKED, never grey) — the engine keeps no memory;
 green means the world still matches the pins, not how
 it got there
mrd skill hook emit the commit-fence contract to stdout: the markdown
 IS the contract, the reader places it; writes no file,
 has no --json face (see § mrd skill hook)
mrd status [--cwd PATH] [--json]
 the bare drift + freshness summary (pure-local,
 O(armed), fetch-less)
mrd sql <QUERY> [--fresh] [--rebuild] [--cwd PATH | --root NAME] [--json]
 **operator face** — SQL over an in-process projection
 of the corpus, served from the drawer's `sql.duckdb`
 cache when a cache root resolves, else an ephemeral
 `:memory:` build (NOT agent core; see § Operator SQL
 face below)
mrd test --corpus <SPEC> the pre-arming corpus runner over synthetic changes
mrd test --history <WS> --rule <PAGE> [--spec <PAGE>]
 the same law replayed against the workspace's own past;
 --spec names the spec page whose ```golden fence
 declares the exceptions (its `rule:` must name <PAGE>)
mrd run <PAGE> [TASK] [-- ARGS]
 run a task block declared in the page's frontmatter
 (`--env K=V`, `--dry`, `--list`, `--json`)
mrd script [--json] [--expect-armed DIGEST]
 the script entry of the run plane: caller-supplied
 inline source on stdin, run as the caller through the
 one write path (`--json` emits the trace; the human
 face is non-normative — see `run-plane.md`).
 `--expect-armed` refuses BEFORE the splice unless what
 this run armed hashes to DIGEST — the commit half of
 the arm/commit split a gating host runs. BY DESIGN
 this door carries no `--scope`/`--scope-bytes`:
 the caller's `--if-fingerprint` stays the world-grain
 entry token, and the finer grain is the run plane's
 own automatic touch-set premises; wire callers
 may still send `guards[]` on the `script` op (§5.4)
mrd new <KIND> <ID> file birth: fill the def's template, validate, birth
 the first rev through the guarded create
mrd unfold <PRESET> materialize a preset's declared scaffold
mrd reconcile <PRESET> reconcile the tree toward a preset's declared scaffold
mrd realise <PAGE> [--dry] [--json]
 the reconciliation loop: observe -> check -> apply
 (only on drift, once) -> re-check
mrd cache ls list the on-disk cache drawers
mrd cache clean [--all] reap stale / orphaned / retired drawers. "Orphaned"
 means the workspace was OBSERVED gone: a workspace path
 that could not be examined keeps its drawer and is
 listed as kept (`skipped_unexaminable` in `--json`),
 because that lookup authorizes a removal
mrd daemon run the registry daemon in the foreground
mrd --version the build identity, one line: package version + the
 tree the build read — a bare commit where that tree
 was clean, `<commit>-dirty` where tracked content
 diverged from it, `unknown` where neither could be
 read (read, never invented)
```

⛔ **`mrd --version` names the tree the build read, not your HEAD**; a bare
commit is not proof the build came from your commit. `build.rs` writes one of
three: bare commit, `-dirty` or `unknown`. None of them marks a fourth state —
a stamp never re-computed, because the binary was built from artifacts seeded
out of another tree whose git paths its freshness still watches. It reads as a
bare commit in a clean tree, read faithfully from a stale artifact of a
repository you cannot see.

`build.rs` reads `MRD_BUILD_SHA` (`env_sha()`) before the probe, so **the env
value is stamped verbatim, with no probe** — a supported input that can name
any commit. Supplying it to make a gate agree invents the answer; the supplier
owns that claim. Source: `crates/mrd/build.rs`.

⛔ **The first rung is the environment.** A supplied value passes the sha
match, the ancestor test and the watch-list grep unseen, so **an unset
`MRD_BUILD_SHA` is a precondition of every rung below.**

Then three checks:

```
env | grep MRD_BUILD_SHA                       # rung 0: the precondition. Any value voids everything below
mrd --version  vs  git rev-parse HEAD          # the SYMPTOM, in that dir, on that dir's own binary
git rev-parse --git-dir ; git rev-parse --git-common-dir   # the CAUSE: what "yours" means
grep -h rerun-if-changed target/debug/.fingerprint/mrd-*/run-build-script-*.json | sort -u
```

**The watch list.** The fourth command prints the paths the build script asked
cargo to watch. A watched path is **yours** when it sits under your own
`--git-dir` or `--git-common-dir` AND every `/worktrees/<name>/` segment in it
names your own worktree. Anything else is **foreign**.

- A common-dir test alone misses a foreign worktree of the same repository,
  which sits inside your common dir.
- The `/worktrees/<name>/` clause is the one check that fires at seed time,
  before HEAD moves and while the sha still agrees.
- Do not tighten it to "any absolute path": a linked worktree's own refs live
  in the common dir, so absolute paths there are expected. The predicate is *a
  different repository or worktree*.
- List every foreign path: the dep-info is a union, and several repositories
  can govern one tree.

⛔ **Only one file decides; never take a verdict from `output`:**

| | File | Read it for |
|---|---|---|
| **INSTRUMENT** | `target/debug/.fingerprint/mrd-*/run-build-script-*.json` | what cargo STORED and what it COMPARES — the verdict |
| **MISLEADING** | `target/debug/build/mrd-*/output` | what the script EMITTED — absolute by nature, understanding only |

Cargo relativises a path under the package root before storing it, so `output`
reads absolute where the stored form is relative. A check that globs `output`
therefore fires on every healthy donor-seeded clone. The absolute form is git's
own: `git rev-parse --git-path HEAD` is `.git/HEAD` in a main tree but
`…/.git/worktrees/<name>/HEAD` in a linked one, and `Path::join` drops the
manifest prefix for an absolute argument.

**The ancestor test.** On a disagreement, `git merge-base --is-ancestor
<baked-sha> HEAD` splits the readings: YES is ordinary staleness; **NO is not
yet the hazard**, because a cherry-picked landing puts your content under a new
sha, so a third rung decides.

**Patch equivalence, the third rung.** Run in the directory under test:

```
cd <the directory under test>                  # NOT the shared checkout you are standing in
git cherry $(git rev-parse HEAD) <baked-sha>
git show <baked-sha> | git patch-id --stable   # compare against the landing directly
```

`-` means the change landed under another sha — benign. `+` proves only that
this patch is not upstream: an amend replaces the patch where a cherry-pick
preserves it, so `+` alone cannot separate *never landed* from *superseded*.
Hence the reflog in the middle row:

| reading | verdict |
|---|---|
| `is-ancestor NO` + `-` | landed under another sha. **BENIGN** |
| `is-ancestor NO` + `+` + `git reflog` shows `commit (amend)` there | **the HANDOVER SENTENCE is stale**, not a hazard. Verify the dir and land what is in it |
| `is-ancestor NO` + `+` + no such reflog entry | the foreign-checkout **HAZARD** stands |

- **Empty output is a third answer, not a pass.** `git cherry` prints nothing
  when the baked sha is already an ancestor of the HEAD given — so the shared
  checkout prints silence where the tree under test answers `+`. Name the HEAD,
  stand in the directory under test, and treat empty as *wrong tree, re-run*.
- **Do not use tree identity.** Hunting the sha's `^{tree}` in `<base>..HEAD`
  holds only when the re-land sits on the same parent; a cherry-pick onto a
  moved base makes a different tree, the normal landing shape here. Tree
  identity is a special case of patch equivalence and must not be used alone.

⚠️ Run the first check against the binary in that directory's own `target/`,
never a PATH-resolved `mrd`. The installed engine is held behind the tree by
design, so it disagrees with HEAD in every development tree; that is the pin
working. For an installed release compare the tag
(`git rev-parse v1.0.0^{commit}`).

**Builds after `2500a4be` cannot enter this state**: the identity probe watches
a sentinel inside its own `OUT_DIR` and no git path, so nothing foreign is left
to inherit.

`mrd help` is the authoritative surface: flags, refusal legs and per-verb exit
codes live there.

**The exit triad is one law across the engine-backed verbs (read / put / pin).**
Exit 1 is the engine refusing, `bad_request` included. A §4.4 batch the engine
judges invalid — overlapping regions, a multi-line upsert value — is a
well-formed invocation the engine refused. Exit 2 is the CLI's own refusal —
an unknown flag, malformed stdin, a contradictory flag pair — before any engine
contact. A script branches on the exit alone: 2, fix the invocation; 1, read
the engine's message.

**The line is shape vs value.** The CLI refuses only shape: an unknown flag, or
stdin that is not the §4.4 batch shape (a field outside an edit object's closed
set included). Every judgment on a value inside a legal shape — a block id
outside the §2.4 charset, an `old` that matches nothing, an overlapping region
— is the engine's, at exit 1 with its structured `--json` frame. A strict CLI
decoder must not apply value laws: that turns an engine refusal into a
bad-invocation report and blanks the `--json` frame the caller branches on.

### Teaching rows — five true facts about this face an agent would not predict

Each row is lawful behaviour, not a defect; it only surprises a reader who
arrived from `wire-contract.md`.

**`<PATH>#FRAG` is a MAP FILTER, never a body read.** `mrd read
notes/plan.md#Goals/Q3` serves the subtree's toc row — address, span, rev — no
body; `--section SEL` serves bodies. A `frag` scopes the subtree, not content.
The usage line prints `<PATH>[#FRAG]` beside `[--section SEL]`; `#FRAG` on a
section returns a map at exit 0.

**`mrd put` / `mrd pin` / `mrd rm` / `mrd retire mark` are wire clients.**
Authenticated IPC to the daemon: hello + socket-law identity check. There is
no direct-publication fallback: when the daemon is down the CLI face refuses
at exit 2 and names the recovery (`mrd daemon`; shorten `XDG_CACHE_HOME` when
sun_path is the cause), never writing locally. Auto-spawn still runs. A
guardless put needs § A.1's fingerprint-or-force: `--force` or `if_node_rev`.
`--dry` is the daemon rehearsal, no in-process candidate diff. A commit rides
the daemon epoch, so `seq` is the ring's, never `0`.

**A `--json` face answers `{workspace, error}` on EVERY leg that can refuse.**
`mrd read --json` answers it on all-fail, ambiguous, duplicate-anchor,
unaddressable-host, §2.4 charset and ambiguous-domain, like `mrd put --json`.
The envelope carries the §8 error body in the v3 vocabulary — A.3's `reason`
and `candidates` included — on stdout, beside the unchanged human stderr line
and exit triad.

**Two legs where the envelope is owed:**

| # | leg | what an envelope-less `--json` stdout would serve |
|---|---|---|
| 1 | the ambiguous-domain refusal (`io_error`, two domain configs) | **EMPTY** — §8's `{cause}` and its remedy swallowed on both faces |
| 2 | the §2.4 charset violation at the unified decoder | **NOTHING** — where a structured frame carrying `code` and `recovery` is owed |

**`io_error` carries its `{cause}` onto the human face.** §8 computes
`io_error{cause}`: the ambiguous-domain refusal names both config files and
which one to delete. The cause is rendered verbatim, nothing invented.

**The human read face carries no VALUE plane.** `props` rows (A.3) are
machine-face facts. `mrd read` prints the rendered text verbatim, so
frontmatter values arrive only as rendered source; read `--json` for the
decoded value and its `prop_rev`.

### The joined selector coat — one dead delimiter, two escapes

A *selector* names one section inside a page. `--section SEL` and `mrd pin`'s
`#SELECTOR` share one human-string door, `wire::ReadSel::parse`. Its heading
arm joins on `/`, so **a heading whose raw text carries `/` is not addressable
by the joined spelling** — it misses rather than serving a different section.
Widening the coat is a C2 change, reserved by ruling (`laws.md` D-1).

- **Per delimiter, per ingress**: `#` is not a delimiter here, so
  `--section 'Top/C#D'` serves, and `PATH#FRAG` splits on the FIRST `#` only —
  `notes.md#Top/C#D` is a frag-scoped map.
- **Two escapes, both published by the toc row the caller already read:** the
  **dewey ordinal** (`--section 1.2`) and the **raw heading segments** as the
  wire's hpath array (`{"hpath":[{"h":"Guide"},{"h":"A/B"}]}`), one entry per
  heading, no joining. The machine plane addresses and pins such a heading
  end-to-end; only the joined coat cannot spell it.
- A miss **teaches both escapes in the refusal**.

### Operator SQL face — `mrd sql` (NOT agent core)

**Normative framing (`wire-contract.md` §10.3–§10.4; `README.md` standing C):**

- Agent core = parse + hash + §4 fact ops (`toc` / `cat` / `extract` / `read` /
  `splice` / …). Orientation surfaces that assume a SQL board are **not** wire
  ops, never the agent path.
- Nothing on the agent path assumes SQL/DB; no wire op, field, or error
  **names DuckDB** (§10.4).
- No `view_path` wire op, no daemon-published `view.duckdb`, no `mrd view` verb
  (§10.4): the projection is the sql face's own, built per query or served from
  the drawer's `sql.duckdb` cache.
- The face also projects **`.base` (Obsidian Bases) files** — relations `base`,
  `base_view`, `base_formula`, plus `link.exclusion_path`
  (`docs/base-projection.md`). View-lane only, no wire surface, bytes in no
  fingerprint: they ride their own `base_fold` witness, reported by the
  freshness frame as a SECOND plane (`base_plane` in `--json`, its own banner
  line in human mode).
- `mrd sql` is **operator convenience**: served from the drawer's append-only
  `sql.duckdb` cache when a cache root resolves (`--rebuild` recreates it),
  else an ephemeral `:memory:` projection of the corpus per query, folding
  post-result for an honest freshness frame. Not a peer of `mrd read` /
  `mrd put` / wire `splice`; a cold build is O(corpus) — slow on a large tree,
  by design.

### `mrd repair` — lost-pin repair

A pin has two independent planes: CLAIM (its `fp1.…` fingerprint,
verified against the live target) and RETRIEVAL (its `hash`, a git blob sha).
**A pin is LOST when both are dark**: the target no longer verifies and git no
longer holds the recorded blob, so nothing answers what the pin covered. A red
pin whose blob is still held is ordinary drift, untouched here. Only a pin
minted at an uncommitted file state can be lost (§ `mrd pin`, the
recorded-blob bullet).

- **The walk**: ONE `git log` plus ONE `cat-file --batch` for the whole run,
  never a spawn per pin or commit. Each recorded version of a lost target is
  rebuilt and put to the same `classify_pin` as the walk, `status` and `check`;
  green means those bytes are the pinned content.
- **The grain differs**: the `hash` is the whole FILE's blob, the fingerprint
  covers ONE SECTION, so a commit whose file bytes differ elsewhere can still
  carry the pinned section.
- **The forgery invariant**: repair rewrites the pin's `hash` only; `object`,
  `selector` and `fingerprint` are never touched. A genuinely drifted target
  stays RED after a successful repair (`walk` still reads content-drifted):
  repair restores the RETRIEVAL plane, never the CLAIM verdict.
- **TRUE LOSS** — no version in that path's history carries the pinned content
  — is reported, never auto-fixed: the engine invents no evidence.
- **Jurisdiction**: the ambient root only. A pin naming another root names
  another object store: skipped, count stated.
- **`--dry`** runs the walk, skips the final lock write, never a diff face.
  Progress counts ride stderr, so `--json` stdout stays machine-clean.
- **The write** uses the existing guarded `lock_write` door; the byte-landing
  door census is unchanged.
- **Exit triad:** 0 nothing lost or all repaired (or `--dry` rehearsed) / 1 at
  least one TRUE LOSS / 2 bad invocation or a tool failure.

### `mrd pin` — the attestation verb

`mrd pin` mints a real `meridian-lock` pin through the shared write
choke-point — one flock, one rename (`wire-contract.md` § A.3) — over IPC, like
`mrd put`.

- **Addressing** `PAGE TARGET#SELECTOR`: two positionals, `#` splits on its
  first occurrence. A page-level pin is REFUSED (it would redden every
  dependent).
- **The selector** (CLI host face): a **heading chain**, a **block anchor**
  (`^id`), or a **dewey ordinal** (`1.2`). The machine mint-plane address is
  **segments only** — `{"hpath":[{"h":"Guide"},{"h":"Leader's Guideline"}]}`,
  `{"anchor":"r-000042"}` — never a joined writeable form
  (`Guide/Leader's-Guideline`, `Guide>…`, sanitized slug) as the canonical
  address (`wire-contract.md` §2.1; segments-only law). A dewey ordinal may
  **resolve**, but never as the canonical write address when the lock stores
  path arrays / segment form. Receipts and armed wire facts are normative:
  never paste a display-joined string into a later `put`.
- **The recorded blob is the whole target FILE at pin time, never a
  section-grain blob**; the fingerprint covers the pinned section, and that
  grain split enables § `mrd repair`. A **committed**-state pin is gc-safe: a
  commit reaches its blob, and reachable objects survive `git gc`. Only an
  **uncommitted**-state pin can be LOST — git may prune its unreferenced blob
  past `gc.pruneExpire`. Committing the file is the only durable anchor.
- **`--vibe`** also writes the blob into git's object store
  (`git hash-object -w`), so the pin is retrievable before any commit
  references it; without it the oid is computed read-only. When git cannot
  answer, the retrieval plane carries no entry — never a fabricated sha.
- **`--dry`** rehearses, writes nothing. **`--json`** prints the whole projected
  splice response under a `pin` key; human output: a confirmation line plus the
  minted fingerprint, the anchor, the blob, and the new workspace fingerprint.
- **`--fingerprint TOKEN`**: the § A.3 pin proof — the `fp1.…` content-identity
  token a sections read served for TARGET#SELECTOR. Optional here, always
  verified when supplied: the engine recomputes the live token under the write
  flock; a wrong token refuses `pin_proof_required`, writes nothing, never
  replaces the live claim.
- **No `--actor`**: proof requiredness keys on a daemon-derived session identity
  and a CLI invocation has no session, so the bare `mrd pin` is
  local-operator-trusted and may pin proofless, as `mrd put` bypasses the host's
  authz.
- **Exit triad:** 0 pinned (or `--dry` rehearsed) / 1 refused
  (`pin_proof_required`, `pin_target_missing`, `write_conflict`, an armed gate
  refusal — the engine's verbatim message) / 2 bad invocation (including a down
  daemon).

A wire client's pin through the daemon carries its own proof: the
`fingerprint` that client's sections read served for that selector rides the
pin (wire-contract § A.3). "You cannot attest content that was never in your
context"; the engine keeps no record of the read.

### `mrd rules` — the effective law, shown

`mrd rules [PATH] [--workspace | --user] [--json]` shows the effective rule
set: what governs at PATH. Registration by tag plus id-based override makes
that set a **computed quantity**.

```text
rules at sessions/s1
 workspace /var/…/ws
 user-scope «local-path» (anchor «local-path»)
 armed-set none
 task.review-notify armed=-
 winner sessions/s1/notify.md rev=018b942787febb31 layer=workspace depth=2 kinds=hook
 shadowed notify.md rev=e0dc53f2203c5969 layer=workspace depth=0 kinds=hook
 collide.here REFUSED collision at layer=workspace depth=2 — this id resolves to nothing
 tied sessions/s1/a.md rev=936e2eddf8bdf331 layer=workspace depth=2 kinds=hook
 tied sessions/s1/b.md rev=cefb207bdf220b88 layer=workspace depth=2 kinds=hook
```

Chain lines spell the resolution scope as `layer=… depth=…`, the two fields
`--json` ships, never `scope=` — at this surface `scope` is the armed artifact's
ARM-ROOT column, a directory. A pasted `workspace:0` refuses at parse with a
teaching.

- **The chain is never collapsed.** Per id: the winning page, then every page it
  shadows, in ladder order (`git config --show-origin`). A collided id renders
  `REFUSED` naming every tied page; it resolves to nothing.
- **Scope ladder**, outermost to innermost: user space (rules under the
  `MERIDIAN.md` anchor's scope) → workspace root → folder/session tree.
  Resolution is **narrowed** to PATH's own chain, so a same-id page on a sibling
  chain is no conflict. `--workspace` prints the workspace-root layer alone,
  `--user` the user layer alone.
- **The user layer is bounded by an anchor, deliberately.** Candidates are
  `<user-scope>/rules/**.md`; the user scope is the directory holding the
  resolved `MERIDIAN.md`. No anchor ⇒ an empty user layer that says so, never a
  `$HOME` walk.
- **The `armed-set` header states what is, never the engine's storage.** An
  unarmed workspace reads `armed-set none`, the whole line: it never adds the
  path where an armed set would live. A present or corrupt artifact names its
  path.
- **`armed=` is a separate column**, read from the attested armed set
  (`meridian/armed-rules.md`), joined on `(id, arm root)` narrowed to PATH —
  never on id alone, never recomputed. `-` registered but unarmed · `<mode>`
  armed on the page that governs · `<mode>@<page>` armed on a DIFFERENT page
  (the freeze: arming pins resolution, later discovery never moves it) ·
  `(drifted)`/`(missing)` when the pinned page no longer stands · `UNREADABLE`
  for a corrupt artifact, never "nothing armed".
- **`drift=` is a THIRD column, on EVERY ledger row** — the pinned rev against
  the rev that page reads now, `off` rows included: `-` the join held ·
  `drifted` · `missing` · `off-drifted` · `off-missing`. It rides every row the
  ledger carries (the cells above, the `armed rows counted above …` sections
  below) and no other: `armed=-` gets no drift cell. `--json` adds `pinned_rev`
  and `live_rev` beside the word.
- **`off-drifted` IS NOT A REDNESS AND TRIPS NO GATE.** A *redness* is a
  finding that moves the exit to 1. An `off` row is not armed: no
  `⚠ page-drift` is owed and `redness` stays null on it. So "no
  page-drift marker" means "no ARMED row drifted", never "no row drifted". The
  word is printed; the exit code is not. `mrd rules` exits 0 on an `off-drifted`
  row and 1 on a `drifted` one.
- **`armed=-` names one thing only: nothing governs HERE.** An armed row whose
  arm root does not contain PATH prints instead beneath `armed rows counted
  above whose arm root does NOT contain this path:`, naming its mode, arm root
  and pinned page. The law: **containment is a fact and reddens nothing**,
  **redness is a fault wherever it lives** and is named and counted — so this
  verb and `mrd status` never disagree about one artifact.
- **One resolver, two consumers.** The verb calls `policy`'s own
  `RuleIndex::discover` → `narrowed_to` → `resolve` and
  `ArmedArtifact::verify_at` — the ONE composition of select-then-verify, not
  `select_at` + `verify` at the call site. The elsewhere population is
  `ArmedArtifact::verify_elsewhere_at`, one composed call in `policy`. The CLI
  holds no override law; a test asserts that structurally.
- **Read-only:** arms nothing, mints no receipt, spends no cap. A gate drives
  every view and asserts the workspace's merkle root *and* its whole file tree
  are unchanged afterwards.
- **Exit triad:** 0 clean / 1 a finding (a collision, a refused rule page, a red
  armed row, an unreadable armed set) / 2 bad invocation, or a PATH outside the
  workspace or not on disk. `mrd rules <nearest-existing-ancestor>` answers the
  hypothetical for a folder that does not exist yet.

**Refusal scoping.** Refusals are narrowed exactly like rules. A scoped query
reddens only for **on-chain** refusals — the exact subtree the refused page
would have governed. Every corpus-wide walk (discovery sweep, ARM act, cutover
sweep) reports ALL refusals it encounters, always. `narrowed_to` filters
refusals through the same predicate it filters rules through; no mount
arithmetic lives in the CLI, which prints what `policy` handed it.

- **`RegisterError` carries its own mount scope**, path-derived: `mount_dir`
  needs no frontmatter, so "cannot be answered" applies to the page's
  registration TAG, never to its mount. A refused page answers that mount
  question through the same `mount_dir_of`, `rules/`-parent lift included.
  **One mount law, not two.**
- **Fail-CLOSED on the refusal itself is unchanged:** a page whose frontmatter
  does not parse never registers, from any path. Scoping decides who HEARS about
  a broken rule page, never whether it is enforced.
- **`mrd rules` on meridian-rs itself exits 0**, measured on the real repo by a
  named e2e gate
  (`meridian_rs_itself_is_clean_while_a_refusal_still_reddens_its_own_subtree`)
  with its other half: a refusal reddens its own subtree, and the corpus-wide
  walk names it. The testsuite's malformed schema fixture (`meridian-md refusal
  fixtures`) therefore sits outside the **hash domain** through a declared
  ignore in `meridian/domain.md`, named under `cannot be answered` at exit 0: on
  disk and tested by the schema pack, but not attested content.

**Exclusion consistency.** A *voice* here is a note on stderr naming what a
walk left out. The declined voices `not offered to registration` and `cannot
be answered` enumerate by the projection's own walk law. One shared predicate
(`fs::domain::dot_segment`) spells §12.1 rule 2 for the hash-domain walk, the
link fallback index and this scan alike. A dot-prefixed segment is therefore
never entered, and `mrd rules` can never caveat a path the record projection
refuses to serve. The custom-ignore class stays voiced and exit-neutral.
Findings, and exit 1 with them, are attributable only to served-corpus
conditions: collisions, on-chain refusals, red armed rows, unreadable
in-domain files, an unreadable armed set. The USER rung has no projection to
be consistent with; its dot-declined pages stay named.

**The excluded-population voices.** Each human note on stderr uses one
spelling: full count, `EXCLUDED_SHOWN` sample, remainder clause
(`capped_sample`), pointer at the complete list on that verb's own `--json`. The
cap bounds the prose; it never re-scopes what was excluded.

| voice | population | complete list |
|---|---|---|
| **The domain-excluded note** — `mrd sql`, `mrd walk --down`, `mrd check`, pageless `mrd repair` | walk law (`fs::declined_markdown`, the one `fs::domain::dot_segment` predicate spelling §12.1 rule 2): custom-ignore class only, never a dot-prefixed path | `excluded` of bare `mrd links --json` — the complete §12.1 outside-domain enumeration (§4.6) |
| **The `links` face itself** — bare `mrd links` | its own answer's `excluded` key, projected through that same predicate: a dot-segment member leaves the count and the sample | the same `excluded` key, byte-identical |
| **The `mrd retire` human render** | CAP class: `retire` certifies absence, so the population is lawfully COMPLETE, dot paths included, and stays so on `files_excluded` | `files_excluded`; the count stays the full population |
| **The `mrd rules` undecidable line** — `cannot be answered` | CAP class, genuinely UNBOUNDED: `register` refuses on `FrontmatterUnparsed` **before** any tag is read (`policy/src/registration.rs`), so every excluded file with a malformed frontmatter block lands here | `not_offered.undecidable`; `not_offered` carries all three declined populations complete |

Bare `mrd links` never re-derives by a second disk walk (the door/face split);
uncapped voicing of its wire key would put dot paths in the voice and unbounded
prose on stderr. The two `not offered to registration` feeds are
registrar-narrowed (`rule_pages_among`) and bounded by construction, so they are
not capped.

**Registration candidates under a dot directory.** A rules-tagged page with an
`id:` under a dot directory registers as NOTHING. So does every page of a
workspace whose own MERIDIAN.md sits on a dot path: that workspace resolves to
the enclosing root and out of domain. Unvoiced, non-registration reads as
working law — `(no rules in effect)` at exit 0, `mrd arm` refusing with a bare
`resolves to nothing`. So `mrd rules` voices these dot-declined REGISTRATION
CANDIDATES: registrar-narrowed (`rule_pages_among`), enumerated by the
addressable walk through the one dot predicate (`fs::dot_declined_markdown`),
one bounded line, complete list on `not_offered.workspace_dot`. Dot-tree pages
whose frontmatter does not parse join `cannot be answered` instead. The
population is candidates, never "all dot markdown" (the wrong-population
guard), the line is exit-neutral, and the other enumerating faces' voices are
untouched. `mrd arm <ID>` is the other half: an `Unresolved` refusal whose id
a domain-excluded candidate carries names the file and the exclusion reason (a
dot-prefixed path segment / a `meridian/domain.md` ignore rule).

### `mrd check` — does the world still match the pins

`mrd check` asks one question: does the world still match the pins? Its colour
words: **green** is clean, **red** is a claim that fails, **grey** is one the
engine could not check. Both layer-0 planes the core reads are memory-free —
the claim plane (pinned content drift) and the pin plane (pin verdicts and the
anchoring state of every pinned blob) — and both are observed against the
CURRENT tree. `check` answers at-rest truth, writing nothing and minting no
receipt. `status = freshness, check = validity`: this verb answers "what
lies?".

**Write history is not assessed: the engine keeps no memory.** History is pinned
to git at lock, and anything between locks is not history. So chain continuity
and last-receipt-vs-live are **not checked here at all: not grey, NOT
CHECKED**. Green means the world still matches the pins, never how it got
there. Every face carries the `write_history: not-assessed` disclosure, naming
that narrowing and pointing at git. The disclosure states the narrowed claim,
never the engine's mechanism.

**The interval this verb spans.** The `worktree` interval — the bytes on disk
— is always assessed. `--staged` adds the interval a commit records: git
commits the INDEX, `domain_snapshot` reads the worktree, so when the index
carries more, the staged bytes are assessed in a second pass. The exit is
worst-of across both intervals: the worse answer decides. Every refusal names
its interval. The interval line states the case: `coincides` (the index adds
nothing, so the one pass IS a commit's interval), `diverges` (N paths differ,
assessed separately), `no-repository`, or — asked but unreadable —
`grey(cannot-assess)`, which fails closed on exit 1.

**The exit triad stays closed** (0 green / 1 finding / 2 bad invocation): grey
rides leg 1 — a grey pin or an unaskable object store refuses
`grey(cannot-assess)` — because unknown is not clean. The reason is a separate
fact and lives in the output.

#### The findings enumeration is COMPLETE, never worst-of

The refusal on stderr is a numbered list — `check refuses (<interval>) — N
findings:` — read as the fix-list. **It carries every finding the refusing
interval holds, red AND grey, in worst-of ORDER but never worst-of SELECTION.**
Grey findings sort after red ones and keep their own reason word; the count is
the count of everything listed. A greyless list would read as clean once the
reds are fixed, while the same grey alone WOULD be the one-finding list.

#### Two independent axes: WHICH BYTES, and WHICH QUESTION

`mrd check [--core] [--staged] [--commit-gate [--require-pins]] [--json]`.

**`--staged` picks the interval.** Forge a pinned section, `git add` it, restore
the governed bytes to the worktree, and an unscoped check answers green over
bytes no commit would record.

**`--commit-gate` picks the question, and implies `--staged`.** It narrows the
exit to ONE interval — the one a commit records — and gates on the pin plane
alone. A pin is a claim about the bytes committed, not about any history.
The passing word is **`pins-hold`**: it names the plane that answered, never
write history. The gate re-reads the index's pin plane at every commit. **This
is why the emitted fence body runs `mrd check --commit-gate` and not `mrd check
--staged`.**

**`--require-pins` tightens the gate, opt-in.** A corpus that declares no pin
PASSES the gate by default: over zero pins the question is vacuously true.
`--require-pins` makes no coverage refuse in the exit code, under its own word
(`no-pin-coverage`, never grey's). A grey pin or an unaskable object store fails
CLOSED either way.

| | gates the exit | reads |
|---|---|---|
| unscoped | worst-of across every interval assessed | claim drift + the pin plane, per interval |
| `--commit-gate` | ONE interval — the one a commit records | the pin plane alone over those bytes |

**The pin population is the pins the workspace DECLARES, not the hash domain.**
`mrd pin` admits a holder page the hash domain excludes — a dot-segment path, a
`meridian/domain.md` ignore rule — and mints the pin at exit 0. The pin plane
therefore reads its rows from **every markdown page under the root**, not only
from the hashed corpus: exclusion removes a page from HASHING, not from
claim-making. Otherwise `--commit-gate` would assert *every pin in the interval
holds* over a population it silently narrowed, and answer GREEN over a drifted
pin.

⚠️ **The narrowing is a property of EXCLUSION, not of dot segments** — the
custom ignore list reproduces it identically.

**What does NOT change is the target's color.** The hash domain still gates
HASHING and never addressing: a pin whose TARGET the domain excludes stays
`grey(outside-hash-domain)`, reported and never gated
(`wire-contract.md` §12.1, verdict-plane clause). Holder and target are
independent axes: widening pin SOURCES never widens the corpus that resolves
pin TARGETS; the excluded holder's bytes never enter the merkle root.

**No permanence, because no memory.** Nothing looks backward: no standing break
is printed, no verdict is carried forward from an older run, and a gated pass
claims nothing about a record — the word is `pins-hold`.

**The declared blind spot.** Pin rows held by domain-excluded pages are read at
their WORKTREE bytes for both intervals. So if a holder is both
domain-excluded AND staged-modified, a pin row added or removed in the index
alone goes unseen.

**The `--json` face.** Each interval emits `{workspace, red, write_history,
core: {drifted_claims}, pins}`; the top-level `red` is worst-of across
intervals, and the `interval` block carries the `state`, `spans_the_commit`, the
`diverged_paths`, and the nested staged answer. The `commit_gate` key is present
ONLY when the scoped question was asked — `{gated_interval, permits, verdict,
detail, gated_planes: ["pins"], write_history, pin_coverage, require_pins}` — an
absent field reads as "not checked", where a `null` would assert a read that
never happened. A top-level `fence` block reports the checkout's commit-fence
coverage — the git hooks of § `mrd skill hook` — on every run.

**The `fence:` line** is a proposition about the local checkout's configuration,
not the corpus, and never touches the exit code: `$GIT_DIR/hooks` is never a
tracked path, so fence coverage is per-checkout and opt-in — a fresh clone being
unfenced is a supported state.

### The composed status line

`mrd status` renders five orthogonal axes on one line, worst-of WITHIN each axis
and never across them:

```
pin green · lock none · anchor at-tip (anchor as-known) · armed off · vibe-debt 0 blobs (0 bytes)
```

| axis | answers | values |
|---|---|---|
| `pin` | armed-set evidence drift: each armed row's live PAGE rev against the `rev` its armed-rules row attested (PAGE rev uniformly, `armed-plane.md` §4) | `green` · `red content-drifted` |
| `lock` | every `meridian-lock` pin's FINGERPRINT verdict, rolled up | `none` · `<color> [N pins]` · `unreadable (<why>)` |
| `anchor` | how current the working copy is against origin's tip, and how that tip is known | `at-tip` / `behind`, qualified by `anchor as-known` |
| `armed` | whether armed law refuses this change | `off` · `warn` · `block` · `armed` (hook mode) |
| `vibe-debt` | how much of the retrieval plane is held by this machine alone | `N blobs (M bytes)` · `unknown (<why>)` |

**`lock` is orthogonal to `pin` and neither subsumes the other.** The `lock`
roll-up is worst-of **red > grey > green** — grey above green, so no
unverifiable pin hides inside a green fleet.

**A green `lock` axis does NOT imply the tree is current. Currency lives on the
`anchor` axis.** A pin verdict is per-pin and content-addressed; currency is a
repository-level fact, never folded into the pin tone.

**`vibe-debt` is a meter, never a gate.** It counts the lock-referenced blobs git
HAS but no commit reaches — the `pending-anchor` population, the window residual
G1 leaves open (`gc.pruneExpire`, git default two weeks). Blobs absent from the
object database (`never-anchored`, pruned or freshly cloned) are NOT counted.
Debt never enters the findings verdict and never refuses a write; zero renders
(`0 blobs (0 bytes)`).

`--json` always-emits both axes: with no pins, `composed.lock.pins` is `0` and
`composed.lock.color` is `null`, never an absent field.

#### The exit reads the ARMED PLANE ONLY — every other axis is a reading (R12)

`status`'s exit-1 leg is narrow:

| axis | moves the exit | |
|---|---|---|
| the ARMED plane — an armed rule drifted, or the armed-rules artifact faulted | **yes** ⇒ exit 1 | the `pin` axis and the `rules:` line (`armed-rules` names the artifact, not the report's voice) |
| `lock` | **no** — `red …`, `grey …` and `unreadable (<why>)` all exit 0 | a READING, not a gate |
| `anchor` | no | freshness, and `status` cannot fetch |
| `vibe-debt` | no | a meter, never a gate (above) |

**This is R12, and it is design, not debt**, asserted by name in
`crates/mrd/tests/u14_check_pin_plane.rs`,
`crates/mrd/tests/u13_per_root_anchoring.rs` (five exit-0 arms across four
`#[test]` fns; four of them name `R12`) and `crates/mrd/tests/status_e2e.rs`
(debt is not a finding).

**So `mrd status || alarm` does NOT fire on attestation drift. The fail-closed
door is `mrd check`**, whose exit triad answers red-or-grey and whose grey leg
fails closed. Read the lock verdict from the composed line, not the exit: red,
grey and lock-refused produce none.

Output is JSON under `--json`, a human table otherwise; exit codes are 0 clean /
1 findings (the armed plane, per the table) / 2 tool failure. The workspace it
ran over is printed with the tier that answered — `status <root> (git-root)` —
and `--json` carries the same word as `source`.

### The resolution ladder — three rungs, and every answer names itself

How one invocation decides which directory is its workspace, and how every
answer names the rung that gave it.

`workspace::resolve` has **three** rungs and no marker tier. **A
`.meridian.toml` or `.meridian.yaml` sitting in a tree is inert** — no code path
in this engine reads either file: it anchors nothing, grants no `[run.caps]`,
changes no answer below.

| Tier | What it answers | How |
|---|---|---|
| `env-override` | the environment named the root | `MERIDIAN_WORKSPACE`, **only when the caller named no path** |
| `git-root` | where the version-control boundary is | the **nearest** ancestor `.git` (directory or worktree pointer file) |
| `cwd-default` | nothing — a convenience default | the canonical cwd |

**An explicitly NAMED path outranks the override.** `workspace::resolve` takes a
`workspace::Base`: `Named(p)` when the operator typed the path on this
invocation (`mrd unregister PATH`, `mrd resolve PATH`, `mrd init PATH`, `mrd
status --cwd PATH`, `mrd sql --cwd PATH`), else `Cwd(p)`. Rung 1 answers only
for `Cwd`, so a named path resolves by `.git` walk or cwd-default; otherwise
`MERIDIAN_WORKSPACE=victim mrd unregister target` would remove VICTIM. The gate
lives in the shared resolver, not per verb.

**Every resolution states which tier answered and which root it named**,
enforced by the type: `Answer` has no public path field, `root` returns `None`
on `cwd-default`, and reaching the defaulted path takes `root_or_cwd`. `mrd
resolve` (`source:` plus the path), `mrd status` (header line) and `mrd init`
(for the directory it declared) print both; so does a refusal —
`page not found: <ref> (workspace <root>, source: env-override)`, same form for
`git-root`. A `cwd-default` miss stays bare: `root` is `None` there.

An answered rung opens the hashed drawer directly. A `cwd-default` tree adopts
a running daemon's registered ancestor if one answers, else degrades to an
ephemeral, per-invocation store that writes nothing; it is never silently
registered. The adopted daemon may be a different build. Adoption exchanges
only a registration record; content then rides a v3 connection, where the
socket law (`docs/wire-contract.md` §A.3) compares `hello.identity.build` at
connect and refuses across builds. So the CLI prints the **refinement**, not
the tier: `daemon-adopted` or `ephemeral`, each naming the root beside it and
implying `cwd-default`. The four words `env-override` / `git-root` /
`daemon-adopted` / `ephemeral` strictly refine the three tiers.

**A root's `MERIDIAN.md` self-declaration is NOT a rung.** `crates/config` reads
it (mount binding, and `crates/run`'s `run.caps.*` / `run.timeout_secs`); the
ladder never does, because existence-only detection cannot tell a
`meridian-root` declaration from a `meridian-config`. So `mrd init` below a git
root declares that directory a root **and still resolves to the git root**. It
says so, and names the two ways to change the answer: `MERIDIAN_WORKSPACE`, or
addressing the root by name through the mount table. Registering the tree with
the daemon also carves a sub-root.

### `mrd skill hook` — the commit fence, as a DOCUMENT

`mrd skill hook` prints one markdown document to stdout and does nothing else:
no file written, no git directory read, no workspace resolved. **The markdown IS
the contract** — what to place, where, when to refuse to place it, how to verify
— and the agent reading it does the placing. Exits 0 (document written) or 2
(bad invocation). There is no `--json` face and **no install verb** (`mrd
hook` is not a subcommand): the rules are prose. Only what cannot be prose is
code — reading disk bytes to say what generation stands there, as `mrd check`'s
`fence:` line.

What the document rules:

- **The door set is three:** `pre-commit`, `pre-merge-commit` and
  `pre-applypatch` are every hook git dispatches for a commit it builds from a
  prepared index, so **one body serves them all**. A set of one let `git merge`
  and `git am` land commits past it.
- **Placed per `$GIT_COMMON_DIR`, not per worktree.** N linked worktrees are N
  meridian workspaces sharing ONE `hooks/` directory. So the fence is written
  once, bakes in no path, and reads the committing worktree from git's working
  directory at run time. Per `--git-dir`, git runs one of N files; per worktree
  top-level overwrites one file N times. The `chmod +x` is part of placing it —
  git silently skips a hook it cannot execute.
- **The body runs `mrd check --commit-gate`** and rejects on its exit, holding
  **zero markdown semantics**: no selector parsed, no rev read, no color word
  spelled, refusal's legal home engine-side.
  `crates/mrd/tests/skill_hook_emit.rs` asserts that over the emitted bytes.
- **Three commit-creating paths stay open:** `git cherry-pick`, `git revert` and
  `git rebase` replay dispatch no veto-capable hook that can read the index. The
  guarantee is *no out-of-band write reaches history through `commit`, `merge`,
  or `am`* — **not** *no drift reaches history*; there, read-time `mrd check` is
  the only guarantee.
- **Coverage is per checkout and opt-in, permanently.** `$GIT_DIR/hooks` is
  never a tracked path, so no clone carries the fence; a global
  `init.templateDir` is refused because it would fence every unrelated
  repository the operator clones or inits. An unfenced fresh clone is a
  supported state, and `mrd check` says so unasked.
- **Escapes at commit time**, both named in every refusal the fence prints:
  `MRD_HOOK_FORCE=1 git commit …` (the fence's `--force`) and git's own
  `git commit --no-verify`.

`MRD_HOOK_FORCE` is parsed, never merely tested for non-emptiness:

| `MRD_HOOK_FORCE` (trimmed, any case) | Verdict |
|---|---|
| `1` `true` `yes` `on` | **bypass**, printed on stderr with the value, stating that nothing was checked |
| `0` `false` `no` `off`, empty, whitespace, unset | **fence normally**, silently |
| anything else | **refuse the commit**, exit 1, naming the value it could not parse |

The fence declares its generation on line 2 as `# mrd-hook-fence <n>`; the
engine parses it and compares with its own, a three-valued relation. The
emitter's design tests hold that line and `crates/mrd/src/hook.rs`'s
`FENCE_VERSION` to each other, so a body change without a bump fails in CI.

| `mrd check` fence state | Meaning | Remedy |
|---|---|---|
| `installed` | every door carries this engine's fence | — |
| `installed-partial` | some doors unfenced | place the body at the rest |
| `installed-superseded` | the fence is older than this engine emits | re-place from `mrd skill hook` |
| `installed-ahead` | **the fence is NEWER than the engine answering** | put the current engine first on PATH — **do NOT re-place**, which would downgrade the fence |
| `installed-unversioned` | marker present, generation undeclarable | refuse rather than guess |
| `foreign-hook` | a door carries a file this engine did not write | move or remove it |

The document tells its reader to refuse the last three states above, and four
placements:

- a submodule — its hooks live at `<super>/.git/modules/<name>/hooks`, which
  this engine does not compute;
- a set `core.hooksPath` — git runs hooks from there, and if that path already
  carries a `pre-commit`, writing there would write into another checkout's
  hook directory;
- a workspace root that is not the worktree top-level;
- a root that is not a git repository at all — a supported workspace state with
  nowhere to put a hook, not a fault.

At commit time the fence **fails closed**. `mrd` absent from `PATH` refuses,
naming both escapes and how to delete the file. An `mrd` predating
`--commit-gate` exits 2; the body then refuses, naming the skew and the
commands that decide it — never a fallback to a check that reads the wrong
bytes.

**Verify with `mrd check`.** Its `fence:` line carries the set's word, the count
of doors carrying the marker, and a teaching; `fence doors:` names each door
with its own word. Under `--json` the same reading is the `fence` object with
`doors[]`, `fenced_doors`, `total_doors`, `engine_version` and
`gates_the_exit: false`. **That line never moves the exit code**: fence coverage
is a property of a local checkout, not of the corpus.

Nothing in this engine writes into a git directory any more, so a root that is
merely looked at comes away byte-identical, including the roots that refuse.

## Tests

`cargo test --workspace` — full suite green; CI gates every merge on it.
Export `CARGO_PROFILE_TEST_DEBUG=0` first — a full-debug `target/` in this
workspace costs ~26G and the flag is the repo's own CI lever. The `testsuite`
crate carries two frozen packs: the ground-truth pack (rung-1 parse truth:
every node reproduced byte-for-byte) and the read/put parity pack
(`data/parity/`, captured from the live host face). Three tests replay them:
`u0_read_parity` (addressing facts), `u4a1_render_parity` (rendered text) and
`u4a2_composed_read` (the composed op through the live serve loop, refusal
texts included). The CLI foundation's end-to-end gates live in
`crates/mrd/tests/e2e.rs`.

### Harness caveat (standing C)

Green tests on **tiny synthetic workspaces** do not prove every surface on a
**real corpus**. In particular:

- the operator SQL face (`mrd sql`, a DuckDB projection) cold-builds the
 whole corpus when no drawer cache serves it — slow on production trees, by
 design (§10.4);
- address and write paths must be verified against segment form and armed
 receipts, not against display-joined strings that only appear in harness
 fixtures.

Do not claim “proven end-to-end” without that caveat.

## Performance

`perfsuite` carries a claims registry (`crates/perfsuite/claims.toml`) whose
verdicts are computed, not asserted, and written to
`crates/perfsuite/results/` (`latest.json`, `RESULTS.md`). Tally over 23
claims: **2 PASS, 7 MEASURED, 14 UNTESTED** — PASS covers cold ingest and codec
bulk cost, the untested are perf rungs awaiting their first baseline. Refresh:

```sh
cargo bench -p perfsuite
```

### The timing mode — `MRD_TIMING`

`perfsuite` measures a tree you built; `MRD_TIMING` measures the binary you
already shipped. It turns on a **timing-only** log — one line per completed
phase and nothing else — on a release binary, with no profiler, no debug build
and no rebuild. stdout, `--json` bodies and exit codes are byte-identical with
it on and off; it writes to stderr or a file, never stdout.

The value names the sink, **trimmed and matched case-insensitively** (`OFF`,
`" off "`, `off` are one answer):

| `MRD_TIMING` | Sink |
|---|---|
| unset, empty or all whitespace, `0`, `off`, `false`, `no` | **off** — no clock is read and nothing is written |
| `1`, `on`, `true`, `yes` | stderr |
| any other value | that path (trimmed), opened append, created if absent |

- **Refused, loudly, degrading to stderr**: a sink whose extension is `.md` or
  `.base` (hash-domain members), and a path that will not open.
- A **relative** path resolves against the process's working directory at the
  first phase. The daemon, auto-spawned with the client's cwd
  (`crates/mrd/src/daemon.rs`), keeps the fd for its lifetime: give it an
  absolute path.
- Several processes may share one file; the lines interleave, so read them by
  `who=`, never in order.
- The value is read **once per process**, at the first phase; a later change
  does nothing, and each refusal is said once.

#### Two line shapes, and the space that tells them apart

A **measurement** opens `mrd-timing` + a SPACE: exactly four `key=value` fields
in fixed order, `\n`-terminated, written in one `write_all`, so concurrent
threads interleave whole lines, never halves:

```text
mrd-timing cmd=run who=p41273.t1 phase=snapshot.read us=402118
```

- `cmd=` — the verb this process was entered with (`mrd run` ⇒ `run`, the
  daemon ⇒ `daemon`); it names the process, never one request. A verb with
  whitespace or a control character is refused and the label stays `mrd`. A
  diagnostic's text is sanitised the same way.
- `who=` — the emitter inside that process: `p<pid>.t<n>`, both halves digits,
  `n` a per-process thread ordinal minted on that thread's first line. **It is
  not a request id** — a reused thread keeps its ordinal — and on the daemon it
  is not a connection either.
  § The daemon's own lane is the only place that answers what
  `t` will actually be on a busy server.
- `phase=` — a dot marks a part of the phase it prefixes (`snapshot.read`
  inside `snapshot`), but containment is wider: `dispatch` contains `snapshot`,
  `eval` and `apply`, and `total` everything. **Do not sum the lines.**
  Containment table: `run-plane.md` § Timing phases.
- `us=` — elapsed wall clock in microseconds, integer, the same noun as the
  wire frame's `meta.duration_us`. No other unit and no float.

A **diagnostic** about the mode opens `mrd-timing` + a COLON:

```text
mrd-timing: cannot open `/nope/t.log` (No such file or directory) — writing to stderr instead.
```

So `grep '^mrd-timing '` — with the space — is exactly the measurements.

#### A line means the phase COMPLETED

A span abandoned on an error path — the `?` on a missing page, an early refusal
— reports nothing. `mrd run missing.md` reports `workspace.resolve`, the
refusal, then `total`, and no `page.load` line. `total` is the one phase that
reports even on a refusal: it measures the process, and the process completed
either way.

**A failure that is itself worth counting says so in its NAME.**
Load-sensitive call sites stop the span under a distinct name:
`currency.floor.<cause>.refused`, `door.floor.<cause>.refused`,
`door.refused.<cause>` (`run-plane.md` § Timing phases). Refusal and success
names are never summed.

Lines print in **completion order**: contained phases first, `total` last.

**Off costs nothing.** A phase whose sink is off holds no `Instant`: no clock
read, no allocation, no formatting, no write — one atomic load of the resolved
sink through `#[inline]` entry points (the release profile has no LTO). The
claim is code shape plus stdout byte-identity in
`crates/mrd/tests/timing_mode.rs`; measured wall-clock numbers are `perf.yml`'s
lane, never the PR lane.

**Two lanes, and only one of them is the caller's.** `mrd run` runs in the
calling process: its phases land on the caller's sink. `mrd script` and wire
clients hand the work to the resident daemon, so those phases land on the
daemon's sink. Nothing rides back on the wire: Law 2 puts host-facing types in
`wire` and `wire-contract.md`, not in an instrument. A client sees only the
frame's `meta.duration_us`, the server-side total. So set `MRD_TIMING` **in
the daemon's environment**: an auto-spawn inherits the client's environment, a
resident daemon kept its own and emits nothing — restart it, or let the idle
horizon do it.

#### The daemon's own lane — `<socket-stem>.log`

**The auto-spawn gives the daemon a voice**, which a detached daemon otherwise
lacks: its stderr is `<socket-stem>.log`, opened append beside the socket and
pidfile keyed off the same stem —
`$XDG_RUNTIME_DIR/mrd/<12hex>.log` on Linux, else
`$HOME/.cache/mrd-run/<12hex>.log`. It carries the daemon's startup and
shutdown lines, any refusal or panic it dies with, the registry's operational
diagnostics and, while the mode is on, every `mrd-timing:` diagnostic, every
measurement that degraded to stderr, and the `MRD_TIMING=1` form.

- **The lane is unconditional; the MEASUREMENTS are the gate.** Null stdio
  would leave a detached daemon deaf to the refusals above and to
  `MRD_TIMING=1`, and mute about everything else. If the lane were gated, a
  daemon that died at startup — panic, unresolvable layout, unbindable socket,
  poisoned state file — would present only as "5 seconds slower". The client
  polls 5 s, degrades to the ephemeral engine, never refuses.
- **What a run that did not ask for the mode pays**: one file beside the
  socket, a few lines per daemon lifetime, not per operation.
- **The degrade quotes it.** When a daemon this run spawned never binds, the
  client reads back the region that child appended and names the cause on its
  own stderr, with the lane's path.
- A voice that will not open is said on the **spawning client's** stderr; the
  daemon starts mute and says so.
- Nothing rotates the file. It is yours to read and to remove.
- `mrd daemon` in the foreground keeps the terminal's stderr. A daemon another
  supervisor started with null stderr is still deaf; this covers the
  auto-spawn.

##### What `t` names here, and what it does not

Read a daemon sink as **folds, not requests**. Every phase the daemon emits
*while serving* comes from the corpus fold in `Registry::warm_or_build`. The
cold gate kicks that fold onto a background `drawer-rebuild` thread,
single-flight per workspace (`crates/registry/src/registry.rs` § `cold_gate`),
never the request thread. The only non-fold line is `phase=total`, on the main
thread at exit — so `t` is *which fold*:

| What you run | What the sink shows |
|---|---|
| Concurrent ops on DIFFERENT cold workspaces | one `who=` per workspace — they separate |
| Concurrent ops on the SAME cold workspace | **one** `who=`. One fold: the op that KICKED it waits (bounded), a later op is refused `corpus_warming` in milliseconds. Two ops, one emitter, nothing lost |
| Any op on a WARM workspace | **no daemon line at all** — nothing was folded, so nothing is measured |
| The prewarm sweep | its own `who=`, no connection behind it |

A missing `t2` is not a request that never ran: it is work already done, or done
once for two askers. For one request's server-side total, use
`meta.duration_us` (§ Two line shapes).

Which phases `mrd run` emits: `run-plane.md` § Timing phases. The instrument is
`crates/timing` (`laws.md` § Crate charters).

Which phases `mrd links` emits (same instrument and grammar): `snapshot.*`
and `corpus.build` come from `fs`, so they fire on the ephemeral path **and**
for every other caller of those functions (`run-plane.md` § Timing phases —
read `cmd=` first). A warm daemon-served `links` emits neither on the
**client** sink: that fold happened in the daemon, on its own sink
(§ Two lanes). A phase that did not run emits no line.

| Phase | Inside | Emitted in | Covers |
|---|---|---|---|
| `total` | — | `mrd::run` | the whole process |
| `daemon.dial` | `total` | `mrd::engine::answer_links` | hello+links on the resident daemon, or the degrade decision; includes the `ensure_daemon` auto-spawn poll up to `SPAWN_READY_TIMEOUT` (5 s) |
| `snapshot` / `snapshot.*` | `total` OR `links.read` | `fs::domain_snapshot_with_leaves` | hash-domain walk + digest + fold. Once per mounted root (`build_docs_at`) |
| `corpus.build` | `total` OR `links.read` | `fs::build_corpus` | UTF-8 + `syntax::parse` of every member. Once per mounted root: the workspace build in `total`, each mount build in `links.read`. Also `sql` / `check` / `walk` / `repair` / `retire` / daemon under their own `cmd=` |
| `links.read` | `total` | `mrd::engine::in_process_links` | everything from the mount narrow to the v3 re-key |
| `json.render` | `total` | `mrd::engine::run_command` | `serde_json::to_string_pretty` of the envelope |
| `json.write` | `total` | same | `println!("{text}")` — stdout is a `LineWriter`, so the trailing newline flushes the envelope inside this span |

## Known gaps

- Perf rungs are largely UNTESTED pending baselines (tally above).
- `policy` verdicts ride every splice response as `[]` until rule packs load;
 packs are the host's concern.
- **`require_fingerprint` has no CLI spelling.** `links.require_fingerprint`
 is a served wire cap (`wire-contract.md` §10.2; release §2.1), but
 `mrd links --require-fingerprint` answers `unknown flag`, exit 2. The §10.2
 posture — refuse with `stale_view`, never answer in an unnamed tense — is
 socket-only.
- **CLI-committed writes advance the fingerprint, never `changes_seq`.** An
 `mrd put` commit moves the fingerprint the daemon serves immediately and mints
 no Delta, so `changes_seq` is unchanged (`wire-contract.md` §18 row 12).
 Polling it as a change monotone misses every CLI-lane write — diff by
 fingerprint (§4.7).

Accepted residuals (attestation surfaces) — documented, not prevented. Full statements in
`wire-contract.md` § Named residuals.

- **G1** a `--vibe` blob is reachable from no ref, so its durability horizon is
 `gc.pruneExpire` (git default two weeks). The vibe-debt gauge measures that
 window; only committing the file anchors it.
- **G2** the write flock serializes COOPERATING writers only; an out-of-band
 write is detected by the drift color, never prevented. The git pre-commit
 fence is stage-3.
- **G3** a pin writes two inodes, not all-or-nothing; a failure between them
 leaves a rev-neutral, slug-derived anchor that a re-pin reuses and heals —
 never silent corruption.
- **G4** refs are intra-root only. Cross-root addressing, the mount table and
 the MERIDIAN.md config engine are stage-3; stage 2 keeps the seam open without
 entrenching "there is exactly one root".
- **G5** anchor promotion into an unowned target churns that file's CAS token;
 accepted for the core loop because it is rev-neutral. The fence and authz
 tightening are stage-3.

Also stage-3, and NOT shipped: full-document re-attest — no read-mint ledger to
unify with the persisted `^receipt` projection. Pin proof rides the request
(wire-contract § A.3); the engine records no reads.
