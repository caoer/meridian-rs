---
type: meridian-config
version: 1
---

# My system (refused: bad-value)

`fp1.span2.b3` has three fields, not four — it is not a well-formed fingerprint
CID-token. Well-formedness is exactly `model::fingerprint::parse_fingerprint`
returning `Some`: four non-empty `.`-separated fields,
`version.codec.hashfn.digest`.

Parse is codec-agnostic on purpose, so this case must NOT be satisfied by
checking for the literal `fp1.span2.b3.` prefix — another mount's pin may use a
different codec and must still parse.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
vault: field-notes
pin: fp1.span2.b3
```
