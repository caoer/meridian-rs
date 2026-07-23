---
actor: worker-a
force: false
---

# bare-flip (fires)

A close flips `status` to `closed` with no Verdict. The convention fires
(`close_verdict`): a close must carry a create-OR-replace Verdict. The legal
path is [[close-with-verdict]].

```json ^put
{
  "path": "tasks/ship-cache.md",
  "edits": [],
  "properties": { "status": "closed" }
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("a bare status flip to closed must be refused")
    if "close_verdict" not in t.result.message:
        fail("the refusal must teach the close_verdict rule")
```
