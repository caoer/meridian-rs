---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field, md.append_section]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# mixed-emitter

One emission carrying a frontmatter descriptor AND a section descriptor. Production
applies that as ONE atomic batch, and a batch whose changed range spans frontmatter
and body has no addressable Delta container — so the one synthesized event names no
field and no section.

```starlark
def on_change(event):
    if "trigger" in event.fields_changed:
        intent(
            action = "md.set_field",
            target = "status",
            payload = "mixed",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
        intent(
            action = "md.append_section",
            target = "Log",
            payload = "- mixed entry",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
