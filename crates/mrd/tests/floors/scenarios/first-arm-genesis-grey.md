---
scenario: first-arm-genesis-grey
convention_rev: arming-from-zero
---

# first-arm journal row — ungated-but-present AND grey (genesis epoch)

The first-arming write is the birth of `conventions/INDEX.md` on a workspace
that has NEVER been armed (no `meridian/attested` marker yet). At gate time the
workspace is never-armed, so the gate is a bit-for-bit no-op: the write LANDS
UNGATED. It is still JOURNALED — the create row is present in the receipt
journal. And it is GREY on the enforcement axis, never green: a never-armed
write carries NO enforcement verdict (`t.result.verdicts` is empty). Grey is the
ABSENCE of a green enforcement verdict, not a token (refusal-amendment §
non-refusing renders — "genesis-epoch write ... grey on the enforcement axis,
never green").

```put
{ "op": "create", "path": "conventions/INDEX.md", "body": "# Attested conventions INDEX\n\nSwept from `conventions/`. Genesis: no row armed yet.\n" }
```

```expect
def expect(t):
    # ungated: the first-arming write lands (never-armed gate is a no-op)
    want(t.result.ok, "the first-arming write lands ungated on a never-armed workspace")
    # grey on the enforcement axis: no green enforcement verdict
    want(len(t.result.verdicts) == 0, "genesis epoch is grey — no enforcement verdict, never green")
    # present: the write is journaled (ungated-but-journaled)
    want("op=create" in t.journal, "the first-arming write is journaled")
    want("conventions/INDEX.md" in t.journal, "the journal row names the arming write")
    # byte-landed
    want("Attested conventions INDEX" in t.doc("conventions/INDEX.md"), "the INDEX bytes landed")
```
