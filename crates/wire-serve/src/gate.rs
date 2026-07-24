//! The armed-plane gate mount (U4.2) — the trusted write path's I/O half.
//!
//! [`policy::gate`] is the pure decision; this module is the disk seam that
//! feeds it. It reads the workspace's OWN attested INDEX (U1.4) and once-armed
//! marker (U2.5 read-contract) — never a caller-supplied set — resolves the
//! armed law through [`policy::resolve_armed_set`], evaluates the change, and
//! maps the outcome onto this crate's needs: advisory `warn` verdicts to merge
//! into the response, or a wire refusal (`convention_fault` / `armed_drift`)
//! that the choke-point returns BEFORE bytes land.
//!
//! It is mounted at the ONE production verdict site in [`crate::write`]
//! (splice/create/remove), so both writer paths (the per-workspace sidecar and
//! the resident registry daemon) gate through it — and, via the same
//! [`load_armed_set`] + [`policy::gate`] pair, the run plane (`crates/run`) gates
//! the bytes it lands too. On a never-armed workspace this is a bit-for-bit
//! no-op: the armed set is [`policy::ArmedSet::NeverArmed`] and the gate adds no
//! verdict and never refuses.

use wire::{ErrorBody, ErrorCode, NodeRev, Path, Severity, Span, Verdict};

/// A disk-backed [`policy::ConventionSource`]: reads a convention folder at
/// `conventions/<slug>/` under the workspace root. This is the trusted-path I/O
/// the gate injects; the security decisions all live in `policy` (this only
/// hands over bytes). The run plane keeps its own byte-identical copy — the
/// shared, load-bearing logic is `policy::resolve_armed_set`, not this glue.
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

/// One convention folder on disk (`conventions/<slug>/`).
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

/// Load + verify a workspace's armed law from disk (the trusted-path half). Reads
/// the attested INDEX bytes and the once-armed marker's presence, then resolves
/// through [`policy::resolve_armed_set`]. Any INDEX read error maps to "absent"
/// — safe, because absent-on-once-armed fails CLOSED and absent-on-never-armed
/// is the no-op. The marker probe fails CLOSED on an ambiguous stat (assume
/// armed) so a doubtful workspace is never silently ungated.
#[must_use]
pub fn load_armed_set(root: &fs::WorkspaceRoot) -> policy::ArmedSet {
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

/// A passing gate's output: the advisory verdicts to merge into the response,
/// plus the `--force`-escaped skips the mount must JOURNAL (U4.3, decision #6 —
/// a forced bypass is journaled AND rendered). `forced_skips` is empty for an
/// ordinary (non-forced) write.
#[derive(Debug, Default)]
pub struct GatePass {
    /// The §11.1 advisory verdicts (warn findings + any forced-skip renders).
    pub verdicts: Vec<Verdict>,
    /// The forced skips — one per `--force`-escaped refusal, journaled by the
    /// mount on a REAL commit.
    pub forced_skips: Vec<ForcedSkip>,
}

/// One `--force`-escaped refusal — the loud record the mount journals.
#[derive(Debug, Clone)]
pub struct ForcedSkip {
    /// The convention / binding-break the force bypassed.
    pub rule: String,
    /// The teaching message the skip carries.
    pub message: String,
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
    let armed_set = load_armed_set(root);
    match policy::gate(change, &armed_set) {
        policy::GateOutcome::Ok(findings) => {
            let forced_skips = findings
                .iter()
                .filter(|f| f.forced)
                .map(|f| ForcedSkip {
                    rule: f.slug.clone(),
                    message: f.message.clone(),
                })
                .collect();
            Ok(GatePass {
                verdicts: findings_to_verdicts(findings, subject_doc),
                forced_skips,
            })
        }
        policy::GateOutcome::Refusal(refusal) => Err(gate_refusal_to_wire(refusal)),
    }
}

/// Project the armed `warn` findings onto wire verdicts (§11.1 advisory shape).
/// Coordinates come from the subject document (path + root rev); a convention
/// finding is document-grained, so it carries the whole-file rev and a zero span.
fn findings_to_verdicts(findings: Vec<policy::GateFinding>, doc: &model::Document) -> Vec<Verdict> {
    if findings.is_empty() {
        return Vec::new();
    }
    let path = doc_path(doc);
    let node_rev = NodeRev(doc.root.node_rev.0.clone());
    findings
        .into_iter()
        .map(|f| Verdict {
            rule: f.slug,
            severity: Severity::Warn,
            path: Path(path.clone()),
            hpath: None,
            span: Span(0, 0),
            node_rev: node_rev.clone(),
            message: format!("{} (legal path: {})", f.message, f.passing_scenario),
        })
        .collect()
}

/// Map a [`policy::GateRefusal`] to its wire `{code, recovery}` envelope, drawn
/// from the closed §8 taxonomy (refusal-amendment). A `block`-severity firing
/// and a fail-closed fault both mint `convention_fault` (env) in U4.2 — the
/// message NAMES the convention/INDEX and cites the passing scenario; U4.4's
/// floor conventions add per-rule codes. Drift mints `armed_drift` (refresh),
/// carrying `armed_rev`/`report_rev` in the `expected`/`actual` comparison slots.
#[must_use]
pub fn gate_refusal_to_wire(refusal: policy::GateRefusal) -> Box<ErrorBody> {
    match refusal {
        policy::GateRefusal::Blocked { violations } => {
            let mut e = ErrorBody::new(ErrorCode::ConventionFault);
            e.path = Some(Path(fs::domain::RESERVED_INDEX_PATH.to_string()));
            e.message = Some(render_violations(&violations));
            Box::new(e)
        }
        policy::GateRefusal::ConventionFault { detail } => {
            let mut e = ErrorBody::new(ErrorCode::ConventionFault);
            e.path = Some(Path(fs::domain::RESERVED_INDEX_PATH.to_string()));
            e.message = Some(detail);
            Box::new(e)
        }
        policy::GateRefusal::ArmedDrift {
            slug,
            armed_rev,
            report_rev,
        } => {
            let message = format!(
                "armed convention `{slug}` drifted: approved rev `{armed_rev}` but the \
                 convention now reads `{report_rev}` — re-arm at the live rev, or revert the law"
            );
            let mut e = ErrorBody::new(ErrorCode::ArmedDrift);
            e.expected = Some(NodeRev(armed_rev));
            e.actual = Some(NodeRev(report_rev));
            e.message = Some(message);
            Box::new(e)
        }
        // U4.3 taxonomy row 9 — a one-sided file↔index change stopped at the
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
        // U4.3 taxonomy row 10 — the INDEX-integrity floor. `path` is the
        // protected file (the INDEX or the marker) the write tried to remove.
        policy::GateRefusal::IndexIntegrity { target, teaching } => {
            let mut e = ErrorBody::new(ErrorCode::IndexIntegrity);
            e.path = Some(Path(target));
            e.message = Some(teaching);
            Box::new(e)
        }
    }
}

/// Render stacked block violations into one teaching message that names each
/// convention and cites its passing scenario (the legal path).
fn render_violations(violations: &[policy::GateViolation]) -> String {
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
    format!(
        "armed change refused by {} convention(s): {body}",
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
/// door reads no evidence hop, so an armed convention that reads `change.edges`
/// sees them fail-closed (drifted, no facts) — never a silent pass on evidence.
#[must_use]
pub fn no_edges(_reference: &str) -> Option<(String, model::Document)> {
    None
}

#[cfg(test)]
mod scenarios {
    //! The U4.2 merge-gate scenarios, exercised through the REAL write path
    //! (`crate::write::{create, splice}`) on an on-disk workspace. Each scenario
    //! arms a convention in the workspace's own `conventions/` folder + INDEX and
    //! proves the gate's decision lands at the wire seam, not just in `policy`.

    use std::path::Path as FsPath;

    use wire::{Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path, PutAt, Recovery, SecRef};

    use crate::write::{CreateArgs, RemoveArgs, SpliceArgs, create, remove, splice};

    /// A real `reviewer-not-owner` CHECK.md: refuses when `actor == owner`.
    const CHECK_MD: &str = "---\npaths:\n  - tasks/**\n---\n\n# reviewer-not-owner\n\n\
        ```starlark\ndef check_change(change):\n    owner = change.doc.frontmatter.get(\"owner\")\n    \
        actor = change.actor\n    if actor != None and owner != None and actor == owner:\n        \
        refuse(message = \"reviewer must not be the owner: \" + actor + \" cannot close their own task\", \
        passing = \"scenarios/reviewer-close.md\")\n```\n";

    fn tmp_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    fn write_convention(root: &fs::WorkspaceRoot, slug: &str, check: &str) {
        let dir = root.0.join("conventions").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CHECK.md"), check).unwrap();
    }

    /// Arm `slug` at `level`, pinned to its live evidence rev, writing the INDEX.
    fn arm(root: &fs::WorkspaceRoot, slug: &str, level: policy::Enforcement) {
        let files = super::DiskConventionFolder {
            base: root.0.join("conventions").join(slug),
        };
        let swept = policy::sweep(&files, slug, policy::CheckLimits::default()).expect("sweeps");
        let rev = swept.rev().to_string();
        let armed = policy::arm(swept, &rev, level).expect("arms at the live rev");
        let index = policy::generate_index(&[armed]);
        std::fs::write(root.0.join(fs::domain::RESERVED_INDEX_PATH), index).unwrap();
    }

    /// Stamp the once-armed marker (U2.5 read-contract): the workspace HAS been armed.
    fn set_marker(root: &fs::WorkspaceRoot) {
        let p = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    /// A create that owner-self-closes a task (actor == owner) — fires the convention.
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

    /// A fully-armed block workspace (convention on disk, INDEX armed, marker set).
    fn armed_block_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let (dir, root) = tmp_ws();
        write_convention(&root, "reviewer-not-owner", CHECK_MD);
        arm(&root, "reviewer-not-owner", policy::Enforcement::Block);
        set_marker(&root);
        (dir, root)
    }

    // ── Scenario 1 — empty armed_set (rulesets=&[]) while INDEX armed → REFUSES ─
    #[test]
    fn s1_empty_caller_set_still_refuses_from_workspace_index() {
        let (dir, root) = armed_block_ws();
        // The caller hands the EMPTY set (`&[]`) — it cannot weaken the decision;
        // the gate reads the workspace's OWN armed INDEX.
        let err = create(&root, 0, &owner_self_create("agent:alice"), &[])
            .expect_err("owner self-close on an armed block workspace must refuse");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert_eq!(err.recovery, Recovery::Env);
        assert!(
            !dir.path().join("tasks/newtask.md").exists(),
            "a refused create writes no file"
        );
    }

    // ── Scenario 5 — the refusal NAMES the convention + cites the passing scenario
    #[test]
    fn s5_refusal_names_convention_and_cites_passing_scenario() {
        let (_dir, root) = armed_block_ws();
        let err = create(&root, 0, &owner_self_create("agent:alice"), &[]).unwrap_err();
        let msg = err.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("reviewer-not-owner"),
            "names the convention: {msg}"
        );
        assert!(
            msg.contains("scenarios/reviewer-close.md"),
            "cites the passing scenario: {msg}"
        );
        assert_eq!(
            err.path.as_ref().map(|p| p.0.as_str()),
            Some(fs::domain::RESERVED_INDEX_PATH),
            "the refusal points at the INDEX"
        );
    }

    /// The reviewer (actor ≠ owner) close LANDS — the legal path.
    #[test]
    fn reviewer_close_lands_on_armed_block() {
        let (dir, root) = armed_block_ws();
        let mut args = owner_self_create("agent:bob");
        args.body = "---\nowner: agent:alice\nstatus: closed\n---\n# Task\n\nbody\n".into();
        create(&root, 0, &args, &[]).expect("a reviewer distinct from the owner may close");
        assert!(
            dir.path().join("tasks/newtask.md").exists(),
            "the write landed"
        );
    }

    // ── Scenario 2 — INDEX deleted on an armed workspace → REFUSES ──────────────
    #[test]
    fn s2_missing_index_on_once_armed_refuses() {
        let (dir, root) = tmp_ws();
        write_convention(&root, "reviewer-not-owner", CHECK_MD);
        set_marker(&root); // the workspace WAS armed…
        // …but the INDEX is gone. Even a benign create fails CLOSED.
        let mut args = owner_self_create("agent:bob");
        args.body = "---\nowner: agent:alice\n---\n# Task\n\nbody\n".into();
        let err =
            create(&root, 0, &args, &[]).expect_err("missing INDEX on once-armed must refuse");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert!(!dir.path().join("tasks/newtask.md").exists());
    }

    // ── Scenario 3 — never-armed → byte-identical no-op ─────────────────────────
    #[test]
    fn s3_never_armed_is_byte_identical_no_op() {
        // Control: a workspace with NO conventions at all.
        let (ctrl_dir, ctrl) = tmp_ws();
        create(&ctrl, 0, &owner_self_create("agent:alice"), &[]).expect("control create lands");
        let control_bytes = std::fs::read(ctrl_dir.path().join("tasks/newtask.md")).unwrap();

        // Subject: an armed BLOCK convention + INDEX on disk, but NO marker →
        // never-armed → the gate is off. The would-fire owner self-close lands,
        // byte-for-byte identical to the ungated control.
        let (dir, root) = tmp_ws();
        write_convention(&root, "reviewer-not-owner", CHECK_MD);
        arm(&root, "reviewer-not-owner", policy::Enforcement::Block);
        // (no set_marker — never armed)
        let out = create(&root, 0, &owner_self_create("agent:alice"), &[])
            .expect("never-armed: the gate is a no-op, the write lands");
        assert!(out.verdicts.is_empty(), "never-armed adds no verdict");
        let subject_bytes = std::fs::read(dir.path().join("tasks/newtask.md")).unwrap();
        assert_eq!(
            subject_bytes, control_bytes,
            "byte-identical to the ungated write"
        );
    }

    // ── Scenario 4 — corrupt INDEX → fails closed naming it ─────────────────────
    #[test]
    fn s4_corrupt_index_fails_closed_naming_it() {
        let (dir, root) = tmp_ws();
        write_convention(&root, "reviewer-not-owner", CHECK_MD);
        set_marker(&root);
        // A page that is NOT a well-formed INDEX — must fail closed, never read as
        // an empty (gate-disabling) armed set.
        std::fs::write(
            root.0.join(fs::domain::RESERVED_INDEX_PATH),
            "not an index at all\n- [x] tampered-row\n",
        )
        .unwrap();
        let err = create(&root, 0, &owner_self_create("agent:bob"), &[])
            .expect_err("a corrupt INDEX fails closed");
        assert_eq!(err.code, ErrorCode::ConventionFault);
        assert!(
            err.message.as_deref().unwrap_or("").contains("corrupt"),
            "names the corruption: {:?}",
            err.message
        );
        assert!(!dir.path().join("tasks/newtask.md").exists());
    }

    // ── the SPLICE mount fires too (site parity) ───────────────────────────────
    #[test]
    fn splice_mount_refuses_owner_self_close() {
        let (dir, root) = armed_block_ws();
        // A task the owner already owns; the owner splices it (actor == owner).
        std::fs::create_dir_all(dir.path().join("tasks")).unwrap();
        std::fs::write(
            dir.path().join("tasks/fix.md"),
            "---\nowner: agent:alice\nstatus: open\n---\n# Fix\n\nbody\n",
        )
        .unwrap();
        let args = SpliceArgs {
            id: None,
            path: Path("tasks/fix.md".into()),
            actor: Some("agent:alice".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
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
        };
        let before = std::fs::read(dir.path().join("tasks/fix.md")).unwrap();
        let err = splice(&root, 0, &args, &[]).expect_err("owner self-splice refuses");
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
        let err = create(&root, 0, &args, &[]).expect_err("a dry rehearsal of a refusal refuses");
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

    // ═══ U4.3 named merge-gate scenarios (real write path) ═══════════════════

    /// A splice that hand-injects a forged armed row into the INDEX (a one-sided
    /// index edit / checkbox flip), aimed at the H1 section so it always
    /// validates. `force` toggles the sanctioned bypass.
    fn checkbox_flip(force: bool) -> SpliceArgs {
        SpliceArgs {
            id: None,
            path: Path(fs::domain::RESERVED_INDEX_PATH.into()),
            actor: Some("agent:mallory".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force,
            edits: vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "Attested conventions INDEX".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: PutAt::End,
                    text: "\n- [x] **forged** · block · `deadbeefdeadbeef` · \
                           [[conventions/forged/CHECK.md]] · `**`"
                        .into(),
                },
                if_node_rev: None,
            }],
            plan_edits: Vec::new(),
        }
    }

    // ── Scenario 1 — one-sided checkbox flip → binding refusal (teaching) ───────
    #[test]
    fn u43_s1_one_sided_checkbox_flip_refuses() {
        let (dir, root) = armed_block_ws();
        let index_path = dir.path().join(fs::domain::RESERVED_INDEX_PATH);
        let before = std::fs::read_to_string(&index_path).unwrap();

        let err = splice(&root, 0, &checkbox_flip(false), &[])
            .expect_err("a direct INDEX edit must break the file↔index binding");
        assert_eq!(err.code, ErrorCode::BindingBreak);
        assert_eq!(err.recovery, Recovery::Fix, "binding-break recovers by fix");
        let msg = err.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("binding-break[side=index]"),
            "the refusal names the index side: {msg}"
        );
        assert!(
            msg.contains("--truth") && msg.contains("--force"),
            "the teaching refusal names the legal paths: {msg}"
        );
        assert_eq!(
            err.path.as_ref().map(|p| p.0.as_str()),
            Some(fs::domain::RESERVED_INDEX_PATH),
            "the refusal points at the INDEX"
        );
        assert_eq!(
            std::fs::read_to_string(&index_path).unwrap(),
            before,
            "a refused INDEX edit lands no bytes (no checkbox flipped)"
        );
    }

    // ── Scenario 2 — forced write → skip JOURNALED and RENDERED ─────────────────
    #[test]
    fn u43_s2_forced_write_journals_and_renders_the_skip() {
        let (dir, root) = armed_block_ws();
        std::fs::create_dir_all(dir.path().join("tasks")).unwrap();
        std::fs::write(
            dir.path().join("tasks/fix.md"),
            "---\nowner: agent:alice\nstatus: open\n---\n# Fix\n\nbody\n",
        )
        .unwrap();
        // The owner self-closes (actor == owner) — the armed convention refuses
        // it. `--force` escapes: the write lands, the skip is journaled + rendered.
        let args = SpliceArgs {
            id: None,
            path: Path("tasks/fix.md".into()),
            actor: Some("agent:alice".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: true,
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
        };
        let out = splice(&root, 0, &args, &[]).expect("--force escapes the armed refusal");

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

        // JOURNALED: a permanent `op=force` row in the reserved journal.
        let journal =
            std::fs::read_to_string(dir.path().join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap();
        assert!(
            journal.contains("op=force") && journal.contains("path=tasks/fix.md"),
            "the forced skip is journaled as a permanent force-row: {journal}"
        );
        assert!(
            journal.contains("forced_rule="),
            "the force-row names the bypassed rule: {journal}"
        );
    }

    // ── Scenario 4 — INDEX-remove + marker-remove → INDEX-integrity floor ───────
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
    fn u43_s4_index_remove_refuses_citing_the_floor() {
        let (dir, root) = armed_block_ws();
        let rev = live_rev(&root, fs::domain::RESERVED_INDEX_PATH);
        let err = remove(
            &root,
            0,
            &remove_args(fs::domain::RESERVED_INDEX_PATH, rev),
            &[],
        )
        .expect_err("removing the INDEX hits the integrity floor");
        assert_eq!(err.code, ErrorCode::IndexIntegrity);
        assert_eq!(err.recovery, Recovery::Fix);
        assert!(
            err.message
                .as_deref()
                .unwrap_or("")
                .contains("INDEX-integrity floor convention"),
            "the refusal cites the floor convention: {:?}",
            err.message
        );
        assert!(
            dir.path().join(fs::domain::RESERVED_INDEX_PATH).exists(),
            "a refused remove deletes nothing"
        );
    }

    #[test]
    fn u43_s4_marker_remove_refuses_citing_the_floor() {
        let (dir, root) = armed_block_ws();
        let rev = live_rev(&root, fs::domain::ATTESTED_MARKER_PATH);
        let err = remove(
            &root,
            0,
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

    /// The INDEX-integrity floor is STRUCTURAL — `--force` does NOT escape a
    /// marker removal (security F2: a forced silent-disarm would defeat the
    /// fail-closed design). The `remove` op carries no force, so this asserts the
    /// pure decision directly: even a forced change refuses.
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

    /// The policy-side reserved-path spellings MUST equal `fs::domain`'s — the
    /// binding law and the disk layer name the SAME files. This cross-crate
    /// assertion fails the moment the two drift (they live in separate crates).
    #[test]
    fn policy_and_fs_agree_on_reserved_paths() {
        assert_eq!(policy::RESERVED_INDEX_PATH, fs::domain::RESERVED_INDEX_PATH);
        assert_eq!(
            policy::ATTESTED_MARKER_PATH,
            fs::domain::ATTESTED_MARKER_PATH
        );
    }
}
