//! The script entry's consumer plane (kernel entry #3, `docs/run-plane.md`
//! § The script entry).
//!
//! It lives HERE and not in `crates/effects` for one structural reason: the
//! commit leg of a script's trace is a §4.4 splice **response**, and the sealed
//! kernel can never see one — it does no I/O. Assembly is consumer-plane by
//! construction.
//!
//! U3 owns the trace; U4's wire client ([`cmd`], [`wire_host`]) fills it. The
//! client is a `Door` away from the socket, so the ops it emits — `hello`,
//! `fingerprint`, `toc`, `cat`, `splice`, and nothing else — are observable
//! without a daemon.

pub mod cmd;
pub mod digest;
pub mod trace;
pub mod wire_host;

pub use digest::{ArmedRow, DOMAIN_TAG, armed_digest};
pub use trace::{
    ArmedEntry, CommitLeg, FaultClass, Refusal, ScriptFault, ScriptOutcome, ScriptTrace, TraceEntry,
};
pub use wire_host::Door;
