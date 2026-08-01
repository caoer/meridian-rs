//! Reaction-mode evaluation for one landed document change.
//!
//! This is the shared feeder leaf used by both hosts. It derives the reaction
//! payload from the same before/after model states as the guarded write and the
//! external watcher, resolves the workspace's attested HOOKs, and evaluates them.
//! It does not interpret `how:`, deliver an intent, or make a delivery claim.

use model::{Document, Edit};

/// A fault while resolving or evaluating the reaction rules for a landed change.
#[derive(Debug)]
pub enum FeedError {
    /// The workspace's attested convention set could not be resolved.
    Armed(policy::GateRefusal),
    /// An already-loaded HOOK faulted during reaction evaluation.
    Hook(policy::HookEvalError),
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Armed(error) => write!(f, "armed convention fault: {error:?}"),
            Self::Hook(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FeedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Armed(_) => None,
            Self::Hook(error) => Some(error),
        }
    }
}

/// Evaluate armed, in-scope HOOKs for one change that has already landed.
///
/// The file revisions come from the supplied model states. They are the cursor
/// coordinates used by the canonical receipt address; no caller can substitute a
/// different post-change revision. Empty evaluations are removed because they
/// armed nothing and must not perturb the no-effects wire bytes.
///
/// # Errors
/// Returns [`FeedError::Armed`] when the attested INDEX cannot resolve, or
/// [`FeedError::Hook`] when an already-loaded predicate faults at evaluation.
pub fn feed_landed_change(
    root: &fs::WorkspaceRoot,
    before: &Document,
    after: &Document,
    edits: &[Edit],
    op: policy::ChangeOp,
    actor: Option<&str>,
) -> Result<Vec<wire::EffectEnvelope>, FeedError> {
    let change = policy::derive_change(
        before,
        after,
        edits,
        policy::Invocation {
            op,
            actor,
            force: false,
        },
        &[],
        &|_| None,
    );
    let event = policy::derive_event(&change, &before.root.node_rev.0, &after.root.node_rev.0, 0);
    let armed = crate::gate::load_armed_set(root);

    let mut outcomes = match &armed {
        policy::ArmedSet::NeverArmed => Vec::new(),
        policy::ArmedSet::Armed(conventions) => {
            policy::evaluate_hooks(conventions, &event).map_err(FeedError::Hook)?
        }
        policy::ArmedSet::Faulted(_) => {
            let policy::GateOutcome::Refusal(refusal) = policy::gate(&change, &armed) else {
                unreachable!("a faulted armed set must fail closed")
            };
            return Err(FeedError::Armed(refusal));
        }
    };

    outcomes.retain(|outcome| {
        !outcome.intents.is_empty() || !outcome.narrowed.is_empty() || !outcome.findings.is_empty()
    });
    Ok(outcomes.into_iter().map(project_outcome).collect())
}

fn project_outcome(outcome: policy::HookOutcome) -> wire::EffectEnvelope {
    wire::EffectEnvelope {
        intents: outcome.intents.into_iter().map(project_intent).collect(),
        narrowed: outcome.narrowed.into_iter().map(project_intent).collect(),
        findings: outcome.findings.into_iter().map(project_finding).collect(),
        how: outcome.how,
    }
}

fn project_intent(intent: policy::Intent) -> wire::Intent {
    wire::Intent {
        rule_id: intent.rule_id,
        seq: intent.seq,
        action: intent.action,
        target: intent.target,
        severity: intent.severity,
        payload: intent.payload,
        receipt: intent.receipt,
    }
}

fn project_finding(finding: policy::HookFinding) -> wire::EffectFinding {
    match finding {
        policy::HookFinding::BudgetExceeded {
            rule_id,
            steps,
            mem,
        } => wire::EffectFinding::BudgetExceeded {
            rule_id,
            steps,
            mem,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use policy::{CheckLimits, ConventionFiles, Enforcement};

    const CHECK: &str = "---\npaths:\n  - tasks/*.md\n---\n\n# task-status-notify\n\n```starlark\ndef check_change(change):\n    pass\n```\n";
    const HOOK: &str = r#"---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps:  [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind != "frontmatter" or delta.key != "status":
            continue
        if delta.new != "review":
            continue
        return intent(
            action = "notify",
            target = event.facts.fm.get("reviewer"),
            severity = "info",
            payload = "task moved to review",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
"#;

    struct Folder(std::path::PathBuf);

    impl ConventionFiles for Folder {
        fn read(&self, rel: &str) -> std::io::Result<String> {
            std::fs::read_to_string(self.0.join(rel))
        }

        fn exists(&self, rel: &str) -> bool {
            self.0.join(rel).exists()
        }
    }

    fn doc(path: &str, status: &str) -> Document {
        let raw = format!("---\ntype: task\nstatus: {status}\nreviewer: e4201e72\n---\n\n# Task\n");
        let mut doc = model::build(raw.clone(), syntax::parse(&raw));
        if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    fn armed_root() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        armed_root_with_hook(HOOK)
    }

    fn armed_root_with_hook(hook: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let temp = tempfile::tempdir().expect("temp workspace");
        let root = fs::WorkspaceRoot(temp.path().to_path_buf());
        let folder = temp.path().join("conventions/task-status-notify");
        std::fs::create_dir_all(&folder).expect("convention folder");
        std::fs::write(folder.join("CHECK.md"), CHECK).expect("CHECK.md");
        std::fs::write(folder.join("HOOK.md"), hook).expect("HOOK.md");

        let swept = policy::sweep(
            &Folder(folder),
            "task-status-notify",
            CheckLimits::default(),
        )
        .expect("convention sweeps");
        let rev = policy::evidence_rev(CHECK);
        let armed = policy::arm(swept, &rev, Enforcement::Warn).expect("convention arms");
        std::fs::write(
            temp.path().join("conventions/INDEX.md"),
            policy::generate_index(&[armed]),
        )
        .expect("INDEX.md");
        let marker = temp.path().join(fs::domain::ATTESTED_MARKER_PATH);
        std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker parent");
        std::fs::write(marker, "attested\n").expect("attested marker");
        (temp, root)
    }

    #[test]
    fn landed_status_change_emits_canonical_armed_intent() {
        let (_temp, root) = armed_root();
        let before = doc("tasks/x.md", "in-progress");
        let after = doc("tasks/x.md", "review");

        let outcomes = feed_landed_change(
            &root,
            &before,
            &after,
            &[],
            policy::ChangeOp::Splice,
            Some("worker"),
        )
        .expect("reaction evaluates");

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].intents.len(), 1);
        let intent = &outcomes[0].intents[0];
        assert_eq!(intent.rule_id, "task-status-notify");
        assert_eq!(intent.target.as_deref(), Some("e4201e72"));
        assert_eq!(
            intent.receipt,
            effects::receipt_address("tasks/x.md", &after.root.node_rev.0)
        );
        let json = serde_json::to_value(&outcomes).expect("serializes");
        assert!(json.to_string().contains("task moved to review"));
        assert!(!json.to_string().contains("delivered"));
    }

    #[test]
    fn never_armed_and_out_of_scope_changes_emit_nothing() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let root = fs::WorkspaceRoot(temp.path().to_path_buf());
        let before = doc("tasks/x.md", "in-progress");
        let after = doc("tasks/x.md", "review");
        assert!(
            feed_landed_change(&root, &before, &after, &[], policy::ChangeOp::Splice, None,)
                .expect("never-armed is a no-op")
                .is_empty()
        );

        let (_armed_temp, armed_root) = armed_root();
        let before = doc("notes/x.md", "in-progress");
        let after = doc("notes/x.md", "review");
        assert!(
            feed_landed_change(
                &armed_root,
                &before,
                &after,
                &[],
                policy::ChangeOp::Splice,
                None,
            )
            .expect("out of scope is silent")
            .is_empty()
        );
    }

    #[test]
    fn in_scope_change_that_does_not_arm_an_intent_is_empty() {
        let (_temp, root) = armed_root();
        let before = doc("tasks/x.md", "todo");
        let after = doc("tasks/x.md", "in-progress");

        assert!(
            feed_landed_change(&root, &before, &after, &[], policy::ChangeOp::Splice, None,)
                .expect("reaction evaluates")
                .is_empty()
        );
    }

    #[test]
    fn helper_never_reads_the_document_from_disk() {
        let (_temp, root) = armed_root();
        assert!(!root.0.join("tasks/x.md").exists());
        let before = doc("tasks/x.md", "in-progress");
        let after = doc("tasks/x.md", "review");
        assert_eq!(
            feed_landed_change(&root, &before, &after, &[], policy::ChangeOp::Splice, None,)
                .expect("held states are sufficient")
                .len(),
            1
        );
    }

    #[test]
    fn loaded_hook_eval_fault_reaches_the_unprojected_feeder_boundary() {
        let faulting = HOOK.replace(
            "    for delta in event.changes:\n",
            "    ignored = event.actor\n    for delta in event.changes:\n",
        );
        let (_temp, root) = armed_root_with_hook(&faulting);
        let before = doc("tasks/x.md", "in-progress");
        let after = doc("tasks/x.md", "review");

        let error = feed_landed_change(&root, &before, &after, &[], policy::ChangeOp::Splice, None)
            .expect_err("event.actor passes the load lint and faults at evaluation");
        assert!(matches!(error, FeedError::Hook(_)), "{error:?}");
    }
}
