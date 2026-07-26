---
type: meridian-config
version: 1
---

# My system (refused: bad-value)

`kind: obsidian` is not one of the two legal kinds. Root kinds are `vault` and
`git-folder` (cross-root-addressing §1); a third would need a ruling, not a
config line. The refusal names the value found and both legal values.

```meridian-mount
name: home-wiki
path: /Users/Shared/projects/home-wiki
kind: obsidian
vault: home-wiki
```
