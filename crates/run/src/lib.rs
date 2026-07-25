//! The mrd-local run plane — S1, pre-eval slice (U2).
//!
//! # Charter
//! **Owns:** everything `mrd run` must resolve BEFORE any evaluation: locating
//! the addressed task block on a page ([`address`]), classifying its fence
//! language ([`fence`]), validating the declared arg/env contract against what
//! the caller supplied ([`contracts`]), and resolving the block's effective
//! capability set deny-by-default ([`caps`]). Pure resolution over the parsed
//! model — no Starlark eval, no bash exec, no splice, no effect.
//!
//! **Never does:** evaluate a block, apply an effect, touch the wire or the
//! daemon. `run` is consumer-plane — the engine cannot tell run exists.
//!
//! # The task grammar (§2.1 reuse, no new syntax)
//! A page declares tasks in frontmatter, one top-level key per task:
//!
//! ```markdown
//! ---
//! task.check-links: "[[#^chk-1]]"
//! task.fix-drift: "[[#^fix-1]]"
//! task.fix-drift.caps: md.set_field, md.append_section
//! task.fix-drift.args: page
//! task.fix-drift.env: HOME_WIKI
//! ---
//! ```
//!
//! The binding value is the Obsidian same-file block linktext (`#^id`, with or
//! without `[[…]]`/alias sugar — the §2.2 walk-plane spelling); the anchor
//! resolves via the strict mint plane (`model::resolve`, loud on missing /
//! ambiguous) to the fenced code block it keys. Cross-file refs
//! (`other.md#^id`) are an S1 NON-GOAL (plan decision #11) and refuse with a
//! typed error. Sub-keys `.caps` / `.args` / `.env` carry the block's explicit
//! capability declaration and its arg/env contract.
//!
//! # Capability law (verdict ruling 3, plan decision #15)
//! Deny-by-default: an undeclared block is read-only. Resolution precedence is
//! explicit frontmatter > `.meridian.toml` `[run.caps]` name-convention > none;
//! conventions NARROW only, never widen. `check-*` / `verify-*` blocks refuse a
//! bash fence loudly at load (`fix-*` does not — fix blocks declare writes and
//! are exactly where bash is wanted). Caps are namespaced strings
//! (`md.set_field`), forward-compatible to target-scoped (`md.set_field:status`).

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
pub mod shim;
pub mod snapshot;

pub use address::{AddressError, ResolvedTask, TaskBinding};
pub use caps::{Cap, CapResolution, CapSet, CapSource, CapsError, Conventions};
pub use contracts::{Contract, ContractError, ContractViolation};
pub use dispatch_bash::{BashDispatch, BashError, BashOutcome, Phase2};
pub use dispatch_starlark::{DispatchError, DispatchOutcome, StarlarkDispatch};
pub use exec::{ExecResult, ExecSpec, ExecStatus, TimeoutConfigError};
pub use executor::{Applied, ApplyRequest, ExecError, ReceiptAddr, WorkspaceLock};
pub use fence::{FenceError, GuaranteeClass, TaskBlock, TaskLanguage};
pub use record::{ExecRecord, ExecRecordSink, RecordError, RunLog, StdoutRecord};
pub use report::{CapsReport, EffectLine, ExecReport, Report, ReportState};
pub use runner::{Generation, RunReport, RunSpec, RunnerError, TaskOutcome};
pub use shim::{ShimDescriptor, ShimError, ShimStream};
pub use snapshot::{Detection, ExecBracket, OpenRefusal};
