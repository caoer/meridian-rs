---
type: meridian-config
version: 1
---

# My system (accepted — a tool declaration with an opaque payload)

Required outcome: one mount and one tool. The tool's `name` and `kind` are read;
its `config:` payload is carried verbatim and NEVER interpreted, including the
`kind: skill` this build knows nothing about.

A tool declaration for a tool this machine has not installed is not a broken
config — it is a statement addressed to someone else. Refusing it would mean a
config becomes invalid by removing a tool.

## Roots

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```

## Tools

```meridian-tool
name: llm-wiki
kind: skill
config:
  entry: LLM_WIKI.md
  vault: field-notes
  nested:
    deeper: true
```
