//! I4 def-conformance (U8c) — the engine-side port of meridian-go's
//! `internal/defs` write-time validator, byte-exact against the U0 defs corpus
//! goldens (`ccc-statusd` `internal/mcpserver/testdata/parity/goldens/defs.json`).
//!
//! # What this is
//! The pure verdict half of Go `defs.CheckWrite`: given the PREV document and
//! the CANDIDATE (post-write) document, reproduce the whole severity ladder for
//! the twelve write-blocking def guards plus the close-stamp repair:
//!
//! ```text
//! error   → refuse (never forceable)
//! warning → refuse unless force (forced rule ids surface in `forced`)
//! repair  → autofill (v1: stamp:close timestamps on a terminal transition)
//! ```
//!
//! Findings are DELTA-scored against prev (key: `rule + "|" + message`,
//! line-shift immune): only a write that INTRODUCES a violation refuses.
//! `def/legacy-entry` never blocks; a kind with NO def anywhere passes
//! (undeclared ≠ contract); a def that EXISTS but fails to load refuses unless
//! forced (rot in the schema layer is loud, never a silent skip).
//!
//! The candidate REBUILD (plan-vocabulary edits → next document — Go
//! `body.ApplyForConformance`) is the write plane's and lives with the splice
//! machinery in `wire-serve`, not here: this module judges two documents.
//!
//! # Laws inherited
//! - **Raw bytes only** (worker A, norm-v2 spec): conformance validates the raw
//!   pre-write bytes and the raw candidate — never norm-v2 canonical bytes.
//!   norm-v2/fingerprint is the hash domain; validation faces stay raw.
//! - **Clock is caller-supplied** (§9): close-stamp repairs take `now` from the
//!   request; the engine mints no time.
//! - **String fidelity**: every message/remedy byte matches the Go lib — the
//!   parity gate compares refusal text verbatim. `go_fmt` carries the Go
//!   formatting quirks (strconv.Quote, `%v` slices).

// Verbatim-port pedantic allowances: function shapes mirror the Go source for
// side-by-side auditability (too_many_lines, manual_let_else, match_same_arms);
// casts are bounded by construction (heading depth ≤ 6, physical line counts);
// naive byte counting avoids a bytecount dependency for a cold path.
#![allow(
    clippy::too_many_lines,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::missing_errors_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::naive_bytecount
)]

mod cascade;
mod check;
mod fm;
mod go_fmt;
mod i4;
mod load;
mod rebuild;
mod shape;

pub use cascade::{ResolveOutcome, discover_layers, resolve};
pub use check::{
    Report, SecView, SectionVerdict, VERDICT_INVALID, VERDICT_LEGACY, VERDICT_VALID, check,
    doc_sections, scan_nested,
};
pub use fm::{FmMeta, FmValue, parse_meta, string_field};
pub use go_fmt::go_quote;
pub use i4::{ConformanceRequest, ConformanceResult, Repair, conformance};
pub use load::{Def, DefError, PropSpec, SectionRule, parse_def};
pub use rebuild::{
    InvalidPropertyKey, MultiLineValue, PlanEdit, SafeKey, ensure_trailing_nl, rebuild, rev8,
    yaml_safe_key, yaml_safe_value,
};

/// One validator finding (Go `types.Finding`), delta-scored by (rule, message).
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub rule_id: String,
    /// "error" | "warn" | "info"
    pub severity: &'static str,
    pub file_path: String,
    /// 0 when unpositioned (Go zero-value semantics).
    pub line: i64,
    pub message: String,
}

/// A structured refusal (Go `body.Error`): code + message + executable remedy.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyError {
    pub code: String,
    pub message: String,
    pub remedy: String,
    pub context: Vec<(String, String)>,
}

impl BodyError {
    /// Go `(*Error).Error()`: `CODE: message — remedy` (remedy-less: no dash).
    #[must_use]
    pub fn render(&self) -> String {
        if self.remedy.is_empty() {
            return format!("{}: {}", self.code, self.message);
        }
        format!("{}: {} — {}", self.code, self.message, self.remedy)
    }
}
