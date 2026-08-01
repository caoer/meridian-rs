---
corpus_test: converging-branches-are-not-a-cycle
corpus: ../tree
counterfactual: true
---

Two peer HOOKs emit the IDENTICAL generation from the IDENTICAL state, so their
branches converge on a byte-identical downstream work item on two different
lineages. No lineage returns to one of its own ancestors, so the verdict is acyclic —
the false-positive class the per-lineage recurrence rule closes.

```conventions
../../conventions/fan-a
../../conventions/fan-b
../../conventions/converge-sink
```

```case
{"name":"pull-the-trigger","doc":"tasks/card.md","set":{"trigger":"go"},"expect":["fan-a","fan-b"]}
```
