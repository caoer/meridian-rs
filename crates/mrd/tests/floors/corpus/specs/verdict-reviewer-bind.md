---
corpus_test: verdict-reviewer-bind-floor
rule: ../../rules/verdict-reviewer-bind.md
corpus: ../tree
---

# verdict-reviewer-bind floor — fire-where-expected

A Verdict naming a reviewer other than the closing actor fires `reviewer_bind`;
a Verdict whose reviewer equals the closing actor passes. The one declared rule
fires: exit 0.

```rules
bound-verdict
```

```case
{ "name": "unbound-verdict", "doc": "verdicts/close-1.md", "actor": "dave", "set": {"outcome": "approve"}, "expect": "bound-verdict" }
```

```case
{ "name": "bound-verdict", "doc": "verdicts/close-1.md", "actor": "carol", "set": {"outcome": "approve"}, "expect": "pass" }
```
