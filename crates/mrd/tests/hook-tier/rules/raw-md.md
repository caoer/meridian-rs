---
tags: [type/rule, rules/hook]
id: raw-md
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# raw-md

A RAW `md.*` descriptor: the direct constructor, with no canonical action and no
receipt. Production HOOK projection rejects it, and since the R13 ruling the
counterfactual branch rejects it identically — the proof validates the same
production-shaped intent a real armed hook would, or it proves nothing.

```starlark
def on_change(event):
    if "status" in event.fields_changed:
        set_field(field = "status", value = "raw")
```
