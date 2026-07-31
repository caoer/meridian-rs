---
corpus_test: deliberately-cyclic-hooks
corpus: ../tree
counterfactual: true
---

```conventions
../../conventions/cycle-alpha
../../conventions/cycle-beta
```

```rules
cycle-alpha
cycle-beta
```

```case
{"name":"enter-alpha","doc":"tasks/card.md","set":{"status":"alpha"},"expect":"cycle-alpha"}
```
