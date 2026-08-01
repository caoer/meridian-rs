//! Reaction-mode evaluation for one landed document change.
//!
//! This is the shared feeder leaf used by both hosts (`docs/laws.md` Law 3): it
//! derives the reaction payload from the same before/after model states the guarded
//! write and the external watcher already hold, resolves the armed HOOKs governing
//! the changed path, and evaluates them. It does not interpret `how:`, deliver an
//! intent, or make a delivery claim.
//!
//! # Resolution is the ARM effective set, and nothing else
//! The armed law comes from the attested artifact at [`policy::armed::ARMED_RULES_PATH`]
//! (registration ruling § 4) through exactly ONE call,
//! [`policy::armed_law::resolve_armed_law`], which pivots on the once-armed state,
//! selects the rows governing the path, freeze-checks them, and loads each one's
//! page. This leaf composes none of that by hand: `verify_at`'s own documentation
//! records that each obvious hand-composition of `select_at` + `verify` is a live
//! defect, in opposite directions.
//!
//! **The once-armed state this leaf supplies is the ARTIFACT's presence, not the
//! `meridian/attested` marker — deliberately, and only until the door re-keys.**
//! Through the cutover window two resolution surfaces share that one marker: the
//! write door still reads the dying `conventions/INDEX.md`. Marking a workspace
//! once-armed for the artifact therefore makes the INDEX door refuse every write
//! (measured: `convention_fault`, *"attested INDEX is missing on a workspace that
//! has been armed"*). One marker cannot pivot two laws, so it binds in the diff that
//! leaves the door reading only one of them. What that defers is the artifact-ABSENT
//! case alone, and deleting the artifact is already refused at the door — its path is
//! reserved ([`fs::domain::ARMED_RULES_PATH`]). The row-DELETION case, which is a
//! present artifact attesting nothing, is not deferred and is closed here.
//!
//! The feeder walks no `conventions/` folder and reads no `kind:` frontmatter to
//! decide what is armed — both are dead registration surfaces under the ruling. A
//! row's `mode` carries its kind, and `armed` is hook vocabulary, so the firing hook
//! rows fall out of that one call with no kind test of our own.
//!
//! # A reaction never vetoes, so every fault is REPORTED
//! Everything here runs AFTER the write has landed, so this leaf can neither refuse
//! nor mutate. That makes silence its only failure mode, and silence was the defect:
//! an artifact fault used to reach a `.unwrap_or_default()` at both call sites and
//! read as "nothing to react to".
//!
//! So faults ride the frame instead, as [`wire::EffectFinding::ArmedFault`] — this
//! host's CHANNEL onto the one artifact-fault surface, rendered in that surface's own
//! words ([`policy::armed_law::ArmedFault`]). The door refuses on the same faults,
//! with the same text; the disposition differs because the ruling splits it by kind,
//! and the vocabulary does not differ at all.
//!
//! A fault is also isolated per ROW: one armed page that will not load reports itself
//! and leaves every other rule at the path firing. Propagating it instead silenced
//! the whole path, which is the same silent disarm one layer down.

use std::cell::RefCell;
use std::collections::BTreeMap;

use model::{Document, Edit};
use policy::armed::{Mode, PageSource};
use policy::armed_law::{ArmedFault, resolve_armed_law};

/// Evaluate the armed, in-scope HOOKs for one change that has already landed, and
/// report every fault that kept one from running.
///
/// The file revisions come from the supplied model states. They are the cursor
/// coordinates the canonical receipt address is built from; no caller can substitute
/// a different post-change revision. Empty evaluations are dropped, because they
/// armed nothing and must not perturb the no-effects wire bytes — a FAULT envelope is
/// never empty and is never dropped.
///
/// There is no error return. A reaction may not fail a write that has already landed,
/// and an unreported fault is the silent disarm this surface exists to close, so the
/// only honest shape is "envelopes, some of which are faults".
#[must_use]
pub fn feed_landed_change(
    root: &fs::WorkspaceRoot,
    before: &Document,
    after: &Document,
    edits: &[Edit],
    op: policy::ChangeOp,
    actor: Option<&str>,
) -> Vec<wire::EffectEnvelope> {
    // The artifact's PRESENCE is this leaf's once-armed pivot. A never-armed
    // workspace has no artifact, and the no-op has to be free as well as silent, so
    // this returns before deriving anything.
    let Some(artifact) = read_artifact(root) else {
        return Vec::new();
    };

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

    let pages = DiskPages::new(root);
    let law = resolve_armed_law(
        Some(&artifact),
        true,
        &change.doc.path,
        &pages,
        policy::CheckLimits::default(),
    );

    // Every fault reaches the operator, including the ones that refuse at the door:
    // the door is not on this path, and a fault nobody reports is the defect.
    let mut envelopes: Vec<wire::EffectEnvelope> =
        law.faults().iter().map(fault_envelope).collect();

    // `mode == armed` is hook vocabulary, so this filter alone leaves the firing HOOK
    // rows — no kind test of our own. A check row governing the same path is the
    // door's business and silently not ours.
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
        // An already-loaded predicate faulting at evaluation aborts the batch — the
        // evaluator's own all-or-nothing, not this leaf's. Reported through the same
        // channel so it is a fault the operator reads, never an empty frame.
        Err(error) => envelopes.push(fault_finding(rule_id_of(&error), &error)),
    }
    envelopes
}

/// One artifact fault as its own envelope: no intents, no `how:` (there is no
/// declaration behind a fault), and the surface's own rendering as the finding.
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

/// Read the attested armed-set artifact, or `None` when it is absent.
///
/// An artifact that exists and cannot be read is NOT `None` — a page that is there
/// and unreadable must never pass for one that was never created, which is the
/// silent disarm in its rawest form. It reads as an empty page instead, which the
/// resolver refuses as corrupt.
fn read_artifact(root: &fs::WorkspaceRoot) -> Option<String> {
    match std::fs::read_to_string(root.0.join(policy::armed::ARMED_RULES_PATH)) {
        Ok(page) => Some(page),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

/// The pinned pages on disk, read once each.
///
/// `policy` performs no I/O, so the walk lives here. The cache matters for more than
/// speed: verification and loading must see the SAME bytes, or a page edited between
/// the two calls would load a declaration the rev check never approved.
struct DiskPages<'a> {
    root: &'a fs::WorkspaceRoot,
    seen: RefCell<BTreeMap<String, String>>,
}

impl<'a> DiskPages<'a> {
    fn new(root: &'a fs::WorkspaceRoot) -> Self {
        Self {
            root,
            seen: RefCell::new(BTreeMap::new()),
        }
    }
}

impl PageSource for DiskPages<'_> {
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

    /// The first tag-registered HOOK page in this tree. It carries BOTH key sets on
    /// purpose: `tags:` + `id:` are the REGISTRATION layer (ruling § 1/§ 2), while
    /// `kind:`/`severity:`/`paths:`/`caps:`/`budget:`/`how:` are C1's declaration
    /// layer, which § 5 leaves untouched. The feeder reads only the former to decide
    /// what is armed.
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

    /// A workspace whose rule page is armed at `mode`, through the real ARM act.
    ///
    /// Pages sit DIRECTLY in the folder they govern (the § 3 "gitignore-style"
    /// layout), so each mounts at that folder under the landed `mount_dir`. The
    /// ruled layout-folder style (`<scope>/rules/*.md` mounting at `<scope>`) is
    /// deliberately not used here: the mount law is ruled but not yet implemented,
    /// and this card consumes armed rows rather than minting the mount rule.
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
        let artifact = policy::armed::arm(
            &index,
            &ArmRoot::parse(arm_root).expect("a legal arm root"),
            pages.iter().map(|(_, body, id, mode)| ArmRequest {
                id: RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: page_rev(body),
            }),
        )
        .expect("the fixture arms");

        write_page(
            temp.path(),
            policy::armed::ARMED_RULES_PATH,
            &artifact.render(),
        );
        let root = fs::WorkspaceRoot(temp.path().to_path_buf());
        (temp, root)
    }

    fn write_page(root: &std::path::Path, rel: &str, body: &str) {
        let absolute = root.join(rel);
        std::fs::create_dir_all(absolute.parent().expect("page parent")).expect("page parent");
        std::fs::write(absolute, body).expect("write page");
    }

    /// The common fixture: the hook armed workspace-wide over `tasks/*.md`.
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

    /// Every `ArmedFault` finding the frame carries, as `(rule id, detail)`.
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

    /// The envelopes that carry an actual reaction, not a fault report.
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

    /// An artifact that EXISTS and cannot be read is a fault, not a never-armed
    /// workspace. Folding the two would be the silent disarm at its rawest: making
    /// the law unreadable would turn the gate off.
    #[test]
    fn an_unreadable_artifact_is_a_fault_never_a_never_armed_workspace() {
        let (temp, root) = armed_root();
        // A directory at the artifact's path reads as an error, not as absence.
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

    // ── the ARM effective set governs, and only it ────────────────────────────

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
        // Silent to the WRITE — a hook never vetoes — but never silent to the
        // operator: the drift is reported through the same channel every other
        // artifact fault takes.
        let found = faults(&outcomes);
        let [(id, detail)] = found.as_slice() else {
            panic!("the drift is reported: {outcomes:?}");
        };
        assert_eq!(id.as_deref(), Some(HOOK_ID));
        assert!(detail.contains("edited after arming"), "{detail}");
    }

    #[test]
    fn selection_is_by_arm_root_so_a_sibling_scope_never_fires() {
        // Armed at `sessions/s1`, so a write under the sibling `sessions/s2` is not
        // under any arm root and selects nothing.
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

    // ── the three fault shapes, through the ONE surface ───────────────────────

    /// **The feed-error shape.** A corrupt artifact on an ARMED workspace used to
    /// map to "no reaction" at both call sites (`.unwrap_or_default()`), so a
    /// gate-disabling edit dressed as a parse reached nobody. It now reaches the
    /// operator on the frame.
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

    /// **F2.** The row deletion LOOKS legitimate — a well-formed artifact page with
    /// its title, preamble and byte-exact header, and zero rows. Bound to the
    /// once-armed marker it is now a reported fault instead of a silent total
    /// disarm; at the door the same fault refuses the write.
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

    /// **F-1.** Two hooks are armed over the same path and one page will not load.
    /// The loop used to propagate that fault with `?`, so ONE bad page silenced
    /// every reaction at the path. The fault is now isolated to its own row.
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

    /// The kind seam has ONE enforcement point (`load_rule`), so a page in the
    /// ruled shape — registration tag, no `kind:` key — fires. Loading through the
    /// filename-shaped hook loader instead asserted `kind: hook`, which let a page
    /// arm cleanly and never fire.
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

    /// An already-loaded predicate faulting at EVALUATION aborts the evaluator's
    /// batch — its all-or-nothing, one layer below this leaf. It travels the same
    /// channel, so the operator reads a fault rather than an empty frame.
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
