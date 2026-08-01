---
type: meridian-config
version: 1
---

# My system (refused: unknown-field)

`paths:` is not a legal field. Every declared field is READ — the human's bytes
are the only source here, so a silently-ignored field is a silently-ignored
intent. An engine-written page like `meridian/armed-rules.md` can be laxer — it
is regenerated from state that still exists. Your bytes are the only copy.

```meridian-mount
name: home-wiki
path: /Users/Shared/projects/home-wiki
paths: /Users/Shared/projects/home-wiki-sessions
kind: vault
vault: home-wiki
```
