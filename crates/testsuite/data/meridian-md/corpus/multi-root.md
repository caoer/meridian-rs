---
type: meridian-config
version: 1
title: ZT's system
---

# My system (accepted — the real topology, three roots)

The load-bearing acceptance. Required outcome: three mounts in document order —
`field-notes` and `sessions` carrying `vault:` names, `archive` carrying NO
`vault:` field (a plain folder — presence of `vault:` is what makes a vault).

Without this case a build that refuses every config, or one that loads only the
first block, still satisfies every refusal case in this pack.

## Roots

The wiki: domains, decisions, effects. This is where law lives.

```meridian-mount
name: field-notes
path: /srv/vaults/field-notes
vault: field-notes
```

The session tree — one directory per working session, partitioned by year and
month. Also an Obsidian vault, under a different vault name than its root name.

```meridian-mount
name: sessions
path: /srv/vaults/field-notes-sessions
vault: field-notes-sessions
```

Archived assets. A plain git folder — no `vault:` leg.

```meridian-mount
name: archive
path: /srv/repos/archive
```
