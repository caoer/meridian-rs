//! U2.4 — **pin**: manifest-edit declaration, atomic fail-closed resolve, ONE
//! CAS splice + one receipt, idempotent, merge law, `check:` evaluation
//! (d2 §2.5; the 23-07 `check:` ruling; refusal-amendment rows 1-4).
//!
//! `pin(root, target, opts)` reads the target page's `inputs:` manifest
//! ([`manifest`]), resolves every declared ref atomically (one unresolvable or
//! ambiguous ref refuses the WHOLE pin with per-item reasons), records each
//! resolved target's node-rev, evaluates any declared `check:` predicate
//! ([`claim`] — a false claim refuses atomically), merges the results into the
//! `^inputs` lock under the union-by-canonical-ref / existing-items-byte
//! -preserved law ([`lock`]), and splices the lock in ONE CAS-guarded write
//! minting one journal receipt (`wire_serve::write::pin_lock`). `dry: true`
//! reports fully — `would_refuse` naming the failed assert — and writes nothing.
//! An unchanged re-pin renders byte-identical bytes and mints nothing
//! (idempotent). The pin-sourced `edge` fact projection lives in [`facts`].

pub mod claim;
pub mod facts;
pub mod lock;
pub mod manifest;

use std::collections::BTreeMap;

use model::selector::{
    GreyReason, Selector, live_anchors, live_headings, nearest, selector_display,
};
use model::{CorpusIndex, Document, Ref, ResolveError, resolve};
use serde::Serialize;

pub use claim::ClaimOutcome;

/// Options for a pin run.
#[derive(Debug, Clone, Default)]
pub struct PinOptions {
    /// Report fully and write nothing (`dry: true` reports `would_refuse`).
    pub dry: bool,
    /// The recorded actor (§9: recorded exactly as given, never invented).
    pub actor: Option<String>,
    /// The recorded timestamp (§9: recorded exactly as given, never invented).
    pub now: Option<String>,
}

/// A tool failure — the pin could not run to a verdict (exit 2). Distinct from a
/// refusal, which IS a verdict (the pin ran and refused, exit 1).
#[derive(Debug)]
pub enum PinError {
    /// The workspace could not be read or parsed.
    Corpus(String),
    /// The target page is not in the corpus.
    TargetNotFound(String),
    /// The CAS-guarded write refused or failed (the page drifted, or I/O).
    Write(String),
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinError::Corpus(m) => write!(f, "cannot read the workspace: {m}"),
            PinError::TargetNotFound(p) => write!(f, "target page not in the corpus: {p}"),
            PinError::Write(m) => write!(f, "pin write failed: {m}"),
        }
    }
}

/// The outcome of a pin run — the pin either pinned (or was a clean idempotent
/// no-op) or refused atomically. Both are verdicts; only a [`PinError`] is a
/// tool failure.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PinResult {
    /// The lock was written (or an unchanged re-pin minted nothing) — exit 0.
    Pinned(PinReport),
    /// One or more refs refused; the whole pin is atomic — nothing landed. On a
    /// `dry` run this is a `would_refuse` (exit 1 all the same).
    Refused(PinRefusal),
}

/// The report of a successful (or idempotent) pin.
#[derive(Debug, Serialize)]
pub struct PinReport {
    /// The pinned page path.
    pub target: String,
    /// Per-item result (colors, rebind candidates, passengers).
    pub items: Vec<PinnedItem>,
    /// `true` when the lock bytes actually changed and a receipt was minted;
    /// `false` for an idempotent no-op or a dry run.
    pub wrote: bool,
    /// This was a dry run (nothing written).
    pub dry: bool,
    /// The minted journal receipt anchor (`r-NNNNNN`); `None` when nothing was
    /// written.
    pub receipt: Option<String>,
    /// The E5 storm protocol: id-mint suggestions for mutation-fragile heading
    /// pins, each carrying the dependents count that would re-pin in the window.
    pub id_mint: Vec<IdMintSuggestion>,
}

/// One pinned item's rendered result.
#[derive(Debug, Serialize)]
pub struct PinnedItem {
    /// The declared ref, verbatim.
    pub declared_ref: String,
    /// The resolved address, or the ref for a grey declared-only item.
    pub to: String,
    /// The pinned node-rev; `None` for a grey declared-only item.
    pub rev: Option<String>,
    /// `content` | `object` | `null` (grey).
    pub rev_class: Option<String>,
    /// The computed color: `green` | `grey(reason)`.
    pub color: String,
    /// The engine-read `check:` predicate selector, if declared.
    pub check: Option<String>,
    /// The rev the predicate text was pinned at (for `check_rev` drift).
    pub check_rev: Option<String>,
}

/// An E5 storm-protocol id-mint suggestion (d2 §3): a heading pin is mutation
/// -fragile; minting a `^block-id` makes it mutation-stable, but applying the id
/// flips the section's `node_rev`, so the id-mint and the dependents' re-pin run
/// as one coordinated window. The suggestion is a CANDIDATE (never applied — no
/// hidden cross-doc write), carrying the dependents count for that window.
#[derive(Debug, Serialize)]
pub struct IdMintSuggestion {
    /// The heading-selector pin that would be more stable as a `^block-id`.
    pub target: String,
    /// How many OTHER pages pin the same target — the re-pin window's size.
    pub dependents: usize,
}

/// An atomic refusal — per-item reasons; nothing landed.
#[derive(Debug, Serialize)]
pub struct PinRefusal {
    /// The pinned page path.
    pub target: String,
    /// Per-item outcomes — the refusing items carry a [`RefusalReason`]; the
    /// others (`reason: None`) resolved fine but did not land (atomic).
    pub items: Vec<ItemOutcome>,
    /// `true` when this is a `dry` rehearsal (`would_refuse`), false for a real
    /// refusal — the render differs, the atomicity does not.
    pub would_refuse: bool,
}

/// One item's outcome inside a refusal: the declared ref + its reason (or
/// `None` when this item itself resolved).
#[derive(Debug, Serialize)]
pub struct ItemOutcome {
    pub declared_ref: String,
    pub reason: Option<RefusalReason>,
}

/// A refusal reason mapped to its `wire-contract-v2-refusal-amendment.md` row:
/// `code` + `recovery` class, the teaching message, candidates (ambiguity /
/// dangling), and the `assert` selector (`check_claim`).
#[derive(Debug, Serialize)]
pub struct RefusalReason {
    /// The §8 code — `ambiguous_ref` (row 1) | `ref_not_found` (row 2) |
    /// `no_match` (row 3) | `check_claim` (row 4).
    pub code: &'static str,
    /// The §8 recovery class — `fix` | `refresh`.
    pub recovery: &'static str,
    /// The teaching message (names the violated rule).
    pub message: String,
    /// Nearest-candidate hints (ambiguity, dangling, selector-unresolved) — a
    /// candidate an author confirms by re-pinning, never an auto-repair.
    pub candidates: Vec<String>,
    /// The `check:` predicate selector whose assertion was false (row 4 only).
    pub assert: Option<String>,
}

impl RefusalReason {
    fn ambiguous(sel: &str, candidates: Vec<String>) -> Self {
        RefusalReason {
            code: "ambiguous_ref",
            recovery: "fix",
            message: format!(
                "selector '{sel}' is ambiguous — {n} matches; address by node index or ^block-id",
                n = candidates.len()
            ),
            candidates,
            assert: None,
        }
    }
    fn dangling(sel: &str, candidates: Vec<String>) -> Self {
        RefusalReason {
            code: "ref_not_found",
            recovery: "refresh",
            message: format!("pinned anchor '{sel}' no longer resolves (dangling)"),
            candidates,
            assert: None,
        }
    }
    fn unresolved(sel: &str, candidates: Vec<String>) -> Self {
        RefusalReason {
            code: "no_match",
            recovery: "fix",
            message: format!("selector '{sel}' resolves to nothing"),
            candidates,
            assert: None,
        }
    }
    fn check_claim(check_sel: &str, detail: &str) -> Self {
        RefusalReason {
            code: "check_claim",
            recovery: "fix",
            message: format!("declared check: '{check_sel}' — {detail}"),
            candidates: Vec::new(),
            assert: Some(check_sel.to_string()),
        }
    }
}

/// The engine's content-class hash-algo label the lock records.
const CONTENT_CLASS: &str = "content";

/// **Pin the target page** (d2 §2.5): resolve its `inputs:` manifest atomically,
/// evaluate declared `check:` predicates, and splice the `^inputs` lock in ONE
/// CAS-guarded write. See the module docs for the full contract.
///
/// # Errors
/// [`PinError`] on a tool failure — an unreadable workspace, a missing target
/// page, or a refused/failed CAS write. A pin that RAN and refused is
/// `Ok(PinResult::Refused)`, not an error.
pub fn pin(
    root: &fs::WorkspaceRoot,
    target: &str,
    opts: &PinOptions,
) -> Result<PinResult, PinError> {
    let (files, _) = fs::domain_snapshot(root).map_err(|e| PinError::Corpus(e.to_string()))?;
    let (index, docs) = fs::build_corpus(files).map_err(|e| PinError::Corpus(e.to_string()))?;
    let target_doc = docs
        .get(target)
        .ok_or_else(|| PinError::TargetNotFound(target.to_string()))?;

    // Nothing declared → nothing to pin (a clean no-op, exit 0).
    let Some(declared) = manifest::read(target_doc) else {
        return Ok(PinResult::Pinned(PinReport {
            target: target.to_string(),
            items: Vec::new(),
            wrote: false,
            dry: opts.dry,
            receipt: None,
            id_mint: Vec::new(),
        }));
    };

    // 1. ATOMIC resolve — fail-closed (any refusal refuses the whole pin).
    let resolved = match resolve_all(&index, &docs, target, &declared) {
        Ok(r) => r,
        Err(items) => {
            return Ok(PinResult::Refused(PinRefusal {
                target: target.to_string(),
                items,
                would_refuse: opts.dry,
            }));
        }
    };

    // 2. `check:` evaluation — a false (or unevaluable) claim refuses atomically.
    let check_revs = match eval_checks(&index, &docs, target, &resolved) {
        Ok(revs) => revs,
        Err(fail) => {
            return Ok(PinResult::Refused(check_claim_refusal(
                target,
                &declared,
                fail.index,
                &fail.check_sel,
                &fail.detail,
                opts.dry,
            )));
        }
    };

    // 3. Merge, splice, and write (or dry / idempotent no-op).
    finalize_pin(
        root,
        target,
        target_doc,
        &docs,
        &resolved,
        &check_revs,
        opts,
    )
}

/// Phase 3 — build the fresh lock items, merge them under the union / byte
/// -preserve law, compute the `^inputs` splice (replace or append), then commit:
/// an unchanged re-pin mints nothing (idempotent), a dry run reports and writes
/// nothing, and a real run splices in ONE CAS write minting one receipt.
fn finalize_pin(
    root: &fs::WorkspaceRoot,
    target: &str,
    target_doc: &Document,
    docs: &BTreeMap<String, Document>,
    resolved: &[Resolved],
    check_revs: &[Option<String>],
    opts: &PinOptions,
) -> Result<PinResult, PinError> {
    let fresh: Vec<lock::Item> = resolved
        .iter()
        .enumerate()
        .map(|(i, r)| lock::Item {
            declared_ref: r.declared.declared_ref.clone(),
            to: r.to.clone(),
            rev: r.rev.clone(),
            rev_class: r.rev.as_ref().map(|_| CONTENT_CLASS.to_string()),
            claim: r.declared.claim.clone(),
            check: r.declared.check.clone(),
            check_rev: check_revs[i].clone(),
        })
        .collect();
    let existing = lock::find_existing(target_doc);
    let block = lock::render_block(&lock::merge(&existing, &fresh));

    // Compute the splice: replace the existing block, or append at EOF.
    let raw = &target_doc.raw;
    let (span_start, span_end, new_text, before_block) = if let Some(span) = &existing.span {
        let before = raw.get(span.clone()).unwrap_or("").to_string();
        (span.start, span.end, block.clone(), before)
    } else {
        let sep = append_separator(raw);
        (
            raw.len(),
            raw.len(),
            format!("{sep}{block}\n"),
            String::new(),
        )
    };
    let new_raw = format!("{}{}{}", &raw[..span_start], new_text, &raw[span_end..]);

    let report = |wrote: bool, receipt: Option<String>| PinReport {
        target: target.to_string(),
        items: render_items(resolved, check_revs),
        wrote,
        dry: opts.dry,
        receipt,
        id_mint: id_mint_suggestions(docs, target, resolved),
    };

    // Idempotent (unchanged bytes) OR dry: mint nothing, write nothing.
    if new_raw == *raw || opts.dry {
        return Ok(PinResult::Pinned(report(false, None)));
    }

    // Real: ONE CAS splice + one receipt through the strict writer.
    let args = wire_serve::write::PinLockArgs {
        id: None,
        path: wire::Path(target.to_string()),
        actor: opts.actor.clone(),
        now: opts.now.clone(),
        if_root: None,
        if_file_rev: wire::NodeRev(target_doc.root.node_rev.0.clone()),
        span: wire::Span(span_start as u64, span_end as u64),
        new_text,
        before_rev: wire::NodeRev(lock::node_rev(before_block.as_bytes())),
        after_rev: wire::NodeRev(lock::node_rev(block.as_bytes())),
        dry: false,
    };
    let outcome =
        wire_serve::write::pin_lock(root, 0, &args).map_err(|e| PinError::Write(wire_err(&e)))?;
    Ok(PinResult::Pinned(report(true, outcome.journal_anchor)))
}

/// Phase 1 — the atomic fail-closed resolve. Each declared ref resolves, is grey
/// (declared-only), or refuses; ANY refusal returns the per-item outcomes (the
/// refusing item carries its reason, the rest `None` — nothing lands).
fn resolve_all(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    target: &str,
    declared: &[manifest::DeclaredItem],
) -> Result<Vec<Resolved>, Vec<ItemOutcome>> {
    let mut resolved = Vec::with_capacity(declared.len());
    let mut outcomes = Vec::with_capacity(declared.len());
    let mut any_refusal = false;
    for item in declared {
        let reason = match resolve_ref(index, docs, target, &item.declared_ref) {
            RefResolve::Pinnable {
                to,
                node_rev,
                content,
                heading_pin,
            } => {
                resolved.push(Resolved {
                    declared: item.clone(),
                    to,
                    rev: Some(node_rev),
                    content,
                    grey: None,
                    heading_pin,
                });
                None
            }
            RefResolve::Grey { to, reason } => {
                resolved.push(Resolved {
                    declared: item.clone(),
                    to,
                    rev: None,
                    content: String::new(),
                    grey: Some(reason),
                    heading_pin: false,
                });
                None
            }
            RefResolve::Refuse(r) => {
                any_refusal = true;
                Some(r)
            }
        };
        outcomes.push(ItemOutcome {
            declared_ref: item.declared_ref.clone(),
            reason,
        });
    }
    if any_refusal {
        Err(outcomes)
    } else {
        Ok(resolved)
    }
}

/// A `check:` evaluation failure — item `index`, its `check_sel`, and the detail.
struct CheckFail {
    index: usize,
    check_sel: String,
    detail: String,
}

/// Phase 2 — evaluate every declared `check:` predicate over its resolved
/// content. Returns the per-item `check_rev` (`Some` for a checked+held item),
/// or a [`CheckFail`] the caller renders as an atomic `check_claim` refusal.
fn eval_checks(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    target: &str,
    resolved: &[Resolved],
) -> Result<Vec<Option<String>>, CheckFail> {
    // The one-hop `doc(ref)` corpus: every resolved same-lock ref → its content.
    let same_lock: BTreeMap<String, String> = resolved
        .iter()
        .map(|r| (r.declared.declared_ref.clone(), r.content.clone()))
        .collect();
    let mut check_revs = vec![None; resolved.len()];
    for (i, r) in resolved.iter().enumerate() {
        let Some(check_sel) = &r.declared.check else {
            continue;
        };
        let Some((source, check_rev)) = resolve_check(index, docs, target, check_sel) else {
            return Err(CheckFail {
                index: i,
                check_sel: check_sel.clone(),
                detail: "the check predicate does not resolve".to_string(),
            });
        };
        let facts = claim::ClaimFacts {
            target: r.content.clone(),
            item_ref: r.declared.declared_ref.clone(),
            item_to: r.to.clone(),
            item_rev: r.rev.clone().unwrap_or_default(),
            item_rev_class: r
                .rev
                .as_ref()
                .map(|_| CONTENT_CLASS.to_string())
                .unwrap_or_default(),
            item_claim: r.declared.claim.clone().unwrap_or_default(),
            docs: same_lock
                .iter()
                .filter(|(k, _)| k.as_str() != r.declared.declared_ref)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        match claim::eval_check_claim(check_sel, &source, &facts, effects::EvalLimits::default()) {
            ClaimOutcome::Holds => check_revs[i] = Some(check_rev),
            ClaimOutcome::Fails(detail) => {
                return Err(CheckFail {
                    index: i,
                    check_sel: check_sel.clone(),
                    detail,
                });
            }
        }
    }
    Ok(check_revs)
}

/// One resolved declared item.
struct Resolved {
    declared: manifest::DeclaredItem,
    to: String,
    rev: Option<String>,
    content: String,
    grey: Option<GreyReason>,
    heading_pin: bool,
}

/// The resolution of one declared ref.
enum RefResolve {
    /// The ref resolves in-corpus — a pinnable node with its rev + content.
    Pinnable {
        to: String,
        node_rev: String,
        content: String,
        heading_pin: bool,
    },
    /// The ref is declared-only grey (transcript, wikilink, or out-of-corpus).
    Grey { to: String, reason: GreyReason },
    /// The ref refuses the whole pin (ambiguous / dangling / unresolved).
    Refuse(RefusalReason),
}

/// Resolve one declared ref against the corpus (d2 §2.5 atomic fail-closed).
fn resolve_ref(
    _index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from: &str,
    declared: &str,
) -> RefResolve {
    let sel = Selector::parse(declared);
    // A transcript ref is grey by class — never resolved (d2 §2.2).
    if sel.is_immutable_root() {
        return RefResolve::Grey {
            to: declared.to_string(),
            reason: GreyReason::ImmutableRoot,
        };
    }
    let (path, has_frag) = match declared.split_once('#') {
        Some((p, _)) => (p, true),
        None => (declared, false),
    };
    // A wikilink form or an out-of-corpus page → grey unmanaged (never refuses).
    let path = if path.is_empty() { from } else { path };
    let Some(doc) = docs.get(path) else {
        return RefResolve::Grey {
            to: declared.to_string(),
            reason: GreyReason::DeclaredUnpinned,
        };
    };
    // Whole-page selector: the document root.
    if !has_frag || matches!(sel, Selector::Page) {
        return RefResolve::Pinnable {
            to: declared.to_string(),
            node_rev: doc.root.node_rev.0.clone(),
            content: doc.raw.clone(),
            heading_pin: false,
        };
    }
    let Some(r#ref) = to_ref(&sel) else {
        // A block id outside the charset cannot address a node — unresolved.
        return RefResolve::Refuse(RefusalReason::unresolved(declared, Vec::new()));
    };
    match resolve(doc, &r#ref) {
        Ok(t) => RefResolve::Pinnable {
            to: declared.to_string(),
            node_rev: t.node_rev.0.clone(),
            content: doc.raw.get(t.span.clone()).unwrap_or("").to_string(),
            heading_pin: matches!(sel, Selector::Heading(_)),
        },
        Err(ResolveError::NotFound) => match &sel {
            // A vanished pinned anchor dangles (row 2); a heading/page resolves
            // to nothing (row 3) — each with the live toc's nearest candidates.
            Selector::Block(id) => RefResolve::Refuse(RefusalReason::dangling(
                declared,
                nearest(id, &live_anchors(doc)),
            )),
            _ => RefResolve::Refuse(RefusalReason::unresolved(
                declared,
                nearest(&selector_display(&sel), &live_headings(doc)),
            )),
        },
        Err(ResolveError::Ambiguous(cands)) => RefResolve::Refuse(RefusalReason::ambiguous(
            declared,
            ambiguity_candidates(doc, &cands),
        )),
    }
}

/// Convert a resolvable selector to its model `Ref` (Heading / Block). Page and
/// transcript classes never reach here (handled before). A bad block-id charset
/// yields `None` (the caller renders it unresolved).
fn to_ref(sel: &Selector) -> Option<Ref> {
    match sel {
        Selector::Heading(hpath) => Some(Ref::Hpath(
            hpath
                .iter()
                .map(|h| model::HpathSeg {
                    h: h.clone(),
                    n: None,
                })
                .collect(),
        )),
        Selector::Block(id) => Ref::anchor(id.clone()).ok(),
        Selector::Page | Selector::ImmutableRoot { .. } => None,
    }
}

/// Name each ambiguous candidate by its in-span `^block-id`, else by 1-based
/// occurrence — a hint an author confirms by re-pinning (never auto-repair).
fn ambiguity_candidates(doc: &Document, cands: &[model::Target]) -> Vec<String> {
    cands
        .iter()
        .enumerate()
        .map(|(i, t)| {
            model::selector::first_anchor_in_span(doc, &t.span)
                .map_or_else(|| format!("n={}", i + 1), |a| format!("^{a}"))
        })
        .collect()
}

/// Resolve a `check:` predicate selector to `(starlark source, check_rev)`. The
/// predicate is a fenced code block at the selector; its inner code is the
/// source and the resolved node's rev is `check_rev`. `None` when the selector
/// does not resolve (the caller refuses `check_claim`, fail-closed).
fn resolve_check(
    _index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from: &str,
    check_sel: &str,
) -> Option<(String, String)> {
    let (path, _) = match check_sel.split_once('#') {
        Some((p, s)) => (p, s),
        None => (check_sel, ""),
    };
    let path = if path.is_empty() { from } else { path };
    let doc = docs.get(path)?;
    let sel = Selector::parse(check_sel);
    let (span, rev) = if matches!(sel, Selector::Page) {
        (doc.root.span.clone(), doc.root.node_rev.0.clone())
    } else {
        let r#ref = to_ref(&sel)?;
        let t = resolve(doc, &r#ref).ok()?;
        (t.span, t.node_rev.0)
    };
    let text = doc.raw.get(span)?;
    Some((extract_starlark(text), rev))
}

/// Extract the starlark source from a resolved check target: the inner code of
/// the first fenced code block, or the whole text (trimmed) when unfenced.
fn extract_starlark(text: &str) -> String {
    let mut lines = text.lines();
    let mut source = String::new();
    // Skip to the first fence line, then take until the closing fence.
    let mut in_fence = false;
    let mut found = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_fence {
                found = true;
                break;
            }
            in_fence = true;
            continue;
        }
        if in_fence {
            source.push_str(line);
            source.push('\n');
        }
    }
    let _ = &mut lines;
    if found {
        source
    } else {
        text.trim().to_string()
    }
}

/// Build a `check_claim` refusal (row 4), tagging item `i` with the reason and
/// the others `reason: None` (atomic — nothing pinned).
fn check_claim_refusal(
    target: &str,
    declared: &[manifest::DeclaredItem],
    i: usize,
    check_sel: &str,
    detail: &str,
    dry: bool,
) -> PinRefusal {
    let items = declared
        .iter()
        .enumerate()
        .map(|(j, item)| ItemOutcome {
            declared_ref: item.declared_ref.clone(),
            reason: if j == i {
                Some(RefusalReason::check_claim(check_sel, detail))
            } else {
                None
            },
        })
        .collect();
    PinRefusal {
        target: target.to_string(),
        items,
        would_refuse: dry,
    }
}

/// The per-item report colors: a pinned item is green (its live rev == the rev
/// just pinned); a grey declared-only item renders grey with its reason.
fn render_items(resolved: &[Resolved], check_revs: &[Option<String>]) -> Vec<PinnedItem> {
    resolved
        .iter()
        .enumerate()
        .map(|(i, r)| PinnedItem {
            declared_ref: r.declared.declared_ref.clone(),
            to: r.to.clone(),
            rev: r.rev.clone(),
            rev_class: r.rev.as_ref().map(|_| CONTENT_CLASS.to_string()),
            color: match &r.grey {
                None => "green".to_string(),
                Some(reason) => format!("grey({})", grey_reason(reason)),
            },
            check: r.declared.check.clone(),
            check_rev: check_revs.get(i).and_then(Clone::clone),
        })
        .collect()
}

/// The grey-reason label for a rendered color.
fn grey_reason(reason: &GreyReason) -> &'static str {
    match reason {
        GreyReason::ImmutableRoot => "immutable-root",
        GreyReason::DeclaredUnpinned => "unmanaged",
        GreyReason::Ambiguous => "ambiguous",
        // pin mints node-rev, so a fresh pin never greys superseded-algo; the
        // label is kept exhaustive-and-correct for the shared enum (U3.4).
        GreyReason::SupersededAlgo => "superseded-algo",
    }
}

/// The E5 storm protocol: for each heading-selector pin (mutation-fragile),
/// suggest a `^block-id` mint carrying the dependents count — the size of the
/// coordinated re-pin window. Dependents = other pages whose lock pins the same
/// address.
fn id_mint_suggestions(
    docs: &BTreeMap<String, Document>,
    target: &str,
    resolved: &[Resolved],
) -> Vec<IdMintSuggestion> {
    resolved
        .iter()
        .filter(|r| r.heading_pin)
        .map(|r| IdMintSuggestion {
            target: r.declared.declared_ref.clone(),
            dependents: dependents_of(docs, target, &r.to),
        })
        .collect()
}

/// Count OTHER pages whose `^inputs` lock pins the address `to`.
fn dependents_of(docs: &BTreeMap<String, Document>, target: &str, to: &str) -> usize {
    docs.iter()
        .filter(|(path, _)| path.as_str() != target)
        .filter(|(_, doc)| {
            lock::find_existing(doc)
                .items
                .iter()
                .any(|(item, _)| item.to == to)
        })
        .count()
}

/// The separator that puts the appended block on its own paragraph: none when
/// the file already ends with a blank line, one newline after a single trailing
/// newline, two when it ends mid-line.
fn append_separator(raw: &str) -> &'static str {
    if raw.ends_with("\n\n") || raw.is_empty() {
        ""
    } else if raw.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

/// A one-line diagnostic for a wire error body (the write refusal render).
fn wire_err(e: &wire::ErrorBody) -> String {
    let code = serde_json::to_value(e.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "error".to_owned());
    match &e.message {
        Some(m) => format!("{code}: {m}"),
        None => code,
    }
}
