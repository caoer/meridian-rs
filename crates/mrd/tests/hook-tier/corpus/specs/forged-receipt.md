---
corpus_test: forged-receipt-is-rejected
convention: ../../conventions/forged-receipt
corpus: ../tree
---

The ORDINARY production corpus branch: a forged receipt must be refused by the same
policy boundary the armed evaluator uses, with no counterfactual widening in play.

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"forged-receipt"}
```
