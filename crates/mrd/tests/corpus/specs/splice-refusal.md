---
corpus_test: a-case-whose-production-edit-the-engine-refuses
rule: ../rules/no-smuggled-heading.md
corpus: ../governed
---

# splice-refusal (fixture spec, exit 2)

The case declares a `match` edit whose `old` occurs ZERO times in the corpus
document, so the production splice refuses `no_match` and the harness cannot
derive an "after" state to check the rule against. That is a bad SPEC, not a
finding about the rule — exit 2, and the operator is owed the engine's own
sentence about what it refused and how to repair it.

Gated by `corpus_suite::a_refused_production_edit_carries_the_engines_sentence`.

```rules
structural-write
```

```case
{ "name": "old-that-does-not-occur",
  "doc": "tasks/gate-check.md",
  "actor": "agent:alice",
  "edits": [
    { "target": {"hpath": [{"h": "Task: gate-check"}, {"h": "Quality Gates"}]},
      "edit": {"match": {"old": "a string that is nowhere in this document at all", "new": "x"}} }
  ],
  "expect": "pass" }
```
