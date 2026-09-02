---
type: meridian-config
version: 1
---

# My system (refused: bad-value)

`home_wiki` contains `_`, which is outside the canonical root-name charset
`[a-z0-9-]` (no leading or trailing `-`). The charset is the complement of the
address grammar's operator set, so no legal name can collide with an address
operator; `_` is additionally excluded to match the CHARSET-GUARD ruling that
refuses legacy underscore ids at every mint position.

The refusal names the offending character and the legal charset.

```meridian-mount
name: home_wiki
path: /srv/vaults/field-notes
vault: field-notes
```
