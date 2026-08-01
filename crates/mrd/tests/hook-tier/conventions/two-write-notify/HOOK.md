---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

# two-write-notify

Fires on EVERY `status` transition and names the new value, so a scenario with two
`^put` writes arms one distinguishable intent per write.

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind == "frontmatter" and delta.key == "status":
            return intent(
                action = "notify",
                target = delta.new,
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
