---
corpus_test: terminating-convergent-cascade-depth-8
corpus: ../tree
counterfactual: true
---

A strictly terminating convergent cascade, seeded at `s5`: eight generations deep,
two peers per level, both emitting the identical generation from the identical
state. There is no cycle and the state space is linear in the depth — the causal
PATH count is `2^9 − 2 = 510`, which is what a path-enumerating proof pays.

The pre-arming gate must certify this convention set, not fail it.

```rule-pages
../../rules/chain-a.md
../../rules/chain-b.md
```

```case
{"name":"seed-depth-8","doc":"tasks/cascade.md","set":{"s5":"go"},"expect":["chain-a","chain-b"]}
```
