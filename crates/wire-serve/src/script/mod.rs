//! The script plane's serve half — ONE implementation both lanes share
//! (wire-contract § A.7; run-plane § The script entry).
//!
//! - [`digest`] — the armed-set digest, the ONE definition (`expect_armed`).
//! - [`trace`] — trace assembly over [`wire::ScriptTrace`]: the commit leg is
//!   the §4.4 splice response embedded verbatim, so assembly lives where
//!   splice responses exist (the sealed kernel can never see one).
//! - [`faces`] — the §4.1/§4.2 read faces served from a borrowed document:
//!   what the in-process host answers `read()` with.
//! - [`exec`] — the in-process attempt: entry world in, [`wire::ScriptTrace`]
//!   out. The daemon's `Op::Script` arm is a thin caller of
//!   [`exec::serve_script`]; the CLI lane keeps its wire-client host in
//!   `mrd::script` and re-exports the shared pieces from here.

pub mod digest;
pub mod exec;
pub mod faces;
pub mod trace;
