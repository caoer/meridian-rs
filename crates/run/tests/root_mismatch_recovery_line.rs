//! Card `run-churn-refusal-recovery-lines`: the run plane's root-mismatch
//! refusal carries a fitted recovery line.
//!
//! Receipt (dogfood r9 prober § F3, receipt A): one call, four targets on the
//! same page and task. Target [0] refused `root mismatch: pinned …, live …
//! — out-of-band change, nothing applied`; targets [1] and [2] applied. The
//! caller supplied no fingerprint — the run plane pinned one for itself, an
//! unrelated fleet write advanced the corpus, and the user's work refused with
//! no next move stated.
//!
//! WORDING ONLY. Re-deriving the pin instead of refusing is the churn-grain
//! design (card `guard-grain-batch-doors`), not this card.
//!
//! The class is `resync`, not the `refresh` the prober's proposal sketched:
//! §8 binds `root_mismatch` to `resync`, and the wire `ErrorCode::RootMismatch`
//! doc states why — a merkle root is not invertible, so the door cannot name
//! the files that drifted and the full re-read is "the set's only honest
//! recovery". `refresh` means re-read ONE node, which this refusal cannot
//! promise is enough.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError};

const PAGE: &str = "\
---
status: todo
---

# Tasks
";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn set_status_done() -> Effect {
    Effect {
        kind: EffectKind::SetField,
        rule_id: "t".to_owned(),
        seq: 0,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: [
            ("field".to_owned(), ArgValue::Str("status".to_owned())),
            ("value".to_owned(), ArgValue::Str("done".to_owned())),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    }
}

/// The refusal the receipt caught, provoked hermetically by a stale pin: the
/// reason leads, then one fitted line naming the move.
#[test]
fn a_stale_pin_refusal_teaches_re_issue_with_recovery_resync() {
    let (_tmp, root) = workspace();
    let live = fs::domain_snapshot(&root).unwrap().1;
    let effects = [set_status_done()];
    let err = executor::apply(
        &root,
        &ApplyRequest {
            page: "page.md",
            task: "fix-x",
            task_rev: "b3:proc-test",
            invocation_id: "inv-1",
            now: Some("2026-07-22T01:00:00Z"),
            effects: &effects,
            authority: &Authority::granted(CapSet::parse("md.set_field").unwrap()),
            pin_root: &MerkleRoot("b3:stale".to_owned()),
            live_root: &live,
            receipt: None,
            takeover: false,
            exec: None,
            actor: None,
            depth: 0,
            delta: None,
        },
    )
    .unwrap_err();

    assert!(matches!(err, ExecError::RootMismatch { .. }), "{err:?}");
    let said = err.to_string();
    let (reason, recovery) = said
        .split_once('\n')
        .unwrap_or_else(|| panic!("the refusal carries a recovery line: {said:?}"));
    assert!(
        reason.starts_with("root mismatch: pinned b3:stale, live ")
            && reason.ends_with("— out-of-band change, nothing applied"),
        "reason first, naming both roots and the disposition: {reason:?}"
    );
    assert_eq!(
        recovery,
        "  → the corpus moved between the guard and this call, and nothing you named \
         is wrong — re-read what this run touches, then re-issue the call (recovery: resync)",
        "the fitted line, in the face's recovery-hint grammar"
    );
}
