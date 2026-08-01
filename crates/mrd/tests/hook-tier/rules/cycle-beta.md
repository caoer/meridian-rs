---
tags: [type/rule, rules/hook]
id: cycle-beta
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# cycle-beta

The other half of the cyclic pair: same trigger, the opposite constant. Between
them the page's `status` returns to a state it already held, which is what makes the
recurrence a real cycle rather than an ever-growing chain.

```starlark
def on_change(event):
    if "status" in event.fields_changed:
        intent(
            action = "md.set_field",
            target = "status",
            payload = "beta",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
