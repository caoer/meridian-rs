---
corpus_test: two-entry-alpha-beta-cycle
corpus: ../tree
counterfactual: true
---

The gate-2 reproducer: TWO initial cases seed distinct lineages into the same cyclic
pair. A global work cache would retire one lineage's item before the other lineage's
descendant reached it, and the run would exit 0 claiming `acyclic` with both edges
present. Recurrence is per lineage, so both edges AND the cycle must be reported.

```rule-pages
../../rules/cycle-alpha.md
../../rules/cycle-beta.md
```

```rules
cycle-alpha
cycle-beta
```

```case
{"name":"enter-one","doc":"tasks/card.md","set":{"status":"one"},"expect":["cycle-alpha","cycle-beta"]}
```

```case
{"name":"enter-two","doc":"tasks/card.md","set":{"status":"two"},"expect":["cycle-alpha","cycle-beta"]}
```
