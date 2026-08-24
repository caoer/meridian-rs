//! Armed-plane gate mount for the run plane (U4.2) — byte-landing parity
//! with the wire-serve write path. Before the executor commits, evaluates the
//! workspace's own armed law over the pending change; a block-severity
//! finding refuses. Never-armed workspaces are a no-op.
//!
//! **Two legs of one law, both mounted here.** The armed plane judges a write
//! through CHECK rules ([`refuse_reason`] → [`policy::gate`]) *and* through
//! MIDDLEWARE rules ([`middleware_emits`] → [`policy::run_middleware`]). The
//! wire door mounts both; the run plane lands bytes through `fs::apply_batch`
//! rather than the wire choke-point, so it mounts both here — otherwise the
//! same rule set governs a put and ignores a fire, which is the two-lanes-
//! disagree class the parity mount exists to kill.
//!
//! The middleware leg is what carries the put frame's `fields` onto a fire's
//! splice writes as `ctx.fields` (design § 6 step 6: *armed middleware
//! evaluates on those writes as on any put, with the frame the put face would
//! have given it*). `ctx.fields` lives on the middleware ctx and nowhere else
//! — a CHECK rule has no `fields` surface, and the delta sink
//! ([`crate::executor::CommitFacts`]) is a NOTIFICATION lane no rule reads.
//!
//! **ONE APPLY, ONE LAW.** Both legs judge against a SINGLE snapshot, which
//! [`crate::executor::apply_under`] resolves once ([`resolve_at`]) and hands
//! to [`middleware_rows`] and [`refuse_reason`] alike. They used to resolve
//! independently, from disk, with nothing excluding a writer between the two
//! reads — `run.lock` does not exclude wire writers (`write.lock` is taken
//! only at the delta bracket, step 7b), so a concurrent splice rewriting
//! `meridian/armed-rules.md` between 3b and 6c had the two legs of ONE write
//! evaluating DIFFERENT law. Both legs fail closed independently, so that
//! window could never produce an unguarded write; what it could produce is a
//! transform emitted by a row disarmed mid-apply. "One write, one law" is the
//! property the armed plane is supposed to have, and a reader reasoning about
//! this mount will assume it holds. Now it does.
//!
//! Sharing the snapshot is the same argument [`resolve_at`]'s own page cache
//! already makes one level down — verification and loading must see the SAME
//! bytes — extended across the two legs.
//!
//! This module adapts run-plane state into [`policy::Change`] /
//! [`policy::MwCtxInput`] and calls the evaluator. It does not own rule
//! evaluation or the batch: WHICH rows fire is
//! [`wire_serve::write::middleware_rows_of`], the overlay world is
//! [`wire_serve::middleware::DoorWorld`], and folding emissions into the
//! pending batch is the executor's. It DOES now own the apply's law
//! resolution — one read, deliberately, rather than two.

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
/// match it — `fs::load`/`model::build` leave the path empty); `edits` are the
/// planned model edits; `actor` is `run:<task>`.
///
/// `law` is the apply's OWN snapshot, resolved once by the caller and shared
/// with the middleware leg (see this module's header) — this function no
/// longer reads disk.
#[must_use]
pub(crate) fn refuse_reason(
    law: &policy::ArmedLaw,
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
    match policy::gate(&change, law) {
        policy::GateOutcome::Ok(_) => None,
        policy::GateOutcome::Refusal(refusal) => Some(describe(refusal)),
    }
}

/// The armed middleware rows that fire on a run-plane write at `page`, `id`
/// ascending — [`wire_serve::write::middleware_rows_of`] verbatim, so the fire
/// lane and the put lane can never disagree about what is armed at a path.
///
/// Takes the apply's OWN law snapshot rather than resolving: one apply, one
/// law (see this module's header). Selection is still wire-serve's.
///
/// Empty on a never-armed workspace, and empty when no armed rule carries a
/// middleware leg in scope — which is the whole cost of the mount there.
#[must_use]
pub(crate) fn middleware_rows(law: &policy::ArmedLaw, page: &str) -> Vec<policy::ArmedRule> {
    wire_serve::write::middleware_rows_of(law, page)
}

/// The pending splice one middleware row evaluates over — the run plane's
/// batch as it stands at that row.
///
/// A struct rather than eight parameters: these six travel together through
/// every row of the mount, and the two `Document`s in particular are only
/// meaningful as a pair.
pub(crate) struct PendingSplice<'a> {
    /// The workspace-relative page under fire.
    pub page: &'a str,
    /// The page as the executor loaded it under the lock.
    pub before: &'a Document,
    /// The pending candidate INCLUDING every earlier row's transforms — the
    /// caller re-derives it between rows, as the wire door does, so row *n*
    /// reads the world row *n-1* left it.
    pub after: &'a Document,
    /// The pending batch's edits.
    pub edits: &'a [Edit],
    /// The §9 identity the receipt attests.
    pub actor: &'a str,
    /// The put frame's opaque § A.2.1 map, delivered verbatim as `ctx.fields`.
    pub fields: &'a BTreeMap<String, String>,
}

/// Evaluate ONE armed middleware row over the run plane's pending splice, and
/// hand back its emissions in order.
///
/// This is the call that closes design § 6 step 6: `fields` is the put frame's
/// opaque § A.2.1 map, delivered verbatim as `ctx.fields` — the same map the
/// birth lane already hands the create door
/// ([`crate::executor::ApplyRequest::fields`]), now reaching the splice-door
/// writes too.
///
/// # Errors
/// The rendered refusal detail, ready for
/// [`crate::executor::ExecError::ArmedRefusal`]: a middleware `refuse` renders
/// in the same voice as a CHECK violation (one rule, one message, one legal
/// path), and an evaluation fault renders as [`policy::ArmedFault::Unevaluable`]
/// — a law that cannot complete never reads as a pass.
pub(crate) fn middleware_emits(
    root: &fs::WorkspaceRoot,
    row: &policy::ArmedRule,
    splice: &PendingSplice<'_>,
) -> Result<Vec<policy::MwEmit>, String> {
    let source = row
        .rule()
        .middleware_source()
        .expect("middleware_rows filtered on the middleware leg");
    let before = path_stamped(splice.before, splice.page);
    let after = path_stamped(splice.after, splice.page);
    let change = policy::derive_change(
        &before,
        &after,
        splice.edits,
        policy::Invocation {
            op: policy::ChangeOp::Splice,
            actor: Some(splice.actor),
            // A fire carries no `force`: the put face's per-write override is
            // caller vocabulary, and the run request has no field for it.
            // Inventing one here would hand every fire the escape hatch a put
            // has to ask for by name.
            force: false,
        },
        &[],
        &|_reference| None,
    );
    // The overlay world the wire door uses, so `ctx.read` / `ctx.sql` answer
    // the same question on both lanes — this file's PENDING bytes shadowing
    // disk. The run plane compiles no cross-file members, so the overlay
    // carries exactly one entry.
    let mut overlay = BTreeMap::new();
    overlay.insert(splice.page.to_owned(), after.raw.clone());
    let world = wire_serve::middleware::DoorWorld {
        root,
        overlay: &overlay,
    };
    let outcome = policy::run_middleware(
        source,
        &policy::MwCtxInput {
            change: &change,
            fields: splice.fields,
        },
        &world,
        row.rule().limits(),
    )
    .map_err(|e| {
        describe(policy::GateRefusal::ArmedLawFault {
            faults: vec![policy::ArmedFault::Unevaluable {
                row: row.row().clone(),
                detail: e.to_string(),
            }],
        })
    })?;
    if !outcome.refusals.is_empty() {
        return Err(describe(policy::GateRefusal::Blocked {
            violations: outcome
                .refusals
                .into_iter()
                .map(|r| policy::GateViolation {
                    rule: row.id().as_str().to_owned(),
                    message: r.message,
                    passing_scenario: r.passing_scenario,
                })
                .collect(),
        }));
    }
    Ok(outcome.emits)
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

    /// Empty run-birth fields for this fixture.
    static TEST_EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

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
        // The act loads the winner it attests — a `path → bytes` map IS a
        // `PageSource`, and these are the bytes just landed.
        let source = BTreeMap::from([(RULE_PAGE_PATH.to_string(), RULE_PAGE.to_string())]);
        let artifact = policy::armed::arm(
            &index,
            &policy::armed::ArmRoot::workspace(),
            [policy::armed::ArmRequest {
                id: policy::RuleId::parse(RULE_ID).expect("a legal id"),
                mode: policy::armed::Mode::Block,
                attested_rev: policy::page_rev(RULE_PAGE),
            }],
            &source,
            policy::CheckLimits::default(),
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
                authority: &Authority::granted(CapSet::parse("md.edit").unwrap()),
                observed_root: &now,
                receipt: Some(ReceiptAddr {
                    path: "receipts/run.md".to_owned(),
                    anchor: "r-000001".to_owned(),
                }),
                actor: None,
                exec: None,
                depth: 0,
                delta: None,
                fields: &TEST_EMPTY_FIELDS,
                birth_seq: None,
                ambient: None,
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

    // ── ONE APPLY, ONE LAW (reviewer 36637e1a on PR 214, finding 5) ─────────

    /// TWO rules, one per leg — a single page may NOT declare both
    /// (`policy::armed::arm` refuses `DualKind`), so the snapshot is
    /// interrogated with DISTINCT evidence per leg: the middleware leg must
    /// name `MW_LEG_ID`, the check leg must name `CHECK_LEG_ID`. Neither
    /// assertion can be satisfied by the other leg's rule.
    const CHECK_LEG_ID: &str = "check-leg-one-law";
    const CHECK_LEG_PATH: &str = "rules/check-leg.md";
    const CHECK_LEG_PAGE: &str = "---\ntags: [type/rule, rules/check]\n\
        id: check-leg-one-law\npaths:\n  - tasks/**\n---\n\n# check-leg-one-law\n\n\
        ```starlark\ndef check_change(change):\n    \
        owner = change.doc.frontmatter.get(\"owner\")\n    \
        actor = change.actor\n    \
        if actor != None and owner != None and actor == owner:\n        \
        refuse(message = \"reviewer must not be the owner\", \
        passing = \"rules/check-leg.md#close\")\n```\n";

    const MW_LEG_ID: &str = "mw-leg-one-law";
    const MW_LEG_PATH: &str = "rules/mw-leg.md";
    const MW_LEG_PAGE: &str = "---\ntags: [type/rule, rules/middleware]\n\
        id: mw-leg-one-law\npaths:\n  - tasks/**\n---\n\n# mw-leg-one-law\n\n\
        ```starlark\ndef middleware(ctx):\n    \
        set_field(path = ctx.after.path, key = \"stamped\", value = \"yes\")\n```\n";

    /// Arm BOTH rules into one artifact — the snapshot under test.
    fn arm_both_legs(root: &fs::WorkspaceRoot) {
        for (path, page) in [(CHECK_LEG_PATH, CHECK_LEG_PAGE), (MW_LEG_PATH, MW_LEG_PAGE)] {
            let p = root.0.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, page).unwrap();
        }
        let index = policy::RuleIndex::discover([
            policy::PageRef {
                layer: policy::ScopeLayer::Workspace,
                page: CHECK_LEG_PATH,
                bytes: CHECK_LEG_PAGE,
            },
            policy::PageRef {
                layer: policy::ScopeLayer::Workspace,
                page: MW_LEG_PATH,
                bytes: MW_LEG_PAGE,
            },
        ]);
        let source = BTreeMap::from([
            (CHECK_LEG_PATH.to_string(), CHECK_LEG_PAGE.to_string()),
            (MW_LEG_PATH.to_string(), MW_LEG_PAGE.to_string()),
        ]);
        let artifact = policy::armed::arm(
            &index,
            &policy::armed::ArmRoot::workspace(),
            [
                policy::armed::ArmRequest {
                    id: policy::RuleId::parse(CHECK_LEG_ID).expect("a legal id"),
                    mode: policy::armed::Mode::Block,
                    attested_rev: policy::page_rev(CHECK_LEG_PAGE),
                },
                policy::armed::ArmRequest {
                    id: policy::RuleId::parse(MW_LEG_ID).expect("a legal id"),
                    mode: policy::armed::Mode::Block,
                    attested_rev: policy::page_rev(MW_LEG_PAGE),
                },
            ],
            &source,
            policy::CheckLimits::default(),
        )
        .expect("the fixture arms")
        .render();
        let artifact_path = root.0.join(fs::domain::ARMED_RULES_PATH);
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(artifact_path, artifact).unwrap();
        set_marker(root);
    }

    /// **Both armed legs consume the caller's snapshot, and neither reads
    /// disk.** Proven by resolving the law and then DELETING the artifact and
    /// the rule page out from under both legs: a leg that re-resolved would no
    /// longer find the rule, so still naming it is only possible by consuming
    /// the snapshot it was handed. A control asserts exactly that about a
    /// fresh resolve at that moment, which is what makes the two assertions
    /// below non-vacuous.
    ///
    /// Deleting is the strongest available stand-in for "the disk changed
    /// between the two legs" and is fully deterministic — the card explicitly
    /// does not want the real race simulated, because that test would be
    /// flaky and would prove less.
    ///
    /// This test could not be WRITTEN against the parent: both legs took
    /// `&WorkspaceRoot` there and resolved for themselves.
    #[test]
    fn both_legs_consume_the_callers_law_snapshot_and_never_reread_disk() {
        let (tmp, root) = workspace();
        arm_both_legs(&root);

        // ONE resolution — what `apply_under` does at 3a′.
        let law = super::resolve_at(&root, "tasks/board.md");
        assert!(
            !super::middleware_rows(&law, "tasks/board.md").is_empty(),
            "precondition: the snapshot carries a middleware row"
        );

        // The world moves. A re-resolving leg would now find neither rule.
        std::fs::remove_file(tmp.path().join(fs::domain::ARMED_RULES_PATH)).unwrap();
        std::fs::remove_file(tmp.path().join(CHECK_LEG_PATH)).unwrap();
        std::fs::remove_file(tmp.path().join(MW_LEG_PATH)).unwrap();
        assert!(
            super::resolve_at(&root, "tasks/board.md")
                .rules()
                .is_empty(),
            "control: a fresh resolve at this moment finds no armed rule — so \
             anything the legs below still see came from the snapshot, and \
             this control is what makes the two assertions non-vacuous"
        );

        // MIDDLEWARE leg (3b): still names ITS OWN rule, from the snapshot.
        let rows = super::middleware_rows(&law, "tasks/board.md");
        assert_eq!(
            rows.iter().map(|r| r.id().as_str()).collect::<Vec<_>>(),
            vec![MW_LEG_ID],
            "the middleware leg reads the snapshot, not the (now empty) disk"
        );

        // CHECK leg (6c): still refuses, naming ITS OWN rule, from the SAME
        // snapshot. Distinct ids, so neither leg's evidence can stand in for
        // the other's.
        let before = model::build(PAGE.to_string(), syntax::parse(PAGE));
        let after_bytes = PAGE.replace("status: todo", "status: done");
        let after = model::build(after_bytes.clone(), syntax::parse(&after_bytes));
        let detail =
            super::refuse_reason(&law, &before, &after, "tasks/board.md", &[], "run:closer")
                .expect("the CHECK leg judges the snapshot, not the (now empty) disk");
        assert!(
            detail.contains(CHECK_LEG_ID),
            "the refusal names the snapshot's check rule: {detail}"
        );
    }

    /// End-to-end: with both legs wired to one snapshot, the armed apply still
    /// refuses exactly as it did — the fix must not disarm the mount it
    /// unifies. Pairs with `s7_*` above, which pin the same behaviour through
    /// the pre-fix wiring's replacement.
    #[test]
    fn one_law_wiring_leaves_the_armed_refusal_intact() {
        let (tmp, root) = workspace();
        arm_ws(&root);
        let err = apply_as_closer(&root).expect_err("the owner closing their own card refuses");
        assert!(
            matches!(&err, ExecError::ArmedRefusal { detail } if detail.contains(RULE_ID)),
            "still the armed refusal, still naming the rule: {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("tasks/board.md")).unwrap(),
            PAGE,
            "refused whole — no byte landed"
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
