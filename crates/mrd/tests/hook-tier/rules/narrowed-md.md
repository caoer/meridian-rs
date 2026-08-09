---
tags: [type/rule, rules/hook]
id: narrowed-md
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# narrowed-md

An ORDINARY `[proto.send]` declaration that emits one admitted intent and one denied
`md.set_field` intent. The denied descriptor is report data, and its spelling must be
byte-identical in both corpus modes: `counterfactual: true` widens which caps a
declaration may CARRY, never how a result is projected or reported.

```starlark
def on_change(event):
    if "status" in event.fields_changed:
        intent(
            action = "notify",
            target = "reviewer",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
        intent(
            action = "md.set_field",
            target = "status",
            payload = "denied",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
