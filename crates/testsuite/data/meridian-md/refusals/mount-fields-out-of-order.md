---
type: meridian-config
version: 1
---

# My system (refused: field-out-of-order)

`kind:` precedes `path:`. Canonical order is name, path, kind, vault, pin — one
spelling per fact, so a diff of two configs compares like with like, and a
misplaced key is a precise refusal instead of an ambiguous one. The refusal
teaches the order.

```meridian-mount
name: field-notes
kind: vault
path: /Users/Shared/projects/field-notes
vault: field-notes
```
