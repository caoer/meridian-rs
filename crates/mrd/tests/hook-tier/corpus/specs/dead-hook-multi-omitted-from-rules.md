---
corpus_test: a-later-silent-hook-is-still-dead
corpus: ../tree
counterfactual: false
---

Two loaded HOOKs, an EMPTY `rules` fence, and a case that fires only the first. The
later, silent HOOK must still be reported dead — liveness comes from the loaded
convention set, never from what an author remembered to repeat in the fence.

```conventions
../../conventions/task-status-notify
../../conventions/never-fires
```

```rules
```

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"task-status-notify"}
```
