---
type: meridian-config
version: 1
---

# My system (refused: unterminated-block)

The mount block's fence never closes, so the rest of the file is swallowed into
the block body. Refused rather than parsed to end-of-file: an unterminated
engine block means the author's intent about where the machine surface ends is
unknown.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
vault: field-notes
