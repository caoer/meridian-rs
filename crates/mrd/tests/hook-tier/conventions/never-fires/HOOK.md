---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# never-fires

A loaded, in-scope HOOK that no case in its corpus can trigger. It exists to be
reported DEAD even when the `rules` fence never names it and a sibling HOOK is live.

```starlark
def on_change(event):
    if "a-key-no-case-touches" in event.fields_changed:
        intent(
            action = "notify",
            target = "nobody",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
