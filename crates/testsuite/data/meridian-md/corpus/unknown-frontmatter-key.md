---
type: meridian-config
version: 1
title: ZT's system
updated: 2026-07-26
tags: [config, meridian]
aliases:
  - my-system
---

# My system (accepted — unknown frontmatter keys are permitted and ignored)

Required outcome: one mount loads; `title`, `updated`, `tags`, and `aliases` are
ignored without a refusal and without a warning.

This is the shipped posture for markdown-as-config frontmatter
(`crates/policy/src/convention.rs:311-314`) and it is what lets a user carry
Obsidian properties on their own entry page. It is safe in v1 only because v1
defines NO optional frontmatter key the engine reads, so a typo of a required
key fails loud as `missing-required-key` instead of being silently dropped.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```
