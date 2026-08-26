//! Mismatch-recovery ladder.
//!
//! On fingerprint mismatch, never instruct a whole-file read. Refusal carries
//! the richest rung it can compute:
//!
//! 1. change diff, when computable;
//! 2. else new content of the node + new fingerprint;
//! 3. else bare mismatch.
//!
//! `guard_required` is rung zero: additive over the existing `cas_mismatch` +
//! `Recovery::Refresh` — no new error code.
//!
//! The engine stores no prior versions: the baseline is the caller's picture in
//! the request (`match.old`); rung 1 diffs it against current bytes.
//! [`DIFF_CAP_BYTES`] keeps rung 1 richer than rung 2 — a far-apart picture
//! trips the cap and falls to rung 2.
//!
//! Form follows the fingerprint scope that tripped: section/block → unified
//! line diff; frontmatter → ops form (a line diff teaches the wrong edit).
//! Scope picks form; no content sniff.
//!
//! # Scope is a security rule
//! A rung discloses the caller's targeted node only — bytes outside would be
//! unasked disclosure. Pinned by
//! `rung_one_discloses_nothing_outside_the_callers_targeted_section`.

use std::fmt::Write as _;

use wire::{Edit, EditShape, ErrorBody, NodeRev, Path, SecRef};

/// The rung-1 size cap, in bytes of rendered diff. Past it the diff has stopped
/// being the cheaper answer and rung 2 is sent instead. Sized at one screen of
/// context.
pub const DIFF_CAP_BYTES: usize = 2048;

/// The caller's targeted node, as the ladder sees it.
struct Rungs<'a> {
    /// How the node is named back to the caller.
    subject: String,
    /// The caller's own picture of the node's bytes, when the request carried
    /// one. `None` ⇒ rung 1 is not computable.
    baseline: Option<&'a str>,
    /// The node's current bytes — the caller's targeted node, whole, and
    /// nothing beyond it.
    current: String,
    /// The token to resend with.
    new_fingerprint: NodeRev,
    /// Set when the frontmatter scope tripped; picks the diff form.
    frontmatter_key: Option<String>,
}

/// The ladder: enrich a `cas_mismatch` envelope with the richest rung that is
/// computable for the edit that tripped, leaving `code`, `recovery`,
/// `expected`, and `actual` exactly as the caller's contract already binds them.
///
/// `edits` is the effective (post-lowering) batch, so the plan face reaches this
/// with its `old` intact and needs no separate path.
///
/// Always stamps `rung`, so a caller never probes the extras to learn what it
/// was given.
pub fn enrich(e: &mut ErrorBody, doc: &model::Document, edits: &[Edit], path: &Path) {
    e.path = Some(path.clone());
    let file = path.0.as_str();
    let Some(r) = tripped(doc, edits) else {
        floor(e, file);
        return;
    };

    if let Some(baseline) = r.baseline
        && let Some(rendered) = match &r.frontmatter_key {
            Some(key) => Some(ops_form(key, &r.current)),
            None => unified_line_diff(baseline, &r.current),
        }
        && rendered.len() <= DIFF_CAP_BYTES
    {
        e.rung = Some(1);
        e.diff = Some(rendered);
        e.new_fingerprint = Some(r.new_fingerprint);
        e.message = Some(format!(
            "{} in {file} changed under the `if_node_rev` your write pinned. {} Fix: apply the \
             `diff` extra to your copy, then resend the edit with `new_fingerprint` as its \
             `if_node_rev` guard, or repeat it with `force` — no re-read is needed.",
            r.subject,
            crate::NO_PARTIAL_WRITE_CLAUSE
        ));
        return;
    }

    e.rung = Some(2);
    e.new_content = Some(r.current);
    e.new_fingerprint = Some(r.new_fingerprint);
    e.message = Some(format!(
        "{} in {file} changed under the `if_node_rev` your write pinned. {} Fix: take \
         `new_content` as that node's current bytes, re-decide your edit, and resend it with \
         `new_fingerprint` as its `if_node_rev` guard, or repeat it with `force` — no re-read \
         is needed.",
        r.subject,
        crate::NO_PARTIAL_WRITE_CLAUSE
    ));
}

/// Rung 3 — the floor. Nothing about the node is computable here (its target no
/// longer resolves, or no guarded edit is identifiable), so the refusal sends
/// the caller to the one node it asked about — still never a whole-file read.
pub fn floor(e: &mut ErrorBody, file: &str) {
    e.rung = Some(3);
    e.message = Some(format!(
        "the `if_node_rev` your write pinned is not this node's — it changed under your plan, \
         and its current bytes are not computable here. {} Fix: re-read the ONE node you \
         targeted in {file} for its current `sec_rev`, re-decide, and resend with that token as \
         `if_node_rev`, or repeat the write with `force`.",
        crate::NO_PARTIAL_WRITE_CLAUSE
    ));
}

/// Which edit tripped: the first guarded edit whose pinned token is not its
/// target's current one — the same first-failure order `model::validate` uses,
/// so the rung describes the edit the verdict is about.
fn tripped<'a>(doc: &model::Document, edits: &'a [Edit]) -> Option<Rungs<'a>> {
    for edit in edits {
        let Some(pinned) = edit.if_node_rev.as_ref() else {
            continue;
        };
        let Ok(target) = crate::read::to_model_ref(&edit.target) else {
            continue;
        };
        let Ok(resolved) = model::resolve(doc, &target) else {
            continue;
        };
        if resolved.node_rev.0 == pinned.0 {
            continue;
        }
        let current = doc.raw.get(resolved.span.clone())?.to_owned();
        return Some(Rungs {
            subject: subject_of(&edit.target),
            baseline: match &edit.edit {
                EditShape::Match { old, .. } => Some(old.as_str()),
                // `put` states what to write, not what the caller believed was
                // there — no baseline; rung 2 is the honest answer. `remove`
                // states even less: it asserts the key EXISTS and nothing about
                // what it held, so it has no baseline to offer either.
                EditShape::Put { .. } | EditShape::Remove {} => None,
            },
            new_fingerprint: NodeRev(resolved.node_rev.0.clone()),
            frontmatter_key: match &edit.target {
                SecRef::FmKey { fm_key } => Some(fm_key.clone()),
                _ => None,
            },
            current,
        });
    }
    None
}

/// How the targeted node is named back — the same spelling rung 0's refusal uses.
fn subject_of(target: &SecRef) -> String {
    match target {
        SecRef::Hpath { hpath } => format!(
            "section \"{}\"",
            hpath
                .iter()
                .map(|s| s.h.as_str())
                .collect::<Vec<_>>()
                .join("/")
        ),
        SecRef::Anchor { anchor } => format!("block \"^{anchor}\""),
        SecRef::FmKey { fm_key } => format!("frontmatter key \"{fm_key}\""),
    }
}

/// The frontmatter form: one JSON-Patch-shaped op carrying the key's current
/// value — a property map is edited by key, and a line diff over `key: value`
/// would teach a text splice into a plane that has no text grain.
///
/// Scoped by construction: exactly one op, for exactly the key the caller
/// addressed.
fn ops_form(key: &str, current: &str) -> String {
    // The resolved fm-key span is the `key: value` line. A line without the
    // separator is left whole rather than guessed at.
    let value = current
        .split_once(':')
        .map_or(current.trim(), |(_, v)| v.trim());
    format!(
        "[{{\"op\":\"replace\",\"path\":\"/{}\",\"value\":{}}}]",
        json_escape(key),
        quote(value)
    )
}

fn quote(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32).expect("String write is infallible");
            }
            c => out.push(c),
        }
    }
    out
}

/// Lines of unchanged context carried on each side of a changed run — the
/// conventional unified-diff window. Bounded context is what keeps a diff
/// cheaper than the content it describes.
const DIFF_CONTEXT_LINES: usize = 3;

/// The section-body form: a unified line diff from the caller's pinned picture
/// to the node's current bytes, in hunks with [`DIFF_CONTEXT_LINES`] of context.
/// Headers name the sides — `pinned` (caller's), `current` (document's) —
/// rather than `a/`/`b/`, which would name files neither side is.
///
/// `None` when the two pictures have no line-level difference; rung 2's bytes
/// serve the caller better.
fn unified_line_diff(pinned: &str, current: &str) -> Option<String> {
    unified_diff("pinned", "current", pinned, current)
}

/// The same renderer with the two side names spelled by the caller — one line
/// diff in the tree, shared by the rehearsal preview (`mrd put --dry`) and
/// `cas_mismatch` rung 1.
///
/// `None` when the two sides have no line-level difference at all.
#[must_use]
pub fn unified_diff(
    before_label: &str,
    after_label: &str,
    before_text: &str,
    after_text: &str,
) -> Option<String> {
    let before: Vec<&str> = before_text.lines().collect();
    let after: Vec<&str> = after_text.lines().collect();
    let lines = numbered(lcs_ops(&before, &after));
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !matches!(l.op, Op::Keep(_)))
        .map(|(i, _)| i)
        .collect();
    let first = *changed.first()?;

    let mut out = format!("--- {before_label}\n+++ {after_label}\n");
    // Two changed runs share a hunk when their context windows would touch —
    // keeps neighbouring edits from printing the same lines twice.
    let mut start = first;
    let mut prev = first;
    for &i in changed.iter().skip(1) {
        if i - prev > DIFF_CONTEXT_LINES * 2 {
            emit_hunk(&mut out, &lines, start, prev);
            start = i;
        }
        prev = i;
    }
    emit_hunk(&mut out, &lines, start, prev);
    Some(out)
}

/// One `@@` hunk covering the changed run `first..=last`, padded by context.
fn emit_hunk(out: &mut String, lines: &[Numbered<'_>], first: usize, last: usize) {
    let lo = first.saturating_sub(DIFF_CONTEXT_LINES);
    let hi = (last + DIFF_CONTEXT_LINES + 1).min(lines.len());
    let slice = &lines[lo..hi];

    // Counts are per side; starts are 1-based, and a side contributing nothing
    // starts at 0 — the unified-diff convention for an empty range.
    let before_len = slice
        .iter()
        .filter(|l| matches!(l.op, Op::Keep(_) | Op::Del(_)))
        .count();
    let after_len = slice
        .iter()
        .filter(|l| matches!(l.op, Op::Keep(_) | Op::Ins(_)))
        .count();
    let before_start = slice
        .iter()
        .find(|l| matches!(l.op, Op::Keep(_) | Op::Del(_)))
        .map_or(0, |l| l.b + 1);
    let after_start = slice
        .iter()
        .find(|l| matches!(l.op, Op::Keep(_) | Op::Ins(_)))
        .map_or(0, |l| l.a + 1);

    writeln!(
        out,
        "@@ -{before_start},{before_len} +{after_start},{after_len} @@"
    )
    .expect("String write is infallible");
    for l in slice {
        match l.op {
            Op::Keep(t) => writeln!(out, " {t}"),
            Op::Del(t) => writeln!(out, "-{t}"),
            Op::Ins(t) => writeln!(out, "+{t}"),
        }
        .expect("String write is infallible");
    }
}

/// An op beside the 0-based line number it sits at on each side — what a hunk
/// header needs and the op list alone cannot say.
struct Numbered<'a> {
    op: Op<'a>,
    b: usize,
    a: usize,
}

fn numbered(ops: Vec<Op<'_>>) -> Vec<Numbered<'_>> {
    let (mut b, mut a) = (0, 0);
    ops.into_iter()
        .map(|op| {
            let at = Numbered { op, b, a };
            match at.op {
                Op::Keep(_) => {
                    b += 1;
                    a += 1;
                }
                Op::Del(_) => b += 1,
                Op::Ins(_) => a += 1,
            }
            at
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Op<'a> {
    Keep(&'a str),
    Del(&'a str),
    Ins(&'a str),
}

/// Longest-common-subsequence line ops. A textbook DP rather than a dependency:
/// the inputs are one node's lines, bounded by the cap's neighbourhood.
fn lcs_ops<'a>(before: &[&'a str], after: &[&'a str]) -> Vec<Op<'a>> {
    let (n, m) = (before.len(), after.len());
    let mut table = vec![0usize; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if before[i] == after[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if before[i] == after[j] {
            ops.push(Op::Keep(before[i]));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            ops.push(Op::Del(before[i]));
            i += 1;
        } else {
            ops.push(Op::Ins(after[j]));
            j += 1;
        }
    }
    ops.extend(before[i..].iter().map(|l| Op::Del(l)));
    ops.extend(after[j..].iter().map(|l| Op::Ins(l)));
    ops
}
