---
corpus_test: one-generation-is-one-batch
corpus: ../tree
counterfactual: true
---

One emission carries a frontmatter descriptor AND a section descriptor. Production
applies it as ONE atomic batch whose synthesized event names no addressable
identity, so the watcher cannot fire and no downstream edge exists. A proof that
split the generation into independent writes would derive two rich single-edit
events and fabricate exactly that edge.

```rule-pages
../../rules/mixed-emitter.md
../../rules/mixed-watcher.md
```

```case
{"name":"pull-the-trigger","doc":"tasks/card.md","set":{"trigger":"go"},"expect":"mixed-emitter"}
```
