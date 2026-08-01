---
type: task
status: open
owner: worker-a
verdict: reject
tags: [type/task]
---

# Ship the projection cache (bounced)

A task whose first review DECIDED — `verdict: reject` — and which went back to
`status: open` for the rework. The re-approve that closes it is a bounce: the
second decision is written through the SAME `put at:upsert` on the `verdict`
key, so it REPLACES the first rather than being refused as a duplicate.
