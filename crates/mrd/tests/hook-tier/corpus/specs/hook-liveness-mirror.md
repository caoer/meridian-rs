---
corpus_test: hook-liveness-cannot-vouch-for-a-check-citation
corpus: ../tree
---

The MIRROR of `citation-collision`: the same two pages, and the same collided name,
running the other way. Here the HOOK `task-status-notify` FIRES and the CHECK that
cites `task-status-notify` as its passing case never does. One merged liveness list
lets the live HOOK answer for the dead citation; two typed namespaces cannot.

```rule-pages
../../rules/citation-collision.md
../../rules/task-status-notify.md
```

```rules
task-status-notify
```

```case
{"name":"mirror","doc":"tasks/card.md","set":{"status":"review"},"expect":"task-status-notify"}
```
