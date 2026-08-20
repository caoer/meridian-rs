//! U3.5a merge gate — the realise engine's scenarios, each driving the real
//! run plane and the real U2.6 guarded create over an on-disk workspace (no
//! in-memory double). Plus engine unit gates: caps union, `--dry-run`, and
//! the converged no-op.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use effects::EvalLimits;
use realise::{
    ApplyBinding, Check, CheckOutcome, Claim, ClaimState, FieldEquals, RealiseSpec, realise,
};

/// A page whose `status` an apply task converges to `done` (the happy claim).
const CONVERGES_PAGE: &str = "\
---
status: todo
task.fix: \"[[#^fix-1]]\"
task.fix.caps: md.edit
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
task.nudge.caps: md.edit
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
task.flip.caps: md.edit
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
        // These scenarios exercise the reconciliation loop, not the convention
        // plane: no declaring root means no ceiling narrows a claim's caps.
        declaring_root: None,
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
        rule: Some("status-move".to_owned()),
        check: field_check("target.md", "status", "done"),
        apply: None, // not apply-capable → drift is pending-agent
        retry_budget: 0,
        card_template: None,
    };

    let report = realise(&root, &[claim], &spec(&scratch)).unwrap();

    // Terminal state is pending-agent, and THIS run minted the card.
    assert_eq!(report.claims.len(), 1);
    let ClaimState::PendingAgent { card } = &report.claims[0].state else {
        panic!("expected pending-agent, got {:?}", report.claims[0].state);
    };
    assert_eq!(
        card.as_deref(),
        Some("board/status-must-be-done.md"),
        "the card mint returns the born card's path"
    );
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

    // The card references its rule by id — key plus one wikilink (18.1).
    assert!(card_body.contains("\nrule: status-move\n"), "{card_body}");
    assert!(card_body.contains("[[status-move]]"), "{card_body}");

    // `created:` is the caller's RFC3339 clock, verbatim (verdict 15.7).
    assert!(
        card_body.contains("\ncreated: 2026-07-23T10:00:00Z\n"),
        "{card_body}"
    );
    let created = card_body
        .lines()
        .find_map(|l| l.strip_prefix("created: "))
        .expect("the card stamps a created:");
    assert!(wire::now_is_rfc3339(created), "created={created:?}");

    // Idempotent by claim selector: a second realise mints no second card.
    let claim2 = Claim {
        selector: "status-must-be-done".to_owned(),
        rule: Some("status-move".to_owned()),
        check: field_check("target.md", "status", "done"),
        apply: None,
        retry_budget: 0,
        card_template: None,
    };
    let report2 = realise(&root, &[claim2], &spec(&scratch)).unwrap();
    let ClaimState::PendingAgent { card: card2 } = &report2.claims[0].state else {
        panic!("still pending-agent on re-run");
    };
    assert!(card2.is_none(), "already scheduled — no second card minted");
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
        rule: None,
        check: field_check("drift.md", "resolved", "done"), // apply never sets this
        apply: Some(binding("drift.md", "nudge")),
        retry_budget: 3,
        card_template: None,
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
    assert!(report.caps_union.admits("md.edit", None));
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
            rule: None,
            check: field_check("conv.md", "status", "done"),
            apply: Some(binding("conv.md", "fix")),
            retry_budget: 3,
            card_template: None,
        },
        // already converged — must run zero applies, mint no receipt
        Claim {
            selector: "already-done".to_owned(),
            rule: None,
            check: field_check("already.md", "status", "done"),
            apply: Some(binding("conv.md", "fix")),
            retry_budget: 3,
            card_template: None,
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
            rule: None,
            check: field_check("conv.md", "status", "done"),
            apply: Some(binding("conv.md", "fix")), // md.edit
            retry_budget: 1,
            card_template: None,
        },
        Claim {
            selector: "c2".to_owned(),
            rule: None,
            check: field_check("flag.md", "flag", "on"),
            apply: Some(binding("flag.md", "flip")), // md.edit
            retry_budget: 1,
            card_template: None,
        },
    ];

    let report = realise(&root, &claims, &spec(&scratch)).unwrap();
    assert!(report.caps_union.admits("md.edit", None));
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
        rule: None,
        check: field_check("conv.md", "status", "done"),
        apply: Some(binding("conv.md", "fix")),
        retry_budget: 3,
        card_template: None,
    };

    let mut dry = spec(&scratch);
    dry.dry_run = true;
    let report = realise(&root, &[claim], &dry).unwrap();

    // Blast radius names the drifted apply-capable claim; nothing applied.
    assert_eq!(report.projected_applies, vec!["converge-status".to_owned()]);
    assert_eq!(report.claims[0].applies, 0);
    assert!(report.claims[0].receipts.is_empty());
    // The declared caps are still reported (the union), but zero were used.
    assert!(report.caps_union.admits("md.edit", None));
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
        rule: None,
        check: field_check("already.md", "status", "done"),
        apply: Some(binding("already.md", "fix")),
        retry_budget: 3,
        card_template: None,
    };

    let report = realise(&root, &[claim], &spec(&scratch)).unwrap();
    assert_eq!(report.claims[0].state, ClaimState::Converged);
    assert_eq!(report.claims[0].applies, 0);
    assert!(!root.0.join("receipts/realise.md").exists());
}

// ---------------------------------------------------------------------------
// F1 (verdict 18.1) — the card references its rule, never embeds it, and its
// `created:` is the caller's RFC3339 clock.
// ---------------------------------------------------------------------------

/// Mint one pending-agent card in a fresh workspace and return its bytes.
fn mint_card(rule: Option<&str>, now: Option<&str>) -> String {
    let (_tmp, root, scratch) = workspace(&[("target.md", "---\nstatus: todo\n---\n\n# Body\n")]);
    let claim = Claim {
        selector: "status-must-be-done".to_owned(),
        rule: rule.map(str::to_owned),
        check: field_check("target.md", "status", "done"),
        apply: None,
        retry_budget: 0,
        card_template: None,
    };
    let mut s = spec(&scratch);
    s.now = now.map(str::to_owned);
    realise(&root, &[claim], &s).unwrap();
    std::fs::read_to_string(root.0.join("board/status-must-be-done.md")).unwrap()
}

/// A card's rule is a reference: any rule-body fragment on a board card would
/// mean the rule was copied out of its page and can now drift from it.
#[test]
fn a_card_never_embeds_a_rule_body() {
    let card = mint_card(Some("status-move"), Some("2026-07-23T10:00:00Z"));
    for embedded in [
        "```starlark",
        "def check_change",
        "def on_change",
        "refuse(",
    ] {
        assert!(
            !card.contains(embedded),
            "card embeds rule body ({embedded}): {card}"
        );
    }
    // What it carries instead: the id, twice — the key and the link.
    assert!(card.contains("rule: status-move"), "{card}");
    assert!(card.contains("[[status-move]]"), "{card}");
}

/// A claim that names no rule gets no rule key and no dangling link — a card
/// never invents a reference it was not given.
#[test]
fn a_ruleless_claim_mints_a_card_with_no_rule_key() {
    let card = mint_card(None, Some("2026-07-23T10:00:00Z"));
    assert!(!card.contains("rule:"), "{card}");
    assert!(!card.contains("[["), "{card}");
    // Everything else about the card is unchanged.
    assert!(card.contains("state: pending-agent"), "{card}");
    assert!(card.contains("created: 2026-07-23T10:00:00Z"), "{card}");
}

/// The clock is an ARGUMENT: the same passed-in `now` renders the same card,
/// byte for byte. Nothing in this path reads a wall clock (§9).
#[test]
fn a_fixed_now_renders_a_byte_identical_card() {
    let first = mint_card(Some("status-move"), Some("2026-07-23T10:00:00Z"));
    let second = mint_card(Some("status-move"), Some("2026-07-23T10:00:00Z"));
    assert_eq!(first, second, "same now ⇒ same card");

    let later = mint_card(Some("status-move"), Some("2026-07-24T10:00:00Z"));
    assert_ne!(first, later, "a different now moves `created:`");
    assert!(later.contains("created: 2026-07-24T10:00:00Z"), "{later}");
}

/// A `now` that is not RFC3339 is refused loud at the mint, never stamped onto
/// a governed page (verdict 15.7 — the format law is validated, not coerced).
#[test]
fn a_malformed_now_refuses_the_card_mint() {
    let (_tmp, root, scratch) = workspace(&[("target.md", "---\nstatus: todo\n---\n\n# Body\n")]);
    let claim = Claim {
        selector: "status-must-be-done".to_owned(),
        rule: Some("status-move".to_owned()),
        check: field_check("target.md", "status", "done"),
        apply: None,
        retry_budget: 0,
        card_template: None,
    };
    let mut s = spec(&scratch);
    s.now = Some("1753264800".to_owned()); // unix seconds — the old shape
    let err = realise(&root, &[claim], &s).unwrap_err();
    assert!(
        format!("{err:?}").contains("not RFC3339"),
        "expected a card-mint refusal, got {err:?}"
    );
    assert!(
        !root.0.join("board/status-must-be-done.md").exists(),
        "no card is born on a malformed clock"
    );
}

// ---------------------------------------------------------------------------
// The receipt address plane: two invocations against the SHARED
// `receipts/realise.md` must not mint the same anchor (contract §6.6).
// ---------------------------------------------------------------------------

/// Every `^id` published in the receipts file, in order.
fn receipt_anchors(root: &fs::WorkspaceRoot) -> Vec<String> {
    std::fs::read_to_string(root.0.join("receipts/realise.md"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.rsplit_once(" ^").map(|(_, id)| id.to_owned()))
        .collect()
}

fn converge_claim(page: &str) -> Claim {
    Claim {
        selector: format!("{page}-status-must-be-done"),
        rule: None,
        check: field_check(page, "status", "done"),
        apply: Some(binding(page, "fix")),
        retry_budget: 2,
        card_template: None,
    }
}

/// THE REGRESSION (dogfood s13-88): the receipt writer numbered every
/// invocation from `^r-000001` into one shared file, so invocation 2 re-minted
/// invocation 1's id and `read --section '^r-000001'` refused ambiguous — the
/// receipt was published and unaddressable. The anchor now carries the
/// caller's invocation id, so it is unique across invocations.
#[test]
fn two_invocations_publish_distinct_receipt_anchors() {
    // Two pages, one per invocation — the shared file is `receipts/realise.md`,
    // which is where the collision lived. (Re-drifting ONE page by hand instead
    // would trip the run plane's foreign-edit guard, not this gate.)
    let (_tmp, root, scratch) =
        workspace(&[("work-a.md", CONVERGES_PAGE), ("work-b.md", CONVERGES_PAGE)]);

    let mut first = spec(&scratch);
    first.invocation_id = "realise-1000-11".to_owned();
    realise(&root, &[converge_claim("work-a.md")], &first).unwrap();

    let mut second = spec(&scratch);
    second.invocation_id = "realise-2000-22".to_owned();
    realise(&root, &[converge_claim("work-b.md")], &second).unwrap();

    let anchors = receipt_anchors(&root);
    assert_eq!(
        anchors.len(),
        2,
        "one receipt per applying invocation: {anchors:?}"
    );
    assert_ne!(
        anchors[0], anchors[1],
        "the shared receipts file must not carry a duplicate block id: {anchors:?}"
    );
    assert!(
        anchors[0].contains("realise-1000-11") && anchors[1].contains("realise-2000-22"),
        "each anchor carries its own invocation id: {anchors:?}"
    );
}

/// An invocation id outside the block-id charset would mint an anchor no
/// strict door can address, so it refuses at the mint — before any apply runs
/// (§2.4 charset, §6.6 the caller's minting duty).
#[test]
fn an_invocation_id_outside_the_block_id_charset_refuses_before_applying() {
    let (_tmp, root, scratch) = workspace(&[("work.md", CONVERGES_PAGE)]);

    let mut s = spec(&scratch);
    s.invocation_id = "realise_bad id".to_owned(); // `_` and a space (ruling 011)
    let err = realise(&root, &[converge_claim("work.md")], &s).unwrap_err();
    assert!(
        format!("{err:?}").contains("BadInvocationId"),
        "expected the charset refusal, got {err:?}"
    );
    assert!(
        !root.0.join("receipts/realise.md").exists(),
        "no receipt is published on a refused mint"
    );
}

// ---------------------------------------------------------------------------
// The card-template plane (docs/laws.md § Amendment — no hard-coded flow):
// a claim's `card_template` page supplies the minted card's ENTIRE vocabulary;
// the engine fills only the slots it owns. The baked body mints only when no
// template is declared (the F1 gates above pin those bytes).
// ---------------------------------------------------------------------------

/// A user card template speaking its OWN flow vocabulary — `status:` (not the
/// engine's `state:`), its own words, its own headings. The engine's slots ride
/// `{{…}}`.
const CARD_TEMPLATE_PAGE: &str = r"---
description: the flow's own card shape — the engine never words it
---

# Template ^template

```record
---
status: needs-human
claim: {{selector}}
reason: {{detail}}
opened: {{now}}
by: {{actor}}
---

# waiting: {{selector}}

Drifted per [[{{rule}}]] — pull this card and converge it by hand.
```
";

/// A template that references `{{detail}}` only in the BODY — the shape that
/// admits a multi-line drift detail.
const CARD_TEMPLATE_BODY_DETAIL: &str = r"---
description: body-detail card shape
---

# Template ^template

```record
---
status: needs-human
---

# waiting: {{selector}}

{{detail}}
```
";

/// A page with a `# Template` heading but NO `^template` anchor on the heading
/// line — the one shape where "declares no ^template" reads as false to the
/// author staring at their visible heading.
const ANCHORLESS_TEMPLATE_PAGE: &str =
    "---\ndescription: x\n---\n\n# Template\n\n```record\nbody\n```\n";

fn template_claim(template: Option<&str>) -> Claim {
    Claim {
        selector: "status-must-be-done".to_owned(),
        rule: Some("status-move".to_owned()),
        check: field_check("target.md", "status", "done"),
        apply: None,
        retry_budget: 0,
        card_template: template.map(str::to_owned),
    }
}

/// THE LAW'S RECEIPT (docs/laws.md § Amendment): the minted card's vocabulary
/// comes from the user's template page — and the engine's own matcher
/// (`FieldEquals`, the same reader rule pages match with) observes the minted
/// card CONVERGED on the user's `status:` spelling. The `state:` mismatch that
/// made engine-minted cards invisible to user rules is dead.
#[test]
fn a_declared_template_supplies_the_cards_entire_vocabulary() {
    let (_tmp, root, scratch) = workspace(&[
        ("target.md", "---\nstatus: todo\n---\n\n# Body\n"),
        ("flows/card.md", CARD_TEMPLATE_PAGE),
    ]);

    let report = realise(
        &root,
        &[template_claim(Some("flows/card.md"))],
        &spec(&scratch),
    )
    .unwrap();
    let ClaimState::PendingAgent { card } = &report.claims[0].state else {
        panic!("expected pending-agent, got {:?}", report.claims[0].state);
    };
    assert_eq!(card.as_deref(), Some("board/status-must-be-done.md"));

    let card = std::fs::read_to_string(root.0.join("board/status-must-be-done.md")).unwrap();

    // The user's vocabulary, slot-filled.
    assert!(card.contains("status: needs-human"), "{card}");
    assert!(card.contains("claim: status-must-be-done"), "{card}");
    assert!(card.contains("opened: 2026-07-23T10:00:00Z"), "{card}");
    assert!(card.contains("by: realise:test"), "{card}");
    assert!(card.contains("# waiting: status-must-be-done"), "{card}");
    assert!(card.contains("[[status-move]]"), "{card}");

    // The engine's baked vocabulary is GONE — no `state:` key, no engine
    // status word, no engine type word, no engine prose.
    for baked in [
        "state:",
        "pending-agent",
        "board-card",
        "Check drifted with no apply-capable claim",
    ] {
        assert!(
            !card.contains(baked),
            "baked vocabulary survived ({baked}): {card}"
        );
    }

    // The drift detail rode the frontmatter as ONE encoded value — the `: `
    // inside it minted no shadow key (§ A.6.3a).
    assert!(
        card.contains("reason: \"target.md: 'status' is 'todo', expected 'done'\""),
        "{card}"
    );

    // The matchability receipt, via the engine's own matcher: a rule watching
    // the USER's spelling observes the minted card converged.
    let observed = FieldEquals {
        page: "board/status-must-be-done.md".to_owned(),
        field: "status".to_owned(),
        expected: "needs-human".to_owned(),
    }
    .observe(&root)
    .unwrap();
    assert_eq!(
        observed,
        CheckOutcome::Converged,
        "user rules match the minted card"
    );
}

/// Idempotency is untouched by the template plane: the card path stays
/// selector-derived, so a re-realise hits the same `if_absent` CAS.
#[test]
fn a_template_card_stays_idempotent_by_selector() {
    let (_tmp, root, scratch) = workspace(&[
        ("target.md", "---\nstatus: todo\n---\n\n# Body\n"),
        ("flows/card.md", CARD_TEMPLATE_PAGE),
    ]);

    let s = spec(&scratch);
    realise(&root, &[template_claim(Some("flows/card.md"))], &s).unwrap();
    let report2 = realise(&root, &[template_claim(Some("flows/card.md"))], &s).unwrap();
    let ClaimState::PendingAgent { card } = &report2.claims[0].state else {
        panic!("still pending-agent on re-run");
    };
    assert!(card.is_none(), "already scheduled — no second card minted");
}

/// A DECLARED template that cannot be read refuses the mint LOUD, naming the
/// page — never a silent fallback to the baked body, which would let a typo'd
/// path resurrect the engine vocabulary invisibly.
#[test]
fn a_missing_template_page_refuses_the_mint_loud() {
    let (_tmp, root, scratch) = workspace(&[("target.md", "---\nstatus: todo\n---\n\n# Body\n")]);

    let err = realise(
        &root,
        &[template_claim(Some("flows/absent.md"))],
        &spec(&scratch),
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("card template flows/absent.md"), "{msg}");
    assert!(
        !root.0.join("board/status-must-be-done.md").exists(),
        "no card is born on a refused template"
    );
}

/// A template page whose `# Template` heading carries no `^template` anchor
/// declares nothing — the refusal teaches the anchor-on-heading-line rule
/// instead of minting the baked body.
#[test]
fn a_template_page_without_a_template_block_refuses() {
    let (_tmp, root, scratch) = workspace(&[
        ("target.md", "---\nstatus: todo\n---\n\n# Body\n"),
        ("flows/card.md", ANCHORLESS_TEMPLATE_PAGE),
    ]);

    let err = realise(
        &root,
        &[template_claim(Some("flows/card.md"))],
        &spec(&scratch),
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("no ^template block"), "{msg}");
    assert!(
        msg.contains("heading LINE carries the `^template` anchor"),
        "{msg}"
    );
    assert!(!root.0.join("board/status-must-be-done.md").exists());
}

/// A drift detail carrying a newline cannot ride a frontmatter slot — the mint
/// refuses (§ A.6.3a, never sanitized) — but the SAME detail fills verbatim in
/// a body slot: the template author chooses which plane carries it.
#[test]
fn a_multiline_detail_refuses_frontmatter_but_fills_body() {
    struct DriftsMultiline;
    impl Check for DriftsMultiline {
        fn observe(&self, _root: &fs::WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
            Ok(CheckOutcome::Drifted {
                detail: "observed: line one\nline two".to_owned(),
            })
        }
    }
    let multiline_claim = |template: &str| Claim {
        selector: "multi".to_owned(),
        rule: None,
        check: Box::new(DriftsMultiline),
        apply: None,
        retry_budget: 0,
        card_template: Some(template.to_owned()),
    };

    // Frontmatter slot: refused, no card born.
    let (_tmp, root, scratch) = workspace(&[
        ("target.md", "---\nstatus: todo\n---\n\n# Body\n"),
        ("flows/card.md", CARD_TEMPLATE_PAGE),
    ]);
    let err = realise(&root, &[multiline_claim("flows/card.md")], &spec(&scratch)).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("contains a newline"), "{msg}");
    assert!(
        msg.contains("{{detail}}"),
        "the refusal names the slot: {msg}"
    );
    assert!(!root.0.join("board/multi.md").exists());

    // Body slot: the same detail mints, verbatim.
    let (_tmp2, root2, scratch2) = workspace(&[
        ("target.md", "---\nstatus: todo\n---\n\n# Body\n"),
        ("flows/card.md", CARD_TEMPLATE_BODY_DETAIL),
    ]);
    realise(
        &root2,
        &[multiline_claim("flows/card.md")],
        &spec(&scratch2),
    )
    .unwrap();
    let card = std::fs::read_to_string(root2.0.join("board/multi.md")).unwrap();
    assert!(card.contains("observed: line one\nline two"), "{card}");
    assert!(card.contains("status: needs-human"), "{card}");
}
