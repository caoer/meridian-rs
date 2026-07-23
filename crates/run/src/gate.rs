//! The armed-plane gate mount for the run plane (U4.2) — byte-landing parity.
//!
//! The run plane lands page bytes through `fs::apply_batch` (`executor::apply`),
//! NOT the wire-serve write choke-point. The byte-landing enumeration (plan §4
//! Block 4) still lists it as gated "through the same evaluator on the change
//! surface it produces", so this module mounts the SAME gate: it reads the
//! workspace's attested INDEX (U1.4) + once-armed marker (U2.5 read-contract) and
//! evaluates the change the executor produces through [`policy::gate`], refusing
//! BEFORE the commit.
//!
//! The load-bearing security decision lives ENTIRELY in `policy`
//! ([`policy::resolve_armed_set`] / [`policy::gate`], single copy, unit-tested).
//! This module is only the disk glue — byte-identical in intent to
//! `wire_serve::gate`; the two copies of the trivial I/O plumbing cannot harbour
//! a security bug because they carry no decision, only bytes.

use model::{Document, Edit, NodeKind};

/// A disk-backed [`policy::ConventionSource`]: `conventions/<slug>/` under root.
struct DiskConventions<'a> {
    root: &'a fs::WorkspaceRoot,
}

impl policy::ConventionSource for DiskConventions<'_> {
    fn files_for<'a>(&'a self, slug: &str) -> Box<dyn policy::ConventionFiles + 'a> {
        Box::new(DiskConventionFolder {
            base: self.root.0.join("conventions").join(slug),
        })
    }
}

struct DiskConventionFolder {
    base: std::path::PathBuf,
}

impl policy::ConventionFiles for DiskConventionFolder {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.base.join(rel))
    }
    fn exists(&self, rel: &str) -> bool {
        self.base.join(rel).exists()
    }
}

/// Load + verify the workspace's armed law from disk (mirrors
/// `wire_serve::gate::load_armed_set`). Absent INDEX maps to `None` (safe:
/// fail-closed on once-armed, no-op on never-armed); the marker probe fails
/// CLOSED on an ambiguous stat.
fn load_armed_set(root: &fs::WorkspaceRoot) -> policy::ArmedSet {
    let index = std::fs::read_to_string(root.0.join(fs::domain::RESERVED_INDEX_PATH)).ok();
    let ever_armed = root
        .0
        .join(fs::domain::ATTESTED_MARKER_PATH)
        .try_exists()
        .unwrap_or(true);
    let source = DiskConventions { root };
    policy::resolve_armed_set(
        index.as_deref(),
        ever_armed,
        &source,
        policy::CheckLimits::default(),
    )
}

/// Gate the run-plane change the executor is about to commit. Returns the refusal
/// detail when the workspace's armed law refuses (the executor turns it into
/// [`crate::executor::ExecError::ArmedRefusal`]), or `None` on a no-op/pass.
///
/// `before`/`after` are the pre/post-apply page states; `page` is the
/// workspace-relative path (stamped onto the change so the convention scope can
/// match it — `fs::load`/`model::build` leave the path empty); `edits` are the
/// planned model edits; `actor` is `run:<task>`.
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
    match policy::gate(&change, &load_armed_set(root)) {
        policy::GateOutcome::Ok(_) => None,
        policy::GateOutcome::Refusal(refusal) => Some(describe(refusal)),
    }
}

/// A one-line refusal detail for [`crate::executor::ExecError::ArmedRefusal`],
/// naming the convention/INDEX and citing the legal path.
fn describe(refusal: policy::GateRefusal) -> String {
    match refusal {
        policy::GateRefusal::Blocked { violations } => {
            let body = violations
                .iter()
                .map(|v| {
                    format!(
                        "`{}`: {} (legal path: {})",
                        v.slug, v.message, v.passing_scenario
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("armed convention(s) refused the run-plane change: {body}")
        }
        policy::GateRefusal::ConventionFault { detail } => detail,
        policy::GateRefusal::ArmedDrift {
            slug,
            armed_rev,
            report_rev,
        } => format!(
            "armed convention `{slug}` drifted: approved rev `{armed_rev}` but the convention \
             now reads `{report_rev}` — re-arm at the live rev, or revert the law"
        ),
        // U4.3 binding law / INDEX-integrity floor — the run plane lands bytes
        // through the same gate, so a run-plane write that edits the INDEX / an
        // armed CHECK.md, or removes the INDEX / marker, is refused here too.
        policy::GateRefusal::BindingBreak { side, teaching, .. } => {
            format!("binding-break[side={}]: {teaching}", side.as_str())
        }
        policy::GateRefusal::IndexIntegrity { teaching, .. } => teaching,
    }
}

/// Clone `doc` with its document path stamped to `page` — `fs::load` /
/// `model::build` leave the path empty, but the convention scope reads it.
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
    //! `fs::apply_batch`, so an armed convention refuses a run-plane apply
    //! through the SAME evaluator — byte-landing parity with the wire seam.

    use std::collections::BTreeMap;

    use effects::{ArgValue, Effect, EffectKind, Provenance};

    use crate::caps::CapSet;
    use crate::executor::{self, ApplyRequest, ExecError, ReceiptAddr};

    /// A task page under `tasks/**` (the convention scope), owned by `run:closer`.
    const PAGE: &str = "---\nowner: run:closer\nstatus: todo\n---\n\n# Board\n\n- item\n";

    const CHECK_MD: &str = "---\npaths:\n  - tasks/**\n---\n\n# reviewer-not-owner\n\n\
        ```starlark\ndef check_change(change):\n    owner = change.doc.frontmatter.get(\"owner\")\n    \
        actor = change.actor\n    if actor != None and owner != None and actor == owner:\n        \
        refuse(message = \"reviewer must not be the owner\", passing = \"scenarios/reviewer-close.md\")\n```\n";

    fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("tasks")).unwrap();
        std::fs::write(tmp.path().join("tasks/board.md"), PAGE).unwrap();
        let root = fs::WorkspaceRoot(tmp.path().to_owned());
        (tmp, root)
    }

    fn write_convention(root: &fs::WorkspaceRoot) {
        let dir = root.0.join("conventions/reviewer-not-owner");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CHECK.md"), CHECK_MD).unwrap();
    }

    fn arm_block(root: &fs::WorkspaceRoot) {
        let files = super::DiskConventionFolder {
            base: root.0.join("conventions/reviewer-not-owner"),
        };
        let swept =
            policy::sweep(&files, "reviewer-not-owner", policy::CheckLimits::default()).unwrap();
        let rev = swept.rev().to_string();
        let armed = policy::arm(swept, &rev, policy::Enforcement::Block).unwrap();
        std::fs::write(
            root.0.join(fs::domain::RESERVED_INDEX_PATH),
            policy::generate_index(&[armed]),
        )
        .unwrap();
    }

    fn set_marker(root: &fs::WorkspaceRoot) {
        let p = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
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
                caps: &CapSet::parse("md.set_field").unwrap(),
                pin_root: &now,
                live_root: &now,
                receipt: Some(ReceiptAddr {
                    path: "receipts/run.md".to_owned(),
                    anchor: "r-000001".to_owned(),
                }),
                takeover: false,
                exec: None,
                depth: 0,
            },
        )
    }

    // ── Scenario 7 — run-plane apply violating an armed convention REFUSES ──────
    #[test]
    fn s7_run_plane_apply_refuses_through_the_same_gate() {
        let (tmp, root) = workspace();
        write_convention(&root);
        arm_block(&root);
        set_marker(&root);

        let before = std::fs::read_to_string(tmp.path().join("tasks/board.md")).unwrap();
        let err = apply_as_closer(&root).expect_err("armed run-plane apply must refuse");
        assert!(
            matches!(err, ExecError::ArmedRefusal { ref detail } if detail.contains("reviewer-not-owner")),
            "the run plane refuses through the armed gate, naming the convention: {err:?}"
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
    /// run-plane gate is a no-op without the marker.
    #[test]
    fn s7_never_armed_run_plane_apply_lands() {
        let (tmp, root) = workspace();
        write_convention(&root);
        arm_block(&root); // INDEX present…
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
