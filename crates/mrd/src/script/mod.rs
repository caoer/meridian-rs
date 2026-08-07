//! The script entry's consumer plane (kernel entry #3, `docs/run-plane.md`
//! § The script entry).
//!
//! It lives HERE and not in `crates/effects` for one structural reason: the
//! commit leg of a script's trace is a §4.4 splice **response**, and the sealed
//! kernel can never see one — it does no I/O. Assembly is consumer-plane by
//! construction.
//!
//! U3 owns the trace; the wire client that fills it is U4.

pub mod trace;

pub use trace::{
    ArmedEntry, CommitLeg, FaultClass, ScriptFault, ScriptOutcome, ScriptTrace, TraceEntry,
};
