---
corpus_test: loaded-hook-is-always-a-liveness-subject
convention: ../../conventions/task-status-notify
corpus: ../tree
---

The `rules` fence deliberately omits `task-status-notify`.

```rules
```

```case
{"name":"never-moves-to-review","doc":"tasks/card.md","set":{"reviewer":"other"},"expect":"pass"}
```
