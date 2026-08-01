---
corpus_test: raw-md-cannot-bypass-canonical-validation
rule: ../../rules/raw-md.md
corpus: ../tree
counterfactual: true
---

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"raw-md"}
```
