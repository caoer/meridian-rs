//! I4 def-conformance (U8c) — engine-side port of meridian-go's write-time
//! def validator. Pure verdict over (prev, candidate) documents the put path
//! consults before any splice. Byte-exact against the U0 defs goldens.

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
    InvalidPropertyKey, MultiLineValue, PlanEdit, SafeKey, Seg, ensure_trailing_nl, rebuild, rev8,
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
