---
corpus_test: dead-hook
rule: ../../rules/task-status-notify.md
corpus: ../tree
---

```rules
task-status-notify
```

```case
{"name":"never-moves-to-review","doc":"tasks/card.md","set":{"reviewer":"other"},"expect":"pass"}
```
