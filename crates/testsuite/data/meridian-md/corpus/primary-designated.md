---
type: meridian-config
version: 1
---

# My system (accepted — one designated primary)

The designation is DECLARED, never derived: the primary here is the SECOND
block, so a reader that "designates" mounts[0], the first vault, or the last
block looks healthy on every other acceptance and fails only here.

```meridian-mount
name: home-wiki
path: /Users/Shared/projects/home-wiki
vault: home-wiki
```

The session tree — the designated primary: the one tree a fleet host's
single-root consumers (change feed, watch loop, journal) anchor.

```meridian-mount
name: sessions
path: /Users/Shared/projects/home-wiki-sessions
primary: true
vault: home-wiki-sessions
```

Archived assets, a plain folder without a `vault:` leg — present so the
accept proves coexistence.

```meridian-mount
name: archive
path: /Users/Shared/repos/archive
```
