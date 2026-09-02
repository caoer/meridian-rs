---
name: agent-sdk-notes
created: 2026-04-02
description: Build agentic applications with the agent SDK (TypeScript). Use when code imports the agent SDK package, working with query(), custom tool servers, subagents, hooks, or multi-turn agent conversations.
repo: skill-forge
branch: main
commit: 0123456789abcdef0123456789abcdef01234567
location:
  - skills/agent-sdk-notes/
checksum: 89abcdef0123456789abcdef0123456789abcdef
draws-from: []
tags: [domain/agent-tooling, type/effect, effect/skill]
created_at: 2026-04-02T00:00:00Z
updated_at: 2026-04-02T00:00:00Z
---

# agent-sdk-notes (effect/skill)

Effect page for the `agent-sdk-notes` skill. skill-forge is the authoring home; this page is the wiki's verifiable pin for the deployed artifact. **Pin-only** — the skill is authored directly in skill-forge with no wiki-synthesis provenance (`draws-from: []`); this page exists so `effects/` is the complete deployed-surface catalog.

## Contract

| Field | Value |
| -- | -- |
| Repo | skill-forge (`git@git.example.test:acme/skill-forge.git`) |
| Branch | `main` |
| Commit | `0123456789abcdef0123456789abcdef01234567` |
| Location | `skills/agent-sdk-notes/` |
| Checksum | `89abcdef0123456789abcdef0123456789abcdef` (git tree object id) |
| Install | `skill-forge/install.sh` symlinks `skills/agent-sdk-notes/` → `~/.local/share/skills/agent-sdk-notes` |

### Checksum computation

Authoritative method (ratified — canonical for EFFECTS.md):

```bash
git -C <skill-forge> rev-parse 0123456789abcdef0123456789abcdef01234567:skills/agent-sdk-notes
```

The git **tree** object id of the deployed path at the pinned commit — content-addressed by git, reproducible from the pin (repo + commit + location) alone on every clone. The `find … | shasum` and `git archive | shasum` methods are rejected (non-deterministic / git-version-dependent).

### Staleness detection

Pin resolves (`git cat-file -t <commit>`), the git-tree checksum over `location` matches, branch HEAD past `commit` triggers a staleness warning. Every meaningful artifact change bumps `commit` + `checksum` + a changelog entry below.

## Changelog

- 2026-04-02: **effects-completeness** — pin-only effect page created for the `agent-sdk-notes` skill (no prior `effects/skills/agent-sdk-notes.md`); pins skill-forge@`0123456`. (session: 03-22-effects-rename)
