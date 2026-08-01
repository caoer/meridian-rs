---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# log-watcher

Observes the executor's OWN synthesized event. It can only fire if the adapted
canonical intent really landed through the production write path and the executor
named the section it changed.

```starlark
def on_change(event):
    for section in event.sections_changed:
        if "Log" in section:
            intent(
                action = "notify",
                target = "log-reader",
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
