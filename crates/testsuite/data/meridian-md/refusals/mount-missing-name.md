---
type: meridian-config
version: 1
---

# My system (refused: missing-required-field)

The mount block declares no `name`. Canonical order is name, path, kind, vault,
pin — so `path` arriving first is reported as the missing required field, not as
a field out of order.

```meridian-mount
path: /Users/Shared/projects/home-wiki
kind: vault
vault: home-wiki
```
