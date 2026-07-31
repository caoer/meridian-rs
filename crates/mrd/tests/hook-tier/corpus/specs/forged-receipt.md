---
corpus_test: forged-receipt-is-rejected
convention: ../../conventions/forged-receipt
corpus: ../tree
counterfactual: true
---

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"forged-receipt"}
```
