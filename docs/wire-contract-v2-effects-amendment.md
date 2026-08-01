# Wire contract v2 amendment — reaction effect envelopes

Status: normative additive amendment to `docs/wire-contract-v2.md` on the notification plane. The frozen v2 document remains unedited. This amendment implements subscribe/notify plan r3 C5 and the reaction shape established by the foundation panel.

## Notification shape

A `DeltaFrame` may carry an `effects` array beside `delta`:

```jsonc
{
  "delta": { "seq": 42, "root_before": "b3:…", "root_after": "b3:…", "files": [] },
  "effects": [{
    "intents": [{
      "rule_id": "task-review-notify",
      "seq": 0,
      "action": "notify",
      "target": "e4201e72",
      "severity": "info",
      "payload": "review requested",
      "receipt": "tasks/x.md#^r-6794ce82d1d5aff1"
    }],
    "narrowed": [],
    "findings": [],
    "how": "how:\n  route: channel-review\n"
  }]
}
```

`effects` is omitted when empty. Therefore every pre-amendment frame with no reaction output keeps its exact JSON bytes. A tolerant v2 client ignores the unknown sibling and reads the same `delta`.

Each array element is one reaction evaluation envelope. `intents` are admitted descriptors. `narrowed` retains complete descriptors rejected by the declared capability ceiling. `findings` carries advisory evaluation findings; slice 1 admits `budget_exceeded { rule_id, steps, mem }` and `armed_fault { rule_id?, detail }`. `how` is opaque data that the engine does not interpret.

An intent states what evaluation armed before delivery. It has exactly `rule_id`, `seq`, `action`, optional `target`, optional `severity`, optional `payload`, and the canonical `receipt` address. It has no `delivered` field and makes no delivery claim.

## Artifact faults are reported, never refused (amendment, 2026-08-01)

`armed_fault` is the reaction plane's channel onto the one artifact-fault surface (`policy::armed_law::ArmedFault`). It says: this workspace's attested armed law could not be honored, so a reaction the artifact attests did not run. `detail` is that surface's own rendering, so the operator reads the same words the write door refuses with; `rule_id` is absent when the fault is about the artifact rather than about one rule.

It is a FINDING and never an error frame, for the same reason the reaction plane exists at all: everything on it runs after the write has landed, and failing a write on a reaction's behalf would hand a hook the veto the ruling denies it. The disposition splits by kind at the surface — a red check row refuses at the door, a red hook row falls silent here — while the vocabulary does not split at all.

A fault envelope carries no intents, no `narrowed`, and an empty `how` (a fault has no declaration behind it). Empty envelopes are still dropped, so a workspace with no fault and no reaction keeps its exact pre-amendment bytes; a fault envelope is never empty and is never dropped. Before this amendment such a fault reached a `.unwrap_or_default()` at both feeder call sites and read as "nothing to react to" — a corrupt or emptied artifact on an armed workspace was indistinguishable from a quiet one.

`transport-proto.EffectFinding.armed_fault = 2` transcribes it.

## Synchronous write feedback

A successful non-dry `splice` may carry the same envelopes under `body.armed.effects`. The shared write leaf derives them only after the batch lands, from the before/after documents already held by the guarded write. The response is complete before the host flushes any live subscription frame, so it states what the write armed without claiming delivery.

`armed.effects` is omitted when empty. Dry, refused, out-of-scope, and never-armed writes therefore preserve their prior response bytes. An external edit has no acting caller and no splice response; its reaction output exists only on the associated `DeltaFrame`, with `delta.actor` absent.

`transport-proto.Armed.effects = 4` transcribes this additive response field. Empty repeated fields preserve the pre-amendment protobuf response bytes.

## Additive schedule path

The envelope is an object rather than a bare intent array. A later schedule consumer can add optional `wake_at` beside `intents` without changing `effects`, any existing intent object, or slice-1 data. Slice 1 does not add `wake_at`. On the subscribe path, `delta.root_after` remains the world fingerprint that witnesses the notification.

## Frozen Delta

`Delta`, `DeltaFile`, and `DeltaNode` are unchanged. In particular, no `keys` sub-array exists: wire §7.4 remains node-grain at birth. Reaction predicates may use finer derived facts, but those facts do not amend the Delta noun.

## Protobuf transcription

`transport-proto` consumes the event-frame slot reserved at transport birth: `Frame.notification = 3`. `DeltaFrame.effects = 2` transcribes the JSON envelope. Empty repeated fields preserve the prior protobuf Delta payload, and the agreement test round-trips populated envelopes and live notification frames.

## Open transport question

This amendment implements the additive `sub` plus `effects[]` path. It does not rule whether the reaction plane ultimately stays on that path or gains the panel's separate `{"op":"react"}` exchange returning `{intents, wake_at, root}`. Wire §16 froze ten operations with no `react`, and the source corpus does not settle whether `react` was deferred or dropped. That decision remains open.
