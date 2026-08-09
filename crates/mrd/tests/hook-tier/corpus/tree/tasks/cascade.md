---
type: task
status: in-progress
s1: seed
s2: seed
s3: seed
s4: seed
s5: seed
s6: seed
s7: seed
s8: seed
s9: seed
s10: seed
s11: seed
s12: seed
s13: seed
---

# Task: cascade

The cascade fixture page. Its thirteen chain fields exist up front because a
cascade step must MODIFY a key to be observable: `run::executor::synthesize_event`
derives `fields_changed` from `model::delta` node entries, and adding a key the
document never had produces no addressable `FmKey` node (measured). A chain built on
new keys would go silent after one generation and prove nothing.
