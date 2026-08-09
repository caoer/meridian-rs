---
corpus_test: divergent-cascade-still-exhausts-fuel
corpus: ../tree
counterfactual: true
---

The direction-of-failure rail. Two peers append DIFFERENT lines to the same section,
so no two causal paths ever meet and the document grows without bound. There is no
recurrence to catch and no fixed point to reach: the graph fuel is the only thing
that stops the proof, and it must keep stopping it.

A repair that collapses the terminating convergent cascade must leave this one
exhausted — a pre-arming gate that certified this set would arm a cascade that never
settles.

```rule-pages
../../rules/diverge-a.md
../../rules/diverge-b.md
```

```case
{"name":"open-the-fan","doc":"tasks/card.md","set":{"trigger":"go"},"expect":["diverge-a","diverge-b"]}
```
