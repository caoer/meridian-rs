//! The script entry's CLI lane (kernel entry #3, `docs/run-plane.md`
//! § The script entry).
//!
//! The trace and digest SHAPES live in `crates/effects` (moved 2026-08-12,
//! the § A.7 in-process serve): the § A.7 daemon lane assembles the
//! `ScriptTrace` and reaches the one `armed_digest`, so the shapes sit where
//! both the producer and this consumer reach them.
//!
//! **One lane** (card `script-door-commit-premise-world-grain-vs-touch-set`).
//! The whole attempt IS the wire `script` op: the daemon pins the entry,
//! expands, evaluates and commits under the §4.6 TOUCH SET, and this module
//! parses argv, sends the program, and renders the trace that comes back. It
//! fills no commit leg of its own — `registry::script_op` is the only driver —
//! which is what keeps exactly ONE commit-premise implementation in the system.
//! ([`wire_host`]'s read lowering and [`cmd`]'s local transaction are the
//! retired lane, unreachable and deleted in that card's PR 2.)
//!
//! The ops this lane emits — `hello` and `script`, and nothing else — are
//! observable without a daemon through the [`Door`] seam.

pub mod cmd;
pub mod wire_host;

pub use effects::digest::{self, ArmedRow, DOMAIN_TAG, armed_digest};
pub use effects::trace::{
    self, ArmedEntry, CommitLeg, FaultClass, Refusal, ScriptFault, ScriptOutcome, ScriptTrace,
    TraceEntry,
};
pub use wire_host::Door;
