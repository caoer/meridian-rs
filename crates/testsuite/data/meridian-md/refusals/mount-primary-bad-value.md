---
type: meridian-config
version: 1
---

# My system (refused: bad-value)

Absence is the only "not primary" spelling. Admitting `primary: false` would
mint a second spelling for the same fact, and every consumer would have to
treat the two as equal forever — the block grammar keeps one spelling per
fact instead.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
primary: false
vault: field-notes
```
