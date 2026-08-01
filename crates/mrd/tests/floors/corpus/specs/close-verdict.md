---
corpus_test: close-verdict-floor
rule: ../../rules/close-verdict.md
corpus: ../tree
---

# close-verdict floor — fire-where-expected

A bare status flip to closed (no Verdict) fires `close_verdict`; a close that
carries a Verdict passes. The one declared rule fires: exit 0.

## The bounce (ported from the retired scenario tier)

`bounce-reject` and `bounce-approve-lands` are the corpus-tier half of the
retired `scenarios/bounce-approve-lands` suite scenario. A Verdict is recorded
create-OR-REPLACE, so a bounce — reject, rework, re-approve — must LAND its
second decision rather than be refused as a duplicate. Both cases go through the
production splice writer's `put at:upsert` on the `verdict` key, and both pass:
`bounce-reject` records the first decision, `bounce-approve-lands` re-upserts
over a page that already carries `verdict: reject`.

The BYTE half of that scenario — that the re-upsert replaces the earlier
`reject` instead of appending beside it — is not a fire/pass fact, so it is
asserted where bytes are observable:
`corpus_tier::tests::a_bounce_re_upsert_replaces_the_earlier_verdict`, over the
same production writer this tier mutates through.

```rules
close-with-verdict
```

```case
{ "name": "bare-flip", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"status": "closed"}, "expect": "close-with-verdict" }
```

```case
{ "name": "close-with-verdict", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"status": "closed", "verdict": "approve"}, "expect": "pass" }
```

```case
{ "name": "bounce-reject", "doc": "tasks/plain-open.md", "actor": "reviewer-b", "set": {"status": "closed", "verdict": "reject"}, "expect": "pass" }
```

```case
{ "name": "bounce-approve-lands", "doc": "tasks/bounced-open.md", "actor": "reviewer-b", "set": {"status": "closed", "verdict": "approve"}, "expect": "pass" }
```
