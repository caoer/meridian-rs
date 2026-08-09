---
type: meridian-config
version: 1
---

# My system (refused: malformed-line)

A `config:` payload line that is not indented. Indentation is the only
structural fact the engine needs from the payload: it is what makes the
payload's extent unambiguous. Everything else about those bytes belongs to the
tool the `kind` names.

```meridian-tool
name: llm-wiki
kind: skill
config:
  entry: LLM_WIKI.md
vault: field-notes
```
