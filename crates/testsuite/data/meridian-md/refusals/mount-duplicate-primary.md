---
type: meridian-config
version: 1
---

# My system (refused: duplicate-primary-designation)

The designation is a role exactly one mount may hold. Two claimants is a
table-level defect like duplicate-mount-name: the parser refuses the whole
table and never picks between them — a daemon that anchored its journal by
either choice would silently retarget when the file is next edited.

```meridian-mount
name: field-notes
path: /srv/vaults/field-notes
primary: true
vault: field-notes
```

```meridian-mount
name: sessions
path: /srv/vaults/field-notes-sessions
primary: true
vault: field-notes-sessions
```
