---
tags: [type/rule, rules/hook]
id: chain-b
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# chain-b

The other peer of the terminating convergent cascade. Byte-for-byte the same law as
`chain-a`: two rules react to each level and emit the SAME next generation, so the
two branches reconverge on one state at every level. Only the emitter differs, which
is why an ancestor-subset rule collapses nothing here and a completed-work memo does.

```starlark
NEXT = {
    "s1": "s2",
    "s2": "s3",
    "s3": "s4",
    "s4": "s5",
    "s5": "s6",
    "s6": "s7",
    "s7": "s8",
    "s8": "s9",
    "s9": "s10",
    "s10": "s11",
    "s11": "s12",
    "s12": "s13",
}

def on_change(event):
    for field in event.fields_changed:
        if field in NEXT:
            return intent(
                action = "md.set_field",
                target = NEXT[field],
                payload = "same",
                receipt = receipt_addr(event.file, event.fingerprint_after),
            )
```
