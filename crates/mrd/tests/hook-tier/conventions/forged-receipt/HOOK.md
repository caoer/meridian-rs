---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind == "frontmatter" and delta.key == "status" and delta.new == "review":
            return intent(
                action = "notify",
                target = "reviewer",
                receipt = "forged",
            )
```
