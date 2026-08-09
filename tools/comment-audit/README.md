# comment-audit

Extracts every source comment in the workspace so a reader can ask which ones
report a **finding** (a fact about the product's state, whose audience is
outside the file) rather than **explain the code** under the cursor.

The script answers the extraction question, never the classification one. It
emits the exhaustive corpus plus a deliberately high-recall candidate filter;
judgement stays with the reader.

## Why a lexer and not grep

`grep '//'` counts URLs inside string literals as comments and splits a
paragraph into unrelated lines. `sweep.py` runs a small Rust lexer that skips
string, raw-string, and char-literal bodies, handles nested `/* */`, and
collapses contiguous comment lines into one **block** — because a finding is
written as a paragraph, and its line reference should name the paragraph.

Guard cases are executable:

```sh
tools/comment-audit/sweep.py selftest    # 9 lexer cases grep gets wrong
```

## Rev pinning

Line references are only meaningful against a stated commit. Point `--root` at
a worktree checked out at the rev you are auditing; every emission carries that
rev in its `_meta` header.

```sh
git worktree add --detach /tmp/pin <rev>
tools/comment-audit/sweep.py blame --root /tmp/pin > candidates.jsonl
head -1 candidates.jsonl        # {"rev": "...", "total_blocks": ..., ...}
```

## Modes

| Mode | Emits |
|---|---|
| `blocks` | every comment block, JSONL |
| `candidates` | blocks matching the finding-language lexicon, JSONL |
| `blame` | `candidates` + the introducing and last-touching commit per block |
| `counts` | per-crate and per-trigger totals, JSON |
| `residue` | deterministic random sample of NON-candidates (`--sample`, `--seed`) — measures what the filter missed |
| `triggers` | the lexicon itself, JSON |
| `selftest` | lexer guard cases |

Filters: `--crate NAME` (repeatable), `--min-words N`.

## The lexicon

Seven trigger families — reachability, not-built, time-indexed, deferred,
session-dependent, migration, flagged — printed by `sweep.py triggers`.

Tuned for **recall, not precision**: a reader discards a false positive in
seconds, while a false negative never reaches the inventory at all. At
`a96070a1` it selects 2,104 of 10,124 blocks (21%). The `residue` mode exists
to keep that claim honest — sample the 79% it rejected and measure the miss
rate rather than asserting one.

## Reproducing an inventory

```sh
tools/comment-audit/sweep.py counts   --root /tmp/pin           # denominators
tools/comment-audit/sweep.py blame    --root /tmp/pin           # dated candidates
tools/comment-audit/sweep.py residue  --root /tmp/pin --sample 100 --seed 20260801
```

Output is stable for a given (rev, seed): `block_id` is
`path:line:sha256(text)[:12]`, so a block survives being moved within its file
and is detectable when its text changes.
