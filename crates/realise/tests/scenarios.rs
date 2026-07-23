//! U3.5a merge gate — the realise engine's three named scenarios (plan Block 3
//! U3.5a Test: line), each driving the REAL run plane and the REAL U2.6 guarded
//! create over an on-disk workspace (no in-memory double — fidelity is the
//! point):
//!
//! - `failing_check_no_apply_mints_pending_agent_and_board_card`
//! - `retry_exhausted_renders_non_convergent`
//! - `no_apply_lands_unrecorded`
//!
//! Plus engine unit gates: caps union, `--dry-run` (zero caps + blast radius),
//! and the converged no-op.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use effects::EvalLimits;
use realise::{ApplyBinding, Check, Claim, ClaimState, FieldEquals, RealiseSpec, realise};

/// A page whose `status` an apply task converges to `done` (the happy claim).
const CONVERGES_PAGE: &str = "\
---
status: todo
task.fix: \"[[#^fix-1]]\"
task.fix.caps: md.set_field
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"done\")
```
^fix-1
";

/// A page whose apply only appends to a log section — it NEVER sets the
/// `resolved` field the check watches, so the claim can never converge.
const NEVER_CONVERGES_PAGE: &str = "\
---
resolved: pending
task.nudge: \"[[#^nudge-1]]\"
task.nudge.caps: md.append_section
---

# Log

```starlark
def run(ctx):
    append_section(section = \"Log\", content = \"nudged\")
```
^nudge-1
";

/// A page an apply sets `flag` to `on` — used for the caps-union gate.
const FLAG_PAGE: &str = "\
---
flag: off
task.flip: \"[[#^flip-1]]\"
task.flip.caps: md.set_field
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"flag\", value = \"on\")
```
^flip-1
";

fn spec(scratch: &Path) -> RealiseSpec {
    RealiseSpec {
        invocation_id: "realise-inv".to_owned(),
        now: Some("2026-07-23T10:00:00Z".to_owned()),
        actor: "realise:test".to_owned(),
        board_dir: "board".to_owned(),
        scratch: scratch.to_path_buf(),
        dry_run: false,
        limits: EvalLimits::default(),
        timeout: Duration::from_secs(30),
    }
}

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    for (name, body) in files {
        let path = tmp.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }
    let scratch = tmp.path().join(".meridian/realise-scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root, scratch)
}

fn field_check(page: &str, field: &str, expected: &str) -> Box<dyn Check> {
    Box::new(FieldEquals {
        page: page.to_owned(),
        field: field.to_owned(),
        expected: expected.to_owned(),
    })
}

fn binding(page: &str, task: &str) -> ApplyBinding {
    ApplyBinding {
        page: page.to_owned(),
        task: task.to_owned(),
        args: Vec::new(),
        env: BTreeMap::new(),
    }
}

fn journal_text(root: &fs::WorkspaceRoot) -> String {
    let path = root.0.join(fs::domain::RESERVED_JOURNAL_PATH);
    std::fs::read_to_string(path).unwrap_or_default()
}

fn receipt_run_lines(root: &fs::WorkspaceRoot) -> usize {
    let path = root.0.join("receipts/realise.md");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("- run "))
        .count()
}

// ---------------------------------------------------------------------------
// Scenario (a): a failing check with NO apply-capable claim mints pending-agent
//               + a board card (born through the guarded create).
// ---------------------------------------------------------------------------

#[test]
fn failing_check_no_apply_mints_pending_agent_and_board_card() {
    let (_tmp, root, scratch) = workspace(&[("target.md", "---\nstatus: todo\n---\n\n# Body\n")]);

    let claim = Claim {
        selector: "status-must-be-done".to_owned(),
        check: field_check("target.md", "status", "done"),
        apply: None, // not apply-capable → drift is pending-agent
        retry_budget: 0,
    };

    let report = realise(&root, &[claim], &spec(&scratch)).unwrap();

    // Terminal state is pending-agent, and THIS run minted the card.
    assert_eq!(report.claims.len(), 1);
    let ClaimState::PendingAgent { card } = &report.claims[0].state else {
        panic!("expected pending-agent, got {:?}", report.claims[0].state);
    };
    assert!(card.is_some(), "the card mint returns its journal anchor");
    assert_eq!(
        report.claims[0].applies, 0,
        "no apply ran for pending-agent"
    );
    assert!(report.claims[0].receipts.is_empty());

    // The board card is a real governed page with the pending-agent frontmatter.
    let card_body = std::fs::read_to_string(root.0.join("board/status-must-be-done.md")).unwrap();
    assert!(card_body.contains("state: pending-agent"), "{card_body}");
    assert!(
        card_body.contains("claim: status-must-be-done"),
        "{card_body}"
    );
    assert!(
        card_body.contains("'status' is 'todo', expected 'done'"),
        "card carries the drift detail: {card_body}"
    );

    // The card was born through the guarded create: a before=absent journal row.
    let journal = journal_text(&root);
    assert!(
        journal.contains("op=create path=board/status-must-be-done.md"),
        "{journal}"
    );
    assert!(
        journal.contains(" before=absent "),
        "guarded birth: {journal}"
    );

    // Idempotent by claim selector: a second realise mints no second card and
    // does not error — the guarded create's if_absent CAS is the idempotency.
    let claim2 = Claim {
        selector: "status-must-be-done".to_owned(),
        check: field_check("target.md", "status", "done"),
        apply: None,
        retry_budget: 0,
    };
    let report2 = realise(&root, &[claim2], &spec(&scratch)).unwrap();
    let ClaimState::PendingAgent { card: card2 } = &report2.claims[0].state else {
        panic!("still pending-agent on re-run");
    };
    assert!(card2.is_none(), "already scheduled — no second card minted");
    let create_rows = journal_text(&root).matches("op=create").count();
    assert_eq!(create_rows, 1, "exactly one card birth across both runs");
}

// ---------------------------------------------------------------------------
// Scenario (b): a retry-exhausted claim renders non-convergent — and every
//               apply it ran is recorded.
// ---------------------------------------------------------------------------

#[test]
fn retry_exhausted_renders_non_convergent() {
    let (_tmp, root, scratch) = workspace(&[("drift.md", NEVER_CONVERGES_PAGE)]);

    let claim = Claim {
        selector: "resolve-drift".to_owned(),
        check: field_check("drift.md", "resolved", "done"), // apply never sets this
        apply: Some(binding("drift.md", "nudge")),
        retry_budget: 3,
    };

    let report = realise(&root, &[claim], &spec(&scratch)).unwrap();

    assert_eq!(report.claims[0].state, ClaimState::NonConvergent);
    assert_eq!(
        report.claims[0].applies, 3,
        "the whole retry budget was spent"
    );
    assert_eq!(
        report.claims[0].receipts.len(),
        3,
        "every apply committed and recorded a receipt"
    );
    // The receipt file carries exactly the applies — no apply landed unrecorded.
    assert_eq!(receipt_run_lines(&root), 3);
    // The apply-capable claim's caps are the verb's authority.
    assert!(report.caps_union.admits("md.append_section", None));
}

// ---------------------------------------------------------------------------
// Scenario (c): no apply lands unrecorded — across a live run, every apply that
//               committed is matched by a receipt line; a converged claim runs
//               zero applies.
// ---------------------------------------------------------------------------

#[test]
fn no_apply_lands_unrecorded() {
    let (_tmp, root, scratch) = workspace(&[
        ("conv.md", CONVERGES_PAGE),
        ("already.md", "---\nstatus: done\n---\n\n# Body\n"),
    ]);

    let claims = vec![
        // converges after exactly one apply
        Claim {
            selector: "converge-status".to_owned(),
            check: field_check("conv.md", "status", "done"),
            apply: Some(binding("conv.md", "fix")),
            retry_budget: 3,
        },
        // already converged — must run zero applies, mint no receipt
        Claim {
            selector: "already-done".to_owned(),
            check: field_check("already.md", "status", "done"),
            apply: Some(binding("conv.md", "fix")),
            retry_budget: 3,
        },
    ];

    let report = realise(&root, &claims, &spec(&scratch)).unwrap();

    // The converging claim: one apply, one receipt, converged.
    assert_eq!(report.claims[0].state, ClaimState::Converged);
    assert_eq!(report.claims[0].applies, 1);
    assert_eq!(report.claims[0].receipts.len(), 1);

    // The already-converged claim: zero applies, zero receipts.
    assert_eq!(report.claims[1].state, ClaimState::Converged);
    assert_eq!(report.claims[1].applies, 0);
    assert!(report.claims[1].receipts.is_empty());

    // The invariant, checked at the tree: committed applies == receipt lines.
    let total_receipts: usize = report.claims.iter().map(|c| c.receipts.len()).sum();
    assert_eq!(total_receipts, 1);
    assert_eq!(
        receipt_run_lines(&root),
        total_receipts,
        "every committed apply landed a receipt line — none unrecorded"
    );
    // conv.md actually converged on disk.
    let conv = std::fs::read_to_string(root.0.join("conv.md")).unwrap();
    assert!(conv.contains("status: done"), "{conv}");
}

// ---------------------------------------------------------------------------
// Engine gates.
// ---------------------------------------------------------------------------

#[test]
fn caps_union_is_the_union_of_every_apply_claims_declared_caps() {
    let (_tmp, root, scratch) = workspace(&[("conv.md", CONVERGES_PAGE), ("flag.md", FLAG_PAGE)]);

    let claims = vec![
        Claim {
            selector: "c1".to_owned(),
            check: field_check("conv.md", "status", "done"),
            apply: Some(binding("conv.md", "fix")), // md.set_field
            retry_budget: 1,
        },
        Claim {
            selector: "c2".to_owned(),
            check: field_check("flag.md", "flag", "on"),
            apply: Some(binding("flag.md", "flip")), // md.set_field
            retry_budget: 1,
        },
    ];

    let report = realise(&root, &claims, &spec(&scratch)).unwrap();
    assert!(report.caps_union.admits("md.set_field", None));
    // Both converged (each apply sets its own field).
    assert!(
        report
            .claims
            .iter()
            .all(|c| c.state == ClaimState::Converged)
    );
}

#[test]
fn dry_run_uses_zero_caps_projects_blast_radius_and_writes_nothing() {
    let (_tmp, root, scratch) = workspace(&[("conv.md", CONVERGES_PAGE)]);
    let before = std::fs::read_to_string(root.0.join("conv.md")).unwrap();

    let claim = Claim {
        selector: "converge-status".to_owned(),
        check: field_check("conv.md", "status", "done"),
        apply: Some(binding("conv.md", "fix")),
        retry_budget: 3,
    };

    let mut dry = spec(&scratch);
    dry.dry_run = true;
    let report = realise(&root, &[claim], &dry).unwrap();

    // Blast radius names the drifted apply-capable claim; nothing applied.
    assert_eq!(report.projected_applies, vec!["converge-status".to_owned()]);
    assert_eq!(report.claims[0].applies, 0);
    assert!(report.claims[0].receipts.is_empty());
    // The declared caps are still reported (the union), but zero were used.
    assert!(report.caps_union.admits("md.set_field", None));
    // No apply and no receipt touched the tree.
    let after = std::fs::read_to_string(root.0.join("conv.md")).unwrap();
    assert_eq!(before, after, "dry-run wrote nothing");
    assert!(!root.0.join("receipts/realise.md").exists());
}

#[test]
fn a_converged_claim_runs_no_apply() {
    // The page already satisfies the check, but declares a real apply task (a
    // converged claim still has a valid, resolvable apply binding).
    let already_page = CONVERGES_PAGE.replace("status: todo", "status: done");
    let (_tmp, root, scratch) = workspace(&[("already.md", already_page.as_str())]);

    let claim = Claim {
        selector: "already".to_owned(),
        check: field_check("already.md", "status", "done"),
        apply: Some(binding("already.md", "fix")),
        retry_budget: 3,
    };

    let report = realise(&root, &[claim], &spec(&scratch)).unwrap();
    assert_eq!(report.claims[0].state, ClaimState::Converged);
    assert_eq!(report.claims[0].applies, 0);
    assert!(!root.0.join("receipts/realise.md").exists());
}
