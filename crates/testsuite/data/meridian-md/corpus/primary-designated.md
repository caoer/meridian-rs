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
path: /srv/vaults/field-notes
vault: field-notes
```

The session tree — the designated primary: the one tree a fleet host's
single-root consumers (change feed, watch loop, journal) anchor.

```meridian-mount
name: sessions
path: /srv/vaults/field-notes-sessions
primary: true
vault: field-notes-sessions
```

Archived assets, a plain folder without a `vault:` leg — present so the
accept proves coexistence.

```meridian-mount
name: archive
path: /srv/repos/archive
```
