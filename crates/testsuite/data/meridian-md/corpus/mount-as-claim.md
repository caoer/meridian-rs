---
type: meridian-config
version: 1
---

# My system (accepted — a mount that pins the root it declares)

The S3-R7 expressibility case: `~/MERIDIAN.md` cannot be attested, so a mount's
pin is the sole mechanism by which the mount table's own integrity is checkable.
Required outcome: two mounts load; `field-notes` carries a well-formed pin token,
`archive` carries none.

The pin token is deliberately NOT a `span2` token on both entries: the parser
must accept any well-formed `version.codec.hashfn.digest` token and must not
constrain the codec.

## Roots

```meridian-mount
name: field-notes
path: /srv/vaults/field-notes
vault: field-notes
pin: fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152
```

```meridian-mount
name: archive
path: /srv/repos/archive
```
