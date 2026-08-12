//! The script entry's CLI lane (kernel entry #3, `docs/run-plane.md`
//! § The script entry).
//!
//! The trace TYPE is `wire::ScriptTrace` and the assembly/digest are
//! `wire_serve::script` since the § A.7 in-process lane landed — ONE shape,
//! one implementation, both lanes (`mrd script --json` and the wire `script`
//! op body cannot drift). This module keeps the CLI lane's own half: the
//! wire-client host ([`cmd`], [`wire_host`]) whose ops — `hello`,
//! `fingerprint`, `toc`, `cat`, `splice`, and nothing else — are observable
//! without a daemon through the `Door` seam.

pub mod cmd;
pub mod wire_host;

/// The armed-set digest — re-exported from the one definition
/// (`wire_serve::script::digest`).
pub mod digest {
    pub use wire_serve::script::digest::{ArmedRow, DOMAIN_TAG, armed_digest};
}

/// The trace vocabulary and its assembly — re-exported from the shared
/// implementation (`wire::ScriptTrace` + `wire_serve::script::trace`).
pub mod trace {
    pub use wire::{
        ArmedEntry, FaultClass, ReadEntry, ScriptFault, ScriptOutcome, ScriptTrace, TraceEntry,
    };
    pub use wire_serve::script::trace::{CommitLeg, Refusal, assemble, fault_of, guard_refused};
}

pub use digest::{ArmedRow, DOMAIN_TAG, armed_digest};
pub use trace::{
    ArmedEntry, CommitLeg, FaultClass, Refusal, ScriptFault, ScriptOutcome, ScriptTrace, TraceEntry,
};
pub use wire_host::Door;
