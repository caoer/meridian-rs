---
type: meridian-config
version: 1
---

# My system (refused: duplicate-tool-name)

Two tool blocks declare the name `llm-wiki`. Names are keys here for the same
reason mount names are: a consumer resolving `llm-wiki` must get one answer.

```meridian-tool
name: llm-wiki
kind: skill
config:
  entry: LLM_WIKI.md
```

```meridian-tool
name: llm-wiki
kind: mcp
config:
  command: llm-wiki-server
```
