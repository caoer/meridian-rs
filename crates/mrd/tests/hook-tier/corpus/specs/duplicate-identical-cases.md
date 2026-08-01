---
corpus_test: duplicate-identical-cases-are-not-a-cycle
rule: ../../rules/cycle-alpha.md
corpus: ../tree
counterfactual: true
---

```case
{"name":"first-enter-alpha","doc":"tasks/card.md","set":{"status":"alpha"},"expect":"cycle-alpha"}
```

```case
{"name":"duplicate-enter-alpha","doc":"tasks/card.md","set":{"status":"alpha"},"expect":"cycle-alpha"}
```
