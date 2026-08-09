---
tags: [type/rule, rules/hook]
id: mixed-watcher
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# mixed-watcher

Watches for the identities a SPLIT proof would have invented. It must stay silent and
be reported dead: production's single mixed batch names neither `status` nor `Log`.

```starlark
def on_change(event):
    if "status" in event.fields_changed:
        intent(
            action = "notify",
            target = "reviewer",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
    for section in event.sections_changed:
        if "Log" in section:
            intent(
                action = "notify",
                target = "log-reader",
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
