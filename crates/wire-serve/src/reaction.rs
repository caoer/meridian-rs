//! Reaction-mode evaluation for one landed document change.
//!
//! Shared feeder leaf (Law 3): resolves armed hooks for a landed
//! change from the same before/after model states, evaluates them, and reports
//! faults. Does not interpret `how:`, deliver an intent, or claim delivery.
//! Resolution is one call (`resolve_armed_law`); the once-armed marker is the
//! pivot (not artifact presence — silent-disarm). Faults ride the frame as
//! [`wire::EffectFinding::ArmedFault`], isolated per row so one bad page cannot
//! silence the path. A reaction never vetoes.

use model::{Document, Edit};
use policy::armed::Mode;
use policy::armed_law::ArmedFault;

use crate::armed_disk;

/// Evaluate armed in-scope hooks for a landed change; report every fault that kept one from running.
///
/// Revisions come from the supplied model states (receipt cursor). Empty evaluations
/// are dropped (no-effects wire bytes); fault envelopes are never dropped. No error
/// return — a reaction must not fail a write that already landed.
#[must_use]
pub fn feed_landed_change(
    root: &fs::WorkspaceRoot,
    before: &Document,
    after: &Document,
    edits: &[Edit],
    op: policy::ChangeOp,
    actor: Option<&str>,
) -> Vec<wire::EffectEnvelope> {
    // Never-armed: free silent no-op. Armed + absent artifact falls through as fault.
    if !armed_disk::once_armed(root) {
        return Vec::new();
    }

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

    let law = armed_disk::resolve_at(root, &change.doc.path);

    // Every fault reaches the operator (door is not on this path).
    let mut envelopes: Vec<wire::EffectEnvelope> =
        law.faults().iter().map(fault_envelope).collect();

    // `mode == armed` is hook vocabulary — no kind test of our own.
    let hooks: Vec<(String, &policy::Hook)> = law
        .rules()
        .iter()
        .filter(|armed| armed.mode() == Mode::Armed)
        .filter_map(|armed| {
            armed
                .rule()
                .hook()
                .map(|hook| (armed.id().as_str().to_string(), hook))
        })
        .collect();
    if hooks.is_empty() {
        return envelopes;
    }

    let event = policy::derive_event(&change, &before.root.node_rev.0, &after.root.node_rev.0, 0);
    match policy::evaluate_loaded_hooks(hooks.iter().map(|(id, hook)| (id.as_str(), *hook)), &event)
    {
        Ok(mut outcomes) => {
            outcomes.retain(|outcome| {
                !outcome.intents.is_empty()
                    || !outcome.narrowed.is_empty()
                    || !outcome.findings.is_empty()
            });
            envelopes.extend(outcomes.into_iter().map(project_outcome));
        }
        // Evaluator all-or-nothing; report via same fault channel, never empty frame.
        Err(error) => envelopes.push(fault_finding(rule_id_of(&error), &error)),
    }
    envelopes
}

/// One artifact fault as its own envelope: no intents, no `how:`, surface rendering as finding.
fn fault_envelope(fault: &ArmedFault) -> wire::EffectEnvelope {
    fault_finding(
        fault.id().map(|id| id.as_str().to_string()),
        &fault.to_string(),
    )
}

fn fault_finding(rule_id: Option<String>, detail: &impl std::fmt::Display) -> wire::EffectEnvelope {
    wire::EffectEnvelope {
        intents: Vec::new(),
        narrowed: Vec::new(),
        findings: vec![wire::EffectFinding::ArmedFault {
            rule_id,
            detail: detail.to_string(),
        }],
        how: String::new(),
    }
}

/// The rule an evaluation fault names, when it names one.
fn rule_id_of(error: &policy::HookEvalError) -> Option<String> {
    match error {
        policy::HookEvalError::MalformedIntent { rule_id, .. } => Some(rule_id.clone()),
        policy::HookEvalError::Eval(_) => None,
    }
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
    use policy::armed::{ArmRequest, ArmRoot};
    use policy::{PageRef, RuleId, RuleIndex, ScopeLayer, page_rev};

    /// Hook page fixture: registration keys + declaration keys (feeder uses registration only).
    const HOOK_ID: &str = "task.review-notify";

    fn hook_page(paths: &str) -> String {
        format!(
            r#"---
tags: [type/rule, rules/hook]
id: {HOOK_ID}
kind: hook
severity: info
paths: [{paths}]
caps:  [proto.send]
budget: {{ steps: 10000, mem: 4194304 }}
how:
  route: {{ info: channel-review }}
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
"#
        )
    }

    fn doc(path: &str, status: &str) -> Document {
        let raw = format!("---\ntype: task\nstatus: {status}\nreviewer: e4201e72\n---\n\n# Task\n");
        let mut doc = model::build(raw.clone(), syntax::parse(&raw));
        if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    /// Workspace with a rule page armed at `mode` via the real ARM act (direct mount, not rules-folder).
    fn armed_workspace(
        page_body: &str,
        mode: Mode,
        arm_root: &str,
        page_path: &str,
    ) -> (tempfile::TempDir, fs::WorkspaceRoot) {
        arm_pages(arm_root, &[(page_path, page_body, HOOK_ID, mode)])
    }

    /// Arm every `(page path, bytes, id, mode)` at `arm_root` through the real ARM act.
    fn arm_pages(
        arm_root: &str,
        pages: &[(&str, &str, &str, Mode)],
    ) -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let temp = tempfile::tempdir().expect("temp workspace");
        for (path, body, ..) in pages {
            write_page(temp.path(), path, body);
        }
        let index = RuleIndex::discover(pages.iter().map(|(path, body, ..)| PageRef {
            layer: ScopeLayer::Workspace,
            page: path,
            bytes: body,
        }));
        // The act loads every firing winner, so it needs the bytes the index
        // was discovered from — a `path → bytes` map IS a `PageSource`.
        let source: std::collections::BTreeMap<String, String> = pages
            .iter()
            .map(|(path, body, ..)| ((*path).to_string(), (*body).to_string()))
            .collect();
        let artifact = policy::armed::arm(
            &index,
            &ArmRoot::parse(arm_root).expect("a legal arm root"),
            pages.iter().map(|(_, body, id, mode)| ArmRequest {
                id: RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: page_rev(body),
            }),
            &source,
            policy::CheckLimits::default(),
        )
        .expect("the fixture arms");

        write_page(
            temp.path(),
            policy::armed::ARMED_RULES_PATH,
            &artifact.render(),
        );
        // Marker + artifact: both required; artifact alone reads as never-armed.
        write_page(temp.path(), fs::domain::ATTESTED_MARKER_PATH, "");
        let root = fs::WorkspaceRoot(temp.path().to_path_buf());
        (temp, root)
    }

    fn write_page(root: &std::path::Path, rel: &str, body: &str) {
        let absolute = root.join(rel);
        std::fs::create_dir_all(absolute.parent().expect("page parent")).expect("page parent");
        std::fs::write(absolute, body).expect("write page");
    }

    /// Hook armed workspace-wide over `tasks/*.md`.
    fn armed_root() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        armed_workspace(
            &hook_page("\"tasks/*.md\""),
            Mode::Armed,
            ".",
            "task-review-notify.md",
        )
    }

    fn feed(
        root: &fs::WorkspaceRoot,
        path: &str,
        from: &str,
        to: &str,
    ) -> Vec<wire::EffectEnvelope> {
        let before = doc(path, from);
        let after = doc(path, to);
        feed_landed_change(
            root,
            &before,
            &after,
            &[],
            policy::ChangeOp::Splice,
            Some("worker"),
        )
    }

    /// Every `ArmedFault` finding as `(rule id, detail)`.
    fn faults(envelopes: &[wire::EffectEnvelope]) -> Vec<(Option<String>, String)> {
        envelopes
            .iter()
            .flat_map(|envelope| &envelope.findings)
            .filter_map(|finding| match finding {
                wire::EffectFinding::ArmedFault { rule_id, detail } => {
                    Some((rule_id.clone(), detail.clone()))
                }
                wire::EffectFinding::BudgetExceeded { .. } => None,
            })
            .collect()
    }

    /// Envelopes that carry a reaction, not a fault report.
    fn reactions(envelopes: &[wire::EffectEnvelope]) -> Vec<&wire::EffectEnvelope> {
        envelopes
            .iter()
            .filter(|envelope| !envelope.intents.is_empty() || !envelope.narrowed.is_empty())
            .collect()
    }

    #[test]
    fn landed_status_change_emits_canonical_armed_intent() {
        let (_temp, root) = armed_root();
        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].intents.len(), 1);
        let intent = &outcomes[0].intents[0];
        assert_eq!(intent.rule_id, HOOK_ID);
        assert_eq!(intent.target.as_deref(), Some("e4201e72"));
        assert_eq!(
            intent.receipt,
            effects::receipt_address("tasks/x.md", &doc("tasks/x.md", "review").root.node_rev.0)
        );
        let json = serde_json::to_string(&outcomes).expect("serializes");
        assert!(json.contains("task moved to review"));
        assert!(!json.contains("delivered"), "no delivery claim: {json}");
    }

    #[test]
    fn a_never_armed_workspace_has_no_artifact_and_emits_nothing() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let root = fs::WorkspaceRoot(temp.path().to_path_buf());
        assert!(feed(&root, "tasks/x.md", "in-progress", "review").is_empty());
    }

    /// Unreadable-but-present artifact is a fault, never never-armed (silent-disarm).
    #[test]
    fn an_unreadable_artifact_is_a_fault_never_a_never_armed_workspace() {
        let (temp, root) = armed_root();
        // Directory at artifact path → error, not absence.
        let artifact = temp.path().join(policy::armed::ARMED_RULES_PATH);
        std::fs::remove_file(&artifact).expect("clear the artifact");
        std::fs::create_dir(&artifact).expect("make it unreadable");

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        assert_eq!(faults(&outcomes).len(), 1, "{outcomes:?}");
    }

    #[test]
    fn an_out_of_scope_path_emits_nothing() {
        let (_temp, root) = armed_root();
        assert!(feed(&root, "notes/x.md", "in-progress", "review").is_empty());
    }

    #[test]
    fn an_in_scope_change_that_arms_no_intent_is_empty() {
        let (_temp, root) = armed_root();
        assert!(feed(&root, "tasks/x.md", "todo", "in-progress").is_empty());
    }

    #[test]
    fn the_feeder_never_reads_the_changed_document_from_disk() {
        let (_temp, root) = armed_root();
        assert!(!root.0.join("tasks/x.md").exists());
        assert_eq!(feed(&root, "tasks/x.md", "in-progress", "review").len(), 1);
    }

    #[test]
    fn a_row_armed_off_does_not_fire_though_its_page_is_intact_and_in_scope() {
        let (_temp, root) = armed_workspace(
            &hook_page("\"tasks/*.md\""),
            Mode::Off,
            ".",
            "task-review-notify.md",
        );
        assert!(
            feed(&root, "tasks/x.md", "in-progress", "review").is_empty(),
            "hook activation is binary and this row is off"
        );
    }

    #[test]
    fn a_pinned_page_edited_after_arming_fails_closed_instead_of_firing_on_new_bytes() {
        let (temp, root) = armed_root();
        let page = temp.path().join("task-review-notify.md");
        let edited = format!(
            "{}\n<!-- edited after arming -->\n",
            hook_page("\"tasks/*.md\"")
        );
        std::fs::write(&page, &edited).expect("edit the pinned page");

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        assert!(
            reactions(&outcomes).is_empty(),
            "the row reddened, so it does not fire on the new bytes"
        );
        // Silent to write, reported to operator via same fault channel.
        let found = faults(&outcomes);
        let [(id, detail)] = found.as_slice() else {
            panic!("the drift is reported: {outcomes:?}");
        };
        assert_eq!(id.as_deref(), Some(HOOK_ID));
        assert!(detail.contains("edited after arming"), "{detail}");
    }

    #[test]
    fn selection_is_by_arm_root_so_a_sibling_scope_never_fires() {
        // Armed at sessions/s1 only — sibling s2 selects nothing.
        let (_temp, root) = armed_workspace(
            &hook_page("\"sessions/**/*.md\""),
            Mode::Armed,
            "sessions/s1",
            "sessions/s1/notify.md",
        );

        assert_eq!(
            feed(&root, "sessions/s1/task.md", "in-progress", "review").len(),
            1,
            "the write inside the arm root fires"
        );
        assert!(
            feed(&root, "sessions/s2/task.md", "in-progress", "review").is_empty(),
            "sibling subtrees never interact — the arm root does not contain this path"
        );
    }

    /// Corrupt artifact on an armed workspace must reach the operator.
    #[test]
    fn a_corrupt_artifact_on_an_armed_workspace_reaches_the_operator() {
        let (temp, root) = armed_root();
        let artifact = temp.path().join(policy::armed::ARMED_RULES_PATH);
        let truncated = std::fs::read_to_string(&artifact)
            .expect("read artifact")
            .replace("| id | page | rev | scope | mode |", "| id | page |");
        std::fs::write(&artifact, truncated).expect("corrupt the artifact");

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        assert!(
            reactions(&outcomes).is_empty(),
            "a corrupt law arms nothing"
        );
        let found = faults(&outcomes);
        let [(id, detail)] = found.as_slice() else {
            panic!("a corrupt artifact never reads as nothing armed: {outcomes:?}");
        };
        assert_eq!(*id, None, "the fault is about the artifact, not a rule");
        assert!(detail.contains("is corrupt"), "{detail}");
    }

    /// Emptied well-formed artifact on armed workspace reports disarm, does not obey it.
    #[test]
    fn an_emptied_artifact_on_an_armed_workspace_reports_the_disarm_rather_than_obeying_it() {
        let (temp, root) = armed_root();
        let emptied = policy::armed::ArmedArtifact::default().render();
        policy::armed::parse_artifact(&emptied).expect("the deletion is well-formed, not corrupt");
        std::fs::write(temp.path().join(policy::armed::ARMED_RULES_PATH), &emptied)
            .expect("delete every row");

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        let found = faults(&outcomes);
        let [(id, detail)] = found.as_slice() else {
            panic!("deleting every row must not read as nothing armed: {outcomes:?}");
        };
        assert_eq!(*id, None);
        assert!(detail.contains("attests no row at all"), "{detail}");
    }

    /// One unloadable row must not silence other reactions at the path.
    #[test]
    fn one_unloadable_row_does_not_silence_the_other_reactions_at_the_path() {
        let good = hook_page("\"tasks/*.md\"");
        let bare = "---\ntags: [type/rule, rules/hook]\nid: bare.rule\n---\n\n# rule\n";
        let (_temp, root) = arm_pages(
            ".",
            &[
                ("task-review-notify.md", &good, HOOK_ID, Mode::Armed),
                ("bare.md", bare, "bare.rule", Mode::Armed),
            ],
        );

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        let live = reactions(&outcomes);
        assert_eq!(live.len(), 1, "the good hook still reacts: {outcomes:?}");
        assert_eq!(live[0].intents[0].rule_id, HOOK_ID);

        let found = faults(&outcomes);
        let [(id, detail)] = found.as_slice() else {
            panic!("the bad row is reported by name: {outcomes:?}");
        };
        assert_eq!(id.as_deref(), Some("bare.rule"));
        assert!(detail.contains("bare.md"), "{detail}");
    }

    /// Kind seam is only at `load_rule`; ruled shape (tag, no `kind:`) still arms and fires.
    #[test]
    fn a_hook_page_without_a_kind_key_arms_and_fires() {
        let ruled = hook_page("\"tasks/*.md\"").replace("kind: hook\n", "");
        assert!(
            !ruled.contains("kind:"),
            "the ruled shape declares no kind:"
        );
        let (_temp, root) = armed_workspace(&ruled, Mode::Armed, ".", "task-review-notify.md");

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        assert!(faults(&outcomes).is_empty(), "{outcomes:?}");
        assert_eq!(reactions(&outcomes).len(), 1, "the tag is the one name");
    }

    /// Loaded-hook eval fault reports via same channel, not dropped as empty frame.
    #[test]
    fn a_loaded_hook_eval_fault_is_reported_rather_than_dropped() {
        let faulting = hook_page("\"tasks/*.md\"").replace(
            "    for delta in event.changes:\n",
            "    ignored = event.actor\n    for delta in event.changes:\n",
        );
        let (_temp, root) = armed_workspace(&faulting, Mode::Armed, ".", "task-review-notify.md");

        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review");
        assert!(reactions(&outcomes).is_empty());
        assert_eq!(
            faults(&outcomes).len(),
            1,
            "event.actor passes the load lint and faults at evaluation: {outcomes:?}"
        );
    }
}
