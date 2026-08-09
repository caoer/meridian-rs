---
type: meridian-config
version: 1
---

# My system (accepted — prose is prose, exactly one mount)

The anti-vacuity case for the scoping law. Required outcome: **exactly one
mount** (`field-notes`), and **no refusal**. Four decoys are present and every one
of them must be inert.

A parser that scans the file for `name:`/`path:`/`kind:` lines, or that treats
any fenced block as a machine surface, loads two or more mounts here and passes
every other acceptance case in this pack.

## Decoy 1 — a fenced block in another language

This is documentation, not configuration. It is a ` ```yaml ` block, so it is
prose:

```yaml
name: not-a-mount
path: /tmp/definitely-not
kind: vault
vault: not-a-mount
```

## Decoy 2 — an engine block nested inside a longer fence

A tutorial showing the reader what a mount block looks like. The outer fence is
four backticks, so the inner three-backtick fence is *content*, not a block:

````text
```meridian-mount
name: also-not-a-mount
path: /tmp/still-not
kind: git-folder
```
````

## Decoy 3 — an indented snippet

An indented code block is not a fenced block:

    name: indented-not-a-mount
    path: /tmp/nope
    kind: git-folder

## Decoy 4 — inline code and bare prose

Writing `meridian-mount` in inline code does not open a block, and neither does
saying that my archive lives at /Users/Shared/repos/archive with kind:
git-folder in an ordinary sentence.

## The one real root

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```
