---
tags: [type/rule, rules/hook]
id: 050-fixture-hook-caps-empty
severity: info
paths: ["**"]
on: "idle>42m*3"
action: |
  message("Idle check {{FIRE_SEQ}}/{{BUDGET}}: if your task is unfinished, continue it now.")
caps: []
budget: { steps: 10000, mem: 4194304 }
how: {}
---
# 050-fixture-hook-caps-empty — direct idle nudge at 42m, budget 3

**A FIXTURE, not a rule.** The second production shape, copied from the sessions
root's `rules/050-hook-idle-default.md`: a `caps: []` hook that declares
`budget:` AND `how:`, so the loader takes its `Some(how_value)` arm — the arm
every armed hook page on that root runs through. It must arm clean.

Key for key on the FRONTMATTER, which is what the loader judges, except `id:` —
a `fixture-` segment so this copy cannot collide with the rule it models. The
`action:` message is shortened; the loader never reads it.

```starlark
def on_change(event):
    pass
```
