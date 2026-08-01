---
corpus_test: narrowed-report-counterfactual-mode
rule: ../../rules/narrowed-md.md
corpus: ../tree
counterfactual: true
---

```rules
narrowed-md
```

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"narrowed-md"}
```
