---
corpus_test: canonical-md-reaches-the-production-executor
corpus: ../tree
counterfactual: true
---

The positive half of the R13 pair: a canonical `md.append_section` intent crosses the
`run` adapter into the production batch executor, and the watcher fires on the
executor's OWN synthesized event — which it can only do if the append really landed
against the isolated proof corpus with production section semantics.

```conventions
../../conventions/canonical-writer
../../conventions/log-watcher
```

```case
{"name":"pull-the-trigger","doc":"tasks/card.md","set":{"trigger":"go"},"expect":"canonical-writer"}
```
