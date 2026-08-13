---
type: meridian-config
version: 1
---

# My system (refused: duplicate-mount-name)

Two blocks bind the canonical name `field-notes`. The mount table is the single
authority for the three-way translation, and a map with two values for one key
is not a map. Refused at parse; the refusal names both blocks' lines.

Note the paths differ, so this is not the same defect as two mounts resolving to
one path — that one needs canonicalization (symlinks, trailing slashes, `..`)
and is U7's, not this schema's. Name uniqueness is decidable from the bytes.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
vault: field-notes
```

```meridian-mount
name: field-notes
path: /Users/Shared/repos/archive
```
