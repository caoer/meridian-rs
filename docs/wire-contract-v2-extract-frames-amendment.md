# Wire contract v2 amendment — the extract response frames

Status: normative amendment to `docs/wire-contract-v2.md` §4.3.
`docs/wire-contract-v2.md` is FROZEN and unedited; this file is the sole
normative text for the `extract` response frame, its node object, and the
per-kind `info` payloads. Advisor ruling, 2026-08-04, on the U27b sweep.

## What this amends, and why it is restoration rather than change

§4.3 specifies the `extract` REQUEST and then describes the response in prose —
"full node objects, 11-variant kind enum …, per-kind `info`, `text_prefix_16b`
…, total node order" — deferring the shape itself to "`crates/wire` §5".

**That is a dangling reference.** No `crates/wire` §5 document exists; the
citation resolves to a section comment inside the implementation. So the frozen
specification of this response shape was the code, and any test taken from the
wire could only DESCRIBE the shape, never VERIFY it — the oracle and the subject
were the same artifact.

Supplying the missing record is restoration, not amendment of frozen content:
nothing here changes what the engine serves, and no byte of the frozen document
moves. The frames below are what §4.3 always meant; they simply were never
written down.

## The worked frames

Computed against the §0.3 S0 fixture through the live serve loop, on a v2
session, exactly as the §4.1/§4.2 worked exchanges are.

### The response body

```json
{"id":5,"op":"extract","path":"notes/plan.md"}
{"id":5,"ok":true,"body":{"path":"notes/plan.md","nodes":[
 {"kind":"frontmatter","span":[0,20],"node_rev":"26796ebec5d0bf1a",
  "text_prefix_16b":"---\ntitle: Plan\n","info":{"keys":["title"]}},
 {"kind":"heading","hpath":[{"h":"Goals"}],"span":[20,136],
  "node_rev":"a6665baff294bd04","text_prefix_16b":"# Goals\n\nShip th"},
 {"kind":"heading","hpath":[{"h":"Goals"},{"h":"Q3"}],"span":[49,72],
  "node_rev":"33d5b0e1b27cb48b","text_prefix_16b":"## Q3\n\nship by A"},
 {"kind":"heading","hpath":[{"h":"Goals"},{"h":"Q4"}],"span":[72,136],
  "node_rev":"4b8bc385a58da0e0","text_prefix_16b":"## Q4\n\n- item on"},
 {"kind":"wikilink","span":[96,110],"node_rev":"d9affe5403cc3cdc",
  "text_prefix_16b":"[[2026-07-18]]\n-","info":{"target":"2026-07-18"}},
 {"kind":"wikilink","span":[124,135],"node_rev":"63be19beb5cec9df",
  "text_prefix_16b":"[[roadmap]]\n","info":{"target":"roadmap"}}]}}
```

The body is exactly `{path, nodes}`. Unlike `toc` (§4.1) it carries neither
`file_rev` nor an ambient `root`: `extract` is a node inventory, not the write
kit, and the commit-guard idiom rides `toc`.

### The node object

Four keys are unconditional — `kind`, `span`, `text_prefix_16b`, and `node_rev`
(MUST while `splice ∈ caps`, the §3.2 rev-presence law). Three ride
conditionally:

| key | present when |
|---|---|
| `hpath` | the node is a `heading` — the §2.1 segment array |
| `info` | the kind has an `info` shape (below); absent for kinds that have none |
| `unterminated` | only when true |

Node order is the frozen total order: `span.start` ascending, `span.end`
descending, then the kind ordinal (declaration order of the 11-variant enum,
`frontmatter` = 0 … `comment` = 10).

### The per-kind `info` payloads

`info` is keyed by the sibling `kind`; kinds not listed omit the key entirely.

| kind | `info` |
|---|---|
| `frontmatter` | `{"keys":["title"]}` — the frontmatter key names, in file order |
| `fence` | `{"info_string":"rust"}` |
| `wikilink`, `embed` | `{"target":"2026-07-18"}` plus optional `heading`, `block`, `alias`; `heading` and `block` are mutually exclusive, and `block` uses the §2.4 charset |
| `callout` | `{"type":"note","fold":"+"}` |
| `task` | `{"checked":true,"depth":1}` |

## What this amendment does NOT do

- **No shape changes.** Every frame above is what the engine already serves.
- **No v3 fields.** `n`, `hpath_text` and `words` share the node struct and are
  v3-additive; a v2 session emits none of them, and that exclusion is the
  frozen shape, not an omission from this record.
- **No new rev, no cap string.** Nothing is added to the wire, so §3.2
  negotiation is untouched.

## Executable record

With the doc half supplied, the U27 pins verify rather than describe:

| pin | asserts |
|---|---|
| `crates/sidecar/tests/u27_v2_key_set_pins.rs::extract_body_and_node_key_sets_are_frozen` | the body, the frontmatter node, the heading node, the wikilink node, and both `info` payloads — each an exhaustive key-set `assert_eq!` from the wire |
| `crates/wire/tests/u27_frozen_key_sets.rs::extract_body_key_set_is_frozen` | the body at the type |
| `crates/wire/tests/u27_frozen_key_sets.rs::extract_node_key_set_is_frozen_plus_the_v3_host_face_trio` | the node type's full admitted key set, v3 trio included, so a fourth additive field cannot appear unseen |
| `crates/wire/tests/u27_frozen_key_sets.rs::extract_info_key_sets_are_frozen` | each per-kind `info` shape |

Before this record, `crates/sidecar/tests/contract_v3.rs::v2_extract_never_carries_addressing_keys`
was the only guard on this surface, and it enumerates three forbidden names —
a denylist a fourth v3-additive field walks past green.
