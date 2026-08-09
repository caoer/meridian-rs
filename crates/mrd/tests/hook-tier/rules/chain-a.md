---
tags: [type/rule, rules/hook]
id: chain-a
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [md.set_field]
budget: { steps: 10000, mem: 4194304 }
how: {}
---

# chain-a

One of two peers driving a strictly TERMINATING convergent cascade: each level's
field advances to the next, and both peers emit the identical generation from the
identical state. The causal PATHS through this graph number `2^(d+1) − 2`; the
distinct states number `d`. A proof that enumerates paths runs out of fuel on a
convention set that has no cycle at all.

The chain ends at `s13`, which no peer answers, so every lineage reaches a fixed
point.

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
