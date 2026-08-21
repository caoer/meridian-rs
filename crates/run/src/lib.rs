//! The mrd-local run plane — `mrd run` (S1).
//!
//! # Charter
//! **Owns:** resolve, dispatch, execute, apply for `mrd run`.
//! Pre-eval: address the task ([`address`]), classify fence ([`fence`]),
//! validate arg/env ([`contracts`]), resolve caps deny-by-default ([`caps`]).
//! Dispatch: `starlark` → hermetic kernel ([`dispatch_starlark`]); `bash` →
//! `setsid` child under wall-clock timeout ([`exec`], [`dispatch_bash`]) with
//! NO governed-tree effect channel — bash observes and reports; governed
//! writes ride the wire faces (MCP `put`) or a starlark task. Starlark
//! converges on the shared executor ([`executor`]): caps at the choke point,
//! one unguarded
//! splice batch (no world pin — no-guard-on-effects ruling, 2026-08-15),
//! receipt in the same commit. [`gate`] refuses armed changes
//! before commit; [`snapshot`] names residual delta around bash; [`record`]
//! stores stdout out-of-tree, content-addressed.
//!
//! **Never does:** touch the wire or the daemon, or roll back an ungoverned
//! write (decision #14 — persists as actor-absent external change). Bash
//! enforcement is **detection, not prevention** (S1): claims ship scoped,
//! never unqualified. Full surface: `docs/run-plane.md`.
//!
//! # Task grammar (§2.1 reuse, no new syntax)
//!
//! ```markdown
//! ---
//! task.check-links: "[[#^chk-1]]"
//! task.fix-drift: "[[#^fix-1]]"
//! task.fix-drift.caps: md.edit, md.create:tasks/*.md
//! task.fix-drift.args: page
//! task.fix-drift.env: HOME_WIKI
//! ---
//! ```
//!
//! Binding value is same-file block linktext (`#^id`); resolved via the strict
//! mint plane. Cross-file refs are S1 NON-GOAL (decision #11). Sub-keys
//! `.caps` / `.args` / `.env` carry capability and input contracts.
//!
//! # Capability law (verdict ruling 3, plan decision #15; caps-redesign 2026-08-19)
//! Deny-by-default: undeclared block is read-only. Precedence: explicit
//! `task.<name>.caps` > root `MERIDIAN.md` `run.caps.<pattern>` > none.
//! Conventions NARROW only. `check-*` / `verify-*` refuse bash at load.
//! Caps are the three verbs (`md.create` / `md.edit` / `md.delete`),
//! optionally path-glob-scoped (`md.create:tasks/*.md`), matched against
//! DECLARED coordinates. Declaring root and timeout are injected by the
//! caller. Full law: [`caps`].

pub mod address;
pub mod caps;
pub mod contracts;
pub mod dispatch_bash;
pub mod dispatch_starlark;
pub mod exec;
pub mod executor;
pub mod fence;
mod fp;
pub mod gate;
pub mod record;
pub mod report;
pub mod runner;
pub mod snapshot;

pub use address::{AddressError, ResolvedTask, TaskBinding};
pub use caps::{
    Authority, Cap, CapResolution, CapSet, CapSource, CapsError, ConventionSource, Conventions,
};
pub use contracts::{Contract, ContractError, ContractViolation};
pub use dispatch_bash::{BashDispatch, BashError, BashOutcome, Phase2};
pub use dispatch_starlark::{DispatchError, DispatchOutcome, StarlarkDispatch};
pub use exec::{ExecResult, ExecSpec, ExecStatus, TimeoutConfigError};
pub use executor::{Applied, ApplyRequest, ExecError, ReceiptAddr, WorkspaceLock};
pub use fence::{FenceError, GuaranteeClass, TaskBlock, TaskLanguage};
pub use record::{ExecRecord, ExecRecordSink, RecordError, RunLog, StdoutRecord};
pub use report::{CapsReport, EffectLine, ExecReport, Report, ReportState};
pub use runner::{Generation, RunReport, RunSpec, RunnerError, TaskOutcome};
pub use snapshot::{Detection, ExecBracket, OpenRefusal};
