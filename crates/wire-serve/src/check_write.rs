//! M1 U8c: serve the `check_write` op — the engine-side `meridiandefs.CheckWrite`.
//!
//! Pipeline (Go `pkg/defs.CheckWrite` verbatim): rebuild the CANDIDATE from
//! put-plan-vocabulary edits over the prev document (`policy::defs::rebuild` —
//! the `ApplyForConformance` port), then run the I4 severity ladder
//! (`policy::defs::conformance`) over prev→candidate. Rebuild failures come
//! back `class:"rebuild"` (the host renders Go's `cerr` wrap: "def-conformance
//! check could not run: CODE: …"); ladder refusals come back `class:"verdict"`.
//! NEVER a write path: no flock, no I3, no CAS, no journal, no disk mutation —
//! the host owns authz + the lock, `splice` owns CAS + the write.

use wire::{CheckWriteEdit, CheckWriteRefuse, CheckWriteRepair, ResponseBody};

/// Serve one `check_write` over the already-loaded prev document.
#[must_use]
pub fn check_write(
    prev: &model::Document,
    target: &str,
    actor: &str,
    now: &str,
    edits: &[CheckWriteEdit],
) -> ResponseBody {
    let build = |raw: &str| model::build(raw.to_string(), syntax::parse(raw));
    let plan: Vec<policy::defs::PlanEdit> = edits
        .iter()
        .map(|e| policy::defs::PlanEdit {
            op: e.op.clone(),
            target: e.at.clone(),
            find: e.find.clone(),
            body: e.body.clone(),
            rev: e.rev.clone(),
            all: e.all,
        })
        .collect();

    let next = match policy::defs::rebuild(prev, &plan, &build) {
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

    let verdict = policy::defs::conformance(&policy::defs::ConformanceRequest {
        target,
        actor,
        force: false, // the put face exposes no caller force path (Go parity)
        now,
        prev,
        next: &next,
        build_doc: &build,
    });
    ResponseBody::CheckWrite {
        refuse: verdict.refuse.map(|e| CheckWriteRefuse {
            class: "verdict".to_string(),
            code: e.code,
            message: e.message,
            remedy: e.remedy,
        }),
        repairs: verdict
            .repairs
            .into_iter()
            .map(|r| CheckWriteRepair {
                key: r.key,
                value: r.value,
            })
            .collect(),
        forced: verdict.forced,
    }
}
