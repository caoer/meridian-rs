//! Armed-plane gate mount for the run plane (U4.2) — byte-landing parity
//! with the wire-serve write path. Before the executor commits, evaluates the
//! workspace's own armed law over the pending change; a block-severity
//! finding refuses. Never-armed workspaces are a no-op.
//!
//! This module adapts run-plane intents into [`policy::Change`] and calls
//! [`policy::gate`]. It does not own armed-set load or rule evaluation.

use std::cell::RefCell;
use std::collections::BTreeMap;

use model::{Document, Edit, NodeKind};

/// Whether this workspace has EVER been armed — the once-armed pivot, read from
/// the MARKER's presence and nothing else.
///
/// Pivoting on the artifact would make deleting it read as "never armed" — the
/// silent-disarm attack the marker defeats. An ambiguous stat fails closed
/// (assume armed).
#[must_use]
pub fn once_armed(root: &fs::WorkspaceRoot) -> bool {
    root.0
        .join(fs::domain::ATTESTED_MARKER_PATH)
        .try_exists()
        .unwrap_or(true)
}

/// Read the attested armed-rules artifact, or `None` when it is absent.
///
/// An artifact that exists and cannot be read is NOT `None`: it reads as an
/// empty page, which the resolver refuses as corrupt — fail closed either way.
#[must_use]
pub fn read_artifact(root: &fs::WorkspaceRoot) -> Option<String> {
    match std::fs::read_to_string(root.0.join(fs::domain::ARMED_RULES_PATH)) {
        Ok(page) => Some(page),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

/// The pinned pages on disk, read once each.
///
/// The cache matters for more than speed: the drift check and the rule load must
/// see the SAME bytes, or a page edited between the two calls would load a
/// declaration the rev check never approved.
struct DiskPages<'a> {
    root: &'a fs::WorkspaceRoot,
    seen: RefCell<BTreeMap<String, String>>,
}

impl policy::armed::PageSource for DiskPages<'_> {
    /// Page paths reach here only from a parsed artifact row, which validated them
    /// as relative, non-escaping workspace paths at intake.
    fn read(&self, page: &str) -> std::io::Result<String> {
        if let Some(held) = self.seen.borrow().get(page) {
            return Ok(held.clone());
        }
        let bytes = std::fs::read_to_string(self.root.0.join(page))?;
        self.seen
            .borrow_mut()
            .insert(page.to_string(), bytes.clone());
        Ok(bytes)
    }
}

/// Resolve the armed law governing a write at `at_path`, reading the marker, the
/// artifact, and the pinned pages from disk.
///
/// Handing the [`policy::ArmedLaw`] back whole — rather than its parts — is what
/// keeps the once-armed pivot from being re-assembled, differently, at the call
/// site.
#[must_use]
pub fn resolve_at(root: &fs::WorkspaceRoot, at_path: &str) -> policy::ArmedLaw {
    let pages = DiskPages {
        root,
        seen: RefCell::new(BTreeMap::new()),
    };
    policy::resolve_armed_law(
        read_artifact(root).as_deref(),
        once_armed(root),
        at_path,
        &pages,
        policy::CheckLimits::default(),
    )
}

/// Gate the run-plane change the executor is about to commit. Returns the refusal
/// detail when the workspace's armed law refuses (the executor turns it into
/// [`crate::executor::ExecError::ArmedRefusal`]), or `None` on a no-op/pass.
///
/// `before`/`after` are the pre/post-apply page states; `page` is the
/// workspace-relative path (stamped onto the change so a rule's declared scope can
/// match it — `fs::load`/`model::build` leave the path empty, and it is also the
/// path the law is resolved AT); `edits` are the planned model edits; `actor` is
/// `run:<task>`.
#[must_use]
pub(crate) fn refuse_reason(
    root: &fs::WorkspaceRoot,
    before: &Document,
    after: &Document,
    page: &str,
    edits: &[Edit],
    actor: &str,
) -> Option<String> {
    let before = path_stamped(before, page);
    let after = path_stamped(after, page);
    let change = policy::derive_change(
        &before,
        &after,
        edits,
        policy::Invocation {
            op: policy::ChangeOp::Splice,
            actor: Some(actor),
            force: false,
        },
        &[],
        &|_reference| None,
    );
    match policy::gate(&change, &resolve_at(root, page)) {
        policy::GateOutcome::Ok(_) => None,
        policy::GateOutcome::Refusal(refusal) => Some(describe(refusal)),
    }
}

/// A one-line refusal detail for [`crate::executor::ExecError::ArmedRefusal`],
/// naming the rule and citing the legal path. Armed-law faults render through
/// [`policy::ArmedFault`]'s own `Display` — the one renderer, shared with the
/// write door — and every refusing fault renders, not just the first.
fn describe(refusal: policy::GateRefusal) -> String {
    match refusal {
        policy::GateRefusal::Blocked { violations } => {
            let body = violations
                .iter()
                .map(|v| {
                    format!(
                        "`{}`: {} (legal path: {})",
                        v.rule, v.message, v.passing_scenario
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("armed rule(s) refused the run-plane change: {body}")
        }
        policy::GateRefusal::ArmedLawFault { faults } => faults
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "),
        // U4.3 binding law / integrity floor — run-plane writes touching the
        // armed artifact, an armed rule page, or the marker refuse here too.
        policy::GateRefusal::BindingBreak { side, teaching, .. } => {
            format!("binding-break[side={}]: {teaching}", side.as_str())
        }
        policy::GateRefusal::IndexIntegrity { teaching, .. } => teaching,
    }
}

/// Clone `doc` with its document path stamped to `page` — `fs::load` /
/// `model::build` leave the path empty, but a rule's declared scope reads it.
fn path_stamped(doc: &Document, page: &str) -> Document {
    let mut out = doc.clone();
    if let NodeKind::Document { path, .. } = &mut out.root.kind {
        path.clear();
        path.push_str(page);
    }
    out
}

#[cfg(test)]
mod scenario {
    //! Scenario 7 (leader ruling): the run plane lands bytes through
    //! `fs::apply_batch`, so an armed rule refuses a run-plane apply through the
    //! SAME evaluator — byte-landing parity with the wire seam.

    use std::collections::BTreeMap;

    use effects::{ArgValue, Effect, EffectKind, Provenance};

    use crate::caps::{Authority, CapSet};
    use crate::executor::{self, ApplyRequest, ExecError, ReceiptAddr};

    /// A task page under `tasks/**` (the rule's declared scope), owned by
    /// `run:closer`.
    const PAGE: &str = "---\nowner: run:closer\nstatus: todo\n---\n\n# Board\n\n- item\n";

    /// The rule's id — the name the artifact keys on and every refusal labels
    /// with. It is deliberately NOT a substring of the page path or the passing
    /// case below, so `detail.contains(RULE_ID)` can only pass when the refusal
    /// actually rendered the rule's name.
    const RULE_ID: &str = "reviewer-not-owner";

    /// Where the rule page lives. An ordinary workspace page — registration is by
    /// TAG plus `id:`, so no folder, filename, or `kind:` key carries identity.
    const RULE_PAGE_PATH: &str = "rules/reviewer.md";

    /// The rule page: a CHECK that refuses when the actor closing the task is its
    /// own owner.
    const RULE_PAGE: &str = "---\ntags: [type/rule, rules/check]\nid: reviewer-not-owner\n\
        paths:\n  - tasks/**\n---\n\n# reviewer-not-owner\n\n\
        ```starlark\ndef check_change(change):\n    owner = change.doc.frontmatter.get(\"owner\")\n    \
        actor = change.actor\n    if actor != None and owner != None and actor == owner:\n        \
        refuse(message = \"reviewer must not be the owner\", passing = \"rules/reviewer.md#reviewer-close\")\n```\n";

    fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("tasks")).unwrap();
        std::fs::write(tmp.path().join("tasks/board.md"), PAGE).unwrap();
        let root = fs::WorkspaceRoot(tmp.path().to_owned());
        (tmp, root)
    }

    /// Land the rule page and the attested artifact that arms it in `block`.
    ///
    /// The production ARM disk-edge writer is out of scope by ruling, so the
    /// fixture performs the act itself: discover the page, run the ONE `arm` act
    /// over it, and write what it rendered.
    fn write_rule_and_artifact(root: &fs::WorkspaceRoot) {
        let page_path = root.0.join(RULE_PAGE_PATH);
        std::fs::create_dir_all(page_path.parent().unwrap()).unwrap();
        std::fs::write(&page_path, RULE_PAGE).unwrap();

        let index = policy::RuleIndex::discover([policy::PageRef {
            layer: policy::ScopeLayer::Workspace,
            page: RULE_PAGE_PATH,
            bytes: RULE_PAGE,
        }]);
        let artifact = policy::armed::arm(
            &index,
            &policy::armed::ArmRoot::workspace(),
            [policy::armed::ArmRequest {
                id: policy::RuleId::parse(RULE_ID).expect("a legal id"),
                mode: policy::armed::Mode::Block,
                attested_rev: policy::page_rev(RULE_PAGE),
            }],
        )
        .expect("the fixture arms");

        let artifact_path = root.0.join(fs::domain::ARMED_RULES_PATH);
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(artifact_path, artifact.render()).unwrap();
    }

    /// The once-armed marker: this workspace HAS been armed.
    fn set_marker(root: &fs::WorkspaceRoot) {
        let p = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    /// Arm the workspace — BOTH files, which is what arming IS. The artifact alone
    /// leaves a workspace the marker says was never armed, and the marker alone is
    /// an `ArmedFault::Missing`.
    fn arm_ws(root: &fs::WorkspaceRoot) {
        write_rule_and_artifact(root);
        set_marker(root);
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
            args: [("field", "status"), ("value", "done")]
                .iter()
                .map(|(k, v)| ((*k).to_owned(), ArgValue::Str((*v).to_owned())))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    /// Run one apply as task `closer` (actor `run:closer` == the page owner).
    fn apply_as_closer(root: &fs::WorkspaceRoot) -> Result<executor::Applied, ExecError> {
        let now = fs::domain_snapshot(root).unwrap().1;
        let effects = [set_status_done()];
        executor::apply(
            root,
            &ApplyRequest {
                page: "tasks/board.md",
                task: "closer",
                task_rev: "b3:proc",
                invocation_id: "inv-1",
                now: Some("2026-07-22T01:00:00Z"),
                effects: &effects,
                authority: &Authority::granted(CapSet::parse("md.set_field").unwrap()),
                pin_root: &now,
                live_root: &now,
                receipt: Some(ReceiptAddr {
                    path: "receipts/run.md".to_owned(),
                    anchor: "r-000001".to_owned(),
                }),
                takeover: false,
                actor: None,
                exec: None,
                depth: 0,
                delta: None,
            },
        )
    }

    // ── Scenario 7 — run-plane apply violating an armed rule REFUSES ────────────
    #[test]
    fn s7_run_plane_apply_refuses_through_the_same_gate() {
        let (tmp, root) = workspace();
        arm_ws(&root);

        let before = std::fs::read_to_string(tmp.path().join("tasks/board.md")).unwrap();
        let err = apply_as_closer(&root).expect_err("armed run-plane apply must refuse");
        // An armed-law fault also renders the id, so assert the rule's own
        // teaching beside it.
        assert!(
            matches!(err, ExecError::ArmedRefusal { ref detail }
                if detail.contains(RULE_ID) && detail.contains("reviewer must not be the owner")),
            "the run plane refuses through the armed gate, naming the rule: {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("tasks/board.md")).unwrap(),
            before,
            "a refused run-plane apply lands no bytes"
        );
        assert!(
            !tmp.path().join("receipts/run.md").exists(),
            "no run receipt on a refused apply"
        );
    }

    /// Control: a never-armed workspace runs the same apply to completion — the
    /// run-plane gate is a no-op without the marker. Deliberately writes the
    /// artifact WITHOUT the marker: that state is the control's whole subject.
    #[test]
    fn s7_never_armed_run_plane_apply_lands() {
        let (tmp, root) = workspace();
        write_rule_and_artifact(&root); // the artifact is present…
        // …but NO marker → never-armed → the run-plane gate is off.
        apply_as_closer(&root).expect("never-armed: the run-plane apply lands");
        assert!(
            std::fs::read_to_string(tmp.path().join("tasks/board.md"))
                .unwrap()
                .contains("status: done"),
            "the apply committed"
        );
    }
}
