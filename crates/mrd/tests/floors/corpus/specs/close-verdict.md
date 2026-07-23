---
corpus_test: close-verdict-floor
convention: ../../conventions/close-verdict
corpus: ../tree
---

# close-verdict floor — fire-where-expected

A bare status flip to closed (no Verdict) fires `close_verdict`; a close that
carries a Verdict passes. The one declared rule fires: exit 0.

```rules
scenarios/close-with-verdict.md
```

```case
{ "name": "bare-flip", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"status": "closed"}, "expect": "scenarios/close-with-verdict.md" }
```

```case
{ "name": "close-with-verdict", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"status": "closed", "verdict": "approve"}, "expect": "pass" }
```
