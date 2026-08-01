---
tags: [type/rule, rules/hook]
id: diverge-a
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.append_section]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# diverge-a

One of two peers whose cascade genuinely DIVERGES: each appends its own distinct line
to the same section, so every causal path lands on a document no other path reaches
and the state space grows exactly as fast as the path count. Nothing recurs, so no
ancestry check can ever fire — only the graph fuel bounds it.

This is the second rail on the N1 repair. A memo that skips completed work must not
turn fuel exhaustion off: this convention set never quiesces, and the gate must keep
refusing to certify it.

```starlark
def on_change(event):
    if "trigger" in event.fields_changed:
        return intent(
            action = "md.append_section",
            target = "Log",
            payload = "- a",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
    for section in event.sections_changed:
        if "Log" in section:
            return intent(
                action = "md.append_section",
                target = "Log",
                payload = "- a",
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
