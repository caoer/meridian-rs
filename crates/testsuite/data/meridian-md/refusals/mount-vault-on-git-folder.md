---
type: meridian-config
version: 1
---

# My system (refused: field-not-permitted-for-kind)

A `git-folder` root has no Obsidian vault, so `vault:` states something that
cannot be true. Ignoring it would let a config assert a vault name nothing
checks; refusing it keeps the three-way map honest about which legs exist for
which kind.

```meridian-mount
name: archive
path: /Users/Shared/repos/archive
kind: git-folder
vault: archive
```
