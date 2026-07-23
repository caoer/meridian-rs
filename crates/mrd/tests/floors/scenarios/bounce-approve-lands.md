---
scenario: bounce-approve-lands
convention_rev: close-verdict-floor
---

# bounce → second approve LANDS (create-OR-replace Verdict)

The close-verdict floor records a Verdict create-OR-replace (`put at:upsert` on
the `verdict` key). A bounce — a reviewer rejects, then re-approves — must LAND
the second decision, not refuse it as a duplicate. The first put writes
`reject`; the second (the bounce) writes `approve` through the SAME upsert, and
it lands: the Verdict reads `approve` after the write, never stuck at `reject`.

```base verdicts/close-1.md
---
type: verdict
reviewer: mrd-test
verdict: pending
---

# Verdict — ship the projection cache
```

```put
{ "op": "splice", "path": "verdicts/close-1.md", "target": {"fm_key": "verdict"}, "at": "upsert", "text": "reject" }
```

```put
{ "op": "splice", "path": "verdicts/close-1.md", "target": {"fm_key": "verdict"}, "at": "upsert", "text": "approve" }
```

```expect
def expect(t):
    want(t.result.ok, "the second (approve) decision must land create-OR-replace")
    want("verdict: approve" in t.doc("verdicts/close-1.md"), "the bounce approve replaced the reject")
    want("reject" not in t.doc("verdicts/close-1.md"), "the reject was overwritten, not appended")
```
