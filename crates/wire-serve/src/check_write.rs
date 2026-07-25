//! M1 U8c: serve the `check_write` op — the engine-side `meridiandefs.CheckWrite`.
//!
//! Pipeline (Go `pkg/defs.CheckWrite` verbatim): rebuild the CANDIDATE from
//! put-plan-vocabulary edits over the prev document (`policy::defs::rebuild` —
//! the `ApplyForConformance` port), then run the I4 severity ladder
//! (`policy::defs::conformance`) over prev→candidate. Rebuild failures come
//! back `class:"rebuild"` (the host renders Go's `cerr` wrap: "def-conformance
//! check could not run: CODE: …"); ladder refusals come back `class:"verdict"`.
//! This OP is never a write path: no flock, no I3, no CAS, no journal, no disk
//! mutation — it is the host's PRE-FLIGHT (D4), and it keeps that role.
//!
//! # The ladder has ONE owner (S4a, D4)
//! [`verdict`] is the single place `policy::defs::conformance` is invoked. This
//! op reaches it after rebuilding its candidate; `write::splice` reaches it
//! INSIDE the D9 write flock, over the `after_doc` it already built. The verdict
//! that AUTHORIZES bytes is the one under the flock — a foreign writer landing
//! between a host's pre-flight and its apply can no longer split the two.

use wire::{CheckWriteEdit, CheckWriteRefuse, CheckWriteRepair, ResponseBody};

/// The markdown parse for def FILES read during resolution and for candidate
/// rebuilds — the `syntax` edge `policy` leaves to its caller, in ONE place so
/// the two entry points cannot parse differently.
#[must_use]
pub fn build_doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// **The ONE I4 conformance-ladder invocation** (S4a, D4 — one owner, two entry
/// points): judge `prev`→`next` against the def layer discovered from `target`,
/// the record's ABSOLUTE path spelling (`discover_layers` walks upward from it,
/// and the refusal strings quote it verbatim).
///
/// `force` is fixed `false`: the put face exposes no caller force path (Go
/// parity), so neither entry point escapes a warning-rung refusal. Callers map
/// the result onto their own face — this op renders `class:"verdict"`, `splice`
/// refuses typed before bytes land.
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

/// An `at` address with its `@fp` decoration peeled — the pre-flight's spelling
/// of the peel `read::to_model_ref` applies to a `SecRef::Anchor` (S10 finding
/// 15). Only the BLOCK-REF forms carry a claim-link slot; a heading path is not
/// one, so it rides verbatim exactly as `to_model_ref` leaves an `hpath`.
/// `split_fp` (one peel, the address owner's rule) is used rather than the
/// document strip, so a laundered address refuses in BOTH entry points instead of
/// resolving in one.
fn strip_fp_address(at: &str) -> String {
    for prefix in ["#^", "^"] {
        if let Some(id) = at.strip_prefix(prefix) {
            let (base, _fp) = syntax::split_fp(id);
            return format!("{prefix}{base}");
        }
    }
    at.to_string()
}

/// **The CANDIDATE this op judges** — the plan lowered over `prev`, carrying the
/// same `@fp` law `splice` applies to the bytes it commits. `pub` because the
/// equivalence of the two entry points is a property worth testing at its own
/// grain, not only through a verdict.
///
/// Stage-2 S10, re-grained by advisor R25 (finding 15): a pre-flight that judges
/// different bytes from the ones `splice` commits is two answers to one question.
/// `splice` peels ADDRESSES at their owner and strips PAYLOADS at document grain,
/// so this builder does both in the same order:
///
/// - `at` is an address (`^id`, a section path). It rides the same
///   `syntax::split_fp` peel `read::to_model_ref` applies to a `SecRef`, so a
///   decorated block ref resolves here exactly as it resolves there — that
///   divergence was the pre-flight refusing the address the committer accepted.
/// - `find` is a NEEDLE against stored bytes (which never carry a token) — peeled
///   for the same reason `Match{old}` is.
/// - `body` rides VERBATIM into the rebuild and is stripped over the CANDIDATE,
///   at the one grain the write path uses, so a token the document law calls a
///   code sample survives here exactly as it survives there.
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
