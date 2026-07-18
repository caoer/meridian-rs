---
name: claude-agent-sdk
created: 2026-07-06
description: Build agentic applications with the Claude Agent SDK (TypeScript). Use when code imports @anthropic-ai/claude-agent-sdk, working with query(), custom MCP tools, subagents, hooks, or multi-turn agent conversations.
repo: cc-continuity
branch: main
commit: 2fd2d098953802895793aa330bd1e5d43bf18a18
location:
  - skills/claude-code/claude-agent-sdk/
checksum: d2e453db47f66a73dc8ea203d247cf3999c75898
draws-from: []
tags: [domain/claude-code, type/effect, effect/skill]
created_at: 2026-07-06T00:00:00Z
updated_at: 2026-07-06T00:00:00Z
---

# claude-agent-sdk (effect/skill)

Effect page for the `claude-agent-sdk` skill. cc-continuity is the authoring home; this page is the wiki's verifiable pin for the deployed artifact. **Pin-only** — the skill is authored directly in cc-continuity with no wiki-synthesis provenance (`draws-from: []`); this page exists so `effects/` is the complete deployed-surface catalog.

## Contract

| Field | Value |
| -- | -- |
| Repo | cc-continuity (`git@github.com:caoer/cc-continuity.git`) |
| Branch | `main` |
| Commit | `2fd2d098953802895793aa330bd1e5d43bf18a18` |
| Location | `skills/claude-code/claude-agent-sdk/` |
| Checksum | `d2e453db47f66a73dc8ea203d247cf3999c75898` (git tree object id) |
| Install | `cc-continuity/install.sh` symlinks `skills/claude-code/claude-agent-sdk/` → `~/.local/share/ucc/shared/skills/claude-agent-sdk` |

### Checksum computation

Authoritative method (W2a-ratified — canonical for EFFECTS.md / meridian-T3):

```bash
git -C <cc-continuity> rev-parse 2fd2d098953802895793aa330bd1e5d43bf18a18:skills/claude-code/claude-agent-sdk
```

The git **tree** object id of the deployed path at the pinned commit — content-addressed by git, reproducible from the pin (repo + commit + location) alone on every clone. The `find … | shasum` and `git archive | shasum` methods are rejected (non-deterministic / git-version-dependent).

### Staleness detection

Pin resolves (`git cat-file -t <commit>`), the git-tree checksum over `location` matches, branch HEAD past `commit` triggers a staleness warning. Every meaningful artifact change bumps `commit` + `checksum` + a changelog entry below.

## Changelog

- 2026-07-06: **effects-completeness (W4 #2)** — pin-only effect page created for the cc-native `claude-agent-sdk` skill (no prior `effects/skills/claude-agent-sdk.md`); pins cc-continuity@`2fd2d09`. (session: 06-22-outbox-effects-rename)
