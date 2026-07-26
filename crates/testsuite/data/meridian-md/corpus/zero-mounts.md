---
type: meridian-config
version: 1
---

# My system (accepted — state D, zero mounts)

Parses clean and declares NO mount. Required outcome: **treated as the absent
case** — current single-root behaviour, not an error. An empty mount table is a
legitimate statement.

This case exists because "empty" and "absent" reaching different code paths is
how a nil-vs-empty bug is born. It must produce the same mount table and the
same resolution behaviour as `absent-no-file`.

I have not declared any roots yet.
