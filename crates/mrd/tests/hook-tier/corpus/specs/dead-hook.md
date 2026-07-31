---
corpus_test: dead-hook
convention: ../../conventions/task-status-notify
corpus: ../tree
---

```rules
task-status-notify
```

```case
{"name":"never-moves-to-review","doc":"tasks/card.md","set":{"reviewer":"other"},"expect":"pass"}
```
