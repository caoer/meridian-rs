# Wire contract v3 amendment — `hello.identity`

Status: shipped on the socket-MCP leg (Unit R1, 2026-08-05).
`docs/wire-contract-v2.md` is FROZEN and unedited; this file is normative for
the `identity` field alone. Everything else about the v3 rev stays
`docs/wire-contract-v3-amendment.md`.

## What this adds

The `hello` RESPONSE gains ONE optional field, `identity`, carrying ONE key:

```jsonc
{"id":1,"ok":true,"body":{
  "proto":1,"server":"meridian-registry/1.0",
  "caps":["toc","…","hello.identity"],
  "identity":{"build":"6c4b1f0a…"},   // the sha the answering binary was built from
  "fingerprint":"b3:…","storage":"…","workspace":"…","contract":"v3"}}
```

`identity.build` is the commit sha baked into the answering binary at compile
time — the same value `mrd --version` prints, from the same `MRD_BUILD_SHA`
env that `crates/mrd/build.rs` bakes.

## Why the field exists

A resident daemon outlives the deploy that replaced its binary. A client that
just installed a new `mrd` has no way to learn that the socket it is talking to
is still served by the old process: every frame it can ask for is a fact about
the CORPUS, and a stale daemon answers those correctly. `proto` cannot serve
here either — it is the contract's number, identical across every build of the
same contract, which is exactly the case a deploy check must catch.

So the client compares the sha it just deployed against the sha the daemon
echoes, and remediates on mismatch. The check is strict equality, and the
decision log's D1 owns what a client does with each outcome.

## The exact shape

- **Type.** `identity` is an OBJECT, never a bare string. The object is the
  extension point: a later amendment adds a sibling key without re-typing the
  slot, and a client that reads `identity.build` today keeps reading it.
- **`build` is a required key of a present `identity`.** An `identity` object
  with no `build` is not a shape this contract emits.
- **`build` is always a non-empty string.** It is `unknown` when the build
  could not name a commit; see § The `unknown` value.
- **v3-only.** A v2 session never grows the key. The typed value carries
  `Option` + `skip_serializing_if`, and the two hosts populate it only under a
  negotiated v3 session, so the frozen v2 hello body is byte-identical —
  `crates/sidecar/tests/dispatch_v2.rs::gate3_hello_caps_equal_frozen_full_list`
  pins that body whole, and it is unmoved by this amendment.
- **Additive.** The envelope is untouched. No new op, no admin verb, no proto
  bump. The field rides the same §3.2 evolution law (`tolerant client — unknown
  response fields are ignored`) that `storage` and `workspace` ride.

## The `unknown` value — a published unknown is not an absent field

Two different facts, two different spellings, and a client MUST distinguish
them:

| On the wire | Means |
|---|---|
| `"identity":{"build":"6c4b1f0a…"}` | The host publishes its build identity, and it is this sha |
| `"identity":{"build":"unknown"}` | The host publishes its build identity, and its build could not name a commit |
| `identity` absent | The host does not publish a build identity at all |

`unknown` is what `crates/mrd/build.rs` bakes when git cannot answer (no repo,
no HEAD, a source tarball). It reaches the wire VERBATIM. It is never mapped to
`null` and never dropped to make the field absent: a client that treated the
two as one would read a build-less daemon as a host that never had the
capability, and would silently skip the check it was written to perform. D1's
unknown policy — `unknown` vs sha is a mismatch, `unknown` vs `unknown` warns
loud once and proceeds — is only expressible because the two stay distinct.

## Caps string: `hello.identity`

The v3 caps projection appends `hello.identity`.

This follows the extension rule already stated for `splice.<field>` in
`docs/wire-contract-v3-amendment.md` § 8: a v3-era FIELD amendment to an op
that exists under v2 is advertised at field grain as `op.field`. `identity`
amends the `hello` response, so `hello.identity` is its name.

The bare `hello` cap is still absent, and that is not an inconsistency. §3.2
discovery honesty says a cap names something a client can ASK for; `hello` is
the door discovery itself comes through, so it is never listed
(`crates/registry/tests/hello.rs` asserts its absence). A dotted field cap
makes no claim about the op's existence — it names an amendment to the answer
the door already gives.

## What this amendment deliberately does NOT put on the wire

A doc that names what it refuses is easier to trust than one that only names
what it ships.

- **No pid, in any form or under any name.** An echoed pid is a kill primitive
  handed to whoever holds the socket, and it is untrustworthy in the one case
  that matters: a pid the daemon reports about itself is only as honest as the
  daemon, and a pid can be reused between the echo and the signal. The kill
  target is the kernel's to supply (`LOCAL_PEERPID` / `SO_PEERCRED`),
  cross-checked against the `daemon.pid` file, with `proc_pidpath` confirming
  the executable, before any signal is sent. That is a client-side sequence and
  it needs nothing from this field. (Decision log D1.)
- **No `pkg_version`.** Every crate in this workspace is `0.0.0`, because the
  workspace publishes nothing. A version key would carry zero information while
  looking authoritative, which is worse than silence.
- **No binary path, no start time, no uptime, no host.** Each answers a
  different question than "is this the build I deployed", and each is a fact
  the daemon asserts about itself rather than one the client can verify against
  something it holds. The field carries exactly what strict equality needs.
- **No admin op.** `hello` is already the handshake every client performs
  before its first read; an identity op would be a second round trip for a fact
  the first one can carry.

## The two hosts

| Host | `identity` under v3 |
|---|---|
| Resident daemon (`mrd daemon` → `registry`) | The sha baked into the running `mrd` binary. `mrd daemon` reads `MRD_BUILD_SHA` and puts it in `registry::Config`; the accept loop carries it to `hello_body`, which emits it |
| Per-request sidecar (`crates/sidecar`) | `{"build":"unknown"}` — the shape, honestly valued |
| Any host with no build sha configured (in-process test servers) | Absent |

The registry crate cannot read `MRD_BUILD_SHA` itself: `crates/mrd/build.rs`
bakes it into the `mrd` crate's compilation environment alone, so a
`env!("MRD_BUILD_SHA")` in `registry` would not compile. The value is
therefore threaded — `mrd daemon` → `Config::build_sha` → the accept loop →
`hello_body` — rather than read at the emission site. A reader tempted to
"simplify" that into a direct `env!` should know it was tried in the design and
is not available.

The sidecar bakes no build sha (it has no build script, and it is scheduled for
deletion), so `unknown` is the true answer for it rather than a placeholder. It
publishes the field anyway so that one v3 client speaks to both hosts with one
code path, and D1's unknown-vs-unknown arm covers what it means.

## v2 freeze guarantee

No v2 byte moves. Stated as the three separate facts that make it true:

1. The typed `ResponseBody::Hello` grows `Option<Identity>` with
   `skip_serializing_if = "Option::is_none"`, so a `None` serializes away
   entirely — no key, no `null`.
2. Both hosts populate it under a negotiated v3 session ONLY. A v2 session's
   hello body is constructed with `None`.
3. The cap is pushed by the v3 projection
   (`crates/wire-serve/src/rev.rs::project_response`), never by a host's `CAPS`
   constant, so the FROZEN v2 caps arrays are unedited.

The enforcing tests are `crates/sidecar/tests/dispatch_v2.rs`
(`gate3_hello_caps_equal_frozen_full_list` pins the whole v2 hello body — caps
list and key set both) and `crates/wire/tests/contract_v2.rs`.

## Tests

- `crates/registry/tests/hello.rs` — a v3 hello carries `identity.build` equal
  to the configured sha; the `unknown` fallback reaches the wire verbatim; a
  daemon with no configured sha omits the field and a v3 client accepts that
  hello (optionality is real, not nominal); `hello.identity` is advertised; a
  v2 session carries neither the field nor the cap.
- `crates/wire/tests/u27_frozen_key_sets.rs` — the maximal hello key set grows
  `identity` and nothing else.
- `crates/sidecar/tests/dispatch_v2.rs` — unchanged and still green: the v2
  hello body is asserted whole against a frozen literal, so the freeze
  guarantee above is enforced by an existing assertion rather than a new one.
