//! Reaction-mode evaluation for one landed document change.
//!
//! This is the shared feeder leaf used by both hosts (`docs/laws.md` Law 3): it
//! derives the reaction payload from the same before/after model states the guarded
//! write and the external watcher already hold, resolves the armed HOOKs governing
//! the changed path, and evaluates them. It does not interpret `how:`, deliver an
//! intent, or make a delivery claim.
//!
//! # Resolution is the ARM effective set, and nothing else
//! The armed set comes from the attested artifact at [`policy::armed::ARMED_RULES_PATH`]
//! (registration ruling § 4), read through exactly ONE call:
//! [`policy::armed::ArmedArtifact::verify_at`], which composes the ruled SELECTION
//! law (per id, the deepest armed row whose arm root contains the path) with the
//! FREEZE check (a row whose pinned page drifted or vanished does not fire on the
//! new bytes) in the only order that is correct in both directions.
//!
//! This leaf calls neither `select_at` nor `verify` on its own. Hand-composing the
//! pair is a live defect either way round — one order fails OPEN by letting a stale
//! outer row govern an inner path whose row reddened, the other refuses TOO WIDE by
//! coupling sibling subtrees. `verify_at`'s own documentation carries the grounds.
//!
//! The feeder walks no `conventions/` folder and reads no `kind:` frontmatter to
//! decide what is armed — both are dead registration surfaces under the ruling. A
//! row's `mode` carries its kind, and `armed` is hook vocabulary, so the firing hook
//! rows fall out of that one call with no kind test of our own.
//!
//! # A reaction never vetoes
//! Everything here runs AFTER the write has landed. A drifted hook row falls silent
//! rather than refusing — refusing a write on a reaction's behalf would hand a hook
//! the veto the ruling denies it. Refusal on a red CHECK row is the door's line
//! ([`policy::armed::ArmedVerdict::refusing`]), not this leaf's.

use std::cell::RefCell;
use std::collections::BTreeMap;

use model::{Document, Edit};
use policy::armed::{ArmedRow, Mode, PageSource};

/// A fault while resolving or evaluating the reaction rules for a landed change.
///
/// Every variant is a REPORT, never a refusal: the write it describes has already
/// landed, and the caller's duty is to emit no reaction — not to fail the write.
#[derive(Debug)]
pub enum FeedError {
    /// The attested armed-set artifact is present but is not a trustworthy armed
    /// set. A corrupt artifact must never read as "nothing armed" — that would be a
    /// gate-disabling edit dressed as a parse.
    Artifact(policy::armed::ArtifactCorrupt),
    /// An armed HOOK row's pinned page did not load as a HOOK declaration. The row
    /// attests the page's REGISTRATION (its tag and rev); the declaration keys are a
    /// separate layer, so an armed page can still be undeclarable.
    ///
    /// **INTERIM — delete this variant when cutover 3a's `load_rule` lands.** The
    /// advisor ruled `load_rule` the ONE enforcement point for the kind seam
    /// (`kind:` present must agree loudly at load, absent derives from the tag).
    /// This variant exists only so the seam is loud rather than silent until then;
    /// keeping it afterwards would leave a second enforcement point alive, which
    /// that ruling forbids.
    Declaration {
        /// The armed id whose page would not load.
        id: String,
        /// The pinned page.
        page: String,
        /// Why the declaration did not load.
        error: policy::LoadError,
    },
    /// An already-loaded HOOK faulted during reaction evaluation.
    Hook(policy::HookEvalError),
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Artifact(error) => error.fmt(f),
            Self::Declaration { id, page, error } => write!(
                f,
                "armed id `{id}` is attested against `{page}`, which does not load as a HOOK \
                 declaration: {error}"
            ),
            Self::Hook(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FeedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Artifact(error) => Some(error),
            Self::Declaration { error, .. } => Some(error),
            Self::Hook(error) => Some(error),
        }
    }
}

/// Evaluate the armed, in-scope HOOKs for one change that has already landed.
///
/// The file revisions come from the supplied model states. They are the cursor
/// coordinates the canonical receipt address is built from; no caller can substitute
/// a different post-change revision. Empty evaluations are dropped, because they
/// armed nothing and must not perturb the no-effects wire bytes.
///
/// # Errors
/// [`FeedError`] — see its variants. Every one means "emit no reaction", never
/// "refuse the write".
pub fn feed_landed_change(
    root: &fs::WorkspaceRoot,
    before: &Document,
    after: &Document,
    edits: &[Edit],
    op: policy::ChangeOp,
    actor: Option<&str>,
) -> Result<Vec<wire::EffectEnvelope>, FeedError> {
    // A never-armed workspace has no artifact. Return before deriving anything: the
    // no-op has to be free as well as silent, so this path perturbs nothing.
    let Some(page) = read_artifact(root)? else {
        return Ok(Vec::new());
    };
    let artifact = policy::armed::parse_artifact(&page).map_err(FeedError::Artifact)?;

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

    // ONE call: `verify_at` composes the selection law and the freeze check in the
    // only order that is correct in both directions. Composing them here instead —
    // in either order — is a live defect, and its doc comment says which. So this
    // leaf never calls `select_at` or `verify` itself.
    //
    // `firing()` is "pinned rev intact AND mode != off", and `armed` is hook
    // vocabulary, so the mode filter alone leaves exactly the firing HOOK rows
    // governing this path. A row that reddened is simply absent — silently, because
    // refusal on a red row is the door's line, never a reaction's.
    let pages = DiskPages::new(root);
    let verdict = artifact.verify_at(&change.doc.path, &pages);
    let live: Vec<&ArmedRow> = verdict
        .firing()
        .iter()
        .filter(|row| row.mode() == Mode::Armed)
        .collect();
    if live.is_empty() {
        return Ok(Vec::new());
    }

    let mut hooks: Vec<(String, policy::Hook)> = Vec::with_capacity(live.len());
    for row in live {
        // `verify` already read this page through the same cache, so a miss here is
        // the page vanishing mid-write. Silence is the ruled answer for a hook.
        let Ok(declaration) = pages.read(row.page()) else {
            continue;
        };
        let hook =
            policy::load_hook(&declaration, policy::CheckLimits::default()).map_err(|error| {
                FeedError::Declaration {
                    id: row.id().as_str().to_string(),
                    page: row.page().to_string(),
                    error,
                }
            })?;
        hooks.push((row.id().as_str().to_string(), hook));
    }

    let event = policy::derive_event(&change, &before.root.node_rev.0, &after.root.node_rev.0, 0);
    let mut outcomes =
        policy::evaluate_loaded_hooks(hooks.iter().map(|(id, hook)| (id.as_str(), hook)), &event)
            .map_err(FeedError::Hook)?;

    outcomes.retain(|outcome| {
        !outcome.intents.is_empty() || !outcome.narrowed.is_empty() || !outcome.findings.is_empty()
    });
    Ok(outcomes.into_iter().map(project_outcome).collect())
}

/// Read the attested armed-set artifact.
///
/// `Ok(None)` is the never-armed workspace — the artifact was never written. Any
/// OTHER read failure is a fault, not an empty armed set: an artifact that exists
/// and cannot be read must never be mistaken for one that was never created.
fn read_artifact(root: &fs::WorkspaceRoot) -> Result<Option<String>, FeedError> {
    match std::fs::read_to_string(root.0.join(policy::armed::ARMED_RULES_PATH)) {
        Ok(page) => Ok(Some(page)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(FeedError::Artifact(policy::armed::ArtifactCorrupt {
            detail: format!(
                "`{path}` exists but could not be read: {e}",
                path = policy::armed::ARMED_RULES_PATH
            ),
        })),
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
        let temp = tempfile::tempdir().expect("temp workspace");
        write_page(temp.path(), page_path, page_body);

        let index = RuleIndex::discover([PageRef {
            layer: ScopeLayer::Workspace,
            page: page_path,
            bytes: page_body,
        }]);
        let artifact = policy::armed::arm(
            &index,
            &ArmRoot::parse(arm_root).expect("a legal arm root"),
            [ArmRequest {
                id: RuleId::parse(HOOK_ID).expect("a legal id"),
                mode,
                attested_rev: page_rev(page_body),
            }],
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
    ) -> Result<Vec<wire::EffectEnvelope>, FeedError> {
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

    #[test]
    fn landed_status_change_emits_canonical_armed_intent() {
        let (_temp, root) = armed_root();
        let outcomes = feed(&root, "tasks/x.md", "in-progress", "review").expect("evaluates");

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
        assert!(
            feed(&root, "tasks/x.md", "in-progress", "review")
                .expect("never-armed is a no-op")
                .is_empty()
        );
    }

    #[test]
    fn an_out_of_scope_path_emits_nothing() {
        let (_temp, root) = armed_root();
        assert!(
            feed(&root, "notes/x.md", "in-progress", "review")
                .expect("out of scope is silent")
                .is_empty()
        );
    }

    #[test]
    fn an_in_scope_change_that_arms_no_intent_is_empty() {
        let (_temp, root) = armed_root();
        assert!(
            feed(&root, "tasks/x.md", "todo", "in-progress")
                .expect("evaluates")
                .is_empty()
        );
    }

    #[test]
    fn the_feeder_never_reads_the_changed_document_from_disk() {
        let (_temp, root) = armed_root();
        assert!(!root.0.join("tasks/x.md").exists());
        assert_eq!(
            feed(&root, "tasks/x.md", "in-progress", "review")
                .expect("held states are sufficient")
                .len(),
            1
        );
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
            feed(&root, "tasks/x.md", "in-progress", "review")
                .expect("evaluates")
                .is_empty(),
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

        assert!(
            feed(&root, "tasks/x.md", "in-progress", "review")
                .expect("a drifted HOOK falls silent — it never refuses")
                .is_empty(),
            "the row reddened, so it does not fire on the new bytes"
        );
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
            feed(&root, "sessions/s1/task.md", "in-progress", "review")
                .expect("evaluates")
                .len(),
            1,
            "the write inside the arm root fires"
        );
        assert!(
            feed(&root, "sessions/s2/task.md", "in-progress", "review")
                .expect("evaluates")
                .is_empty(),
            "sibling subtrees never interact — the arm root does not contain this path"
        );
    }

    #[test]
    fn a_corrupt_artifact_is_a_fault_never_an_empty_armed_set() {
        let (temp, root) = armed_root();
        let artifact = temp.path().join(policy::armed::ARMED_RULES_PATH);
        let truncated = std::fs::read_to_string(&artifact)
            .expect("read artifact")
            .replace("| id | page | rev | scope | mode |", "| id | page |");
        std::fs::write(&artifact, truncated).expect("corrupt the artifact");

        let error = feed(&root, "tasks/x.md", "in-progress", "review")
            .expect_err("a corrupt artifact never reads as nothing armed");
        assert!(matches!(error, FeedError::Artifact(_)), "{error:?}");
    }

    #[test]
    fn an_armed_page_that_does_not_declare_a_hook_is_reported_by_name() {
        // Registration and declaration are two layers: this page registers by tag
        // and arms, but carries no HOOK declaration for C1's loader to read.
        let bare = format!("---\ntags: [type/rule, rules/hook]\nid: {HOOK_ID}\n---\n\n# rule\n");
        let (_temp, root) = armed_workspace(&bare, Mode::Armed, ".", "bare.md");

        let error =
            feed(&root, "tasks/x.md", "in-progress", "review").expect_err("the page cannot load");
        let FeedError::Declaration { id, page, .. } = &error else {
            panic!("expected a declaration fault, got {error:?}");
        };
        assert_eq!(id, HOOK_ID);
        assert_eq!(page, "bare.md");
    }

    #[test]
    fn a_loaded_hook_eval_fault_reaches_the_unprojected_feeder_boundary() {
        let faulting = hook_page("\"tasks/*.md\"").replace(
            "    for delta in event.changes:\n",
            "    ignored = event.actor\n    for delta in event.changes:\n",
        );
        let (_temp, root) = armed_workspace(&faulting, Mode::Armed, ".", "task-review-notify.md");

        let error = feed(&root, "tasks/x.md", "in-progress", "review")
            .expect_err("event.actor passes the load lint and faults at evaluation");
        assert!(matches!(error, FeedError::Hook(_)), "{error:?}");
    }
}
