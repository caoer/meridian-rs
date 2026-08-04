//! The armed-plane gate mount (U4.2) — the trusted write path's I/O half.
//!
//! [`policy::gate`] is the pure decision; this module is the disk seam that feeds
//! it. It reads the workspace's OWN once-armed marker and attested armed-rules
//! artifact through [`crate::armed_disk`] — never a caller-supplied set — resolves
//! the armed law AT THE WRITE'S PATH, evaluates the change, and maps the outcome
//! onto this crate's needs: advisory verdicts to merge into the response, or a
//! wire refusal that the choke-point returns BEFORE bytes land.
//!
//! It is mounted at the ONE production verdict site in [`crate::write`]
//! (splice/create/remove), so both writer paths (the per-workspace sidecar and the
//! resident registry daemon) gate through it — and, via the same
//! [`policy::gate`] pair, the run plane (`crates/run`) gates the bytes it lands
//! too. On a never-armed workspace this is a bit-for-bit no-op: the law reports
//! never-armed and the gate adds no verdict and never refuses.
//!
//! # The law is resolved per WRITE PATH
//! An arm is rooted, so "what governs this write" is a question about the write's
//! own path. The door asks it once, at the target path, and hands the answer to
//! the pure decision — it never resolves a workspace-wide set and filters after.

use wire::{ErrorBody, ErrorCode, NodeRev, Path, Severity, Span, Verdict};

use crate::armed_disk;

/// A passing gate's output: the advisory verdicts to merge into the response.
///
/// A `--force`-escaped skip used to leave this struct by a second door — a
/// `forced_skips` list the mount journaled as permanent `op=force` rows. The
/// journal is gone (ZT 2026-08-02), and P15 puts the whole record on the rendered
/// surface instead: [`findings_to_verdicts`] marks a bypassed rule `forced:`, so
/// the verdicts below carry it and there is nothing left for a second channel.
#[derive(Debug, Default)]
pub struct GatePass {
    /// The §11.1 advisory verdicts (warn findings, surviving armed-law faults, and
    /// the `forced:`-marked renders of any `--force`-escaped skip).
    pub verdicts: Vec<Verdict>,
}

/// Build the `rulepack-api@2` change surface for one write and gate it — the ONE
/// call the write choke-point makes at each of its three verdict sites (splice,
/// create, remove). `before`/`after` are the pre/post states; `edits` the model
/// edits (empty for a whole-file birth/death); `op` the change op; `force` the
/// U4.3 `--force` bypass; `subject_doc` the state whose path + root rev the
/// advisory verdicts carry (the `after` for a splice/create, the `before` for a
/// remove). U4.2 gates the change + states, so declared-evidence edges are empty
/// (`no_edges`).
///
/// # Errors
/// A `convention_fault` / `armed_drift` / `binding_break` / `index_integrity`
/// refusal — the write must not land.
#[allow(clippy::too_many_arguments)]
pub fn gate_write(
    root: &fs::WorkspaceRoot,
    before: &model::Document,
    after: &model::Document,
    edits: &[model::Edit],
    op: policy::ChangeOp,
    actor: Option<&str>,
    force: bool,
    subject_doc: &model::Document,
) -> Result<GatePass, Box<ErrorBody>> {
    let change = policy::derive_change(
        before,
        after,
        edits,
        policy::Invocation { op, actor, force },
        &[],
        &no_edges,
    );
    enforce_gate(root, &change, subject_doc)
}

/// The path a write targets — the after state's, falling back to the before
/// state's for a remove. It is the path the armed law is resolved at, so it must
/// be the same one [`policy::gate`] classifies the door law against.
fn target_path(change: &policy::Change) -> &str {
    if change.doc.path.is_empty() {
        change.before.path.as_str()
    } else {
        change.doc.path.as_str()
    }
}

/// Gate one already-built change through the workspace's armed law, after CAS and
/// before bytes land. Returns the advisory verdicts + forced skips to merge into
/// the response on a pass/no-op, or a wire refusal the caller returns instead of
/// committing. `subject_doc` supplies the path + root rev the advisory verdicts
/// carry.
///
/// # Errors
/// A `convention_fault` / `armed_drift` / `binding_break` / `index_integrity`
/// refusal — the write must not land.
pub fn enforce_gate(
    root: &fs::WorkspaceRoot,
    change: &policy::Change,
    subject_doc: &model::Document,
) -> Result<GatePass, Box<ErrorBody>> {
    let law = armed_disk::resolve_at(root, target_path(change));
    match policy::gate(change, &law) {
        policy::GateOutcome::Ok(findings) => Ok(GatePass {
            verdicts: findings_to_verdicts(findings, subject_doc),
        }),
        policy::GateOutcome::Refusal(refusal) => Err(gate_refusal_to_wire(refusal)),
    }
}

/// Project the armed findings onto wire verdicts (§11.1 advisory shape).
/// Coordinates come from the subject document (path + root rev); a rule finding is
/// document-grained, so it carries the whole-file rev and a zero span.
///
/// **P15 — a forced bypass NAMES the plane it bypassed, here.** A `--force`-escaped
/// skip used to be recorded twice: once as an ordinary advisory verdict and once as
/// a permanent `op=force` journal row carrying `forced_rule=`. The journal is gone
/// (ZT 2026-08-02, remove-no-replacement), so this rendered surface is the ONLY
/// place the bypass can be stated — and an unmarked verdict would render a bypassed
/// rule identically to one that merely warned. The `forced:` prefix is that mark:
/// `force` stays one word at the door (ZT's "fingerprint-or-force"), and what it
/// escaped is legible in the response it escaped through.
fn findings_to_verdicts(findings: Vec<policy::GateFinding>, doc: &model::Document) -> Vec<Verdict> {
    if findings.is_empty() {
        return Vec::new();
    }
    let path = doc_path(doc);
    let node_rev = NodeRev(doc.root.node_rev.0.clone());
    findings
        .into_iter()
        .map(|f| Verdict {
            severity: Severity::Warn,
            path: Path(path.clone()),
            hpath: None,
            span: Span(0, 0),
            node_rev: node_rev.clone(),
            message: if f.forced {
                format!(
                    "forced: bypassed {} — {} (legal path: {})",
                    f.rule, f.message, f.passing_scenario
                )
            } else {
                format!("{} (legal path: {})", f.message, f.passing_scenario)
            },
            rule: f.rule,
        })
        .collect()
}

/// Map a [`policy::GateRefusal`] to its wire `{code, recovery}` envelope, drawn
/// from the closed §8 taxonomy (refusal-amendment).
///
/// # The envelope is chosen from the fault; the WORDS are never re-written
/// [`policy::ArmedFault`] is the one artifact-fault surface and its `Display` is
/// the one renderer. This seam picks which wire code carries it — a DRIFTED row
/// mints `armed_drift` (recovery: refresh) and fills the `expected`/`actual` rev
/// slots, because that is the fault an operator resolves by re-arming; every
/// other armed-law fault mints `convention_fault` (recovery: env). Choosing a
/// channel is this host's job. Choosing words would be a second vocabulary.
#[must_use]
pub fn gate_refusal_to_wire(refusal: policy::GateRefusal) -> Box<ErrorBody> {
    match refusal {
        policy::GateRefusal::Blocked { violations } => {
            let mut e = ErrorBody::new(ErrorCode::ConventionFault);
            e.path = Some(Path(fs::domain::ARMED_RULES_PATH.to_string()));
            e.message = Some(render_violations(&violations));
            Box::new(e)
        }
        policy::GateRefusal::ArmedLawFault { faults } => armed_law_fault_to_wire(&faults),
        // U4.3 taxonomy row 9 — a one-sided artifact↔page change stopped at the
        // door. `path` echoes the engine-managed file; `message` names the side
        // and the legal path (the `side` is prefixed so it reads on the wire).
        policy::GateRefusal::BindingBreak {
            side,
            path,
            teaching,
            legal_path,
        } => {
            let mut e = ErrorBody::new(ErrorCode::BindingBreak);
            e.path = Some(Path(path));
            e.message = Some(format!(
                "binding-break[side={}]: {teaching} (legal path: {legal_path})",
                side.as_str()
            ));
            Box::new(e)
        }
        // U4.3 taxonomy row 10 — the integrity floor. `path` is the protected file
        // (the artifact or the marker) the write tried to remove.
        policy::GateRefusal::IndexIntegrity { target, teaching } => {
            let mut e = ErrorBody::new(ErrorCode::IndexIntegrity);
            e.path = Some(Path(target));
            e.message = Some(teaching);
            Box::new(e)
        }
    }
}

/// Envelope every refusing armed-law fault. The message renders them ALL — they
/// are one condition (the law at this path is not enforceable), and reporting one
/// would send the operator round the loop once per fault.
fn armed_law_fault_to_wire(faults: &[policy::ArmedFault]) -> Box<ErrorBody> {
    let message = faults
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");

    // A drifted row is the one fault with a rev PAIR to report, and `refresh` is
    // the recovery that fits it. Any other fault riding along keeps its words in
    // the message; the code follows the fault an operator can act on first.
    if let Some((armed_rev, report_rev)) = faults.iter().find_map(drift_revs) {
        let mut e = ErrorBody::new(ErrorCode::ArmedDrift);
        e.expected = Some(NodeRev(armed_rev));
        e.actual = Some(NodeRev(report_rev));
        e.message = Some(message);
        return Box::new(e);
    }
    let mut e = ErrorBody::new(ErrorCode::ConventionFault);
    e.path = Some(Path(fs::domain::ARMED_RULES_PATH.to_string()));
    e.message = Some(message);
    Box::new(e)
}

/// The `(attested rev, live rev)` pair a drifted row carries, or `None`.
fn drift_revs(fault: &policy::ArmedFault) -> Option<(String, String)> {
    let policy::ArmedFault::Red(red) = fault else {
        return None;
    };
    let policy::armed::Redness::Drifted { report_rev } = red.why() else {
        return None;
    };
    Some((red.row().rev().to_string(), report_rev.clone()))
}

/// Render stacked block violations into one teaching message that names each rule
/// and cites its passing case (the legal path).
fn render_violations(violations: &[policy::GateViolation]) -> String {
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
    format!(
        "armed change refused by {} rule(s): {body}",
        violations.len()
    )
}

/// The document's own path (the `NodeKind::Document` stamp), or empty.
fn doc_path(doc: &model::Document) -> String {
    if let model::NodeKind::Document { path, .. } = &doc.root.kind {
        path.clone()
    } else {
        String::new()
    }
}

/// Build a path-stamped empty document — the absent BEFORE state of a `create`
/// or absent AFTER state of a `remove`, so the change surface still carries the
/// file's path for scoping.
#[must_use]
pub fn absent_doc(path: &Path) -> model::Document {
    let mut doc = model::build(String::new(), syntax::parse(""));
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        p.clone_from(&path.0);
    }
    doc
}

/// A no-op resolver for declared-evidence edges: U4.2 gates the change + states,
/// not the one-hop pinned evidence (that resolver is a later refinement). The
/// door reads no evidence hop, so an armed rule that reads `change.edges` sees
/// them fail-closed (drifted, no facts) — never a silent pass on evidence.
#[must_use]
pub fn no_edges(_reference: &str) -> Option<(String, model::Document)> {
    None
}

#[cfg(test)]
mod scenarios {
    //! The U4.2/U4.3 merge-gate scenarios, exercised through the REAL write path
    //! (`crate::write::{create, splice, remove}`) on an on-disk workspace. Each
    //! scenario arms a rule PAGE in the workspace's own artifact and proves the
    //! gate's decision lands at the wire seam, not just in `policy`.

    use std::path::Path as FsPath;

    use policy::armed::Mode;
    use wire::{Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path, PutAt, Recovery, SecRef};

    use crate::write::{CreateArgs, RemoveArgs, SpliceArgs, create, remove, splice};

    /// The rule page every scenario arms: refuses when `actor == owner`, scoped to
    /// `tasks/**`. It registers by TAG and carries NO `kind:` key — the tag is the
    /// one name (ruling § 1).
    const RULE_PAGE: &str = "---\ntags: [type/rule, rules/check]\nid: reviewer.not-owner\n\
        paths:\n  - tasks/**\n---\n\n# reviewer.not-owner\n\n\
        ```starlark\ndef check_change(change):\n    owner = change.doc.frontmatter.get(\"owner\")\n    \
        actor = change.actor\n    if actor != None and owner != None and actor == owner:\n        \
        refuse(message = \"reviewer must not be the owner: \" + actor + \" cannot close their own task\", \
        passing = \"reviewer-not-owner.md#reviewer-close\")\n```\n";

    /// Where the scenarios keep that page. Nothing depends on the location — that
    /// is the point of tag registration — so an ordinary content directory is used
    /// rather than a reserved-looking one.
    const RULE_PATH: &str = "rules/reviewer-not-owner.md";

    /// A page that REGISTERS (so it can be armed) but declares no evaluable law —
    /// the "arms but will not load" shape.
    const BARE: &str = "---\ntags: [type/rule, rules/check]\nid: bare.check\n---\n\n# bare\n";

    /// A loadable rule page that is never armed — the control for the page-side
    /// binding break.
    const DRAFT: &str = "---\ntags: [type/rule, rules/check]\nid: draft.check\n\
        paths:\n  - tasks/**\n---\n\n# draft\n\n\
        ```starlark\ndef check_change(change):\n    pass\n```\n";

    fn tmp_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    fn write_page(root: &fs::WorkspaceRoot, path: &str, bytes: &str) {
        let full = root.0.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }

    /// Render the attested artifact arming every `(page, bytes, id, mode)` at the
    /// workspace root, pinned to each page's live rev.
    fn artifact_for(pages: &[(&str, &str, &str, Mode)]) -> String {
        let index =
            policy::RuleIndex::discover(pages.iter().map(|(path, bytes, ..)| policy::PageRef {
                layer: policy::ScopeLayer::Workspace,
                page: path,
                bytes,
            }));
        policy::armed::arm(
            &index,
            &policy::armed::ArmRoot::workspace(),
            pages
                .iter()
                .map(|(_, bytes, id, mode)| policy::armed::ArmRequest {
                    id: policy::RuleId::parse(id).expect("a legal id"),
                    mode: *mode,
                    attested_rev: policy::page_rev(bytes),
                }),
        )
        .expect("the fixture arms")
        .render()
    }

    fn write_artifact(root: &fs::WorkspaceRoot, body: &str) {
        write_page(root, fs::domain::ARMED_RULES_PATH, body);
    }

    /// Stamp the once-armed marker: the workspace HAS been armed.
    ///
    /// Every fixture that arms writes BOTH files. The artifact alone leaves a
    /// workspace the marker says was never armed (so the gate is off); the marker
    /// alone is `ArmedFault::Missing`. Arming is one act over two files, and a
    /// fixture that writes one of them is not testing an armed workspace.
    fn set_marker(root: &fs::WorkspaceRoot) {
        write_page(root, fs::domain::ATTESTED_MARKER_PATH, "");
    }

    /// Arm `RULE_PAGE` at `mode`: the page on disk, the artifact pinning it, and
    /// the marker.
    fn armed_ws(mode: Mode) -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let (dir, root) = tmp_ws();
        write_page(&root, RULE_PATH, RULE_PAGE);
        write_artifact(
            &root,
            &artifact_for(&[(RULE_PATH, RULE_PAGE, "reviewer.not-owner", mode)]),
        );
        set_marker(&root);
        (dir, root)
    }

    fn armed_block_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        armed_ws(Mode::Block)
    }

    /// A create that owner-self-closes a task (actor == owner) — fires the rule.
    fn owner_self_create(actor: &str) -> CreateArgs {
        CreateArgs {
            id: None,
            path: Path("tasks/newtask.md".into()),
            body: format!("---\nowner: {actor}\nstatus: closed\n---\n# Task\n\nbody\n"),
            actor: Some(actor.into()),
            now: None,
            if_root: None,
            dry: false,
        }
    }

    // ── Scenario 1 — empty caller set while the workspace is armed → REFUSES ────
    #[test]
    fn s1_empty_caller_set_still_refuses_from_the_workspace_artifact() {
        let (dir, root) = armed_block_ws();
        // The caller hands the EMPTY set (`&[]`) — it cannot weaken the decision;
        // the gate reads the workspace's OWN attested artifact.
        let err = create(&root, None, &owner_self_create("agent:alice"), &[])
            .expect_err("owner self-close on an armed block workspace must refuse");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert_eq!(err.recovery, Recovery::Env);
        assert!(
            !dir.path().join("tasks/newtask.md").exists(),
            "a refused create writes no file"
        );
    }

    // ── Scenario 5 — the refusal NAMES the rule ID + cites the passing case ─────
    #[test]
    fn s5_refusal_names_the_rule_id_and_cites_the_passing_case() {
        let (_dir, root) = armed_block_ws();
        let err = create(&root, None, &owner_self_create("agent:alice"), &[]).unwrap_err();
        let msg = err.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("reviewer.not-owner"),
            "the refusal names the rule by ID, not by a folder slug: {msg}"
        );
        assert!(
            msg.contains("reviewer-not-owner.md#reviewer-close"),
            "cites the passing case: {msg}"
        );
        assert_eq!(
            err.path.as_ref().map(|p| p.0.as_str()),
            Some(fs::domain::ARMED_RULES_PATH),
            "the refusal points at the artifact"
        );
    }

    /// The reviewer (actor ≠ owner) close LANDS — the legal path.
    #[test]
    fn reviewer_close_lands_on_armed_block() {
        let (dir, root) = armed_block_ws();
        let mut args = owner_self_create("agent:bob");
        args.body = "---\nowner: agent:alice\nstatus: closed\n---\n# Task\n\nbody\n".into();
        create(&root, None, &args, &[]).expect("a reviewer distinct from the owner may close");
        assert!(
            dir.path().join("tasks/newtask.md").exists(),
            "the write landed"
        );
    }

    // ── Scenario 3 — never-armed → byte-identical no-op ─────────────────────────
    #[test]
    fn s3_never_armed_is_byte_identical_no_op() {
        // Control: a workspace with no rules at all.
        let (ctrl_dir, ctrl) = tmp_ws();
        create(&ctrl, None, &owner_self_create("agent:alice"), &[]).expect("control create lands");
        let control_bytes = std::fs::read(ctrl_dir.path().join("tasks/newtask.md")).unwrap();

        // Subject: an armed BLOCK rule page AND its artifact on disk, but NO marker
        // → never-armed → the gate is off. The would-fire owner self-close lands,
        // byte-for-byte identical to the ungated control. A stray artifact cannot
        // arm a workspace; only an attested arm, which sets the marker, can.
        let (dir, root) = tmp_ws();
        write_page(&root, RULE_PATH, RULE_PAGE);
        write_artifact(
            &root,
            &artifact_for(&[(RULE_PATH, RULE_PAGE, "reviewer.not-owner", Mode::Block)]),
        );
        // (no set_marker — never armed)
        let out = create(&root, None, &owner_self_create("agent:alice"), &[])
            .expect("never-armed: the gate is a no-op, the write lands");
        assert!(out.verdicts.is_empty(), "never-armed adds no verdict");
        assert_eq!(
            std::fs::read(dir.path().join("tasks/newtask.md")).unwrap(),
            control_bytes,
            "byte-identical to the ungated write"
        );
    }

    // ── the SPLICE mount fires too (site parity) ───────────────────────────────
    #[test]
    fn splice_mount_refuses_owner_self_close() {
        let (dir, root) = armed_block_ws();
        write_page(
            &root,
            "tasks/fix.md",
            "---\nowner: agent:alice\nstatus: open\n---\n# Fix\n\nbody\n",
        );
        let before = std::fs::read(dir.path().join("tasks/fix.md")).unwrap();
        let err = splice(
            &root,
            None,
            &content_splice("tasks/fix.md", "agent:alice", false),
            &[],
            None,
        )
        .expect_err("owner self-splice refuses");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert_eq!(
            std::fs::read(dir.path().join("tasks/fix.md")).unwrap(),
            before,
            "a refused splice lands no bytes"
        );
    }

    /// A DRY run of a refused write still refuses — a rehearsal of a forbidden
    /// write is forbidden (the gate runs before the dry short-circuit).
    #[test]
    fn dry_refused_write_still_refuses() {
        let (_dir, root) = armed_block_ws();
        let mut args = owner_self_create("agent:alice");
        args.dry = true;
        let err =
            create(&root, None, &args, &[]).expect_err("a dry rehearsal of a refusal refuses");
        assert_eq!(err.code, ErrorCode::ConventionFault);
    }

    /// A workspace-relative reserved-path sanity check: the marker path is
    /// non-md, so it never enters the hash domain (no root perturbation).
    #[test]
    fn marker_is_outside_the_hash_domain() {
        assert!(
            !fs::domain::Domain::new().contains(FsPath::new(fs::domain::ATTESTED_MARKER_PATH)),
            "the once-armed sentinel is never hashed"
        );
    }

    // ═══ THE THREE FAULT SHAPES, THROUGH THE ONE SURFACE ═════════════════════
    //
    // These arrived as three separate defect reports and would have minted three
    // vocabularies if fixed separately. They are ONE condition — a workspace that
    // HAS been armed cannot honour its law — reported by ONE surface
    // (`policy::ArmedFault`), and each test below reaches that surface through the
    // real write door rather than through the resolver directly.

    /// **Shape 1 — the row-deletion disarm.** The deletion LOOKS legitimate: a
    /// well-formed artifact page with the byte-exact header and ZERO data rows,
    /// which is exactly what deleting every row leaves behind. Before the
    /// once-armed pivot this parsed clean and armed nothing — a silent TOTAL
    /// disarm performed with an ordinary edit.
    #[test]
    fn fault_shape_row_deletion_disarm_refuses_instead_of_disarming() {
        let (dir, root) = armed_block_ws();
        let emptied = policy::armed::ArmedArtifact::default().render();
        policy::armed::parse_artifact(&emptied)
            .expect("the deletion is well-formed, not corrupt — that is what makes it dangerous");
        write_artifact(&root, &emptied);

        // A write that the armed rule would have PASSED still refuses: the law is
        // not "nothing", it is unreadable, and unreadable fails closed.
        let mut args = owner_self_create("agent:bob");
        args.body = "---\nowner: agent:alice\n---\n# Task\n\nbody\n".into();
        let err =
            create(&root, None, &args, &[]).expect_err("zero rows on an armed workspace refuses");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert!(
            err.message
                .as_deref()
                .unwrap_or("")
                .contains("spelled `off`"),
            "the teaching names the LEGITIMATE way to disarm: {:?}",
            err.message
        );
        assert!(!dir.path().join("tasks/newtask.md").exists());
    }

    /// **Shape 2 — a corrupt artifact on an armed workspace.** A corrupt page
    /// never reads as "nothing armed"; that would be a gate-disabling edit dressed
    /// up as a parse failure.
    #[test]
    fn fault_shape_corrupt_artifact_fails_closed_naming_it() {
        let (dir, root) = armed_block_ws();
        write_artifact(&root, "not an artifact at all\n| forged | row |\n");
        let mut args = owner_self_create("agent:bob");
        args.body = "---\nowner: agent:alice\n---\n# Task\n\nbody\n".into();
        let err = create(&root, None, &args, &[]).expect_err("a corrupt artifact fails closed");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert!(
            err.message.as_deref().unwrap_or("").contains("corrupt"),
            "names the corruption: {:?}",
            err.message
        );
        assert!(!dir.path().join("tasks/newtask.md").exists());
    }

    /// **Shape 3 — one faulting row must not silence the rules beside it.** The
    /// defect propagated the first fault out of the per-row loop, so ONE armed page
    /// missing its declaration disarmed every other rule at the same path. Here the
    /// blocking rule still governs while the broken row reports itself.
    #[test]
    fn fault_shape_one_bad_row_does_not_silence_the_rules_beside_it() {
        let (dir, root) = tmp_ws();
        write_page(&root, RULE_PATH, RULE_PAGE);
        write_page(&root, "rules/bare.md", BARE);
        write_artifact(
            &root,
            &artifact_for(&[
                (RULE_PATH, RULE_PAGE, "reviewer.not-owner", Mode::Block),
                ("rules/bare.md", BARE, "bare.check", Mode::Block),
            ]),
        );
        set_marker(&root);

        let err = create(&root, None, &owner_self_create("agent:alice"), &[])
            .expect_err("the workspace refuses — but the question is WHY");
        let msg = err.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("bare.check"),
            "the broken row reports itself BY NAME: {msg}"
        );
        assert!(!dir.path().join("tasks/newtask.md").exists());
    }

    /// The other half of shape 1's law, and why `Disarmed` is not merely "no armed
    /// rows": an attested-`off` row IS an attestation. A workspace that deliberately
    /// enforces nothing gates nothing and faults not at all — otherwise the
    /// fail-closed pivot would make deliberate disarming impossible.
    #[test]
    fn rows_attested_off_are_a_legitimate_disarm_and_never_a_fault() {
        let (dir, root) = armed_ws(Mode::Off);
        create(&root, None, &owner_self_create("agent:alice"), &[])
            .expect("an attested-off workspace enforces nothing and faults not at all");
        assert!(dir.path().join("tasks/newtask.md").exists());
    }

    // ═══ U4.3 named merge-gate scenarios (real write path) ═══════════════════

    /// A splice that hand-injects a forged armed row into the ARTIFACT — the
    /// artifact-side binding break (the checkbox flip's successor). `force` toggles
    /// the sanctioned bypass.
    fn forged_row(force: bool) -> SpliceArgs {
        SpliceArgs {
            id: None,
            origin: crate::guard::Origin::InProcess,
            path: Path(fs::domain::ARMED_RULES_PATH.into()),
            actor: Some("agent:mallory".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force,
            edits: vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "Attested armed rules".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: PutAt::End,
                    text: "\n| `forged.check` | rules/forged.md | `deadbeefdeadbeef` | . | block |"
                        .into(),
                },
                if_node_rev: None,
            }],
            plan_edits: Vec::new(),
            pin: None,
        }
    }

    /// A content splice at `path` by `actor` — the ordinary write the door judges.
    fn content_splice(path: &str, actor: &str, force: bool) -> SpliceArgs {
        SpliceArgs {
            id: None,
            origin: crate::guard::Origin::InProcess,
            path: Path(path.into()),
            actor: Some(actor.into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force,
            edits: vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "Fix".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: PutAt::Content,
                    text: "done\n".into(),
                },
                if_node_rev: None,
            }],
            plan_edits: Vec::new(),
            pin: None,
        }
    }

    // ── Scenario 1 — a hand-edited artifact row → binding refusal (teaching) ────
    #[test]
    fn u43_s1_hand_edited_artifact_row_refuses() {
        let (dir, root) = armed_block_ws();
        let artifact_path = dir.path().join(fs::domain::ARMED_RULES_PATH);
        let before = std::fs::read_to_string(&artifact_path).unwrap();

        let err = splice(&root, None, &forged_row(false), &[], None)
            .expect_err("a direct artifact edit must break the artifact↔page binding");
        assert_eq!(err.code, ErrorCode::BindingBreak);
        assert_eq!(err.recovery, Recovery::Fix, "binding-break recovers by fix");
        let msg = err.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("binding-break[side=index]"),
            "the refusal names the artifact side: {msg}"
        );
        assert!(
            msg.contains("attest/arm path") && msg.contains("--force"),
            "the teaching refusal names the legal paths: {msg}"
        );
        assert_eq!(
            err.path.as_ref().map(|p| p.0.as_str()),
            Some(fs::domain::ARMED_RULES_PATH),
            "the refusal points at the artifact"
        );
        assert_eq!(
            std::fs::read_to_string(&artifact_path).unwrap(),
            before,
            "a refused artifact edit lands no bytes (no row forged)"
        );
    }

    /// **The page side, re-keyed.** The folder generation knew an armed law by its
    /// path shape (`conventions/<slug>/CHECK.md`). A rule page has no reserved
    /// shape — it is an ordinary page wherever its author put it — so the binding
    /// asks MEMBERSHIP in what the workspace attested. Editing an armed page in
    /// place still drifts the pinned rev under the artifact that arms it.
    #[test]
    fn u43_page_side_break_is_membership_not_a_path_shape() {
        let (dir, root) = armed_block_ws();
        let before = std::fs::read_to_string(dir.path().join(RULE_PATH)).unwrap();
        let args = SpliceArgs {
            edits: vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "reviewer.not-owner".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: PutAt::End,
                    text: "\nedited in place\n".into(),
                },
                if_node_rev: None,
            }],
            ..content_splice(RULE_PATH, "agent:mallory", false)
        };
        let err = splice(&root, None, &args, &[], None)
            .expect_err("editing an ARMED page in place must break the binding");
        assert_eq!(err.code, ErrorCode::BindingBreak);
        assert!(
            err.message
                .as_deref()
                .unwrap_or("")
                .contains("binding-break[side=file]"),
            "the refusal names the page side: {:?}",
            err.message
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(RULE_PATH)).unwrap(),
            before,
            "a refused page edit lands no bytes"
        );
    }

    /// An UNARMED rule page is freely editable — only ARMED pages are bound. This
    /// is the control that keeps the page-side break from degenerating into "no
    /// rule page may ever be edited", which would make authoring impossible.
    #[test]
    fn u43_an_unarmed_rule_page_is_freely_editable() {
        let (dir, root) = armed_block_ws();
        write_page(&root, "rules/draft.md", DRAFT);
        let args = SpliceArgs {
            edits: vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "draft".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: PutAt::End,
                    text: "\nstill drafting\n".into(),
                },
                if_node_rev: None,
            }],
            ..content_splice("rules/draft.md", "agent:alice", false)
        };
        splice(&root, None, &args, &[], None).expect("an unarmed rule page is ordinary content");
        assert!(
            std::fs::read_to_string(dir.path().join("rules/draft.md"))
                .unwrap()
                .contains("still drafting"),
            "the edit landed"
        );
    }

    // ── Scenario 2 — forced write → the skip is RENDERED (P15) ─────────────────
    /// A `--force`-escaped skip used to be recorded twice: a permanent `op=force`
    /// journal row AND a verdict. The journal is gone (ZT 2026-08-02), so the
    /// rendered verdict is the ONLY record — and P15 requires it to NAME the plane
    /// the force bypassed, which an unmarked warn verdict does not.
    #[test]
    fn u43_s2_forced_write_renders_the_skip() {
        let (dir, root) = armed_block_ws();
        write_page(
            &root,
            "tasks/fix.md",
            "---\nowner: agent:alice\nstatus: open\n---\n# Fix\n\nbody\n",
        );
        // The owner self-closes (actor == owner) — the armed rule refuses it.
        // `--force` escapes: the write lands and the skip is rendered.
        let out = splice(
            &root,
            None,
            &content_splice("tasks/fix.md", "agent:alice", true),
            &[],
            None,
        )
        .expect("--force escapes the armed refusal");

        // RENDERED: a forced verdict rides the response naming the skip.
        let wire::ResponseBody::Splice { verdicts, .. } = out.body else {
            panic!("splice returns a Splice body");
        };
        assert!(
            verdicts.iter().any(|v| v.message.contains("FORCED")),
            "the forced skip renders as a visible verdict row: {verdicts:?}"
        );

        // The write LANDED (the bypass is real, not a silent refusal).
        assert!(
            std::fs::read_to_string(dir.path().join("tasks/fix.md"))
                .unwrap()
                .contains("done"),
            "a forced write lands its bytes"
        );

        // P15: the verdict NAMES the bypassed rule — the record the force-row
        // used to carry. Without the `forced:` mark a bypassed rule would render
        // identically to one that merely warned, and nothing else records it now.
        let forced: Vec<&wire::Verdict> = verdicts
            .iter()
            .filter(|v| v.message.starts_with("forced: bypassed "))
            .collect();
        assert_eq!(
            forced.len(),
            1,
            "exactly one forced-skip verdict: {verdicts:?}"
        );
        assert!(
            forced[0].message.contains(&forced[0].rule),
            "the forced verdict names the rule it bypassed: {:?}",
            forced[0]
        );
    }

    // ── Scenario 4 — artifact-remove + marker-remove → the integrity floor ──────
    /// The current whole-file rev of `path` — the remove-what-you-read CAS the
    /// remove op checks BEFORE the gate, so the test must pass the live rev.
    fn live_rev(root: &fs::WorkspaceRoot, path: &str) -> NodeRev {
        let doc = fs::load(root, FsPath::new(path)).expect("loads");
        NodeRev(doc.root.node_rev.0.clone())
    }

    fn remove_args(path: &str, rev: NodeRev) -> RemoveArgs {
        RemoveArgs {
            id: None,
            path: Path(path.into()),
            if_file_rev: rev,
            actor: Some("agent:mallory".into()),
            now: None,
            if_root: None,
            dry: false,
        }
    }

    #[test]
    fn u43_s4_artifact_remove_refuses_citing_the_floor() {
        let (dir, root) = armed_block_ws();
        let rev = live_rev(&root, fs::domain::ARMED_RULES_PATH);
        let err = remove(
            &root,
            None,
            &remove_args(fs::domain::ARMED_RULES_PATH, rev),
            &[],
        )
        .expect_err("removing the armed-rules artifact hits the integrity floor");
        assert_eq!(err.code, ErrorCode::IndexIntegrity);
        assert_eq!(err.recovery, Recovery::Fix);
        assert!(
            err.message
                .as_deref()
                .unwrap_or("")
                .contains("integrity floor"),
            "the refusal cites the floor: {:?}",
            err.message
        );
        assert!(
            dir.path().join(fs::domain::ARMED_RULES_PATH).exists(),
            "a refused remove deletes nothing"
        );
    }

    #[test]
    fn u43_s4_marker_remove_refuses_citing_the_floor() {
        let (dir, root) = armed_block_ws();
        let rev = live_rev(&root, fs::domain::ATTESTED_MARKER_PATH);
        let err = remove(
            &root,
            None,
            &remove_args(fs::domain::ATTESTED_MARKER_PATH, rev),
            &[],
        )
        .expect_err("removing the once-armed marker hits the integrity floor");
        assert_eq!(err.code, ErrorCode::IndexIntegrity);
        assert!(
            dir.path().join(fs::domain::ATTESTED_MARKER_PATH).exists(),
            "the once-armed marker survives — the silent-disarm attack is refused"
        );
    }

    /// The integrity floor is STRUCTURAL — `--force` does NOT escape a marker
    /// removal (security F2: a forced silent-disarm would defeat the fail-closed
    /// design). The `remove` op carries no force, so this asserts the pure decision
    /// directly: even a forced change refuses.
    #[test]
    fn u43_index_integrity_is_not_force_escapable() {
        use policy::{ChangeOp, DoorLaw, classify_door_law};
        let d = classify_door_law(ChangeOp::Remove, fs::domain::ATTESTED_MARKER_PATH, &|_| {
            false
        });
        assert!(
            matches!(d, DoorLaw::IndexIntegrity { .. }),
            "the marker floor is structural, not a force-escapable binding break: {d:?}"
        );
    }
}
