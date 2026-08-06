//! Serve the `check_write` op — the engine-side `meridiandefs.CheckWrite`.
//!
//! Rebuild the candidate from put-plan-vocabulary edits over the prev document
//! (`policy::defs::rebuild`), then run the I4 severity ladder
//! (`policy::defs::conformance`) over prev→candidate. Rebuild failures come
//! back `class:"rebuild"`; ladder refusals come back `class:"verdict"`. This op
//! is never a write path — no flock, no I3, no CAS, no journal, no disk
//! mutation; it is the host's pre-flight (D4).
//!
//! [`verdict`] is the single place `policy::defs::conformance` is invoked: this
//! op after rebuilding its candidate, `write::splice` inside the D9 write flock
//! over the `after_doc` it built. The verdict that authorizes bytes is the one
//! under the flock.

use wire::{CheckWriteEdit, CheckWriteRefuse, CheckWriteRepair, ResponseBody};

/// The markdown parse for def files read during resolution and for candidate
/// rebuilds — the `syntax` edge `policy` leaves to its caller, in one place so
/// the two entry points cannot parse differently.
#[must_use]
pub fn build_doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The one I4 conformance-ladder invocation (S4a, D4): judge `prev`→`next`
/// against the def layer discovered from `target`, the record's absolute path
/// spelling (`discover_layers` walks upward from it, and the refusal strings
/// quote it verbatim).
///
/// `force` is fixed `false`: the put face exposes no caller force path, so
/// neither entry point escapes a warning-rung refusal.
#[must_use]
pub fn verdict(
    prev: &model::Document,
    next: &model::Document,
    target: &str,
    actor: &str,
    now: &str,
) -> policy::defs::ConformanceResult {
    policy::defs::conformance(&policy::defs::ConformanceRequest {
        target,
        actor,
        force: false,
        now,
        prev,
        next,
        build_doc: &build_doc,
    })
}

/// An `at` address with its `@fp` decoration peeled — the same peel
/// `read::to_model_ref` applies to a `SecRef::Anchor`. Only the block-ref forms
/// carry a claim-link slot; a heading path rides verbatim. Uses `split_fp`
/// rather than the document strip, so a laundered address refuses in both entry
/// points instead of resolving in one.
fn strip_fp_address(at: &[wire::HpathSeg]) -> Vec<policy::defs::Seg> {
    // Heading segments keep `n`: dropping it made the pre-flight refuse
    // E_AMBIGUOUS on addresses the splice resolves over duplicate headings.
    if let [only] = at {
        for prefix in ["#^", "^"] {
            if let Some(id) = only.h.strip_prefix(prefix) {
                let (base, _fp) = syntax::split_fp(id);
                return vec![policy::defs::Seg {
                    h: format!("{prefix}{base}"),
                    n: None,
                }];
            }
        }
    }
    at.iter()
        .map(|s| policy::defs::Seg {
            h: s.h.clone(),
            n: s.n,
        })
        .collect()
}

/// The candidate this op judges — the plan lowered over `prev`, carrying the
/// same `@fp` law `splice` applies to the bytes it commits. `pub` so the
/// equivalence of the two entry points is testable at its own grain.
///
/// Peels addresses at their owner and strips payloads at document grain, in
/// `splice`'s order — a pre-flight must judge the bytes `splice` commits:
///
/// - `at` rides the same `syntax::split_fp` peel `read::to_model_ref` applies
///   to a `SecRef`, so a decorated block ref resolves here exactly as there.
/// - `find` is a needle against stored bytes (which never carry a token) —
///   peeled for the same reason `Match{old}` is.
/// - `body` rides verbatim into the rebuild and is stripped over the candidate
///   at the one grain the write path uses.
///
/// # Errors
/// The rebuild's own `BodyError` (the host renders it `class:"rebuild"`).
pub fn candidate(
    prev: &model::Document,
    edits: &[CheckWriteEdit],
) -> Result<model::Document, policy::defs::BodyError> {
    let plan: Vec<policy::defs::PlanEdit> = edits
        .iter()
        .map(|e| policy::defs::PlanEdit {
            op: e.op.clone(),
            target: strip_fp_address(&e.at),
            find: syntax::strip_fp(&e.find).into_owned(),
            body: e.body.clone(),
            rev: e.rev.clone(),
            all: e.all,
        })
        .collect();

    let next = policy::defs::rebuild(prev, &plan, &build_doc)?;
    Ok(match syntax::strip_fp(&next.raw) {
        std::borrow::Cow::Borrowed(_) => next,
        std::borrow::Cow::Owned(raw) => build_doc(&raw),
    })
}

/// Serve one `check_write` over the already-loaded prev document.
#[must_use]
pub fn check_write(
    prev: &model::Document,
    target: &str,
    actor: &str,
    now: &str,
    edits: &[CheckWriteEdit],
) -> ResponseBody {
    let next = match candidate(prev, edits) {
        Ok(doc) => doc,
        Err(e) => {
            return ResponseBody::CheckWrite {
                refuse: Some(CheckWriteRefuse {
                    class: "rebuild".to_string(),
                    code: e.code,
                    message: e.message,
                    remedy: e.remedy,
                }),
                repairs: Vec::new(),
                forced: Vec::new(),
            };
        }
    };

    let judged = verdict(prev, &next, target, actor, now);
    ResponseBody::CheckWrite {
        refuse: judged.refuse.map(|e| CheckWriteRefuse {
            class: "verdict".to_string(),
            code: e.code,
            message: e.message,
            remedy: e.remedy,
        }),
        repairs: judged
            .repairs
            .into_iter()
            .map(|r| CheckWriteRepair {
                key: r.key,
                value: r.value,
            })
            .collect(),
        forced: judged.forced,
    }
}
