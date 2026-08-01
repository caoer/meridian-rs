---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.append_section]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# canonical-writer

A CANONICAL counterfactual intent: it reaches the production batch executor through
the `run` adapter with the target/value semantics the ruling names — `target` is the
exact heading text, `payload` is the appended content.

```starlark
def on_change(event):
    if "trigger" in event.fields_changed:
        intent(
            action = "md.append_section",
            target = "Log",
            payload = "- canonical entry",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
