---
type: note
version: 1
---

# My system (refused: wrong-type-value)

`type:` is present but is not `meridian-config`. This is the case that fires
when `MERIDIAN_CONFIG` is aimed at an ordinary page — a real accident, and one
that must never half-load. The refusal names both the value found and the value
required.
