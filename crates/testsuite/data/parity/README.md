# read/put parity pack (COPIED — the authoritative home is the upstream MCP server's testdata)

Byte-exact copy of the upstream `testdata/parity/` corpus, captured from the
live `readText`/`putText` path of the Go MCP server this engine replaced.
`.gitattributes -text` — the corpus bytes ARE the test; never normalize line
endings or whitespace.

This copy is the engine-side inner loop: `crates/testsuite` gates addressing
facts (`n`/`depth`/`title`/`hpath`/`words`/`sec_rev`) and the rendered text
against the same captured goldens. On re-capture, re-copy — disagreements
between this pack and the upstream harness are findings, not things to edit
around.

`$SESSION` in golden `text`/`path` fields is the harness's session-dir
placeholder; engine-side tests substitute their own workspace prefix.
