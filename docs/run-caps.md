# Run-plane capabilities (S1)

The capability law of `mrd run` (verdict ruling 3, plan decision #15):
**deny-by-default**. An undeclared block is read-only — it can compute, but
no effect of its executes. This document states the resolution algorithm,
the ceiling semantics, and the `.meridian.toml` schema as shipped.

## The cap grammar

A capability is a namespaced string: `ns.name`, optionally target-scoped as
`ns.name:target`.

```
md.set_field            # any frontmatter field
md.set_field:status     # ONLY the `status` field — strictly narrower
md.append_section       # any section
```

A target-scoped cap is strictly narrower than its untargeted form; an
untargeted cap admits every target of its kind. The cap table is a
**string-keyed type in `crates/run`**, not the kernel's `BTreeSet<EffectKind>`
(decision #24) — the kernel enum cannot express target scoping.

## Declaration surfaces

1. **Explicit frontmatter** — beside the task binding:

   ```markdown
   ---
   task.fix-drift: "[[#^fix-1]]"
   task.fix-drift.caps: md.set_field:status, md.append_section
   ---
   ```

   A present-but-empty `caps` declaration is an EXPLICIT read-only grant,
   distinct from no declaration.

2. **`.meridian.toml` `[run.caps]` name-convention table** — pattern → cap
   list; a pattern is a literal task name or a trailing-`*` prefix glob:

   ```toml
   [run.caps]
   "fix-*"     = ["md.set_field", "md.append_section"]
   "fix-note"  = ["md.set_field:status"]   # longest pattern wins
   ```

   The most specific (longest) matching pattern applies. An absent file or
   section is the empty table; a malformed file is a loud typed error —
   an unreadable policy file never silently becomes "no policy".

## Resolution: grant, then ceilings

Precedence for the **grant**: explicit frontmatter > convention match >
none (deny-by-default, read-only).

Conventions **narrow only, never widen** — a matching convention over an
EXPLICIT grant acts as a ceiling: each granted cap survives as its meet with
the ceiling (untargeted ∩ targeted = the targeted form; incomparable caps
drop). Two ceilings exist:

1. the matching `[run.caps]` convention, over an explicit grant;
2. the **builtin read-only ceiling** — absolute, non-overridable — for task
   names matching `check-*` or `verify-*`.

Narrowing is never silent: every cap that did not survive intact is reported
in `narrowed[]` (and shown by `mrd run --list`).

## The bash refusal (and the `fix-*` carve-out)

A `check-*` / `verify-*` task carrying a **bash** fence refuses loudly at
load (verdict ruling 3): a read-only-by-convention name gets no exec.
`fix-*` is deliberately **not** in that list (ZT-ratified carve-out) — fix
blocks declare writes, and bash is exactly where they are wanted.

This refusal is a run-plane refusal: exit 1, not exit 2 — the invocation is
well-formed; the plane says no.

## Where caps bind: the choke point

Resolved caps are enforced at the executor choke point, **before any I/O**:
every `md.*` descriptor is validated as (kind, target) against the block's
effective set; one violation refuses the entire batch and nothing applies.
Both dispatch paths (starlark and the bash shim stream) pass through the
same validation — there is no second write path to grant around.

`--dry` shows the block's resolved caps **byte-identical to the choke-point
caps** (S14): what the dry run displays is exactly what the real run
enforces.

## Inputs are deny-by-default too

`task.<name>.args` / `task.<name>.env` declare the block's input contract.
The CLI supplies exactly the declared positional count, and every `--env`
key must be declared — an undeclared supplied key refuses (it also catches
`--env` typos loudly). Env **values** never enter any record; run records
carry sorted env key names only (S7).
