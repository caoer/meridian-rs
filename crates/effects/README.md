# effects — the effect kernel

Pure Starlark evaluation: **rules in, effect descriptors out. Zero I/O, zero
integration. Advisory-only.** The engine's organ for the `on_change` hook, in
its own crate (decisions/0003, decision #7 rename-and-demote). Runs parallel to
the resident-daemon spine and touches nothing it owns — no splice choke point,
no wire, no daemon — and never will.

```rust
use effects::{eval, ChangeEvent, Rule};

let rules = vec![Rule::new("status-teach", r#"
def on_change(event):
    if "status" in event.fields_changed:
        notice(message = "status changed — valid values: todo, review, done.")
"#)];

let event = ChangeEvent {
    file: "tasks/t.md".into(),
    sections_changed: vec![],
    fields_changed: vec!["status".into()],
    fingerprint_before: "aaa".into(),
    fingerprint_after: "bbb".into(),
    depth: 0,
};

let effects = eval(&rules, &event).unwrap();   // -> Vec<Effect>, deterministic
```

`eval(rules, event)` is a **pure function of `(rules, event)`**: deterministic,
fuel-limited, depth-capped, panic-safe. The same input yields the byte-identical
effect set across runs and thread counts — cursor replay (0003 §4) depends on it.

## The rule surface

A rule is a fenced-free Starlark source defining one hook:

```python
def on_change(event):
    ...
```

`event` (0003 §3 — one hook, section-level payload):

| field | type | meaning |
|---|---|---|
| `event.file` | str | the changed file's workspace path |
| `event.sections_changed` | list[str] | section paths whose content changed |
| `event.fields_changed` | list[str] | frontmatter field names whose value changed |
| `event.fingerprint_before` / `_after` | str | the diff fingerprints |
| `event.depth` | int | cascade depth (0 = user-originated) |

Effect-descriptor constructors (the ENTIRE builtin surface — **no I/O builtins**,
every argument named):

| constructor | kind | domain |
|---|---|---|
| `set_field(field, value, message=?)` | `md.set_field` | md |
| `append_section(section, content, message=?)` | `md.append_section` | md |
| `refresh_view(view)` | `daemon.refresh_view` | daemon |
| `send(to, message)` | `proto.send` | proto |
| `remind(message, at=?)` | `proto.remind` | proto |
| `ask(message, options=?)` | `proto.ask` | proto |
| `notice(message)` | `proto.notice` | proto |
| `warn(message, section=?)` | `proto.warn` | proto |

A rule reaches these plus the Starlark standard library and nothing else: no
file, net, os, clock, random, `print`, or `load`. An effect is inert data a
consumer executes; the kernel never applies one.

> Descriptor kinds and argument names are a **settled-direction sketch, not a
> freeze** — alignment with the fused stage-2 winner may rename them (0003 §
> reserved). Delete-don't-migrate makes that cheap; build no migration layer.

## Effect domains & capability routing (0003 §2, §5)

Effects are namespaced by executor: `md.*` (engine applies to the tree,
depth-capped), `daemon.*` (resident powers), `proto.*` (wire-client advisory
feedback). `eval` emits ALL effects; a consumer's `CapabilitySet` is the
downstream filter (`route(effects) -> (admitted, rejected)`), kept separate so
`eval` stays pure.

## Acknowledged limitation — cursor replay collapses transitions (0003 §4)

At-least-once delivery is fingerprint-cursor replay, not a queue. An outage
between a consumer's cursor and live replays the **net** diff: `todo→review→done`
re-emits as `todo→done`, and "entered review" **never fires**. Lost by design
under the advisory law. The escape: a rule that must record every transition
writes it INTO the tree at write time (`append_section` / an `md.*` effect) —
disk carries the history the wire never does.

## Determinism, fuel & safety

- **Deterministic** — rules run in slice order; effects carry a per-rule `seq`;
  Starlark is deterministic (insertion-ordered dicts, no clock/random).
- **Fuel** — `EvalLimits { fuel, mem, max_call_depth, max_depth, max_source_bytes }`.
  A runaway loop, recursion bomb, or huge allocation terminates with
  `EvalError::Budget` — never hangs. Parse-nesting is depth-bounded before parsing
  (issue #66 guard); evaluation runs on a dedicated large-stack thread so the
  parser cannot overflow the native stack and abort.
- **Hostile input** — malformed source, wrong-typed args, forgery attempts all
  resolve to a typed `EvalError`; provenance (`rule_id`) is kernel-stamped and
  cannot be forged.

## Tests (decisions/0003 § Testing methodology)

| Layer | Tool | Run |
|---|---|---|
| Golden descriptor snapshots | insta | `cargo test -p effects --test golden` |
| Invariants (determinism, keys) | proptest | `cargo test -p effects --test determinism` |
| Fuel / bombs | — | `cargo test -p effects --test fuel` |
| Hostility / purity / cascade / capability | — | `cargo test -p effects` |
| Fuzz (source + event) | cargo-fuzz | `cargo +nightly fuzz run eval_source` (see `fuzz/`) |
| Mutation (tests bite) | cargo-mutants | `cargo mutants -p effects --test-workspace=false` |
| Coverage | cargo-llvm-cov | `cargo llvm-cov -p effects` |

The stable `cargo test` path (golden + proptest + fuzz-shaped `robustness.rs`)
runs the whole suite without any extra cargo subcommand; the fuzz targets and
mutation/coverage tools are the deeper CI gates.

## Not in this crate

**Dead — never coming here (decision #7 rename-and-demote):** the splice-point
carve-in. This kernel is advisory-only by charter; it will never gain a
correctness path.

**Elsewhere or reserved (0003 § reserved):** the wire `effects[]` field, the
executor, cursor replay (consumer-side), and the `mrd rules test` /
corpus-replay product surfaces. The kernel exposes the pure `eval` primitive
those build on.
