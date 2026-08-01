---
corpus_test: forged-receipt-is-rejected-counterfactually-too
convention: ../../conventions/forged-receipt
corpus: ../tree
counterfactual: true
---

The counterfactual twin of `forged-receipt.md`. Widening which caps a declaration may
carry must not weaken receipt validation by one byte.

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"forged-receipt"}
```
