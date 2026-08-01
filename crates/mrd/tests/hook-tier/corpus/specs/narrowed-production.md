---
corpus_test: narrowed-report-production-mode
rule: ../../rules/narrowed-md.md
corpus: ../tree
---

```rules
narrowed-md
```

```case
{"name":"move-to-review","doc":"tasks/card.md","set":{"status":"review"},"expect":"narrowed-md"}
```
