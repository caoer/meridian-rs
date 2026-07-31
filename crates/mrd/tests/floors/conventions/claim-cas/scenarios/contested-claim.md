---
actor: worker-b
force: false
---

# contested-claim (fires)

`worker-b` claims a task already owned by `worker-a`. The convention fires: the
claim is contested, the loser (`worker-b`) is refused, and the refusal NAMES the
winner (`worker-a`) — `claim_cas`. The legal path is [[uncontested-claim]].

```json ^put
{
  "op": "splice",
  "path": "tasks/wire-journal.md",
  "actor": "worker-b",
  "force": false,
  "edits": [
    {"target": {"fm_key": "owner"}, "edit": {"put": {"at": "upsert", "text": "worker-b"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("a contested claim must be refused")
    if "worker-a" not in t.result.message:
        fail("the refusal must name the winner (worker-a)")
```
