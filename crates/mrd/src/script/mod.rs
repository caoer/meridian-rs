//! The script entry's CLI lane (kernel entry #3, `docs/run-plane.md`
//! § The script entry).
//!
//! The trace and digest SHAPES live in `crates/effects` (moved 2026-08-12,
//! the § A.7 in-process serve): the § A.7 daemon lane assembles the same
//! `ScriptTrace` and reaches the same one `armed_digest`, so the shapes sit
//! where both lanes reach them. The sealed kernel still performs no I/O —
//! each lane's own driver fills the commit leg: this module's wire client
//! ([`cmd`], [`wire_host`]) on the CLI lane, `registry::script_op` on the
//! in-process lane.
//!
//! The ops this lane emits — `hello`, `fingerprint`, `toc`, `cat`, `splice`,
//! and nothing else — are observable without a daemon through the [`Door`]
//! seam.

pub mod cmd;
pub mod wire_host;

pub use effects::digest::{self, ArmedRow, DOMAIN_TAG, armed_digest};
pub use effects::trace::{
    self, ArmedEntry, CommitLeg, FaultClass, Refusal, ScriptFault, ScriptOutcome, ScriptTrace,
    TraceEntry,
};
pub use wire_host::Door;
