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
| The `meridian-tool` block grammar: its engine-read half and its opaque half | Tool semantics — no unit in stage 3; deliberately unowned |
| That a mount entry **may pin the root it declares**, and what a well-formed pin token is | What the pin's target file is, and how the claim is checked — **implementation** |
| The self-hosting rev: which bytes, which hash law, what it is spelled | Drift reporting and the verb that shows it — **implementation** |
| The canonical root-**name** charset (the floor) | The `root:` **address** grammar, prefix-vs-literal-path ambiguity, `resolve_linkpath` — see `address-grammar.md` |
| The refusal shape: reason words, and where a refusal points | The engine implementation and its crate placement (D4) — **implementation** |

**Not specified, deliberately.** Project-local walk-up discovery (nearest-ancestor `MERIDIAN.md`) is
**deferred by the ratifying decision** (§2: *"Deferred, not rejected … it adds a resolution ambiguity
v1 does not need"*). Building it here would be over-completion (bound discipline). The resolution chain in §2 is
exactly two rungs and has no third.

Two boundaries are flagged rather than assumed, in §12.

## 1. The precedent this EXTENDS — `meridian/armed-rules.md`

The attested armed-rules artifact is markdown-as-config **shipping today**: engine-managed, inside the
hash domain, drift-tracked by a pinned rev. The MERIDIAN.md ruling's self-hosting model therefore
already ships once; this schema extends a proven pattern rather than introducing one.

> The precedent was originally `conventions/INDEX.md`, a checkbox list keyed by convention folder
> slug. The registration cutover retired the folder loader and the INDEX with it; the artifact is its
> successor and carries the same properties on a different grain — a table keyed by `(rule id, arm
> root)` rather than a checklist keyed by slug. Every row of the table below was re-verified against
> the successor, because a precedent cited at a deleted `file:line` teaches nothing.

### 1.1 The existing mechanism, stated

| Aspect | `meridian/armed-rules.md`, as shipped | file:line |
|---|---|---|
| In-file shape | fixed H1 title + free prose preamble + a markdown TABLE; **no frontmatter, no fenced blocks** | `crates/policy/src/armed.rs` (`ARTIFACT_TITLE` / `ARTIFACT_HEADER`, `ArmedArtifact::render`) |
| Row grammar | five `|`-separated columns — id · page · pinned rev · arm root (scope) · mode. Backtick-quoted cells, and `|`/backtick/control characters are **unrepresentable** rather than escaped | `crates/policy/src/armed.rs` (`ArmedRow::render`, `validate_workspace_path`) |
| One reader, on purpose | `parse_artifact` is the **only** reader and is fail-closed. The INDEX shipped two (a tolerant `armed_from_index` and a strict `parse_index_strict`); two readers of one attestation is two answers to one question, so the successor keeps the strict one | `crates/policy/src/armed.rs` (`parse_artifact`) |
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
| D-a | **Engine is sole writer**; a hand edit is a refused `BindingBreak` | **Human is sole author**; the engine never writes it | The INDEX is a generated attestation artifact. `MERIDIAN.md` is *"a new user's first contact … one readable file"* (ruling §1). A binding-break guard on a file the engine does not write would refuse every legitimate edit |
| D-b | **Middot-separated checklist rows** | **`key: value` lines inside a fenced block** | Row grammar is cheap to *generate* and hostile to *hand-write* — a missing ` · ` is invisible in an editor. `key: value` is the grammar the repo already hand-authors (`crates/lock`, lock/def frontmatter, `meridian/domain.md` domain frontmatter) |
| D-c | **No frontmatter** | **Required frontmatter** (`type`, `version`) | The INDEX is found at one reserved path, so its identity is positional. `MERIDIAN.md` can be aimed anywhere by `MERIDIAN_CONFIG`, so it must be able to say *what it is* and *which schema it speaks* — otherwise a mis-set env var half-loads an unrelated page |
| D-d | Malformed row names the row **text** (`{line:?}`), never its **number** (`crates/policy/src/armed.rs:1039`, `parse_row`) | Every refusal carries a **1-based file line** | The ratified requirement is a refusal *"naming what is broken **and where**"*. The INDEX precedent cannot satisfy it. The in-repo model that can is `crates/lock` — `LockError::Malformed { line, reason: &'static str }` (`crates/lock/src/lib.rs:390`) — so §8 extends **lock's** error shape, not the INDEX's |
| D-e | Absent-vs-malformed pivots on a **separate marker file** | Absent and malformed are decided by **the file alone** | The once-armed marker exists because disarming must not be silent. `MERIDIAN.md` has no such asymmetry: there is nothing to disarm, and every machine legitimately starts with no file (D6) |
| D-f | Round-trip was **lossy** — the scope column was rendered and never read back (the retired INDEX; the successor's `parse_row` reads all five columns, `crates/policy/src/armed.rs:1039`) | Every declared field is **read**; an unread field is refused as unknown | A lossy round-trip is tolerable when the engine regenerates the file. Here the human's bytes are the only source, so a silently-ignored field is a silently-ignored intent |

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
| **B** | **Present but malformed** | **Fail loud.** A teaching refusal naming what is broken and **where**. **NO partial mount table. NO default-root fallback** | one of §8's classes | Ratified fail-loud (§4 of the ruling) is about a *broken* config. A partial load would make the system's own definition half-true |
| **C** | **`MERIDIAN_CONFIG` set to a path that is not a readable regular file** — absent, a directory, or unreadable | **Fail loud** | `config-path-unusable` | **This is NOT state A.** The operator stated an intent that cannot be honoured; silently falling back to `~/MERIDIAN.md` would mask it. D6 ruled two states; this is the third |
| **D** | **Parses clean and declares ZERO mounts** | **Treated as state A** — current single-root behaviour. **Not an error** | *(no refusal)* | An empty mount table is a legitimate statement. Named because "empty" and "absent" reaching different code paths is how a nil-vs-empty bug is born |

**State D is a behavioural identity, not a similarity.** A zero-mount config and an absent config MUST
produce the same mount table (the empty one) and the same resolution behaviour. The config's own rev
(§7) still exists in state D and does not in state A — that is the only permitted difference, and it
is an observation, never a branch.

**The green-path control (parse-acceptance(c)).** State B's refusals are only meaningful beside the acceptances:
states A and D leave behaviour unchanged, and a well-formed multi-mount config in
`the multi-root fixture case` loads every entry it declares. A build that refused every config would satisfy
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
which would defeat the entire "one readable file that explains itself" purpose the ruling states.

**Corollary, and it is load-bearing:** a fenced block that is *not* an engine block-language — a
` ```yaml ` example, a ` ```text ` diagram, an indented snippet — is prose, **even if its contents look
exactly like a mount block.** Fixture `the prose-decoy fixture case` is the anti-vacuity case: it carries
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

One shipped behaviour governs how the namespace renders, and U36 narrowed it from per-namespace to
per-language:

1. **The render face elides ENGINE-EMITTED languages only.** `ToonRenderer::with_meridian_elision`
 drops the blocks `lock::is_engine_emitted` names (`crates/render/src/lib.rs:194-209`): a
 `meridian-lock` is machine-written and elides, while a `meridian-mount` or `meridian-tool` is
 user-authored and **renders**. The raw `cat` face rides everything verbatim either way.

The former second behaviour — the form-2 chain reader skipping engine blocks — left the tree with
the retired `^inputs` plane, so there is no chain reader left to mis-read a mount block. What still
makes the namespace the correct home is §3.1 itself: it is the one predicate the strict parse scopes
on, and per-language elision means a human-authored block in it is never hidden from its author.

**The verification trap this section was written to name is therefore gone in its old form, and the
requirement survives for the honest remainder.** An agent running `mrd read ~/MERIDIAN.md` today
sees the prose **and** the mount blocks, so the old false conclusion — "no mount blocks visible, the
parse failed" — cannot happen. What the rendered face still never shows is the parse **verdict**: a
mount block's bytes render whether or not the config parser accepted them. **The user-reachable
verb that publishes the parsed mount table must therefore still not be the rendered read face.**
Implementation owns which verb it is; this spec's requirement is only that criterion 1's evidence
not be measured on a surface that shows the block's bytes but never its acceptance.

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
| The frontmatter is not parseable YAML | `frontmatter-unparseable`, carrying the parser's own message |

## 5. The `meridian-mount` block grammar

One block declares one mount entry. The grammar is **line-oriented `key: value`, one field per line,
in canonical order** — modelled directly on `lock::parse` (`crates/lock/src/lib.rs:543`), which is
the repo's one strict hand-parsed block grammar with per-line refusals.

```meridian-mount
name: field-notes
path: «local-path»
kind: vault
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
| 3 | `kind` | exactly `vault` or `git-folder` | **yes** | absent → `missing-required-field`. Any other value → `bad-value`, naming the value found and the two legal values |
| 4 | `vault` | Obsidian vault name, non-empty string | **iff `kind: vault`** | required and absent → `missing-required-field`, naming the kind that requires it. Present when `kind: git-folder` → `field-not-permitted-for-kind` |
| 5 | `pin` | fingerprint CID-token (§5.3) | no | present and not a well-formed token → `bad-value` |

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

**Blank lines and comment lines are refused** as `malformed-line`. Prose about a mount belongs beside
the block, not inside it — that is what §3's scoping law buys, and it keeps the block grammar with one
spelling per fact. (`lock::parse` refuses the same way: *"unrecognized line (canonical order: …)"*.)

**`duplicate-mount-name` is in scope and `path` collision is NOT.** Name uniqueness is a pure in-file
property, decidable from the bytes. Two mounts resolving to the same *path* is decidable only after
canonicalization (symlinks, trailing slashes, `..`), and **Implementation owns the mount-path law** — canonicalize
at bind, inherit `workspace::deny_reason`, refuse equal-or-nested mounts (canonicalize-at-bind). One owner per fact:
this schema does not also test paths lexically, because two owners disagreeing about "same path" is a
worse failure than one owner deciding late.

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
the CHARSET-GUARD ruling that refuses legacy underscore ids at every mint position
(`crates/testsuite/data/charset-guard/discrimination.json`), and the result is exactly the lowercase-
kebab convention this repo already uses for its own fixture families.

**This is a FLOOR for implementation, not a ceiling.** implementation owns the address grammar and may narrow what a `root:`
prefix accepts; it must not widen it past this charset, because a name outside this charset cannot be
*bound* and so could never resolve.

### 5.3 `pin:` — mount-as-claim (canonicalize-at-bind, load-bearing)

The ratifying decision §3: *"Mounts can be claims. A mount entry may pin the root it declares (e.g.,
the fingerprint of that root's entry page)."* canonicalize-at-bind makes this load-bearing rather than a nicety —
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
`git-folder` root's pin grain is the file and a `vault` root's is a parsed span
(cross-root-addressing §3) — those are different codecs, and pinning one here would forbid the other.

**What the pin's target is, and how the claim is checked, is implementation's** (§12, boundary 2). This spec
requires only that a build which cannot verify a pin says so; it must never treat an unverifiable pin
as verified — *outside sight never renders as verified* (R26), and under grey-exit-1 a grey refuses on
exit 1 with its own reason word.

## 6. The `meridian-tool` block grammar

The ruling §1 gives `MERIDIAN.md` *"the declarations for tooling built on top — agent-facing efficiency
layers and imperative user-facing tools alike."* **No stage-3 unit owns tool semantics, and
`tag-based-mounting` — the design input that would shape them — is still `resolution: open`.** Designing
a tool system here would be over-completion (bound discipline). So this schema specifies the **grammar and the
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

**Why this does not contradict the strictest-parse ruling.** Fail-loud is about a *broken* config
(ruling §4: *"A broken `MERIDIAN.md` (bad frontmatter, malformed mount block)"*). A tool declaration
for a tool this machine has not installed is **not broken** — it is a statement addressed to someone
else. Refusing it would mean a config becomes invalid by *removing* a tool, which is the opposite of
fail-loud's intent. The shipped analogue is the wire's tolerant-code law: *"Clients treat unrecognized
codes as `recovery`-dispatched"* (`crates/wire/src/lib.rs:1350-1353`), and `laws.md` § Additivity.

**Rejected alternative, named so it is not restored:** *"define a closed set of tool kinds in v1 and
refuse the rest."* v1 would then define **zero** kinds (no unit owns any), so every tool declaration
the ruling calls for would refuse — a grammar in which nothing legal can be written. Rejected.

**What is NOT deferred:** the engine's half — `name`, `kind`, uniqueness, and the block's structural
integrity — is parsed strictly, so a malformed *declaration* still fails loud. The opacity is of the
payload's meaning, never of the block's shape.

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

Criterion 1's last clause requires that *"the file itself carries a rev, so editing it out of band
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
`page_rev(live page)` is (`crates/policy/src/armed.rs`, `verify_rows`). That is the ratified mitigation
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
| Something **present** | its own line | `wrong-type-value`, `unsupported-version`, `bad-value`, `unknown-field`, `field-out-of-order`, `field-not-permitted-for-kind`, `malformed-line`, `frontmatter-unparseable` |
| Something **absent** | the opening line of the construct that should have carried it — the block's opening fence for a block field, **line 1** for a frontmatter key or a frontmatter fence fault | `missing-required-field`, `missing-required-key`, `no-frontmatter`, `unterminated-block` |
| A **duplicate** | the **second** occurrence, and the message names the first | `duplicate-field`, `duplicate-mount-name`, `duplicate-tool-name` |

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
| `missing-required-field` | a required **block** field is absent (including a kind-conditional one) |
| `unknown-field` | a block line's key is not in that block's legal set |
| `duplicate-field` | a key appears twice in one block |
| `field-out-of-order` | a block's fields are not in canonical order |
| `field-not-permitted-for-kind` | a field is present that its block's `kind` forbids |
| `bad-value` | a value violates its field's type or charset |
| `malformed-line` | a block body line is not `key: value`, or a `config:` payload line is not indented |
| `unterminated-block` | an engine block's fence never closes |
| `duplicate-mount-name` | two `meridian-mount` blocks declare the same `name` |
| `duplicate-tool-name` | two `meridian-tool` blocks declare the same `name` |

### 8.3 The teaching content

A refusal carries: the reason word, the config path, the 1-based file line, what was found, and what
is legal. The repo's strongest refusal templates are `crates/policy/src/binding.rs:148-156` (names the
file, why it is off-limits, and the legal routes) and `crates/model/src/selector.rs:569` (names each
candidate by two independent addresses, then `Fix:`). The shape for this plane:

```
refused: ~/MERIDIAN.md line 14: unknown field `paths` in a meridian-mount block —
legal fields are name, path, kind, vault, pin (in that order). No mount table was
loaded; the config is not partially applied. Fix: remove the line or spell the
field you meant.
```

**Three clauses are mandatory and each closes a specific failure:** naming the line (the ratified
"and where"); stating **"no mount table was loaded"** (the ratified no-partial-load, made visible so a
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
 attest.** The plan's §5 row that once called drift on it *"an ordinary red on the ordinary
 machinery"* was false and is corrected there; this spec does not restore it.
- The config's own rev is a **reported number**, never a verdict (§7.3).
- The ratified mitigation is mount-as-claim (§5.3): a mount's `pin` is checkable because its target
 lives inside an attestable root.

**The residual that mitigation does NOT close, stated exactly.** A mount's pin protects the root that
mount declares. It does not protect the mount table's *membership*: **deleting a mount block deletes
its own pin along with it.** Under grey-exit-1 an unmounted root renders grey and the fence refuses on
exit 1 — so removing a mount converts a red into a grey that must be `--force`d past, which is why
grey-exit-1 rules grey to exit 1 rather than 0. The residual is therefore bounded and visible, not silent,
but it is real: **the fence's only bypass is an edit to exactly this file, and this file cannot be
attested.** No v1 mechanism closes it, and this schema does not pretend one does.

## 10. The fixture corpus

`crates/testsuite/data/meridian-md/` — the corpus implementation and implementation consume.

| Path | Carries |
|---|---|
| `README.md` | the corpus law: what each case must state, and the escalation clause |
| `cases.json` | **every case paired with its required outcome** — the manifest is the pairing |
| `corpus fixture cases` | well-formed configs (the acceptances) |
| `refusal fixture cases` | malformed configs, one per malformed class (the refusals) |

Cases with no file — state A, state C, the env-var cases — **cannot be fixtures**, because a file
cannot express its own absence. They live in `cases.json` with `"fixture": null` and an `env` block.
This is why the manifest, not the file tree, is the pairing mechanism.

`cases.json` is shaped on the shipped probe-pack convention
(`crates/testsuite/data/harness/p2-walk-probes.json`): each case carries `id`, `fixture`, `env`,
`expect`, `law`, and `kills`. **`kills` states what wrong implementation the case rules out** — the
anti-vacuity discipline, written into the data rather than trusted to the reader.

## 11. Rejected alternatives, with reasons

- **The mount table in frontmatter (`mounts:` as a YAML list).** Rejected: the ratifying decision asks
 for *"prose beside machine sections"*, and a frontmatter list admits no prose beside any entry. It
 also puts the mount table under the frontmatter parser, whose in-repo error type carries no
 structured location (§1.3 D-d) — so "and where" would be unobtainable for the very grammar most
 likely to be mis-typed.
- **One `meridian-mount` block holding a table of all entries.** Rejected: it forfeits prose-beside-
 each-mount (the literate pattern the ruling names), and it makes a mount's own pin (§5.3) a row
 field rather than a statement beside the root it claims. The per-block form also gives refusals a
 natural coarse address (*which* block) beside the fine one (which line).
- **INDEX-style middot checklist rows.** Rejected: §1.3 D-b. The INDEX's row grammar is generated by
 the engine; a hand-written ` · ` is invisible in an editor and unlearnable from a refusal.
- **A closed set of tool kinds in v1.** Rejected: §6.1 — v1 owns zero kinds, so the grammar would admit
 nothing.
- **Making `MERIDIAN.md` authoritative for canonical root names.** Rejected — and named so it is not
 restored: it contradicts *"MERIDIAN.md binds, it doesn't baptize"* (cross-root-addressing §1a) and
 reintroduces the one-machine-only name that section exists to prevent. `name:` in a mount block is a
 **binding**, and implementation checks it against the root's own declaration.
- **A declared `expected_rev:` key for self-drift.** Rejected: self-referential and unsatisfiable
 (§7.3).
- **Project-local walk-up discovery.** Not rejected — **deferred by the ratifying decision** (§0). Not
 built here.

## 12. Boundaries flagged, not assumed

**Boundary 1 — the root-name charset, shared with implementation.** §5.2 fixes the charset a name may use *in the
file*, derived as the complement of the address grammar's operator set. Implementation owns the address grammar and
must accept exactly the names this schema admits. **A consequence implementation should know it inherits:** because
a name cannot contain `:`, the `sessions:notes.md` prefix-vs-literal-path ambiguity (plan §6, ruled to
implementation) becomes decidable as *"is the pre-colon token a **bound** mount name?"* — which requires the mount
table at resolve time, i.e. exactly D4a's injection into `model::CorpusIndex::resolve_ref`. Stated as a
consequence, **not ruled here.**

**Boundary 2 — the pin's target, shared with implementation.** §5.3 fixes what a well-formed `pin` token *is*. It
does not fix **which file** a mount's pin names (the root's self-declaration entry page, whose location
is the D7/implementation seeding question) nor **what bytes** it covers (whole file for `git-folder`, a parsed span
for `vault` — different codecs, cross-root-addressing §3). If implementation needs a second field to name the pin's
target explicitly, adding it is a v1 schema amendment, not a v2 bump — the field would be optional and
new, and §4's rule about optional engine-read keys applies to it.

Neither boundary blocks implementation: §2, §3, §4, §7, and §8 are complete and independent of both.
