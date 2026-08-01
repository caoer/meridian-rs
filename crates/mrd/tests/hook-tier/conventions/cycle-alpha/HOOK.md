---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# cycle-alpha

Half of the deliberately cyclic pair. It emits a CANONICAL intent — the same
`action` / `target` / `payload` / `receipt` shape production validates — because the
proof executes it through the production intent→executor adapter, not a proof-only
mapping.

It reads only `fields_changed`, because that is all a cascade event carries: the
executor's apply→event synthesis has no values to attach, so a value-reading
predicate could never fire on a real follow-on generation.

```starlark
def on_change(event):
    if "status" in event.fields_changed:
        intent(
            action = "md.set_field",
            target = "status",
            payload = "alpha",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
