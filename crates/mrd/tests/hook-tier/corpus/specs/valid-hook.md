---
corpus_test: valid-proto-send-hook
rule: ../../rules/task-status-notify.md
corpus: ../tree
---

```rules
task-status-notify
```

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"task-status-notify"}
```

```case
{"name":"other-key-is-silent","doc":"tasks/card.md","set":{"reviewer":"other"},"expect":"pass"}
```
