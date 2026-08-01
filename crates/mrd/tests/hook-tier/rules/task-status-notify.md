---
tags: [type/rule, rules/hook]
id: task-status-notify
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
  batching: 30s
  wake_policy: never-cold
---

# task-status-notify

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind != "frontmatter" or delta.key != "status":
            continue
        if delta.new != "review":
            continue
        return intent(
            action = "notify",
            target = event.facts.fm.get("reviewer"),
            severity = "info",
            payload = "task %s → review" % event.file,
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
