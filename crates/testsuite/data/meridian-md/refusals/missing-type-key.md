---
version: 1
title: ZT's system
---

# My system (refused: missing-required-key)

`type:` is absent. Unknown frontmatter keys are permitted and ignored, but the
two the engine reads are both REQUIRED — which is what makes unknown-key
tolerance safe: a typo of `type` (`typ:`, `Type:`) is refused as a missing
required key, never silently dropped.
