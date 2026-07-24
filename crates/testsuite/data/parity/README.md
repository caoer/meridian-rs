# U0 read/put parity pack (COPIED — authoritative home is ccc-statusd)

Byte-exact copy of `ccc-statusd internal/mcpserver/testdata/parity/` at
`worker/m1-d-parity-go` @ `50b15ae` (U0 golden capture from the LIVE Go
`readText`/`putText` path). `.gitattributes -text` — the corpus bytes ARE the
test; never normalize line endings or whitespace.

The authoritative cutover gate is the U0 harness in ccc-statusd (it drives
the MCP face end-to-end). This copy is the engine-side inner loop: U2 gates
addressing facts (`n`/`depth`/`title`/`hpath`/`words`/`sec_rev`) and U4a1
gates the rendered text against the same captured goldens. On re-capture,
re-copy — disagreements between this pack and the harness are findings, not
things to edit around.

`$SESSION` in golden `text`/`path` fields is the harness's session-dir
placeholder; engine-side tests substitute their own workspace prefix.
