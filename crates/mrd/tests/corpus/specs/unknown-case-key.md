---
corpus_test: an-unknown-case-key-is-a-hard-fault
rule: ../rules/no-smuggled-heading.md
corpus: ../tree
---

# unknown-case-key (fixture spec, exit 2)

A case key the grammar does not carry is a spec the author got wrong, not a case
with a dropped clause. Silently ignoring it is how the content gap survived
unnoticed: an author who wrote `append_section` got a green-looking run over a
case that mutated nothing.

```rules
structural-write
```

```case
{ "name": "author-reaches-for-a-verb-that-does-not-exist",
  "doc": "tasks/b3-gatecheck.md",
  "actor": "agent:alice",
  "append_section": {"Quality Gates": "## Smuggled"},
  "expect": "structural-write" }
```
