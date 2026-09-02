---
type: meridian-config
version: 1
---

# My system (refused: unknown-field — the retired `kind:`)

`kind:` left the mount schema (kind-sweep, 2026-08-13): vault-ness is carried
by `vault:` presence alone, and `primary:` is legal on any mount. A config
still carrying the field refuses through the unknown-field door — no silent
tolerance, no compatibility window. The remedy is the door's own: remove the
line.

```meridian-mount
name: field-notes
path: /srv/vaults/field-notes
kind: vault
vault: field-notes
```
