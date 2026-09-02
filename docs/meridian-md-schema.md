---
type: result
id: schema
status: spec
created: 2026-07-26
tags: [type/result, domain/meridian-rs, topic/meridian-rs, topic/config]
owns: ["MERIDIAN.md config parse"]
---

# MERIDIAN.md in-file schema — the config plane's parse law

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

Status: normative for the `MERIDIAN.md` parse.

**Standing law (restated — no external decision files):** one entry point
(`MERIDIAN_CONFIG` → `$HOME/MERIDIAN.md`); markdown over TOML; config-is-content;
fail-loud strictest parse; mount table is a three-way map (name ↔ vault ↔ path);
the root declares, `MERIDIAN.md` binds; grey refuses on exit 1 with a distinct
reason word; `$HOME/MERIDIAN.md` cannot be attested (mount-as-claim is the
mitigation). Cross-root grammar: `address-grammar.md`.

This spec is **prose a parser can be tested against**. Every key states its type, whether it is
required, and the refusal on violation. Every rule has at least one fixture in
`crates/testsuite/data/meridian-md/`, and every fixture states its required outcome in
`crates/testsuite/data/meridian-md/cases.json`. A rule with no fixture is a defect in this spec.

## 0. What this spec owns — and what it does not

| Owned here (the **in-file schema**) | Owned elsewhere |
|---|---|
| Where the file is found, and the four resolution states | — |
| Which bytes of the page are a **machine surface** and which are prose | — |
| Frontmatter keys the engine reads: name, type, required, refusal | — |
| The `meridian-mount` block grammar: fields, types, order, refusals | **Mount table SEMANTICS — implementation**: canonicalization at bind, `deny_reason`, equal-or-nested refusal, declared-vs-bound checking, the grey classes |
| The `meridian-tool` block grammar: its engine-read half and its opaque half | Tool semantics — deliberately unowned |
| That a mount entry **may pin the root it declares**, and what a well-formed pin token is | What the pin's target file is, and how the claim is checked — **implementation** |
| The self-hosting rev: which bytes, which hash law, what it is spelled | Drift reporting and the verb that shows it — **implementation** |
| The canonical root-**name** charset (the floor) | The `root:` **address** grammar, prefix-vs-literal-path ambiguity, `resolve_linkpath` — see `address-grammar.md` |
| The refusal shape: reason words, and where a refusal points | The engine implementation and its crate placement (D4) — **implementation** |

**Not specified, deliberately.** Project-local walk-up discovery (nearest-ancestor `MERIDIAN.md`) is
**deferred, not rejected** — it adds a resolution ambiguity v1 does not need. Building it here would
be over-completion. The resolution chain in §2 is exactly two rungs and has no third.

Two boundaries are flagged rather than assumed, in §12.

## 1. The precedent this EXTENDS — `meridian/armed-rules.md`

The attested armed-rules artifact is markdown-as-config **shipping today**: engine-managed, inside the
hash domain, drift-tracked by a pinned rev. The self-hosting model therefore
already ships once; this schema extends a proven pattern rather than introducing one.

> The artifact is a table keyed by `(rule id, arm root)`; this document calls it **the INDEX** for
> short. Every row of the table below is verified against `crates/policy/src/armed.rs`, because a
> precedent cited at a wrong `file:line` teaches nothing.

### 1.1 The existing mechanism, stated

| Aspect | `meridian/armed-rules.md`, as shipped | file:line |
|---|---|---|
| In-file shape | fixed H1 title + free prose preamble + a markdown TABLE; **no frontmatter, no fenced blocks** | `crates/policy/src/armed.rs` (`ARTIFACT_TITLE` / `ARTIFACT_HEADER`, `ArmedArtifact::render`) |
| Row grammar | five `|`-separated columns — id · page · pinned rev · arm root (scope) · mode. Backtick-quoted cells, and `|`/backtick/control characters are **unrepresentable** rather than escaped | `crates/policy/src/armed.rs` (`ArmedRow::render`, `validate_workspace_path`) |
| One reader, on purpose | `parse_artifact` is the **only** reader and is fail-closed. Two readers of one attestation (a tolerant one beside a strict one) would be two answers to one question, so there is exactly the strict one | `crates/policy/src/armed.rs` (`parse_artifact`) |
| Strictness scope | the **title** and the **column header** must match exactly; every data row must parse; the **preamble is not byte-checked** | `crates/policy/src/armed.rs` (`parse_artifact`) |
| Pinned rev | `rev` = `page_rev(page bytes)` = `blake3(bytes)[:16]`, 16 lowercase hex — the same rev law the world model mints (contract §1), now applied to the RULE PAGE rather than to one file inside a folder | `crates/policy/src/registration.rs` (`page_rev`) |
| Drift | at the door, `page_rev(live page) != row.rev` → `ArmedFault::Red(Redness::Drifted)` → the write refuses (check) or the fault is reported (hook) | `crates/policy/src/armed.rs` (`verify_rows`), `crates/policy/src/armed_law.rs` |
| Malformed posture | `ArtifactCorrupt { detail: String }` → `ArmedFault::Corrupt`, **fail closed** — a corrupt artifact must never silently read as "nothing armed" | `crates/policy/src/armed.rs`, `crates/policy/src/armed_law.rs` |
| Emptied posture | a well-formed artifact attesting ZERO rows on a once-armed workspace is `ArmedFault::Disarmed`, not a disarm. An attestation of absence is a row spelled `off`; zero rows is the ABSENCE of attestation, which is what deleting every row leaves behind | `crates/policy/src/armed_law.rs` |
| Absent posture | pivots on a **separate marker** (`meridian/attested`): never-armed → the file is not even read; once-armed → absent is a fault | `crates/policy/src/armed_law.rs` (`resolve_armed_law`) |
| Writer | the **engine** is the sole writer; a hand edit is a `BindingBreak` teaching refusal | `crates/policy/src/binding.rs` (`classify_door_law`) |

### 1.2 Where this schema FOLLOWS it

1. **Strictness is scoped to a machine surface; prose is prose.** The INDEX pins its title exactly and
 parses every row strictly, while leaving the preamble free. §3 states the same law for
 `MERIDIAN.md`, with an explicit marker for where the machine surface begins.
2. **Malformed fails closed and names the damage.** `ArtifactCorrupt`'s detail
 (*"row is not a closed table row: …"*, `crates/policy/src/armed.rs:1039`, `parse_row`)
 is the shape of §8's refusal. Nothing half-loads.
3. **The pinned rev is the node_rev family, not a fingerprint.** `page_rev` is `blake3(bytes)[:16]`.
 §7 reuses that law verbatim for the config's own rev — **no new hash law is minted here.**
4. **A drifted pin refuses; it never silently re-arms.** `ArmedFault::Red(Redness::Drifted)`'s
 door verdict (§1.1) is the model for the mount-as-claim posture in §7.3.
5. **The reserved-path constant is mirrored in two crates with a cross-crate drift test**
 (`crates/policy/src/armed.rs:26`, `crates/fs/src/domain.rs:50`;
 test at `crates/wire-serve/tests/reserved_paths.rs:10`). §2.4 requires the same for `MERIDIAN.md`'s
 filename and env-var name.

### 1.3 Where this schema deliberately DIFFERS — and why

| # | INDEX does | `MERIDIAN.md` does | Why the difference is required |
|---|---|---|---|
| D-a | **Engine is sole writer**; a hand edit is a refused `BindingBreak` | **Human is sole author**; the engine never writes it | The INDEX is a generated attestation artifact. `MERIDIAN.md` is *"a new user's first contact … one readable file"*. A binding-break guard on a file the engine does not write would refuse every legitimate edit |
| D-b | **Middot-separated checklist rows** | **`key: value` lines inside a fenced block** | Row grammar is cheap to *generate* and hostile to *hand-write* — a missing ` · ` is invisible in an editor. `key: value` is the grammar the repo already hand-authors (`crates/lock`, lock/def frontmatter, `meridian/domain.md` domain frontmatter) |
| D-c | **No frontmatter** | **Required frontmatter** (`type`, `version`) | The INDEX is found at one reserved path, so its identity is positional. `MERIDIAN.md` can be aimed anywhere by `MERIDIAN_CONFIG`, so it must be able to say *what it is* and *which schema it speaks* — otherwise a mis-set env var half-loads an unrelated page |
| D-d | Malformed row names the row **text** (`{line:?}`), never its **number** (`crates/policy/src/armed.rs:1039`, `parse_row`) | Every refusal carries a **1-based file line** | The requirement is a refusal *"naming what is broken **and where**"*. The INDEX precedent cannot satisfy it. The in-repo model that can is `crates/lock` — `LockError::Malformed { line, reason: &'static str }` (`crates/lock/src/lib.rs:390`) — so §8 extends **lock's** error shape, not the INDEX's |
| D-e | Absent-vs-malformed pivots on a **separate marker file** | Absent and malformed are decided by **the file alone** | The once-armed marker exists because disarming must not be silent. `MERIDIAN.md` has no such asymmetry: there is nothing to disarm, and every machine legitimately starts with no file (D6) |
| D-f | A lossy round-trip would do no harm — the engine regenerates the file (`parse_row` does read all five columns, `crates/policy/src/armed.rs`) | Every declared field is **read**; an unread field is refused as unknown | A lossy round-trip is tolerable when the engine regenerates the file. Here the human's bytes are the only source, so a silently-ignored field is a silently-ignored intent |

## 2. Resolution — the bootstrap chain and the FOUR states

### 2.1 The chain (exactly two rungs)

1. **`MERIDIAN_CONFIG`**, when set to a non-empty value — the override. Its value is the path, used verbatim (no `~` expansion, no glob, no search).
2. **`$HOME/MERIDIAN.md`** — the default.

`MERIDIAN_CONFIG` set to an **empty or whitespace-only** value states no path and is treated as
**unset**; the chain falls to rung 2. Rationale: rung 1 exists to honour a stated intent, and an empty
string states none — this is the same nil-vs-empty distinction §2.2 state D draws for the mount table,
applied on the env axis so the two cannot diverge.

`$HOME` unset or empty makes rung 2 unresolvable and **refuses** (§8, `home-unresolvable`). It is not
the absent case: the absent case means *the default path was resolved and nothing is there*.

### 2.2 The four states — all four are NAMED, and two pairs are deliberately kept apart

| # | State | Required behaviour | Reason word | Why it is named |
|---|---|---|---|---|
| **A** | **Absent** — the chain resolved a path and no file exists there (rung 2 only) | **Current single-root behaviour, unchanged.** Not an error, not a warning | *(no refusal)* | Every machine starts here. Failing would brick the CLI on first run. Matches the house posture across all three shipped config formats: absent → silent default |
| **B** | **Present but malformed** | **Fail loud.** A teaching refusal naming what is broken and **where**. **NO partial mount table. NO default-root fallback** | one of §8's classes | Fail-loud is about a *broken* config. A partial load would make the system's own definition half-true |
| **C** | **`MERIDIAN_CONFIG` set to a path that is not a readable regular file** — absent, a directory, or unreadable | **Fail loud** | `config-path-unusable` | **This is NOT state A.** The operator stated an intent that cannot be honoured; silently falling back to `~/MERIDIAN.md` would mask it. D6 names two states; this is the third |
| **D** | **Parses clean and declares ZERO mounts** | **Treated as state A** — current single-root behaviour. **Not an error** | *(no refusal)* | An empty mount table is a legitimate statement. Named because "empty" and "absent" reaching different code paths is how a nil-vs-empty bug is born |

**State D is a behavioural identity, not a similarity.** A zero-mount config and an absent config MUST
produce the same mount table (the empty one) and the same resolution behaviour. The config's own rev
(§7) still exists in state D and does not in state A — that is the only permitted difference, and it
is an observation, never a branch.

**The green-path control.** State B's refusals are only meaningful beside the acceptances:
states A and D leave behaviour unchanged, and a well-formed multi-mount config
(`corpus/multi-root.md`) loads every entry it declares. A build that refused every config would satisfy
state B alone. `cases.json` carries acceptance cases and refusal cases in one manifest for exactly
this reason.

### 2.3 What "readable regular file" means

State C's test is: the path resolves, `metadata` succeeds, and the target is a regular file (or a
symlink to one) the process can read. A directory, a dangling symlink, a special file, and a
permission error are all `config-path-unusable`, each naming the path and the underlying reason.
Distinguishing them further is not this schema's business — the operator's next action is identical.

### 2.4 Two reserved names, and the drift test they need

`MERIDIAN.md` (the filename) and `MERIDIAN_CONFIG` (the env var) are reserved names. Wherever they are
spelled in more than one crate, the shipped precedent applies: a cross-crate test asserts the constants
agree, as `the_armed_rules_artifact_has_one_spelling` does for `meridian/armed-rules.md`
(`crates/wire-serve/tests/reserved_paths.rs`). Implementation owns placing the constants; this spec fixes the spellings.

## 3. The document shape — the machine surface, and the prose around it

`MERIDIAN.md` is an ordinary markdown page. Two kinds of bytes live in it:

- **The machine surface** — the frontmatter block, plus every fenced code block whose info-string
 names an engine block-language (§3.1). The engine parses these strictly.
- **Prose** — everything else. The engine **never parses it and never refuses because of it.**

**This is the law that makes markdown-as-config safe**, and it is the INDEX's own law generalized,
shipped today in its successor: the artifact pins its title and its rows and leaves its preamble free
(`crates/policy/src/armed.rs:996`, `parse_artifact`).
Without this scoping, adding a sentence of documentation to your own config could break your system —
which would defeat the entire "one readable file that explains itself" purpose.

**Corollary, and it is load-bearing:** a fenced block that is *not* an engine block-language — a
` ```yaml ` example, a ` ```text ` diagram, an indented snippet — is prose, **even if its contents look
exactly like a mount block.** Fixture `corpus/prose-decoys.md` is the anti-vacuity case: it carries
three convincing decoys beside one real mount and must yield exactly one mount.

### 3.1 The engine block-language namespace — reused, not invented

The repo reserves the whole `meridian-*` fence-info prefix as the engine's block-language namespace,
and the predicate is a **prefix test, deliberately not an enumerated list**
(`crates/lock/src/lib.rs:55-68`):

```rust
pub const NAMESPACE_PREFIX: &str = "meridian-";
pub fn is_meridian_lang(lang: &str) -> bool {
    lang.split_whitespace()
        .next()
        .is_some_and(|tok| tok.starts_with(NAMESPACE_PREFIX))
}
```

So this schema adds two languages inside a namespace that already admits them:

| Info-string | Carries | Cardinality |
|---|---|---|
| ` ```meridian-mount ` | exactly **one** mount entry | zero or more per file |
| ` ```meridian-tool ` | exactly **one** tool declaration | zero or more per file |

The **first whitespace token** of the info string decides the language, matching every existing reader
(`crates/lock/src/lib.rs:65`, `crates/policy/src/pack.rs:376`, `crates/run/src/fence.rs:131`). A
trailing string (` ```meridian-mount the wiki `) is tolerated and ignored.

**One entry per block, not one table block.** Rejected alternative and its reasons in §11.

### 3.2 The consequence of the namespace that MUST be named

One shipped behaviour governs how the namespace renders, per-language rather than per-namespace:

1. **The render face elides ENGINE-EMITTED languages only.** `ToonRenderer::with_meridian_elision`
 drops the blocks `lock::is_engine_emitted` names (`crates/render/src/lib.rs:194-209`): a
 `meridian-lock` is machine-written and elides, while a `meridian-mount` or `meridian-tool` is
 user-authored and **renders**. The raw `cat` face rides everything verbatim either way.

No other reader skips engine blocks, so nothing mis-reads a mount block. What
makes the namespace the correct home is §3.1 itself: it is the one predicate the strict parse scopes
on, and per-language elision means a human-authored block in it is never hidden from its author.

**The verification requirement.** A reader running `mrd read ~/MERIDIAN.md`
sees the prose **and** the mount blocks, so "no mount blocks visible, the parse failed" is not a
conclusion the rendered face can produce. What the rendered face never shows is the parse
**verdict**: a mount block's bytes render whether or not the config parser accepted them. **The
user-reachable verb that publishes the parsed mount table must therefore not be the rendered read
face.** Implementation owns which verb it is; this spec's requirement is only that acceptance
evidence not be measured on a surface that shows the block's bytes but never its acceptance.

## 4. Frontmatter keys

The frontmatter is the first block of the file: bytes `0..3` are `---\n`, terminated by a closing
`---` line. This is the shipped frontmatter shape (`crates/testsuite/data/gt/ground-truth/README.md:21`:
*"only when bytes 0..3 are `---\n` (BOM-prefixed `---` is NOT frontmatter)"*).

| Key | Type | Required | Refusal on violation |
|---|---|---|---|
| `type` | string, exactly `meridian-config` | **yes** | absent → `missing-required-key` naming `type`. Present with any other value → `wrong-type-value`, naming the value found and the value required |
| `version` | integer | **yes** | absent → `missing-required-key` naming `version`. Non-integer → `bad-value`. An integer this build does not implement → `unsupported-version`, naming the value found |

**v1 is `version: 1`.** A future format bumps it; a reader refuses a version it does not implement and
**never guesses a future format** — `LockError::UnsupportedVersion`'s own law
(`crates/lock/src/lib.rs:382`). The same discipline appears on the hash-domain declaration
page `meridian/domain.md` (`version` + ignore list): a domain-rule change bumps the fingerprint
prefix so a `b3:` cursor can never silently match a `b3a:` world (`wire-contract.md` §12.3).

**Unknown frontmatter keys are permitted and ignored.** This is the shipped posture for this plane's
own parse — *"Unknown keys are permitted and ignored"* (`crates/config/src/lib.rs:672`) — and it is
what lets a user carry `title:`, `updated:`, or Obsidian properties on their own entry page.

**And that tolerance is safe here only because of a deliberate design rule: v1 defines NO optional
frontmatter key the engine reads.** Both keys are required, so a typo of either fails loud as
`missing-required-key` rather than being silently dropped. The hazard of unknown-key tolerance is a
misspelled *optional* key silently doing nothing; v1 closes it by having none. **A future version that
adds an optional engine-read key must state how it closes this hazard**, or it reopens it.

Malformed frontmatter itself:

| Condition | Refusal |
|---|---|
| The file does not open with `---\n` | `no-frontmatter` |
| The frontmatter block is not terminated by a closing `---` | `no-frontmatter` (naming that the fence never closed) |
| The frontmatter block opens and closes but carries no keys | `missing-required-key` naming `type` — see below |
| The frontmatter is not parseable YAML | `frontmatter-unparseable`, carrying the parser's own message |

**An empty *closed* block is a missing key, not a missing block.** `---\n---` opens with `---\n` and
closes its fence, so neither `no-frontmatter` condition holds; what it does is declare no `type:`,
which is the key table's case. The distinction is not cosmetic: the markdown parser mints no
frontmatter node for an empty metadata block, so a door that reads only the parse tree refuses it as
`no-frontmatter` and tells the author their file *"does not open with a closed `---` frontmatter
block"* — a false statement about bytes the author is looking at, on the one door whose whole job is
to teach. `crates/config/src/lib.rs` (`closed_empty_frontmatter`) recognises the shape before the
`no-frontmatter` refusal is minted.

## 5. The `meridian-mount` block grammar

One block declares one mount entry. The grammar is **line-oriented `key: value`, one field per line,
in canonical order** — modelled directly on `lock::parse` (`crates/lock/src/lib.rs:543`), which is
the repo's one strict hand-parsed block grammar with per-line refusals.

```meridian-mount
name: field-notes
path: «local-path»
vault: field-notes
pin: fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152
```

### 5.1 Fields

Canonical order is the table's order. Each line is `key`, `:`, one space, then the value; the value is
the rest of the line with trailing whitespace trimmed.

| # | Field | Type | Required | Refusal on violation |
|---|---|---|---|---|
| 1 | `name` | canonical root name (§5.2) | **yes** | absent → `missing-required-field`. Charset violation, empty, or leading/trailing `-` → `bad-value`, naming the offending character and the legal charset |
| 2 | `path` | non-empty string, a filesystem path | **yes** | absent → `missing-required-field`. Empty or whitespace-only → `bad-value` |
| 3 | `primary` | literal `true` (§5.1a) | no | present and not `true` → `bad-value`, naming the one legal value — absence is the only "not primary" spelling |
| 4 | `vault` | Obsidian vault name, non-empty string | no | present and empty → `bad-value`. Presence IS vault-ness: a block carrying `vault:` names its Obsidian vault; a block without one is not a vault, and no second field restates that fact |
| 5 | `pin` | fingerprint CID-token (§5.3) | no | present and not a well-formed token → `bad-value` |
| 6 | `alias` | canonical root name (§5.2) — the second lookup spelling (§5.1b) | no | present and empty → `bad-value`. Charset violation → `bad-value`, naming the offending character. Equal to any mount's `name`, or to another mount's `alias` → `alias-shadows-name`, refusing the whole table |

> **There is no `kind` field.** Vault-ness is `vault:` presence alone, and
> `primary: true` is legal on any mount (the primary root is where a host
> daemon writes, which does not require an Obsidian vault registration) —
> nothing on any serve path branches on a mount's kind. A `kind:` line
> refuses through the ordinary `unknown-field` door, with the door's own
> remedy: remove the line.

Structural refusals over the block as a whole:

| Condition | Refusal |
|---|---|
| A line whose key is not in the table | `unknown-field`, naming the key and the legal set |
| A key appearing twice | `duplicate-field`, naming the key and both lines |
| A key appearing before a key that precedes it in canonical order | `field-out-of-order`, naming the canonical order |
| A body line that is not `key: value` (including a bare key with no `: `) | `malformed-line` |
| The fence never closes | `unterminated-block` |
| The block body is empty | `missing-required-field` naming `name` |
| Two blocks in the file declaring the same `name` | `duplicate-mount-name`, naming both blocks' lines |
| Two blocks in the file carrying `primary: true` | `duplicate-primary-designation`, naming both blocks' lines — the designation is a role exactly one mount may hold, and the parser never picks between two (§5.1a) |
| An `alias` equal to any block's `name`, or to another block's `alias` | `alias-shadows-name`, naming both lines — lookup is name-first-then-alias, so a shadowed alias is a spelling that can never reach the mount declaring it (§5.1b) |

**Blank lines and comment lines are refused** as `malformed-line`. Prose about a mount belongs beside
the block, not inside it — that is what §3's scoping law buys, and it keeps the block grammar with one
spelling per fact. (`lock::parse` refuses the same way: *"unrecognized line (canonical order: …)"*.)

**`duplicate-mount-name` is in scope and `path` collision is NOT.** Name uniqueness is a pure in-file
property, decidable from the bytes. Two mounts resolving to the same *path* is decidable only after
canonicalization (symlinks, trailing slashes, `..`), and **Implementation owns the mount-path law** — canonicalize
at bind, inherit `workspace::deny_reason`, refuse equal-or-nested mounts (canonicalize-at-bind). One owner per fact:
this schema does not also test paths lexically, because two owners disagreeing about "same path" is a
worse failure than one owner deciding late.

### 5.1a `primary:` — the declared-primary designation (v1-additive)

An optional `primary: true` line designates its mount as the **primary root** — a binding ROLE
consumed by hosts (the one tree their single-root consumers anchor: change feed, watch loop,
journal placement — the rule set lives with the host). The
engine's own duty is mechanism only: parse the designation, refuse its illegal shapes (the §5.1
rows above), and report it verbatim on the `mounts` wire row (`wire-contract.md` §A.5) and both
config faces. **The engine never acts on the designation** — no engine behavior branches on it.

Grammar consequences, each a §5.1 row: the value is the literal `true` and nothing else, because
absence is the only "not primary" spelling — admitting `primary: false` would mint a second
spelling for one fact. Two designations refuse the whole table
(`duplicate-primary-designation`, the `duplicate-mount-name` class): the designation is DECLARED,
never derived, so the parser never picks between two claimants — and no consumer may fall back to
`mounts[0]`, the only vault, or any other derivation when it is absent.

This is a v1-additive field of the shape §12 boundary 2 anticipates ("the field would be optional and
new"): mount blocks are closed-schema, so `primry: true` refuses as `unknown-field` at parse — the
silent-typo hazard §4 names is closed by construction.

### 5.1b `alias:` — the second lookup spelling (v1-additive)

An optional `alias:` line gives its mount a **second name callers may spell**. It exists for one
problem: a skill, a doc or a daemon wants to hard-code ONE constant — `sessions:` — but every
machine is free to call that tree whatever it likes, and the engine bakes in no root names (the
no-baked-names law, `laws.md`). One optional field maps the constant, per machine, in one line.

```meridian-mount
name: field-notes-sessions
path: «local-path»
vault: field-notes-sessions
alias: sessions
```

> **The lookup order is `name` first, then `alias`.** A root whose `name` already IS the constant
> needs **no alias line** — a name is its own alias, and that is the whole rule; there is no
> "default", no fallback, and no special case. The order is not a tie-break between two candidates:
> `alias-shadows-name` means a table where both could answer cannot load.

**`primary:` is NOT consulted.** The designation stays exactly what §5.1a says it
is — a role parsed, reported, and never acted on. A table with no mount named or aliased `sessions`
falls to the implicit default mount when that binds (§5.1c); with the default unscaffolded it
refuses `sessions:` as an unbound root, and the refusal teaches the lines that would fix it:

```text
declare `alias: sessions` on the mount that holds that tree
```

The cost is one line per machine, written once; the gain is that `primary` means nothing it did not
mean before, and that a refusal never sends a reader to declare a mount for a tree they already have.

**An alias is a LOOKUP spelling and never a STORED one.** Receipts, pins, `mint {…}` paths, `sub`
rows and every canonical `root:path` a door echoes carry the mount's `name`. `mrd resolve sessions:x`
answers `root: field-notes-sessions (alias sessions)` and `ref: field-notes-sessions:x` — the alias
appears where it explains the resolution and nowhere that anything is written down. Address law and
the resolution order live in `address-grammar.md` §4.6a.

**Uniqueness is table-level, and it refuses the whole file.** An alias equal to any mount's `name` —
including the name declared LATER in the file, and including its own mount's — or to another mount's
alias is `alias-shadows-name`, carrying §8.3's no-partial-load clause like every other table-level
refusal. The parser never picks between two claimants; a shadowed alias is either unreachable (the
name wins) or ambiguous (two mounts claim it), and neither is a state a mount table may be in.

This is a second v1-additive field in the shape §12 boundary 2 anticipates, and it inherits the
closed-schema guarantee with it: `alais: sessions` refuses as `unknown-field` at parse, so the
silent-typo hazard §4 names is closed by construction here too.

### 5.1c The implicit default `sessions` mount (v1-additive)

When no mount is **named or aliased `sessions`**, the bound table gains one implicit mount:

```text
name: sessions
path: $HOME/.local/share/ucc/sessions
```

The motive completes §5.1b's constant: a consumer hard-codes ONE
spelling, `sessions:`; §5.1b maps it per machine; this section answers the machine that has
mapped nothing — a fresh host needs a sessions tree before anyone has authored a config. The
no-baked-names law (`laws.md`) draws this exact boundary itself: *"a directory a user is expected
to author into is a value read from their markdown, **defaulted in code at most**"* — the constant
is a fallback, the user's declaration is the answer, the `preset::DEFAULT_ROOT_RECORD` shape.

The rules, each load-bearing:

- **Declared wins, always.** Any mount named or aliased `sessions` — whatever its path —
  suppresses the implicit mount entirely. So does any declared mount whose path equals, contains,
  or is contained by the default path (the INV-2/INV-4 checks run with the declared table already
  bound, and the implicit candidate is always the second occurrence).
- **It appears only when it binds.** The implicit candidate passes through the same per-entry
  checks a declared block does — canonicalize, the deny ceiling, uniqueness and nesting, the
  root's own declaration naming `sessions` (§4). Anything short of `bound` suppresses it
  SILENTLY: no grey row, no refusal, no changed exit code. A grey default row on every
  unscaffolded machine would put `mrd config` at exit 1 forever — the §2.2 state-A "failing would
  brick first run" reasoning, applied here.
- **Nothing else is defaulted.** The implicit mount carries no `primary:` (the designation stays
  declared-only — §5.1a; `primary` is not consulted), no `vault:`, no
  `alias:`, no `pin:`.
- **Scaffolding is explicit.** The engine never creates the directory or its declaration —
  effects live in verbs. One line, once per machine:
  `mkdir -p ~/.local/share/ucc/sessions && mrd init ~/.local/share/ucc/sessions --name sessions`.
  The unbound-`sessions:` refusal teaches it (`address-grammar.md` §4.6a).
- **A refusing declared table still refuses whole.** The default enters only a table that loads
  clean — state A included: an absent config plus a scaffolded default yields a one-row table.
- **Faces.** Both config faces mark the row (`mrd config` prints `(implicit default)`, `--json`
  carries `"implicit": true`); the `mounts` wire row is unchanged in shape — an implicit row
  rides as a real bound root (`wire-contract.md` §A.5), because it is one; provenance is a
  config-face fact.

### 5.2 The canonical root-name charset

```
name ::= lower ( lower | "-" )* lower | lower
lower ::= [a-z0-9]
```

A name is one or more characters from `[a-z0-9-]`, must not start or end with `-`, and must not be
empty. Maximum length 64 bytes.

**The charset is derived, not chosen.** A root name appears inside stored, shared content: the
agent-plane address `[root:]path[#selector]`, lock `ref:` and `objects:` keys, and the
`obsidian://` URI's vault parameter. The legal charset is therefore **the complement of the address
grammar's operator set** — `:` `#` `@` `/` `.` `%` and whitespace are excluded because each already
carries meaning in an address, so **no legal name can ever collide with an address operator.** Case is
folded out because names travel through URIs and case-insensitive filesystems. `_` is excluded to match
the charset guard that refuses underscore ids at every mint position
(`crates/testsuite/data/charset-guard/discrimination.json`), and the result is exactly the lowercase-
kebab convention this repo already uses for its own fixture families.

**This is a FLOOR for the address grammar, not a ceiling.** `address-grammar.md` owns the address
grammar and may narrow what a `root:` prefix accepts; it must not widen it past this charset, because a name outside this charset cannot be
*bound* and so could never resolve.

### 5.3 `pin:` — mount-as-claim (canonicalize-at-bind, load-bearing)

Mounts can be claims: a mount entry may pin the root it declares (e.g., the fingerprint of that
root's entry page). Canonicalize-at-bind makes this load-bearing rather than a nicety —
`~/MERIDIAN.md` cannot itself be attested (§9), so a mount's pin is **the sole mechanism by which the
mount table's own integrity is checkable.**

The schema's whole job here is to make the claim **expressible**, with a well-formedness rule the
parser can test:

> `pin:` carries a fingerprint CID-token: four `.`-separated non-empty fields,
> `version.codec.hashfn.digest`. It is well-formed iff `model::fingerprint::parse_fingerprint`
> returns `Some` (`crates/model/src/fingerprint.rs:137`).

**Parse is codec-agnostic on purpose** — the shipped rule is *"Parse is codec-agnostic, so tokens
minted by newer codecs/hash-fns still parse; whether this build can verify one is
`verify_content`'s question"* (`crates/model/src/fingerprint.rs:68-70`). This schema
therefore constrains the **token shape only**. It does not constrain the codec, which matters because a
plain-folder root's pin grain is the file and a vault root's is a parsed span
(§12, boundary 2) — those are different codecs, and pinning one here would forbid the other.

**What the pin's target is, and how the claim is checked, is implementation's** (§12, boundary 2). This spec
requires only that a build which cannot verify a pin says so; it must never treat an unverifiable pin
as verified — *outside sight never renders as verified*, and under grey-exit-1 a grey refuses on
exit 1 with its own reason word.

## 6. The `meridian-tool` block grammar

`MERIDIAN.md` also carries *"the declarations for tooling built on top — agent-facing efficiency
layers and imperative user-facing tools alike."* **Nothing owns tool semantics yet, and the design
input that would shape them (tag-based mounting) is open.** Designing
a tool system here would be over-completion. So this schema specifies the **grammar and the
posture**, and no semantics.

```meridian-tool
name: llm-wiki
kind: skill
config:
 entry: LLM_WIKI.md
 vault: field-notes
```

| # | Field | Type | Required | Refusal on violation |
|---|---|---|---|---|
| 1 | `name` | same charset as §5.2 | **yes** | absent → `missing-required-field`; charset violation → `bad-value` |
| 2 | `kind` | non-empty token, `[a-z0-9-]` | **yes** | absent → `missing-required-field`; empty or charset violation → `bad-value` |
| 3 | `config:` | marker line introducing an opaque payload | no | see below |

Structural rules are §5.1's, with two additions:

- `config:` is a **bare marker line** (no value). Every following line until the closing fence is the
 **payload**, and must be indented by at least one space. A non-indented line after `config:` →
 `malformed-line`. The payload's last line ends the block.
- Two blocks declaring the same `name` → `duplicate-tool-name`.

### 6.1 The payload is engine-OPAQUE, and this is a stated rule

**The engine validates that the payload is present and indented; it never interprets a byte of it.**
The payload belongs to the tool the `kind` names.

**Why this does not contradict the strictest-parse law.** Fail-loud is about a *broken* config
(bad frontmatter, a malformed mount block). A tool declaration
for a tool this machine has not installed is **not broken** — it is a statement addressed to someone
else. Refusing it would mean a config becomes invalid by *removing* a tool, which is the opposite of
fail-loud's intent. The shipped analogue is the wire's tolerant-code law: *"Clients treat unrecognized
codes as `recovery`-dispatched"* (`crates/wire/src/lib.rs:1350-1353`), and `laws.md` § Additivity.

**Rejected alternative, named so it is not restored:** *"define a closed set of tool kinds in v1 and
refuse the rest."* v1 would then define **zero** kinds (nothing owns any), so every tool declaration
would refuse — a grammar in which nothing legal can be written. Rejected.

**What is NOT deferred:** the engine's half — `name`, `kind`, uniqueness, and the block's structural
integrity — is parsed strictly, so a malformed *declaration* still fails loud. The opacity is of the
payload's meaning, never of the block's shape.

## 6a. The `^config` value block — user data, address-reached (v1-additive)

**The rule:** `mrd config get` finds the `^config` block in the `MERIDIAN.md` file; the block is a
starlark block whose `config` function returns the config; and the config can be anything — it is
not limited.

`MERIDIAN.md` carries, beside the mount table, whatever machine-local values this machine's tools need.
The surface is **one block, addressed by the block id `^config`**, whose fence language is `starlark`
and whose `config()` function returns the value:

The block, written out (fence lines shown as literal text so this example is inert here):

    ```starlark
    def config():
        return {
            "repos_root": {
                "work-wiki": "/path/to/work/repos",
                "field-notes": "/path/to/home/repos",
            },
        }
    ```
    ^config

The `^config` line sits on its own line directly below the closing fence — the Obsidian own-line form,
which the model's host widening (`anchor_host_span`) attaches to the fence itself, so the id keys the
CODE BLOCK and not an empty paragraph.

| Fact | Law |
|---|---|
| Address | the block id `^config` — `Ref::anchor("config")`, the mint-plane lookup (`crates/model/src/lib.rs`, `resolve`) |
| Fence language | `starlark`, classified by `run::fence::classify` — the same first-token rule §3.1 states |
| Entry | a zero-argument `config()`; its return value IS the config |
| Return type | **anything the sandbox can serialize** — mapping, list, string, number, bool, `None`. The engine declares no schema, no key whitelist, no required key |
| Reader | `mrd config get [KEY]` — bare prints the whole returned value, `KEY` a dot-path to one member (§6a.3) |
| Evaluation | the effect kernel's sealed evaluator with the standard globals ONLY — no effect constructors, `load` disabled, `EvalLimits::default()` bounds (`effects::eval_value`). A config block reaches no file, no network, no process |

### 6a.1 Why this does not touch the strict parse

§3 scopes the strict parse to the frontmatter plus the `meridian-*` fenced blocks, and calls every
other byte prose. **A ```` ```starlark ```` block is prose to that scan and stays prose** — the mount
parser never sees it, so a broken `config()` can never cost this machine its mount table, and
`mrd config` binds roots exactly as it did before the block existed.

That is the difference this section turns on: **the mount plane is SCANNED, the config block is
ADDRESSED.** Nothing hunts for a starlark block; one verb resolves one block id on demand. So the
namespace argument of §3.1 does not apply — `^config` is not a third engine block-language, it is a
page address, and the ordinary anchor grammar (`[A-Za-z0-9-]`, `syntax::is_block_id`) already admits
it.

### 6a.2 The refusal ladder — every rung is loud, none is empty-success

`mrd config get` is a read that either prints a value or refuses with teaching. Each rung names what is
wrong, where, and the fix; none of them prints an empty line and exits 0 (schema §8's posture, this
plane's own words):

| Condition | Refusal |
|---|---|
| the chain resolved no file (state A) | there is no config to get; name the path and the block to add |
| no block carries `^config` | name the file and show the block's shape |
| two blocks carry `^config` | ambiguous — the mint plane never picks; name the count |
| `^config` does not key a fenced code block | name what it keyed |
| the fence is not `starlark` | name the language found and the one required |
| the source will not parse, or faults, or exhausts the budget | the evaluator's own message, verbatim |
| no `config()` is defined | name the entry the block owes |
| `config()` returned something unserializable (a function, a lambda) | name it — the config is data |
| a `KEY` segment was asked of a non-mapping | name the type found and where the walk stopped |
| a `KEY` segment is absent | name where it stopped and the keys that ARE there |

**The mount table's state is not a rung.** `mrd config get` reads the config block; it never calls
`bind()`, so an unbound or missing root refuses `mrd config` (exit 1) and leaves `mrd config get`
answering normally. The two verbs answer about two different things and are deliberately not coupled.

### 6a.3 `KEY` — the dot-path, and the one rule that keeps every key reachable

A `KEY` addresses a member of the returned value. It is a dot-path, resolved with **exact key first,
then the dot as a separator**, at every level:

1. Does this mapping have a member named by the WHOLE remaining key? Then that member is the answer.
2. Otherwise split at the first `.`: the head must be a member, descend into it, repeat with the tail.

The order is what makes the grammar safe on an arbitrary config. A config really carrying a member
named `a.b` stays addressable by its own name, and only a config with no such member reads `a.b` as
"`b` inside `a`" — so no key an author can write becomes unaddressable, which a split-first rule would
have done silently.

**Nesting is not a convenience.** The first real config keys `repos_root` BY WIKI, because a repos root
is a fact about a wiki and not about a machine:

    mrd config get repos_root.work-wiki      -> /path/to/work/repos
    mrd config get repos_root.field-notes      -> /path/to/home/repos
    mrd config get repos_root                -> the mapping, as JSON

A top-level-only `KEY` would have made the correct shape the one nobody could read, and the flat
machine-wide spelling — the one that is wrong for every wiki but one — the convenient one.

### 6a.4 What is deliberately NOT specified

- **No key schema.** The config is unlimited by rule. A future engine-read key
  inside `config()` would reopen §4's misspelled-optional-key hazard and must state how it closes it.
- **One block, not many.** A second `^config` is ambiguous, not a merge: the mint plane never silently
  picks, and merging two blocks would need a precedence rule nobody has ruled on.
- **No list index.** A KEY segment addresses a mapping's member and nothing else. Indexing into a list
  would need a second grammar and a rule for a mapping whose key is `0`; bare `mrd config get` prints
  the whole value for a caller that wants to walk it.
- **No wire op.** The config block is read by the CLI in the calling process, from the file that
  process's own chain resolves — the same honesty `mrd config` states about `answered by: this process`.
  A daemon serving its own `MERIDIAN.md` would be answering about a different machine's config.

## 7. The self-hosting rev — config is content

### 7.1 The rev

`MERIDIAN.md` is parsed by the engine's own parser, so it acquires a rev by the shipped law with **no
new mechanism**:

> `config_rev` = the document root node's `node_rev` = `blake3(raw file bytes)[:16]`, 16 lowercase hex.

Verified in the tree, not asserted: the root node's span is `0..raw.len`
(`crates/model/src/lib.rs:204`) and its rev is `node_rev(raw.as_bytes, &root_span)`
(`crates/model/src/lib.rs:210`), where `node_rev` is `blake3(span bytes)[:16]`
(`crates/model/src/lib.rs:310-311`). This is byte-identically the law the armed
artifact's pinned `rev` already uses for a rule page (`crates/policy/src/registration.rs`,
`page_rev`) — §1.2 rule 3.

`config_rev` is spelled `file_rev` wherever the wire already spells a whole-page rev
(`crates/wire/src/lib.rs:1199`). **One name per thing: no new rev noun is minted for the config.**

### 7.2 The rev is computable where the config lives — this is not accidental

`model::build(raw: String, nodes: Vec<syntax::DialectNode>)` (`crates/model/src/lib.rs:165`) is a pure
function: no workspace, no I/O, no git. So `config_rev` is computable for a file in `$HOME`, which is a
**denied workspace path** (`DenyReason::HomeDir`, `crates/workspace/src/lib.rs:305`) and can never
be promoted into one. The rev exists there; the *attestation plane* does not. §9 states exactly how far
that gets us.

### 7.3 What the rev is FOR, and the honest scope of "renders as ordinary drift"

The requirement is that *"the file itself carries a rev, so editing it out of band
renders as ordinary drift."* The precise, satisfiable reading:

- **The rev is reported.** Any surface that publishes the loaded config reports its `config_rev`, so
 an operator or agent can compare it to a value they hold. Editing the file changes the rev. This is
 the whole of what the config's own rev delivers, and it is real.
- **The rev is NOT checked against a stored baseline inside the file.** A config that declared its own
 expected rev would be self-referential — changing the declared value changes the rev — so no such
 key exists, and none may be added.
- **The rev is NOT a drift verdict.** There is no attestation baseline for `~/MERIDIAN.md` (§9). A
 build that renders a *verdict* on the config's freshness would be manufacturing one.

**Drift that IS a verdict is the mount pins' (§5.3), not the config's own rev's.** A mount entry's
`pin` names a root's entry page — which lives *inside* an attestable root — so `pin` vs the live
fingerprint is an ordinary comparison on ordinary machinery, exactly as an armed row's pinned `rev` vs
`page_rev(live page)` is (`crates/policy/src/armed.rs`, `verify_rows`). That is the mitigation
working, and it is the only place in this plane where "drift" is a checkable claim rather than a
reported number.

## 8. The refusal law

Every state-B refusal names **what is broken** and **where**, and does so through a closed reason set.

### 8.1 The shape

Extend `crates/lock`'s error type, which is the in-repo model that carries a structured location
(`crates/lock/src/lib.rs:390`) — not the INDEX's, which carries none (§1.3 D-d):

```rust
Malformed { line: usize, reason: &'static str } // the shape to extend
```

Required additions for this schema, because a config refusal is read by a human looking at a file, not
at a slice:

1. **`line` is 1-based in the FILE**, not within the block. `lock::parse` numbers within the block
 slice because a lock block is machine-written and machine-read; a human editing `MERIDIAN.md` is
 looking at file lines. A block-relative number is convertible to a file line because the block node
 carries its byte span (`crates/lock/src/lib.rs:527-534` is the collection pattern), so this costs
 an addition, not a mechanism.
2. **The refusal names the config path**, since `MERIDIAN_CONFIG` means the file may be anywhere.
3. **`reason` stays `&'static str` — a closed set, never free text.** This is what makes the reason
 word testable and keeps a refusal's spelling from drifting (the same discipline as
 `D1_TEACHING_REFUSAL_EXEMPLAR`, `crates/model/src/selector.rs:569`).

### 8.1a Which line a refusal points at

A refusal about *nothing* has no line unless the rule says which one. Three cases, exhaustive:

| The fault is about | The line is | Cases |
|---|---|---|
| Something **present** | its own line | `wrong-type-value`, `unsupported-version`, `bad-value`, `unknown-field`, `field-out-of-order`, `malformed-line`, `frontmatter-unparseable` |
| Something **absent** | the opening line of the construct that should have carried it — the block's opening fence for a block field, **line 1** for a frontmatter key or a frontmatter fence fault | `missing-required-field`, `missing-required-key`, `no-frontmatter`, `unterminated-block` |
| A **duplicate** | the **second** occurrence, and the message names the first | `duplicate-field`, `duplicate-mount-name`, `duplicate-tool-name`, `duplicate-primary-designation` |

The absent case points at the construct's opening rather than at "where it should have gone" because
canonical order makes the latter computable but not obvious to a reader — the opening fence is a line
the author can see and is always present.

State C and `home-unresolvable` carry the config path and no line: there are no bytes to point at.

### 8.2 The closed reason set

| Reason word | Fires when |
|---|---|
| `config-path-unusable` | state C: `MERIDIAN_CONFIG` names something that is not a readable regular file |
| `home-unresolvable` | rung 2 cannot be built: `$HOME` unset or empty |
| `no-frontmatter` | the file does not open with `---\n`, or the frontmatter fence never closes |
| `frontmatter-unparseable` | the frontmatter is not parseable YAML |
| `missing-required-key` | a required **frontmatter** key is absent |
| `wrong-type-value` | `type:` is present and is not `meridian-config` |
| `unsupported-version` | `version:` is an integer this build does not implement |
| `missing-required-field` | a required **block** field is absent |
| `unknown-field` | a block line's key is not in that block's legal set |
| `duplicate-field` | a key appears twice in one block |
| `field-out-of-order` | a block's fields are not in canonical order |
| `bad-value` | a value violates its field's type or charset |
| `malformed-line` | a block body line is not `key: value`, or a `config:` payload line is not indented |
| `unterminated-block` | an engine block's fence never closes |
| `duplicate-mount-name` | two `meridian-mount` blocks declare the same `name` |
| `duplicate-tool-name` | two `meridian-tool` blocks declare the same `name` |
| `duplicate-primary-designation` | two `meridian-mount` blocks carry `primary: true` (§5.1a) |
| `alias-shadows-name` | a `meridian-mount` block's `alias` equals some block's `name` or another block's `alias` (§5.1b) |

### 8.3 The teaching content

A refusal carries: the reason word, the config path, the 1-based file line, what was found, and what
is legal. The repo's strongest refusal templates are `crates/policy/src/binding.rs:148-156` (names the
file, why it is off-limits, and the legal routes) and `crates/model/src/selector.rs:569` (names each
candidate by two independent addresses, then `Fix:`). The shape for this plane:

```
refused: ~/MERIDIAN.md line 14: unknown field `paths` in a meridian-mount block —
legal fields are name, path, primary, vault, pin (in that order). No mount
table was loaded; the config is not partially applied. Fix: remove the line or
spell the field you meant.
```

**Three clauses are mandatory and each closes a specific failure:** naming the line (the
"and where"); stating **"no mount table was loaded"** (the no-partial-load law, made visible so a
reader cannot assume a partial config took effect); and a `Fix:` naming the legal form (the shipped
rule that a refusal cites the passing scenario — `crates/policy/src/check_eval.rs:502-513`, where
`refuse(message, passing)` makes it structurally impossible to refuse without it).

### 8.4 First refusal wins, and it is the only one

A malformed config produces **exactly one** refusal — the first, in file order. Reasons: the file does
not half-load, so there is no state in which a second fault is meaningful; and a cascade of derived
faults buries the one the operator must fix. This matches `lock::parse` (`crates/lock/src/lib.rs:543`)
and `parse_artifact` (`crates/policy/src/armed.rs:996`), both of which return on the first fault.

## 9. The stated limit — `~/MERIDIAN.md` cannot be attested (canonicalize-at-bind)

Carried, not papered over. `$HOME` is not a git repo, has no receipt journal and no merkle hash
domain, and is a **denied workspace path** (`DenyReason::HomeDir`,
`crates/workspace/src/lib.rs:305`) so it can never be promoted into one to acquire a journal.
Therefore:

- **The single authority for every cross-root ref is the one artifact the attestation plane cannot
 attest.** Drift on it is not *"an ordinary red on the ordinary machinery"*, and this spec does
 not pretend it is.
- The config's own rev is a **reported number**, never a verdict (§7.3).
- The mitigation is mount-as-claim (§5.3): a mount's `pin` is checkable because its target
 lives inside an attestable root.

**The residual that mitigation does NOT close, stated exactly.** A mount's pin protects the root that
mount declares. It does not protect the mount table's *membership*: **deleting a mount block deletes
its own pin along with it.** Under grey-exit-1 an unmounted root renders grey and the fence refuses on
exit 1 — so removing a mount converts a red into a grey that must be `--force`d past, which is why
grey-exit-1 rules grey to exit 1 rather than 0. The residual is therefore bounded and visible, not silent,
but it is real: **the fence's only bypass is an edit to exactly this file, and this file cannot be
attested.** No v1 mechanism closes it, and this schema does not pretend one does.

## 10. The fixture corpus

`crates/testsuite/data/meridian-md/` — the corpus the implementation consumes.

| Path | Carries |
|---|---|
| `README.md` | the corpus law: what each case must state, and the escalation clause |
| `cases.json` | **every case paired with its required outcome** — the manifest is the pairing |
| `corpus/` | well-formed configs (the acceptances) |
| `refusals/` | malformed configs, one per malformed class (the refusals) |

Cases with no file — state A, state C, the env-var cases — **cannot be fixtures**, because a file
cannot express its own absence. They live in `cases.json` with `"fixture": null` and an `env` block.
This is why the manifest, not the file tree, is the pairing mechanism.

`cases.json` is shaped on the shipped probe-pack convention
(`crates/testsuite/data/harness/p2-walk-probes.json`): each case carries `id`, `fixture`, `env`,
`expect`, `law`, and `kills`. **`kills` states what wrong implementation the case rules out** — the
anti-vacuity discipline, written into the data rather than trusted to the reader.

## 11. Rejected alternatives, with reasons

- **The mount table in frontmatter (`mounts:` as a YAML list).** Rejected: the design asks
 for *"prose beside machine sections"*, and a frontmatter list admits no prose beside any entry. It
 also puts the mount table under the frontmatter parser, whose in-repo error type carries no
 structured location (§1.3 D-d) — so "and where" would be unobtainable for the very grammar most
 likely to be mis-typed.
- **One `meridian-mount` block holding a table of all entries.** Rejected: it forfeits prose-beside-
 each-mount (the literate pattern), and it makes a mount's own pin (§5.3) a row
 field rather than a statement beside the root it claims. The per-block form also gives refusals a
 natural coarse address (*which* block) beside the fine one (which line).
- **INDEX-style middot checklist rows.** Rejected: §1.3 D-b. The INDEX's row grammar is generated by
 the engine; a hand-written ` · ` is invisible in an editor and unlearnable from a refusal.
- **A closed set of tool kinds in v1.** Rejected: §6.1 — v1 owns zero kinds, so the grammar would admit
 nothing.
- **Making `MERIDIAN.md` authoritative for canonical root names.** Rejected — and named so it is not
 restored: it contradicts *"MERIDIAN.md binds, it doesn't baptize"* (`address-grammar.md` § 3,
 INV-5) and reintroduces the one-machine-only name that law exists to prevent. `name:` in a mount
 block is a **binding**, and implementation checks it against the root's own declaration.
- **A declared `expected_rev:` key for self-drift.** Rejected: self-referential and unsatisfiable
 (§7.3).
- **Project-local walk-up discovery.** Not rejected — **deferred** (§0). Not built here.

## 12. Boundaries flagged, not assumed

**Boundary 1 — the root-name charset, shared with the address grammar.** §5.2 fixes the charset a name may use *in the
file*, derived as the complement of the address grammar's operator set. `address-grammar.md` owns the address grammar and
must accept exactly the names this schema admits. **A consequence the address grammar inherits:** because
a name cannot contain `:`, the `sessions:notes.md` prefix-vs-literal-path ambiguity (`address-grammar.md`
§ 4) becomes decidable as *"is the pre-colon token a **bound** mount name?"* — which requires the mount
table at resolve time, i.e. exactly D4a's injection into `model::CorpusIndex::resolve_ref`. Stated as a
consequence, **not ruled here.**

**Boundary 2 — the pin's target, shared with implementation.** §5.3 fixes what a well-formed `pin` token *is*. It
does not fix **which file** a mount's pin names (the root's self-declaration entry page, whose location
is the D7 seeding question) nor **what bytes** it covers (whole file for a plain-folder root, a parsed span
for a vault root — different codecs). If implementation needs a second field to name the pin's
target explicitly, adding it is a v1 schema amendment, not a v2 bump — the field would be optional and
new, and §4's rule about optional engine-read keys applies to it.

Neither boundary blocks implementation: §2, §3, §4, §7, and §8 are complete and independent of both.
