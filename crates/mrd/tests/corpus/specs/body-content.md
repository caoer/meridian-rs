---
corpus_test: a-case-drives-a-content-reading-check
rule: ../rules/no-smuggled-heading.md
corpus: ../tree
---

# body-content (fixture spec, exit 0)

The CONTENT half of the case surface. Both cases write the document BODY through
the production edit grammar — an `hpath` target and a `put at:end` — which is the
only way a corpus case can move `change.sections_changed` or the world-model
nodes the law compares across the write. The firing case smuggles a heading into
a section edit; the passing case makes the same kind of section write and adds no
heading, so the citation is live rather than merely silent.

```rules
structural-write
```

```case
{ "name": "smuggle-a-heading",
  "doc": "tasks/b3-gatecheck.md",
  "actor": "agent:alice",
  "edits": [
    { "target": {"hpath": ["Task: b3-gatecheck", "Quality Gates"]},
      "edit": {"put": {"at": "end", "text": "\n## Smuggled\n\nrestructured under cover of a section edit.\n"}} }
  ],
  "expect": "structural-write" }
```

```case
{ "name": "plain-section-append",
  "doc": "tasks/b3-gatecheck.md",
  "actor": "agent:alice",
  "edits": [
    { "target": {"hpath": ["Task: b3-gatecheck", "Quality Gates"]},
      "edit": {"put": {"at": "end", "text": "\n- every receipt names the command that produced it.\n"}} }
  ],
  "expect": "pass" }
```
