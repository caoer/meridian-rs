---
type: meridian-config
version: 1
---

# My system (accepted — one designated primary)

The designation is DECLARED, never derived: the primary here is the SECOND
block, so a reader that "designates" mounts[0], the first vault, or the last
block looks healthy on every other acceptance and fails only here.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```

The session tree — the designated primary: the one tree a fleet host's
single-root consumers (change feed, watch loop, journal) anchor.

```meridian-mount
name: sessions
path: /Users/Shared/projects/field-notes-sessions
kind: vault
primary: true
vault: field-notes-sessions
```

Archived assets, a plain git folder — a kind that may never carry the
designation, present so the accept proves coexistence.

```meridian-mount
name: archive
path: /Users/Shared/repos/archive
kind: git-folder
```
