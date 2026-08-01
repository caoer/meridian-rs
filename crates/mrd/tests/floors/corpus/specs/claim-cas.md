---
corpus_test: claim-cas-floor
rule: ../../rules/claim-cas.md
corpus: ../tree
---

# claim-cas floor — fire-where-expected

A contested claim (stealing an already-owned slot) fires `claim_cas`; claiming
an unclaimed slot passes. The one declared rule fires: exit 0.

```rules
uncontested-claim
```

```case
{ "name": "contested-claim", "doc": "tasks/claimed.md", "actor": "worker-b", "set": {"owner": "worker-b"}, "expect": "uncontested-claim" }
```

```case
{ "name": "uncontested-claim", "doc": "tasks/unclaimed.md", "actor": "worker-b", "set": {"owner": "worker-b"}, "expect": "pass" }
```
