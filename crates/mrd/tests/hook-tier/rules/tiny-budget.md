---
tags: [type/rule, rules/hook]
id: tiny-budget
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 1, mem: 4194304 }
how:
  route: { info: channel-review }
---

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind == "frontmatter" and delta.key == "status" and delta.new == "review":
            return intent(
                action = "notify",
                target = event.facts.fm.get("reviewer"),
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
