---
tags: [type/rule, rules/hook]
id: diverge-b
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.append_section]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# diverge-b

The other diverging peer. Same trigger, a different appended line — that one byte is
what makes the two branches irreconcilable at every level, so the reachable state
graph is a binary tree with no shared node and no fixed point.

```starlark
def on_change(event):
    if "trigger" in event.fields_changed:
        return intent(
            action = "md.append_section",
            target = "Log",
            payload = "- b",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
    for section in event.sections_changed:
        if "Log" in section:
            return intent(
                action = "md.append_section",
                target = "Log",
                payload = "- b",
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
