---
tags: [type/rule, rules/check]
id: close-verdict
paths:
  - tasks/**
---

# close-verdict (floor rule U4.4)

A close is a create-OR-replace Verdict, never a bare status flip. When a task
transitions to `status: closed`, the change MUST carry a `verdict` decision
(`approve` / `reject`) — the Verdict is written create-OR-replace (`put
at:upsert` on the `verdict` key), so a re-decision (a bounce: reject then
approve) LANDS rather than being refused as a duplicate. A bare flip to closed
with no Verdict is refused (`close_verdict`, recovery `fix`). The legal path is
the `close-with-verdict` case; the `bounce-approve-lands` case is the one that
demonstrates the create-OR-replace half.

FLOOR rule (plan U4.4). The `passing =` citation names a corpus-tier CASE — the
scenario page it used to name retired with its tier, and a refusal may not cite
a page that is not in the tree.

```starlark
def check_change(change):
    # Judge only the CLOSE transition (status becomes `closed`).
    if "status" not in change.fields_changed:
        return
    if change.doc.frontmatter.get("status") != "closed":
        return
    verdict = change.doc.frontmatter.get("verdict")
    if verdict == None or verdict == "":
        refuse(
            message = "close_verdict: a close must carry a Verdict (create-OR-replace `verdict:` approve/reject) — a bare status flip is refused",
            passing = "close-with-verdict",
        )
```
