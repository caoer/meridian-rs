---
type: meridian-config
version: 1
title: ZT's system
---

# My system (accepted — the real topology, three roots)

The load-bearing acceptance. Required outcome: three mounts in document order —
`field-notes` (vault), `sessions` (vault), `archive` (git-folder) — with the
git-folder carrying NO `vault:` field.

Without this case a build that refuses every config, or one that loads only the
first block, still satisfies every refusal case in this pack.

## Roots

The wiki: domains, decisions, effects. This is where law lives.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```

The session tree — one directory per working session, partitioned by year and
month. Also an Obsidian vault, under a different vault name than its root name.

```meridian-mount
name: sessions
path: /Users/Shared/projects/field-notes-sessions
kind: vault
vault: field-notes-sessions
```

Archived assets. A plain git folder: no parse, no sections, file-grain pins.

```meridian-mount
name: archive
path: /Users/Shared/repos/archive
kind: git-folder
```
