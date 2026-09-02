---
type: session
status: active
created: 2026-03-14
domain: fleet-config
backfilled: true
backfilled-by: 2026-03-20-compound-sweep
tags: []
---

> [!info] Update (2026-03-25)
> relay-a has since been rebuilt from the declarative image. References to the hand-installed base below are historical.

# Ad-hoc: declarative rebuild support for foreign hosts

Session backfilled by the compound sweep. The original session ran without a pipeline tracker.

## Pipeline
- [ ] work (inferred: foreign-host rebuild implementation)
- [ ] review
- [x] compound (this run)

## Inferred scope

Added a foreign-host classification to `fleet rebuild --remote`, enabling declarative management on hosts that do not boot the fleet image. First activation completed on relay-a (a rented VM in the example region).
