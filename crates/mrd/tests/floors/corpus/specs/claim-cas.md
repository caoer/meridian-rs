---
corpus_test: claim-cas-floor
convention: ../../conventions/claim-cas
corpus: ../tree
---

# claim-cas floor — fire-where-expected

A contested claim (stealing an already-owned slot) fires `claim_cas`; claiming
an unclaimed slot passes. The one declared rule fires: exit 0.

```rules
scenarios/uncontested-claim.md
```

```case
{ "name": "contested-claim", "doc": "tasks/claimed.md", "actor": "worker-b", "set": {"owner": "worker-b"}, "expect": "scenarios/uncontested-claim.md" }
```

```case
{ "name": "uncontested-claim", "doc": "tasks/unclaimed.md", "actor": "worker-b", "set": {"owner": "worker-b"}, "expect": "pass" }
```
