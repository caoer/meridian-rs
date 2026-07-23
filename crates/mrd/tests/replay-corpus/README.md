# Synthetic replay corpus

The committed, public-safe fixture the corpus-replay harness (`mrd rules
replay`) runs against — in the `rules-replay` CI lane (nightly + on changes to
`crates/mrd`/`crates/effects`) and in `crates/mrd/tests/replay.rs`.

## Layout

- `rules/` — the `.star` rule set. Each file's stem is the rule id.
- `snapshots/` — ordered corpus states (`00-init`, `01-review`, `02-active`).
  An empty baseline precedes the first, so the first state's files are `created`
  events; consecutive states are diffed per file to synthesize the event stream.

## The rules encode the regression gate — do not "fix" the dead one

- `dead_priority` keys on a `priority` frontmatter field **no document here ever
  carries**. It runs on every event and NEVER fires — that is the point. The CI
  lane asserts it is reported under "Dead rules (never fired)". If a change makes
  it fire, the lane goes red (a real regression: dead-rule detection broke, or
  the fixture drifted).
- `live_status` fires whenever `status` changes — exactly **3×** over the corpus
  (doc.md creation, then two `status` transitions), emitting `proto.notice`.
- `live_section` fires whenever a section's content changes — exactly **3×**
  (doc.md + notes/log.md creation, then the notes/log.md body edit), emitting
  `proto.warn`.

Each snapshot transition changes exactly one thing per file (a frontmatter field
OR a body line, never both at once) so the synthesized `sections_changed` /
`fields_changed` attribution stays predictable.

## The other lane — a real corpus

`scripts/replay-standing.sh` points the same tool at a real workspace's git
history (e.g. the field-notes tree) on a schedule; see its header for cron /
launchd wiring. The report body is deterministic; only the artifact filename
carries the run time.
