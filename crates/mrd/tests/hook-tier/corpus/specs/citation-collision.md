---
corpus_test: check-citation-cannot-vouch-for-a-hook
corpus: ../tree
---

A CHECK refusal cites `task-status-notify`, which is also the sibling HOOK's slug.
The CHECK expectation must still match, and the same-named silent HOOK must still be
reported dead.

```conventions
../../conventions/citation-collision
../../conventions/task-status-notify
```

```rules
task-status-notify
```

```case
{"name":"collide","doc":"tasks/card.md","set":{"status":"collide"},"expect":"task-status-notify"}
```
