---
corpus_test: one-generation-is-one-batch
corpus: ../tree
counterfactual: true
---

One emission carries a frontmatter descriptor AND a section descriptor. Production
applies it as ONE atomic batch — and since the sub-node-grain ruling (2026-08-14)
that batch's synthesized event names BOTH its addressable identities (the changed
key and the changed section), so the watcher fires on production truth: one case,
one event, one real downstream edge. The one-batch law still binds the proof: a
harness that split the generation into independent writes would derive TWO events
and fire the watcher twice — the single fire is the pin.

```rule-pages
../../rules/mixed-emitter.md
../../rules/mixed-watcher.md
```

```case
{"name":"pull-the-trigger","doc":"tasks/card.md","set":{"trigger":"go"},"expect":"mixed-emitter"}
```
