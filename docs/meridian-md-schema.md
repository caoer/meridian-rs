---
type: result
id: schema
status: spec
created: 2026-07-26
tags: [type/result, domain/meridian-rs, topic/meridian-rs, topic/config]
owns: ["MERIDIAN.md config parse"]
---

# MERIDIAN.md in-file schema — the config plane's parse law

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

Status: normative for the `MERIDIAN.md` parse.

`MERIDIAN.md` is the one file a machine reads to learn which markdown trees it
can reach. Each **mount** entry in it binds a root name — the name of one
markdown tree — to a local path, and the mount table is the set of those
entries. This document rules how the engine parses that file: where it is
found, which bytes are parsed strictly, which keys and fields are legal, and
how a broken file is refused. Every key and field below states its type,
whether it is required, and the refusal on violation. It does not rule what
the mount table means once bound, nor what a tool declaration does; §0 lists
those boundaries.

**Standing law (no external decision files):** one entry point
(`MERIDIAN_CONFIG` → `$HOME/MERIDIAN.md`); markdown over TOML; config is
content; fail-loud strictest parse; the mount table is a three-way map
(name ↔ vault ↔ path); the root declares, `MERIDIAN.md` binds; grey refuses on
exit 1 with a distinct reason word; `$HOME/MERIDIAN.md` cannot be attested
(mount-as-claim is the mitigation). Cross-root grammar: `address-grammar.md`.

Every rule has a fixture in `crates/testsuite/data/meridian-md/`, with its
required outcome in `crates/testsuite/data/meridian-md/cases.json`. A rule with
no fixture is a defect in this spec.

## 0. What this spec owns

| Owned here (the **in-file schema**) | Owned elsewhere |
|---|---|
| Where the file is found; the four resolution states | — |
| Which bytes are **machine surface**, which are prose | — |
| Frontmatter keys the engine reads: name, type, required, refusal | — |
| `meridian-mount` grammar: fields, types, order, refusals | Mount table **semantics** (implementation): canonicalize at bind, `deny_reason`, equal-or-nested refusal, declared-vs-bound checking, grey classes |
| `meridian-tool` grammar: engine-read half and opaque half | Tool semantics — deliberately unowned |
| A mount entry **may pin the root it declares**; the well-formed pin token | The pin's target file and how the claim is checked (implementation) |
| The self-hosting rev: bytes, hash law, spelling | Drift reporting and the verb that shows it (implementation) |
| The canonical root-**name** charset (the floor) | `root:` **address** grammar, prefix-vs-literal-path ambiguity, `resolve_linkpath`: `address-grammar.md` |
| Refusal shape: reason words, where a refusal points | Engine implementation and crate placement (D4) |

**Not specified.** Project-local walk-up discovery (nearest-ancestor
`MERIDIAN.md`) is deferred, not rejected: it adds a resolution ambiguity v1 does
not need. Two boundaries are flagged in §12.

## 1. The precedent this extends — `meridian/armed-rules.md`

The attested armed-rules artifact already ships markdown-as-config:
engine-managed, inside the hash domain, drift-tracked by a pinned rev. The
self-hosting model therefore already ships once, and this schema extends a
proven pattern rather than inventing one.

> The artifact is a table keyed by `(rule id, arm root)`, called **the INDEX**
> here.

### 1.1 The existing mechanism

| Aspect | `meridian/armed-rules.md`, as shipped | file:line |
|---|---|---|
| In-file shape | fixed H1 title, free prose preamble, one markdown table; **no frontmatter, no fenced blocks** | `crates/policy/src/armed.rs` (`ARTIFACT_TITLE` / `ARTIFACT_HEADER`, `ArmedArtifact::render`) |
| Row grammar | five `|`-separated columns: id · page · pinned rev · arm root (scope) · mode; backtick-quoted cells; `|`, backtick, control characters **unrepresentable**, not escaped | `crates/policy/src/armed.rs` (`ArmedRow::render`, `validate_workspace_path`) |
| One reader | `parse_artifact`, the **only** reader, fail-closed | `crates/policy/src/armed.rs` (`parse_artifact`) |
| Strictness scope | **title** and **column header** exact; every data row parses; **preamble not byte-checked** | `crates/policy/src/armed.rs` (`parse_artifact`) |
| Pinned rev | `rev` = `page_rev(page bytes)` = `blake3(bytes)[:16]`, 16 lowercase hex — the world model's rev law (contract §1) on the rule page | `crates/policy/src/registration.rs` (`page_rev`) |
| Drift | at the door, `page_rev(live page) != row.rev` → `ArmedFault::Red(Redness::Drifted)`; the write refuses (check) or the fault is reported (hook) | `crates/policy/src/armed.rs` (`verify_rows`), `crates/policy/src/armed_law.rs` |
| Malformed posture | `ArtifactCorrupt { detail: String }` → `ArmedFault::Corrupt`, **fail closed**; never reads as "nothing armed" | `crates/policy/src/armed.rs`, `crates/policy/src/armed_law.rs` |
| Emptied posture | a well-formed artifact with zero rows on a once-armed workspace → `ArmedFault::Disarmed`, not a disarm: an attested absence is a row spelled `off`; zero rows is the absence of attestation | `crates/policy/src/armed_law.rs` |
| Absent posture | pivots on a **separate marker** (`meridian/attested`): never-armed → not read; once-armed → a fault | `crates/policy/src/armed_law.rs` (`resolve_armed_law`) |
| Writer | the **engine** is sole writer; a hand edit is a `BindingBreak` teaching refusal | `crates/policy/src/binding.rs` (`classify_door_law`) |

### 1.2 Where this schema follows it

1. **Strictness is scoped to a machine surface; prose is prose** (§3).
2. **Malformed fails closed and names the damage.** §8's refusal takes the shape
   of `ArtifactCorrupt`'s detail (*"row is not a closed table row: …"*,
   `crates/policy/src/armed.rs:1039`, `parse_row`).
3. **The pinned rev is the node_rev family, not a fingerprint.** §7 reuses
   `page_rev` = `blake3(bytes)[:16]`; no new hash law.
4. **A drifted pin refuses; it never silently re-arms** — the model for
   mount-as-claim in §7.3.
5. **A reserved-path constant mirrored in two crates gets a cross-crate drift
   test** (`crates/policy/src/armed.rs:26`, `crates/fs/src/domain.rs:50`, test
   `crates/wire-serve/tests/reserved_paths.rs:10`); §2.4 requires it for
   `MERIDIAN.md`'s filename and env var.

### 1.3 Where this schema differs

| # | INDEX does | `MERIDIAN.md` does | Why |
|---|---|---|---|
| D-a | **Engine is sole writer**; a hand edit is a refused `BindingBreak` | **Human is sole author**; the engine never writes it | The INDEX is generated; such a guard would refuse every legitimate edit |
| D-b | **Middot-separated checklist rows** | **`key: value` lines in a fenced block** | A missing ` · ` is invisible in an editor; `key: value` is what the repo hand-authors (`crates/lock`, lock/def frontmatter, `meridian/domain.md`) |
| D-c | **No frontmatter** | **Required frontmatter** (`type`, `version`) | `MERIDIAN_CONFIG` can aim anywhere, so the file must say what it is and which schema it speaks; the INDEX's identity is its path |
| D-d | names the row **text** (`{line:?}`), never its **number** (`crates/policy/src/armed.rs:1039`, `parse_row`) | Every refusal carries a **1-based file line** | A refusal names what is broken **and where**, so §8 extends **lock's** `LockError::Malformed { line, reason: &'static str }` (`crates/lock/src/lib.rs:390`) |
| D-e | Absent-vs-malformed pivots on a **separate marker file** | The file alone decides | Nothing here can be disarmed; every machine legitimately starts with no file (D6) |
| D-f | lossy round-trip is harmless — the engine regenerates the file (`parse_row` reads all five columns) | Every declared field is **read**; an unread field is refused as unknown | The human's bytes are the only source: an ignored field is an ignored intent |

## 2. Resolution — the bootstrap chain and the four states

### 2.1 The chain (exactly two rungs)

1. **`MERIDIAN_CONFIG`**, when set to a non-empty value — the override. Its
   value is the path, used verbatim: no `~` expansion, no glob, no search.
2. **`$HOME/MERIDIAN.md`** — the default.

An **empty or whitespace-only** `MERIDIAN_CONFIG` states no path and counts as
**unset**, so the chain falls to rung 2. This is the same nil-vs-empty
distinction §2.2 state D draws for the mount table, applied to the env var.

`$HOME` unset or empty makes rung 2 unresolvable and **refuses** (§8,
`home-unresolvable`). That is not the absent case: absent means the default
path resolved and no file is there.

### 2.2 The four states

| # | State | Required behaviour | Reason word | Why it is named |
|---|---|---|---|---|
| **A** | **Absent** — a path resolved and no file is there (rung 2 only) | **Current single-root behaviour, unchanged.** Not an error, not a warning | *(no refusal)* | Every machine starts here; failing would brick the first run |
| **B** | **Present but malformed** | **Fail loud**: a teaching refusal naming what is broken and **where**. **No partial mount table, no default-root fallback** | one of §8's classes | A partial load would make the system's own definition half-true |
| **C** | **`MERIDIAN_CONFIG` names a path that is not a readable regular file** (absent, a directory, unreadable) | **Fail loud** | `config-path-unusable` | **Not state A**: a stated intent that cannot be honoured, so no silent fallback to `~/MERIDIAN.md`. D6 names two states; this is the third |
| **D** | **Parses clean and declares zero mounts** | **Treated as state A**, current single-root behaviour. **Not an error** | *(no refusal)* | An empty mount table is a legitimate statement; keeps "empty" and "absent" on one code path |

**State D is a behavioural identity, not a similarity.** A zero-mount config and
an absent config MUST produce the same (empty) mount table and the same
resolution behaviour. Only the config's own rev (§7) differs: it exists in D,
not A — an observation, never a branch.

**The green-path control.** `cases.json` carries acceptance cases beside
refusals: A and D leave behaviour unchanged, and a well-formed multi-mount
config (`corpus/multi-root.md`) loads every entry it declares. A build that
refused every config would satisfy state B alone.

### 2.3 What "readable regular file" means

State C's test: the path resolves, `metadata` succeeds, and the target is a
regular file (or a symlink to one) the process can read. A directory, a dangling
symlink, a special file and a permission error are all `config-path-unusable`,
each naming the path and the reason. Nothing finer is distinguished.

### 2.4 Two reserved names and their drift test

`MERIDIAN.md` (the filename) and `MERIDIAN_CONFIG` (the env var) are reserved
names. Where either is spelled in more than one crate, a cross-crate test
asserts the constants agree, as `the_armed_rules_artifact_has_one_spelling` does
for `meridian/armed-rules.md` (`crates/wire-serve/tests/reserved_paths.rs`).
Implementation places the constants; this spec fixes the spellings.

## 3. The document shape — the machine surface and the prose

`MERIDIAN.md` is an ordinary markdown page with two kinds of bytes:

- **Machine surface** — the frontmatter block, plus every fenced block whose
  info-string names an engine block-language (§3.1). Parsed strictly.
- **Prose** — everything else. The engine **never parses it and never refuses
  because of it** (the INDEX's law generalized: `crates/policy/src/armed.rs:996`,
  `parse_artifact`).

Without this scoping, adding a sentence of documentation to a config could
break the machine that reads it.

**Corollary:** a fenced block that is *not* an engine block-language (a
` ```yaml ` example, a ` ```text ` diagram, an indented snippet) is prose,
**even if its contents look exactly like a mount block.** Fixture
`corpus/prose-decoys.md` carries three decoys beside one real mount and must
yield exactly one mount.

### 3.1 The engine block-language namespace

The repo reserves the whole `meridian-*` fence-info prefix as the engine's
block-language namespace; the predicate is a **prefix test, not an enumerated
list** (`crates/lock/src/lib.rs:55-68`):

```rust
pub const NAMESPACE_PREFIX: &str = "meridian-";
pub fn is_meridian_lang(lang: &str) -> bool {
    lang.split_whitespace()
        .next()
        .is_some_and(|tok| tok.starts_with(NAMESPACE_PREFIX))
}
```

This schema adds two languages inside that namespace:

| Info-string | Carries | Cardinality |
|---|---|---|
| ` ```meridian-mount ` | exactly **one** mount entry | zero or more per file |
| ` ```meridian-tool ` | exactly **one** tool declaration | zero or more per file |

The **first whitespace token** of the info string decides the language, as in
every existing reader (`crates/lock/src/lib.rs:65`,
`crates/policy/src/pack.rs:376`, `crates/run/src/fence.rs:131`). A trailing
string (` ```meridian-mount the wiki `) is tolerated and ignored.

**One entry per block, not one table block.** Rejected alternative: §11.

### 3.2 The consequence of the namespace

This section governs how a block in the namespace appears when the engine shows
the file. Elision — dropping a block from the rendered page — is per-language,
not per-namespace.

**The render face elides engine-emitted languages only.**
`ToonRenderer::with_meridian_elision` drops the blocks
`lock::is_engine_emitted` names (`crates/render/src/lib.rs:194-209`):
`meridian-lock` is machine-written and elides, while `meridian-mount` and
`meridian-tool` are user-authored and **render**. The raw `cat` face carries
everything verbatim. No other reader skips engine blocks.

**The verification requirement.** A block's bytes render whether or not the
parser accepted them, so the rendered face never shows the parse **verdict**.
`mrd read ~/MERIDIAN.md` shows the prose and the mount blocks either way, so
what a reader sees there says nothing about whether the parse succeeded.
**The user-reachable verb that publishes the parsed mount table must therefore
not be the rendered read face**; implementation owns which verb it is.

## 4. Frontmatter keys

The frontmatter says what the file is and which schema it speaks (§1.3 D-c).
It is the file's first block: bytes `0..3` are `---\n` (a BOM-prefixed `---` is
**not** frontmatter), terminated by a closing `---` line
(`crates/testsuite/data/gt/ground-truth/README.md:21`).

| Key | Type | Required | Refusal on violation |
|---|---|---|---|
| `type` | string, exactly `meridian-config` | **yes** | absent → `missing-required-key` naming `type`; other value → `wrong-type-value`, naming the value found and the value required |
| `version` | integer | **yes** | absent → `missing-required-key` naming `version`; non-integer → `bad-value`; an integer this build does not implement → `unsupported-version`, naming the value found |

**v1 is `version: 1`.** A future format bumps it; a reader refuses a version it
does not implement and **never guesses a future format**
(`LockError::UnsupportedVersion`, `crates/lock/src/lib.rs:382`). The hash-domain
page `meridian/domain.md` (`version` + ignore list) holds the same discipline
(`wire-contract.md` §12.3).

**Unknown frontmatter keys are permitted and ignored**
(`crates/config/src/lib.rs:672`), so `title:`, `updated:` or Obsidian
properties may ride along. That is safe only because **v1 defines no optional
frontmatter key the engine reads**. Both keys are required, so a typo of either
fails loud as `missing-required-key` instead of being silently dropped. The
hazard of unknown-key tolerance is a misspelled *optional* key that silently
does nothing, and v1 has none. **A future version that adds an optional
engine-read key must state how it closes this hazard.**

Malformed frontmatter itself:

| Condition | Refusal |
|---|---|
| The file does not open with `---\n` | `no-frontmatter` |
| The frontmatter block is not terminated by a closing `---` | `no-frontmatter` (naming that the fence never closed) |
| The frontmatter block opens and closes but carries no keys | `missing-required-key` naming `type` — see below |
| The frontmatter is not parseable YAML | `frontmatter-unparseable`, carrying the parser's own message |

**An empty *closed* block is a missing key, not a missing block.** `---\n---`
opens with `---\n` and closes its fence, so neither `no-frontmatter` condition
holds. The markdown parser mints no frontmatter node for it, so a reader that
trusted the parse tree alone would refuse it as `no-frontmatter` — a false
statement about bytes the author can see. `crates/config/src/lib.rs`
(`closed_empty_frontmatter`) recognises the shape before the `no-frontmatter`
refusal is minted.

## 5. The `meridian-mount` block grammar

One block declares one mount entry. The grammar is **line-oriented `key: value`, one field per line,
in canonical order**, modelled on `lock::parse` (`crates/lock/src/lib.rs:543`).

```meridian-mount
name: field-notes
path: «local-path»
vault: field-notes
pin: fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152
```

### 5.1 Fields

Canonical order is the table's order. Each line is `key`, `:`, one space, then the value. The value
is the rest of the line, with trailing whitespace trimmed. An absent required field refuses
`missing-required-field`.

| # | Field | Type | Required | Refusal on violation |
|---|---|---|---|---|
| 1 | `name` | canonical root name (§5.2) | **yes** | bad charset, empty, or leading/trailing `-` → `bad-value`, naming the character and the legal charset |
| 2 | `path` | non-empty filesystem path | **yes** | empty or whitespace-only → `bad-value` |
| 3 | `primary` | literal `true` (§5.1a) | no | not `true` → `bad-value`, naming the one legal value |
| 4 | `vault` | non-empty Obsidian vault name | no | present but empty → `bad-value`. Presence is vault-ness; no `vault:` means not a vault |
| 5 | `pin` | fingerprint CID-token (§5.3) | no | malformed token → `bad-value` |
| 6 | `alias` | canonical root name (§5.2), second lookup spelling (§5.1b) | no | empty or bad charset → `bad-value`, naming the character; equal to any mount's `name` or another mount's `alias` → `alias-shadows-name`, refusing the whole table |

> **There is no `kind` field.** Vault-ness is `vault:` presence alone; `primary: true` is legal on any
> mount. No serve path branches on a mount's kind. A `kind:` line refuses as `unknown-field`; the
> remedy is to remove the line.

Structural refusals over the block:

| Condition | Refusal |
|---|---|
| A key not in the table | `unknown-field`, naming the key and the legal set |
| A key twice | `duplicate-field`, naming the key and both lines |
| A key out of canonical order | `field-out-of-order`, naming the canonical order |
| A body line that is not `key: value` (including a bare key with no `: `) | `malformed-line` |
| The fence never closes | `unterminated-block` |
| Empty block body | `missing-required-field` naming `name` |
| Two blocks with the same `name` | `duplicate-mount-name`, naming both lines |
| Two blocks with `primary: true` | `duplicate-primary-designation`, naming both lines (§5.1a) |
| An `alias` equal to any block's `name` or another block's `alias` | `alias-shadows-name`, naming both lines (§5.1b) |

**Blank lines and comment lines are refused** as `malformed-line`.

**`duplicate-mount-name` is in scope and `path` collision is not.** Name uniqueness is decidable
from the bytes alone. Same-path is decidable only after canonicalization (symlinks, trailing slashes,
`..`), and **implementation owns the mount-path law**: canonicalize at bind, inherit
`workspace::deny_reason`, refuse equal-or-nested mounts. The parser never compares paths lexically.
The bind step — where a declared mount becomes a bound root — is the one owner of "same path".

### 5.1a `primary:` — the declared-primary designation (v1-additive)

An optional `primary: true` line designates its mount as the **primary root**: the one tree a host's
single-root consumers anchor on, and where a host daemon writes. It need not be a vault. The role
binds hosts — change feed, watch loop, journal placement — and their rule set lives with the host.
The engine parses the designation, refuses its illegal shapes (§5.1), and reports it verbatim on the
`mounts` wire row (`wire-contract.md` §A.5) and both config faces. **The engine never acts on the
designation.**

- The value is the literal `true` and nothing else: absence is the only "not primary" spelling, so
  `primary: false` would mint a second spelling for one fact.
- Two designations refuse the whole table (`duplicate-primary-designation`, the
  `duplicate-mount-name` class). The designation is declared, never derived: the parser never picks
  between claimants, and where it is absent no consumer may fall back to `mounts[0]`, to the only
  vault, or to any other derivation.

`primary:` is **v1-additive** — optional and new, so adding it amends v1 rather than bumping the
version (§12, boundary 2). Mount blocks are closed-schema: only the fields in §5.1's table are legal,
so `primry: true` refuses as `unknown-field` at parse. That closes §4's silent-typo hazard.

### 5.1b `alias:` — the second lookup spelling (v1-additive)

An optional `alias:` line gives its mount a **second name callers may spell**. A skill, a doc or a
daemon can then hard-code one constant — `sessions:` — on a machine that names that tree something
else. The engine itself bakes in no root names (the no-baked-names law, `laws.md`), so the mapping
lives in one line of the user's own config.

```meridian-mount
name: field-notes-sessions
path: «local-path»
vault: field-notes-sessions
alias: sessions
```

> **The lookup order is `name` first, then `alias`.** A root whose `name` already is the constant
> needs **no alias line**: no default, no fallback, no special case.

**`primary:` is not consulted.** With no mount named or aliased `sessions`, lookup falls to the
implicit default mount if that binds (§5.1c). If the default is not scaffolded, `sessions:` refuses
as an unbound root and the refusal teaches the fix:

```text
declare `alias: sessions` on the mount that holds that tree
```

**An alias is a lookup spelling, never a stored one.** Receipts, pins, `mint {…}` paths, `sub` rows
and every canonical `root:path` a door echoes carry the mount's `name`. So `mrd resolve sessions:x`
answers `root: field-notes-sessions (alias sessions)`, `ref: field-notes-sessions:x`: the alias
appears only where it explains the resolution. Address law and the resolution order live in
`address-grammar.md` §4.6a.

**Uniqueness is table-level and refuses the whole file.** An alias equal to any mount's `name` is
`alias-shadows-name` — including a name declared later in the file, and its own mount's name — and so
is an alias equal to another mount's `alias`. That refusal carries §8.3's no-partial-load clause.
`alias` is likewise v1-additive and closed-schema: `alais: sessions` refuses as `unknown-field`.

### 5.1c The implicit default `sessions` mount (v1-additive)

This section answers the machine that has mapped nothing: a fresh host needs a `sessions` tree
before anyone has authored a config. When no mount is **named or aliased `sessions`**, the bound
table gains one implicit mount:

```text
name: sessions
path: $HOME/.local/share/ucc/sessions
```

The no-baked-names law (`laws.md`) permits such a directory to be *"defaulted in code at most"* — the
user's declaration is the answer and the constant the fallback, in the `preset::DEFAULT_ROOT_RECORD`
shape.

- **Declared wins, always.** A mount named or aliased `sessions` suppresses the implicit mount,
  whatever its path; so does one whose path equals, contains, or is inside the default path.
  The INV-2/INV-4 checks run against the bound declared table, with the implicit candidate second.
- **It appears only when it binds.** Same per-entry checks as a declared block: canonicalize, deny
  ceiling, uniqueness and nesting, the root's own declaration naming `sessions` (§4). Anything short
  of `bound` suppresses it silently — no grey row, no refusal, no changed exit code (§2.2 state A).
- **Nothing else is defaulted.** No `primary:` (declared-only, §5.1a), no `vault:`, no `alias:`, no
  `pin:`.
- **Scaffolding is explicit**; effects live in verbs. The engine creates neither the directory nor its
  declaration: `mkdir -p ~/.local/share/ucc/sessions && mrd init ~/.local/share/ucc/sessions --name sessions`.
  The unbound-`sessions:` refusal teaches it (`address-grammar.md` §4.6a).
- **A refusing declared table still refuses whole.** The default enters only a clean-loading table,
  state A included: absent config plus scaffolded default yields a one-row table.
- **Faces.** `mrd config` prints `(implicit default)`, `--json` carries `"implicit": true`; the
  `mounts` wire row keeps its shape, an implicit row riding as a real bound root
  (`wire-contract.md` §A.5).

### 5.2 The canonical root-name charset

This is the charset a mount's `name` and `alias` must use (§5.1).

```
name ::= lower ( lower | "-" )* lower | lower
lower ::= [a-z0-9]
```

A name is one or more characters from `[a-z0-9-]`, never starting or ending with `-`, never empty.
Maximum length 64 bytes.

**The charset is derived, not chosen.** A root name travels inside stored content: addresses
`[root:]path[#selector]`, lock `ref:`/`objects:` keys, `obsidian://` vault parameters. So the legal
charset is **the complement of the address grammar's operator set**: `:` `#` `@` `/` `.` `%` and
whitespace are excluded, because each already carries meaning in an address. **No legal name can
ever collide with an address operator.** Case is folded out because names travel through URIs and
case-insensitive filesystems. `_` is excluded to match the charset guard that refuses underscore ids
at every mint position (`crates/testsuite/data/charset-guard/discrimination.json`).

**This is a floor for the address grammar, not a ceiling.** `address-grammar.md` may narrow what a
`root:` prefix accepts, never widen it past this charset: a name outside it cannot be *bound*, so it
could never resolve.

### 5.3 `pin:` — mount-as-claim

A mount entry may pin the root it declares, e.g. that root's entry-page fingerprint. `~/MERIDIAN.md`
cannot itself be attested (§9), so a mount's pin is **the sole mechanism by which the mount table's
own integrity is checkable.**

> `pin:` carries a fingerprint CID-token: four `.`-separated non-empty fields,
> `version.codec.hashfn.digest`. It is well-formed iff `model::fingerprint::parse_fingerprint`
> returns `Some` (`crates/model/src/fingerprint.rs:137`).

**Parse is codec-agnostic on purpose**: tokens minted by newer codecs or hash-fns still parse, and
whether this build can verify one is `verify_content`'s question
(`crates/model/src/fingerprint.rs:68-70`). So the schema constrains the **token shape only**, not the
codec — a plain-folder root's pin grain is the file, a vault root's a parsed span (§12, boundary 2).

**What the pin's target is, and how the claim is checked, is implementation's** (§12, boundary 2). A
build that cannot verify a pin must say so; it must never treat that pin as verified. *Outside
sight never renders as verified*, and under grey-exit-1 a grey refuses on exit 1 with its own reason
word.

## 6. The `meridian-tool` block grammar

`MERIDIAN.md` also carries declarations for tooling built on top: agent-facing efficiency layers and
imperative user-facing tools. **Nothing owns tool semantics yet, and tag-based mounting is still
open**, so this schema specifies grammar and posture, not semantics.

```meridian-tool
name: llm-wiki
kind: skill
config:
 entry: LLM_WIKI.md
 vault: field-notes
```

| # | Field | Type | Required | Refusal on violation |
|---|---|---|---|---|
| 1 | `name` | §5.2's charset | **yes** | absent → `missing-required-field`; bad charset → `bad-value` |
| 2 | `kind` | non-empty `[a-z0-9-]` token | **yes** | absent → `missing-required-field`; empty or bad charset → `bad-value` |
| 3 | `config:` | marker line introducing an opaque payload | no | see below |

Structural rules are §5.1's, with two additions:

- `config:` is a **bare marker line** (no value). Every following line until the closing fence is the
 **payload**, and must be indented by at least one space. A non-indented line after `config:` →
 `malformed-line`. The payload's last line ends the block.
- Two blocks declaring the same `name` → `duplicate-tool-name`.

### 6.1 The payload is engine-opaque

**The engine validates that the payload is present and indented; it never interprets a byte of it.**
The payload belongs to the tool the `kind` names.

A declaration for a tool this machine has not installed is not a *broken* config; refusing it would
make a config invalid by *removing* a tool. Same posture as the wire's tolerant-code law —
*"Clients treat unrecognized codes as `recovery`-dispatched"* (`crates/wire/src/lib.rs:1350-1353`) —
and `laws.md` § Additivity.

**Rejected alternative:** a closed set of tool kinds in v1 — v1 owns zero kinds, so every
declaration would refuse.

**What is not deferred:** `name`, `kind`, uniqueness and the block's structure are parsed strictly; a
malformed *declaration* still fails loud. The opacity is of the payload's meaning, never the block's
shape.

## 6a. The `^config` value block — user data, address-reached (v1-additive)

**The rule:** `mrd config get` finds the `^config` block in the `MERIDIAN.md` file; the block is a
starlark block whose `config` function returns the config; and the config can be anything — it is
not limited. So `MERIDIAN.md` carries, beside the mount table, whatever machine-local values its
tools need. The surface is **one block, addressed by the block id `^config`**. The example below
shows its fence lines as literal text, so it is inert here:

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

The `^config` line sits on its own line below the closing fence: the Obsidian own-line form. Host
widening (`anchor_host_span`) attaches that line to the fence itself, so the id keys the code block
and not an empty paragraph.

| Fact | Law |
|---|---|
| Address | the block id `^config` — `Ref::anchor("config")`, the mint-plane lookup (`crates/model/src/lib.rs`, `resolve`) |
| Fence language | `starlark`, classified by `run::fence::classify` — §3.1's first-token rule |
| Entry | a zero-argument `config()`; its return value is the config |
| Return type | **anything the sandbox can serialize** — mapping, list, string, number, bool, `None`. No schema, no key whitelist, no required key |
| Reader | `mrd config get [KEY]` — bare prints the whole value, `KEY` a dot-path to one member (§6a.3) |
| Evaluation | the effect kernel's sealed evaluator, standard globals only — no effect constructors, `load` disabled, `EvalLimits::default()` bounds (`effects::eval_value`). Reaches no file, no network, no process |

### 6a.1 Why this does not touch the strict parse

§3 scopes the strict parse to the frontmatter plus the `meridian-*` blocks; every other byte is
prose. **A ```` ```starlark ```` block is prose to that scan and stays prose**, so a broken
`config()` cannot cost this machine its mount table.

**The mount plane is scanned, the config block is addressed.** Nothing hunts for a starlark block;
one verb resolves one block id on demand. So §3.1's namespace argument does not apply: `^config` is a
page address, not a third engine block-language, and the anchor grammar (`[A-Za-z0-9-]`,
`syntax::is_block_id`) admits it.

### 6a.2 The refusal ladder

`mrd config get` either prints a value or refuses. Each rung names what is wrong, where, and
the fix; none prints an empty line and exits 0 (§8's posture):

| Condition | Refusal |
|---|---|
| no file resolved (state A) | no config to get; name the path and the block to add |
| no `^config` block | name the file and show the block's shape |
| two `^config` blocks | ambiguous — the mint plane never picks; name the count |
| `^config` does not key a fenced code block | name what it keyed |
| the fence is not `starlark` | name the language found and the one required |
| the source will not parse, faults, or exhausts the budget | the evaluator's own message, verbatim |
| no `config()` defined | name the entry the block owes |
| `config()` returned something unserializable (a function, a lambda) | name it — the config is data |
| a `KEY` segment asked of a non-mapping | name the type found and where the walk stopped |
| a `KEY` segment absent | name where it stopped and the keys that are there |

**The mount table's state is not a rung.** `mrd config get` never calls `bind()`, so an unbound or
missing root refuses `mrd config` (exit 1) and leaves `mrd config get` answering normally.

### 6a.3 `KEY` — the dot-path

A `KEY` addresses a member of the returned value: a dot-path resolved with **exact key first, then
the dot as a separator**, at every level:

1. Does this mapping have a member named by the whole remaining key? Then that member is the answer.
2. Otherwise split at the first `.`: the head must be a member, descend into it, repeat with the tail.

So no author-written key becomes unaddressable: a member literally named `a.b` stays addressable.
The first real config keys `repos_root` by wiki, because a repos root is a fact about a wiki, not a
machine:

    mrd config get repos_root.work-wiki      -> /path/to/work/repos
    mrd config get repos_root.field-notes      -> /path/to/home/repos
    mrd config get repos_root                -> the mapping, as JSON

### 6a.4 What is deliberately not specified

- **No key schema.** A future engine-read key inside `config()`
  reopens §4's misspelled-optional-key hazard and must state how it closes it.
- **One block, not many.** A second `^config` is ambiguous, not a merge; merging would need a
  precedence rule nobody has ruled on.
- **No list index.** A KEY segment addresses a mapping's member and nothing else; indexing a list
  would need a second grammar and a rule for a mapping whose key is `0`. Bare `mrd config get` prints
  the whole value for a caller that wants to walk it.
- **No wire op.** The CLI reads the block in the calling process, from the file that process's own
  chain resolves (`mrd config`'s own `answered by: this process`). A daemon serving its own
  `MERIDIAN.md` would answer about a different machine's config.

## 7. The self-hosting rev

### 7.1 The rev

`MERIDIAN.md` is parsed by the engine's own parser, so its rev needs **no new mechanism**:

> `config_rev` = the document root node's `node_rev` = `blake3(raw file bytes)[:16]`, 16 lowercase hex.

The root span is `0..raw.len`, its rev `node_rev(raw.as_bytes, &root_span)`
(`crates/model/src/lib.rs:204`, `:210`, `:310-311`) — the law the armed artifact's pinned `rev` already
uses for a rule page (§1.2 rule 3, `crates/policy/src/registration.rs`, `page_rev`).

`config_rev` is spelled `file_rev`, the wire's whole-page rev noun (`crates/wire/src/lib.rs:1199`);
**no new rev noun is minted for the config.**

### 7.2 The rev is computable where the config lives

`model::build(raw: String, nodes: Vec<syntax::DialectNode>)` (`crates/model/src/lib.rs:165`) is pure:
no workspace, no I/O, no git. So `config_rev` is computable for a file in `$HOME`, a **denied workspace
path** (`DenyReason::HomeDir`, `crates/workspace/src/lib.rs:305`) that can never be promoted into one.
The rev exists there; the *attestation plane* does not (§9).

### 7.3 What the rev is for

The requirement is *"the file itself carries a rev, so editing it out of band renders as ordinary
drift."* Precisely:

- **Reported.** Every surface publishing the loaded config reports its `config_rev`; an edit changes
  it.
- **No baseline inside the file.** A key declaring the config's own expected rev would be
  self-referential; none exists, none may be added.
- **Not a drift verdict.** No attestation baseline exists for `~/MERIDIAN.md` (§9).

**Drift that is a verdict is the mount pins' (§5.3), not the config's own rev's.** A mount's `pin` names
a root's entry page inside an attestable root, so `pin` against the live fingerprint is ordinary
machinery (`verify_rows`, `crates/policy/src/armed.rs`). It is this plane's only checkable drift
claim.

## 8. The refusal law

Every state-B refusal names **what is broken** and **where**, through a closed reason set.

### 8.1 The shape

Extend `crates/lock`'s error type, which carries a structured location
(`crates/lock/src/lib.rs:390`), not the INDEX's (§1.3 D-d):

```rust
Malformed { line: usize, reason: &'static str } // the shape to extend
```

This schema also requires:

1. **`line` is 1-based in the file**, not within the block as `lock::parse` numbers: a human editing
   `MERIDIAN.md` is looking at file lines. The block node carries its byte span, so converting costs
   an addition, not a new mechanism (`crates/lock/src/lib.rs:527-534`).
2. **The refusal names the config path**: `MERIDIAN_CONFIG` means the file may be anywhere.
3. **`reason` stays `&'static str` — a closed set, never free text**, so the reason word is testable
   (`D1_TEACHING_REFUSAL_EXEMPLAR`, `crates/model/src/selector.rs:569`).

### 8.1a Which line a refusal points at

Which line a refusal names depends on the fault. Three cases, exhaustive:

| The fault is about | The line is | Cases |
|---|---|---|
| Something **present** | its own line | `wrong-type-value`, `unsupported-version`, `bad-value`, `unknown-field`, `field-out-of-order`, `malformed-line`, `frontmatter-unparseable` |
| Something **absent** | the opening line of the construct that should have carried it — the block's opening fence for a block field, **line 1** for a frontmatter key or a frontmatter fence fault | `missing-required-field`, `missing-required-key`, `no-frontmatter`, `unterminated-block` |
| A **duplicate** | the **second** occurrence, and the message names the first | `duplicate-field`, `duplicate-mount-name`, `duplicate-tool-name`, `duplicate-primary-designation` |

State C and `home-unresolvable` carry the config path and no line.

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

A refusal carries the reason word, the config path, the 1-based file line, what was found, and what is
legal (templates: `crates/policy/src/binding.rs:148-156`, `crates/model/src/selector.rs:569`):

```
refused: ~/MERIDIAN.md line 14: unknown field `paths` in a meridian-mount block —
legal fields are name, path, primary, vault, pin (in that order). No mount
table was loaded; the config is not partially applied. Fix: remove the line or
spell the field you meant.
```

**Three clauses are mandatory:** the line; **"no mount table was loaded"** (the no-partial-load law);
and a `Fix:` naming the legal form — `refuse(message, passing)` cannot be called without the
passing scenario (`crates/policy/src/check_eval.rs:502-513`).

### 8.4 First refusal wins

A malformed config produces **exactly one** refusal — the first, in file order: the file does not
half-load. `lock::parse` (`crates/lock/src/lib.rs:543`) and
`parse_artifact` (`crates/policy/src/armed.rs:996`) return on the first fault too.

## 9. The stated limit — `~/MERIDIAN.md` cannot be attested

`$HOME` is not a git repo — no receipt journal, no merkle hash domain — and is a **denied workspace
path** (`DenyReason::HomeDir`, `crates/workspace/src/lib.rs:305`), never promotable into one.
Therefore:

- The single authority for every cross-root ref is the one artifact the attestation plane cannot
  attest, so drift on it is not *"an ordinary red on the ordinary machinery"*.
- The config's own rev is a **reported number**, never a verdict, and the mount pins are the mitigation
  (§7.3, §5.3).

**The residual that mitigation does not close.** A pin protects the root its mount declares, not the
mount table's *membership*: **deleting a mount block deletes its own pin along with it.** Under
grey-exit-1 an unmounted root renders grey and the fence refuses on exit 1. So dropping a mount turns
a red into a grey that must be `--force`d past, which is why grey rules to exit 1 rather than 0.
Bounded and visible, but real: **the fence's only bypass is an edit to exactly this file, which
cannot be attested.** No v1 mechanism closes it.

## 10. The fixture corpus

`crates/testsuite/data/meridian-md/` — the corpus the implementation consumes.

| Path | Carries |
|---|---|
| `README.md` | the corpus law: what each case must state, and the escalation clause |
| `cases.json` | **every case paired with its required outcome** — the manifest is the pairing |
| `corpus/` | well-formed configs (the acceptances) |
| `refusals/` | malformed configs, one per malformed class (the refusals) |

Cases with no file — state A, state C, the env-var cases — **cannot be fixtures** (a file cannot express
its own absence); they live in `cases.json` with `"fixture": null` and an `env` block.

`cases.json` follows the shipped probe-pack convention
(`crates/testsuite/data/harness/p2-walk-probes.json`): each case carries `id`, `fixture`, `env`,
`expect`, `law`, and `kills`. **`kills` states what wrong implementation the case rules out** — the
anti-vacuity discipline.

## 11. Rejected alternatives

- **Mount table in frontmatter (`mounts:` as a YAML list)** — no prose beside an entry, and the
  frontmatter parser's error type carries no structured location (§1.3 D-d).
- **One `meridian-mount` block for all entries** — no prose beside each mount, a mount's pin (§5.3)
  becomes a row field, and refusals lose the coarse address (*which* block).
- **INDEX-style middot checklist rows** — §1.3 D-b: engine-generated grammar; a hand-written ` · ` is
  invisible in an editor.
- **A closed set of tool kinds in v1** — §6.1: v1 owns zero kinds, so the grammar would admit nothing.
- **`MERIDIAN.md` authoritative for canonical root names** — contradicts *"MERIDIAN.md binds, it doesn't
  baptize"* (`address-grammar.md` § 3, INV-5); `name:` in a mount block is a **binding**;
  implementation checks it against the root's own declaration.
- **A declared `expected_rev:` key** — self-referential and unsatisfiable (§7.3).
- **Project-local walk-up discovery** — not rejected but **deferred** (§0); not built here.

## 12. Boundaries flagged, not assumed

**Boundary 1 — the root-name charset, shared with the address grammar.** §5.2 fixes the charset a name
may use *in the file*; `address-grammar.md`, which owns the address grammar, must accept exactly those
names. **A consequence the address grammar inherits:** because a name cannot contain `:`, the
`sessions:notes.md` prefix-vs-path ambiguity (`address-grammar.md` § 4) is decidable as *"is the
pre-colon token a **bound** mount name?"* — which needs the mount table at resolve time (D4a's injection
into `model::CorpusIndex::resolve_ref`). **Not ruled here.**

**Boundary 2 — the pin's target, shared with implementation.** §5.3 fixes what a well-formed `pin` token
*is*, not **which file** a mount's pin names (the root's self-declaration entry page — the D7 seeding
question) nor **what bytes** it covers (whole file for a plain-folder root, a parsed span for a vault
root). Naming the target with a second field is a v1 schema amendment, not a v2 bump: optional and new, so
§4's rule about optional engine-read keys applies.

Neither blocks implementation: §2, §3, §4, §7 and §8 are complete without them.
