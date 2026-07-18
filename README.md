# meridian-rs

Rust rewrite of meridian's markdown layer on a forked pulldown-cmark — grown rung by
rung from parser sidecar into the daemon that owns the md-as-fs world model.

## Plan of record

The binding vision (three laws, 6-rung capability ladder, fork scope, process-boundary
ruling, rules-as-data, sequencing) lives in the session tree:

`field-notes-sessions/year=2026/month=07/16-17-ccc-session-md-as-file-system/results/meridian-rs-vision.md`

The three laws, restated:

1. **Rust is the world model, Go is the actor.** Rust answers: what do the files say,
   what changed, what's valid. Go decides who does what about it.
2. **Truth geography:** disk = markdown files, the only durable truth. Rust memory =
   derived world model, disposable by design.
3. **One-way arrows:** files → Rust → Go. Recovery at every layer is "re-derive from
   the layer below."

## Architecture candidates

Crate-architecture candidates are developed as branches (`candidate/*`), each a
skeleton workspace whose README states the candidate's thesis. `main` stays minimal
until a candidate is chosen (review round + ZT gate).
