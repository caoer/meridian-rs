---
scenario: create-file-pass
convention_rev: seed@1
---

A guarded `create` births a file and journals a `create` row; `t.journal`
carries the birth, `t.doc` reads the born bytes.

```put
{ "op": "create", "path": "notes/new.md", "body": "# New\n\n- born\n" }
```

```expect
def expect(t):
    want(t.result.ok, "the birth must commit")
    want("op=create" in t.journal, "the birth is journaled")
    want("notes/new.md" in t.journal, "the journal row names the born path")
    want("# New" in t.doc("notes/new.md"), "the born bytes are on disk")
```
