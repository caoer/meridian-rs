---
corpus_test: terminating-convergent-cascade-depth-12
corpus: ../tree
counterfactual: true
---

The same terminating convergent cascade seeded at the head of the chain: twelve
generations deep, `2^13 − 2 = 8190` causal paths over thirteen states. Depth 8 is the
measured defect; this one proves the repair is not a fuel bump that a slightly deeper
good convention set would breach again.

```rule-pages
../../rules/chain-a.md
../../rules/chain-b.md
```

```case
{"name":"seed-depth-12","doc":"tasks/cascade.md","set":{"s1":"go"},"expect":["chain-a","chain-b"]}
```
