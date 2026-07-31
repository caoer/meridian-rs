---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind == "frontmatter" and delta.key == "status" and delta.new == "beta":
            set_field(field = "status", value = "alpha")
```
