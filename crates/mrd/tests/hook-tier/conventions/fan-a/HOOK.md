---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# fan-a

One of two peers that emit the IDENTICAL generation from the identical state, so
their two acyclic branches converge on the same downstream work item.

```starlark
def on_change(event):
    if "trigger" in event.fields_changed:
        intent(
            action = "md.set_field",
            target = "step",
            payload = "converged",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
