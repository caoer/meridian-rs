---
type: meridian-config
version: 1
---

# My system (refused: unknown-field)

`paths:` is not a legal field. Every declared field is READ — the human's bytes
are the only source here, so a silently-ignored field is a silently-ignored
intent. That is the one place this schema deliberately differs from
`conventions/INDEX.md`, whose scope column is rendered and never read back.

```meridian-mount
name: home-wiki
path: /Users/Shared/projects/home-wiki
paths: /Users/Shared/projects/home-wiki-sessions
kind: vault
vault: home-wiki
```
