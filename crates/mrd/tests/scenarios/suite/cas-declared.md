---
scenario: cas-declared
convention_rev: seed@1
cas: true
---

CAS is omitted by default; this scenario DECLARES it (`cas: true`), so the
harness supplies the current ambient root as the `^put`'s `if_root` world guard.
The guard matches the live world, so the guarded write commits.

```base notes/plan.md
# Plan

body
```

```put
{ "op": "splice", "path": "notes/plan.md", "target": {"hpath": [{"h": "Plan"}]}, "at": "end", "text": "- more\n" }
```

```expect
def expect(t):
    want(t.result.ok, "a guarded write against the live root commits")
    want("- more" in t.doc("notes/plan.md"), "the guarded append landed")
```
