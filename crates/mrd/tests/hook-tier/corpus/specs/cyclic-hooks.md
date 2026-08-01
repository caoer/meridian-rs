---
corpus_test: deliberately-cyclic-hooks
corpus: ../tree
counterfactual: true
---

Two HOOKs write the same field with opposite constants, so the page's state returns
to one it already held. This is a counterfactual proof fixture, not a capability
grant: `md.*` loads here only through the corpus loader, and `SLICE1_CAPS` is
untouched.

```conventions
../../conventions/cycle-alpha
../../conventions/cycle-beta
```

```rules
cycle-alpha
cycle-beta
```

```case
{"name":"enter-status","doc":"tasks/card.md","set":{"status":"seed"},"expect":["cycle-alpha","cycle-beta"]}
```
