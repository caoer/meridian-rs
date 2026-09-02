---
---

# My system (refused: missing-required-key)

The frontmatter block above opens with `---` and closes with `---`. Between the
two fences it carries nothing, so it declares no `type:` — and `type` is the
first of the two keys the engine requires. Refused at line 1.

Neither `no-frontmatter` condition holds here. The file *does* open with `---`
and the fence *does* close, so a refusal saying this file "does not open with a
closed `---` frontmatter block" would be a false statement about bytes the
author is looking at — on the one door whose whole job is to teach. An empty
*closed* block is a missing key, not a missing block (schema §4).

The distinction has to be made from the bytes, not from the parse tree: the
markdown parser mints no frontmatter node for an empty metadata block, so a
door that reads only the tree cannot tell this file from one with no
frontmatter at all. Both look like "no frontmatter node"; only one of them is.

The mount block below is well-formed. It is here so that a build which treated
an empty frontmatter as "no keys needed" — defaulting `type` and `version` —
would publish a mount table this file's outcome says must never exist.

```meridian-mount
name: field-notes
path: /srv/vaults/field-notes
vault: field-notes
```
