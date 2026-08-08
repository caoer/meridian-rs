//! The I4 severity ladder — Go `internal/defs/i4.go` `SpliceConformance`:
//! error → refuse (never forceable); warning → refuse unless force;
//! repair → autofill (v1: close stamps on a terminal transition). Findings are
//! DELTA-scored against prev (key `rule|message`); `def/legacy-entry` never
//! blocks; an undeclared kind passes; a malformed def refuses-unless-forced.

use std::path::Path;

use super::cascade::{ResolveOutcome, discover_layers, resolve};
use super::check::{check, scan_nested};
use super::fm::{self};
use super::load::Def;
use super::{BodyError, Finding};

/// The verdict request: prev + candidate documents plus write context.
/// `now` is the caller-supplied clock (RFC3339 or naive) — repairs derive
/// their stamp from it; the engine mints no time (§9).
pub struct ConformanceRequest<'a> {
    pub target: &'a str,
    pub actor: &'a str,
    pub force: bool,
    pub now: &'a str,
    pub prev: &'a model::Document,
    pub next: &'a model::Document,
    /// Markdown parse for def FILES read during resolution (the `syntax` edge
    /// stays the caller's — policy consumes pre-built documents).
    pub build_doc: &'a dyn Fn(&str) -> model::Document,
}

/// One autofill edit (v1: `OpSetProperty` only).
#[derive(Debug, Clone, PartialEq)]
pub struct Repair {
    pub key: String,
    pub value: String,
}

/// The verdict (Go `body.ConformanceResult`).
#[derive(Debug, Clone, Default)]
pub struct ConformanceResult {
    pub refuse: Option<BodyError>,
    pub repairs: Vec<Repair>,
    pub forced: Vec<String>,
}

/// The I4 hook. Never mutates anything — repairs return as edits for the
/// write path to apply through its own plan machinery.
#[must_use]
pub fn conformance(req: &ConformanceRequest<'_>) -> ConformanceResult {
    let (kind, preset) = record_kind(req.next);
    if kind.is_empty() {
        // Not a typed record: only the substrate law applies.
        if let Some(refuse) = nested_delta(req) {
            return ConformanceResult {
                refuse: Some(refuse),
                ..Default::default()
            };
        }
        return ConformanceResult::default();
    }

    let def = match resolve(
        &kind,
        &preset,
        &discover_layers(Path::new(req.target)),
        req.build_doc,
    ) {
        ResolveOutcome::Resolved(d) => d,
        ResolveOutcome::NoDef => {
            if let Some(refuse) = nested_delta(req) {
                return ConformanceResult {
                    refuse: Some(refuse),
                    ..Default::default()
                };
            }
            return ConformanceResult::default();
        }
        ResolveOutcome::Malformed(err) => {
            if let Some(refuse) = nested_delta(req) {
                return ConformanceResult {
                    refuse: Some(refuse),
                    ..Default::default()
                };
            }
            if req.force {
                return ConformanceResult {
                    forced: vec!["def/malformed".to_string()],
                    ..Default::default()
                };
            }
            return ConformanceResult {
                refuse: Some(BodyError {
                    code: "E_CONFORMANCE".to_string(),
                    message: format!(
                        "the {kind} def failed to load, so this write cannot be validated: {err}"
                    ),
                    remedy: "fix the def file (fail closed), or re-submit with force to override — forced writes render a `forced:` verdict and surface in the def census".to_string(),
                    context: vec![
                        ("kind".to_string(), kind.clone()),
                        ("rule".to_string(), "def/malformed".to_string()),
                    ],
                }),
                ..Default::default()
            };
        }
    };

    let repairs = plan_close_stamps(req.next, &def, req.now);
    let (mut new_errs, new_warns) = delta_findings(
        &check(req.prev, &def, req.target).findings,
        &check(req.next, &def, req.target).findings,
    );
    if !repairs.is_empty() {
        // The repair resolves exactly the "terminal without closed_at"
        // direction of the biconditional — drop it from the error set.
        new_errs.retain(|f| f.rule_id != "def/biconditional");
    }
    if !new_errs.is_empty() {
        return ConformanceResult {
            refuse: Some(conformance_refusal(
                &kind,
                req.target,
                &new_errs,
                &format!(
                    "the write was refused; the file is unchanged. Fix the violation (`md def check {}` explains the def) — errors are never forceable",
                    req.target
                ),
            )),
            ..Default::default()
        };
    }
    if !new_warns.is_empty() && !req.force {
        return ConformanceResult {
            refuse: Some(conformance_refusal(
                &kind,
                req.target,
                &new_warns,
                "fix the content, or re-submit with force to override — forced warnings render a `forced:` verdict and surface in the def census (R-force)",
            )),
            ..Default::default()
        };
    }
    let mut res = ConformanceResult {
        repairs,
        ..Default::default()
    };
    if req.force {
        res.forced = rule_ids(&new_warns);
    }
    res
}

/// The candidate's type/preset; unreadable frontmatter yields "".
fn record_kind(doc: &model::Document) -> (String, String) {
    match fm::parse_meta(&doc.raw) {
        Ok(Some(meta)) => (
            fm::string_field(&meta, "type"),
            fm::string_field(&meta, "preset"),
        ),
        _ => (String::new(), String::new()),
    }
}

/// Refuse a write that INTRODUCES a nested-frontmatter violation on a document
/// with no def to validate against. Never forceable.
fn nested_delta(req: &ConformanceRequest<'_>) -> Option<BodyError> {
    let (new_errs, _) = delta_findings(
        &scan_nested(req.prev, req.target),
        &scan_nested(req.next, req.target),
    );
    if new_errs.is_empty() {
        return None;
    }
    Some(conformance_refusal(
        "",
        req.target,
        &new_errs,
        "the write was refused; the file is unchanged. Flatten the value (nested frontmatter is an error always, from any writer) — never forceable",
    ))
}

/// The v1 repair rung: terminal status + an empty `stamp: close` property →
/// autofill with `now` in the Go stamp format `2006-01-02T15:04:05` (the naive
/// prefix of an RFC3339 `now`).
fn plan_close_stamps(doc: &model::Document, def: &Def, now: &str) -> Vec<Repair> {
    let Some(spec) = def.props.get("status") else {
        return Vec::new();
    };
    if spec.terminal.is_empty() {
        return Vec::new();
    }
    let Ok(Some(meta)) = fm::parse_meta(&doc.raw) else {
        return Vec::new();
    };
    let status = fm::string_field(&meta, "status");
    if !spec.terminal.contains(&status) {
        return Vec::new();
    }
    let stamp = stamp_now(now);
    let mut out = Vec::new();
    for (key, p) in &def.props {
        let empty = fm::is_empty(meta.get(key.as_str()));
        if p.stamp == "close" && empty {
            out.push(Repair {
                key: key.clone(),
                value: stamp.clone(),
            });
        }
    }
    out // BTreeMap iteration = sorted keys, matching Go's sort.Strings
}

/// `2006-01-02T15:04:05` from an RFC3339/naive `now` (seconds precision).
fn stamp_now(now: &str) -> String {
    let naive: String = now.chars().take(19).collect();
    naive
}

/// Findings present in next but NOT in prev, split by severity rung. Keying is
/// (rule, message) — line numbers shift under a splice and must not make an
/// old finding look new. `def/legacy-entry` warns never block.
fn delta_findings(prev: &[Finding], next: &[Finding]) -> (Vec<Finding>, Vec<Finding>) {
    let seen: std::collections::HashSet<String> = prev
        .iter()
        .map(|f| format!("{}|{}", f.rule_id, f.message))
        .collect();
    let mut errs = Vec::new();
    let mut warns = Vec::new();
    for f in next {
        if seen.contains(&format!("{}|{}", f.rule_id, f.message)) {
            continue;
        }
        match f.severity {
            "error" => errs.push(f.clone()),
            "warn" if f.rule_id != "def/legacy-entry" => {
                warns.push(f.clone());
            }
            _ => {}
        }
    }
    (errs, warns)
}

/// Render a refusal naming the first violation (and how many more), the def
/// kind, and the executable remedy.
fn conformance_refusal(kind: &str, target: &str, findings: &[Finding], remedy: &str) -> BodyError {
    let mut msg = findings[0].message.clone();
    if findings.len() > 1 {
        use std::fmt::Write as _;
        let _ = write!(msg, " (and {} more)", findings.len() - 1);
    }
    let subject = if kind.is_empty() {
        "the record's def".to_string()
    } else {
        format!("the {kind} def")
    };
    BodyError {
        code: "E_CONFORMANCE".to_string(),
        message: format!("write violates {subject}: {msg}"),
        remedy: remedy.to_string(),
        context: vec![
            ("kind".to_string(), kind.to_string()),
            ("target".to_string(), target.to_string()),
            ("rules".to_string(), rule_ids(findings).join(",")),
        ],
    }
}

fn rule_ids(findings: &[Finding]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in findings {
        if !out.contains(&f.rule_id) {
            out.push(f.rule_id.clone());
        }
    }
    out
}
