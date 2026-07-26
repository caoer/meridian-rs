---
type: meridian-config
version: 1
---

# My system (refused: missing-required-field)

The tool block declares no `kind`. The tool's PAYLOAD is engine-opaque; the
tool's DECLARATION is not. A malformed declaration still fails loud — the
opacity is of the payload's meaning, never of the block's shape.

```meridian-tool
name: llm-wiki
config:
  entry: LLM_WIKI.md
```
