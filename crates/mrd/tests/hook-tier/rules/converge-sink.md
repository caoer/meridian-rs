---
tags: [type/rule, rules/hook]
id: converge-sink
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# converge-sink

The convergence point: both peer branches reach this rule in the same state with the
same emission, so the two work items are byte-identical on different lineages. Its
own write triggers nothing, so each lineage terminates.

```starlark
def on_change(event):
    if "step" in event.fields_changed:
        intent(
            action = "md.set_field",
            target = "done",
            payload = "yes",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
