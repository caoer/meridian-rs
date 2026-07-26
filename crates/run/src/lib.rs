//! The mrd-local run plane — `mrd run` (S1, as shipped).
//!
//! # Charter
//! **Owns:** the whole of `mrd run` — resolve, dispatch, execute, apply.
//! Pre-eval resolution locates the addressed task block ([`address`]),
//! classifies its fence language ([`fence`]), validates the declared arg/env
//! contract ([`contracts`]) and resolves the block's capability set
//! deny-by-default ([`caps`]). Past that, [`runner`] dispatches on the fence
//! language — `starlark` to the hermetic kernel ([`dispatch_starlark`]),
//! `bash` to a `setsid` child under a wall-clock timeout ([`exec`],
//! [`dispatch_bash`]) whose only tree mutation is the effect-shim fd
//! ([`shim`]) — and both converge on the ONE write path, the shared executor
//! ([`executor`]): caps validated at the choke point, one `if_root`-pinned
//! splice batch, the receipt in the same commit. [`gate`] refuses an armed
//! change before that commit; [`snapshot`] names any residual delta around
//! every bash step; [`record`] stores stdout out-of-tree, content-addressed.
//!
//! **Never does:** touch the wire or the daemon, or roll back an ungoverned
//! write (decision #14 — it persists as actor-absent external change). `run`
//! is consumer-plane — the engine cannot tell run exists. Bash enforcement is
//! **detection, not prevention** (ratified S1 scope): the claim ships scoped,
//! never unqualified. Full surface + accepted gaps: `docs/run-plane.md`.
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
