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

Watches the identities of the mixed batch. Since the sub-node-grain ruling the
single production batch names BOTH `status` and `Log`, so this watcher fires once,
on one event — a split proof would fire it twice, which the suite refuses.

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
