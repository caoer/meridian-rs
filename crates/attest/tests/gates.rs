//! U2.5 attest — the Test scenario set (the merge gate). Attest composes the
//! realised-gate, the pin, and the receipt; it refuses iff (a) resolution fails,
//! (b) the realised-gate is unmet, or (c) a declared `check:` predicate is false.
//! Named results:
//!
//! - `pending_agent_claim_refuses_naming_claim_state_last_receipt` — class (b):
//!   a pending-agent card in the target's own compose closure refuses attest,
//!   naming claim + state + last-receipt (row 5, `realised_gate`/`retry`); the
//!   refusal writes NOTHING (no lock, no receipt).
//! - `drifted_pin_attest_lands_re_pinned` — the load-bearing NON-refusal: a
//!   drifted pin re-pins and lands (drift is rev currency, which pin refreshes —
//!   never a refusal). FALSIFICATION verified: making drift refuse-class makes
//!   this test FAIL (documented inline).
//! - `false_claim_attest_refuses_class_c` — class (c): a false `check:` at the
//!   pinned rev refuses atomically (row 4, `check_claim`/`fix`); nothing lands.
//! - `dry_never_refuses_reports_would_refuse` — `dry` over a would-refuse case
//!   reports `would_refuse` (naming the assert / the gate reasons) and writes
//!   nothing, over BOTH class (b) and class (c).
//! - `grey_input_never_refuses` — a grey declared-only input greens on the first
//!   pin; attest records, never refuses.
//! - `taxonomy_conformance` — every attest refusal maps to its amendment row
//!   (code + recovery): row 5 (b), rows 1-3 (a), row 4 (c).

use attest::board::BoardGate;
use attest::{AttestOptions, AttestRefusal, AttestResult, GateOutcome, RealisedGate, attest};

/// Build a temp workspace from `(path, content)` files.
fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(p) = full.parent() {
            std::fs::create_dir_all(p).expect("mkdir");
        }
        std::fs::write(&full, content).expect("write fixture");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn ws_owned(files: &[(&'static str, String)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    ws(&refs)
}

/// The reserved journal's text (empty when never written).
fn journal(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

/// Read a workspace file's bytes.
fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// Count `^r-` journal receipt rows.
fn journal_rows(root: &fs::WorkspaceRoot) -> usize {
    journal(root).lines().filter(|l| l.contains("^r-")).count()
}

fn dry() -> AttestOptions {
    AttestOptions {
        dry: true,
        actor: Some("alice".into()),
        now: None,
    }
}
fn real() -> AttestOptions {
    AttestOptions {
        dry: false,
        actor: Some("alice".into()),
        now: None,
    }
}

/// The default board gate, over a `board/` directory (the realise engine's
/// pending-agent cards). Scenarios that create no `board/` dir observe a passing
/// (Realised) gate — the pin runs.
fn board_gate() -> BoardGate {
    BoardGate::new("board")
}

/// A pending-agent board card for `claim`, as the realise engine mints it (a
/// `state` and a `claim`), plus the `last_receipt` the gate refusal names. The
/// FILENAME is irrelevant — the gate scopes by the card's `claim:` selector page.
fn pending_card(claim: &str, last_receipt: &str) -> String {
    format!(
        "---\ntype: board-card\nstate: pending-agent\nclaim: {claim}\nlast_receipt: {last_receipt}\ncreated: 2026-07-23\n---\n\n# pending-agent: {claim}\n\nCheck drifted with no apply-capable claim in scope.\n"
    )
}

/// The a.md fixture with a `check:` predicate over `b.md#Body`'s content (the
/// same shape U2.4 pin's gate uses — attest composes that pin).
fn check_fixture(body: &str) -> [(&'static str, String); 2] {
    let a = "---\ninputs:\n  - {ref: 'b.md#Body', check: 'a.md#Predicate'}\n---\n\n# Predicate\n\n```starlark\ndef check_claim(t):\n    return 'converged' in t.target\n```\n".to_string();
    let b = format!("# Body\n\n{body}\n");
    [("a.md", a), ("b.md", b)]
}

// ---------------------------------------------------------------------------
// Scenario 1 — class (b): a pending-agent claim refuses, naming claim + state +
// last-receipt. A refused attest writes nothing.
// ---------------------------------------------------------------------------

#[test]
fn pending_agent_claim_refuses_naming_claim_state_last_receipt() {
    // a.md is pinnable (its manifest resolves), but its OWN claim `a.md#state`
    // is recorded pending-agent on the board — the realised-gate must refuse
    // BEFORE the pin, so no receipt stamps over unconverged content.
    let (_d, root) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Body\n---\n\n# A\n\nbody\n"),
        ("b.md", "# Body\n\ncontent\n"),
        ("board/card.md", &pending_card("a.md#state", "r-000042")),
    ]);

    let result = attest(&root, "a.md", &board_gate(), &real()).expect("attest runs");
    let AttestResult::Refused(AttestRefusal::RealisedGate(gate)) = result else {
        panic!("a pending-agent claim must refuse class (b): {result:?}");
    };
    assert!(!gate.would_refuse, "a real (non-dry) refusal");
    // Names claim + state + last-receipt (d2 §2.5).
    assert_eq!(gate.claim, "a.md#state", "names the claim");
    assert_eq!(gate.state, "pending-agent", "names the state");
    assert_eq!(
        gate.last_receipt.as_deref(),
        Some("r-000042"),
        "names the last receipt"
    );
    // Row 5 — realised_gate{claim,state}, recovery retry.
    assert_eq!(gate.code, "realised_gate", "taxonomy row 5 code");
    assert_eq!(gate.recovery, "retry", "row 5 recovery class");

    // A refused attest writes NOTHING: no ^inputs lock, no receipt (the pin
    // never ran — class (b) refuses ahead of it).
    assert!(
        !read(&root, "a.md").contains("^inputs"),
        "class (b) refusal writes no lock (the pin never runs)"
    );
    assert_eq!(journal_rows(&root), 0, "no receipt minted");
}

/// Scope law (d2 §2.5): a pending-agent card for an UPSTREAM page (not the
/// target's own compose closure) does NOT refuse this attest — it is that page's
/// refusal, rendered through the walk. The target attests cleanly.
#[test]
fn upstream_pending_claim_does_not_refuse_this_attest() {
    let (_d, root) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Body\n---\n\n# A\n\nbody\n"),
        ("b.md", "# Body\n\ncontent\n"),
        // The card names b.md's claim, not a.md's — out of a.md's closure.
        ("board/card.md", &pending_card("b.md#state", "r-000007")),
    ]);
    let result = attest(&root, "a.md", &board_gate(), &real()).expect("attest runs");
    assert!(
        matches!(result, AttestResult::Attested(ref r) if r.pin.wrote),
        "an upstream page's pending claim is not this pin's refusal: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 — the load-bearing NON-refusal: a drifted pin re-pins and lands.
// ---------------------------------------------------------------------------

/// Attest `a.md` (pins `b.md#Body`), then EDIT `b.md`'s body so the pinned rev
/// drifts, then re-attest: the realised-gate holds (no pending card), so the pin
/// RE-PINS the drift — it never refuses. The lock refreshes to the new rev and a
/// second receipt mints.
///
/// FALSIFICATION (verify by hand, then restore): make drift refuse-class — e.g.
/// have the gate return `Unmet` on any drift, or make pin refuse when the live
/// rev ≠ the locked rev — and this test FAILS at the `Attested` match below.
/// "Drifted pins never refuse" is exactly what this asserts.
#[test]
fn drifted_pin_attest_lands_re_pinned() {
    let (_d, root) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Body\n---\n\n# A\n\nbody\n"),
        ("b.md", "# Body\n\noriginal content\n"),
    ]);

    // First attest: gate passes (no board dir) → pin lands the lock + receipt.
    let first = attest(&root, "a.md", &board_gate(), &real()).expect("attest runs");
    assert!(
        matches!(first, AttestResult::Attested(ref r) if r.pin.wrote && r.pin.receipt.is_some()),
        "first attest pins + mints a receipt: {first:?}"
    );
    let lock_v1 = read(&root, "a.md");
    assert_eq!(journal_rows(&root), 1, "one receipt after the first attest");

    // Drift b.md's body → its node-rev changes → the pin is stale.
    std::fs::write(root.0.join("b.md"), "# Body\n\nDRIFTED new content\n").expect("drift b.md");

    // Re-attest: the drifted pin RE-PINS (never refuses) → new rev + new receipt.
    let after = attest(&root, "a.md", &board_gate(), &real()).expect("attest runs");
    let AttestResult::Attested(report) = after else {
        panic!("a drifted pin must RE-PIN, never refuse: {after:?}");
    };
    assert!(report.pin.wrote, "the re-pin wrote the refreshed rev");
    let lock_v2 = read(&root, "a.md");
    assert_ne!(lock_v1, lock_v2, "the lock refreshed to the drifted rev");
    assert_eq!(journal_rows(&root), 2, "a second receipt minted (re-pin)");
}

// ---------------------------------------------------------------------------
// Scenario 3 — class (c): a false check_claim refuses atomically; nothing lands.
// ---------------------------------------------------------------------------

#[test]
fn false_claim_attest_refuses_class_c() {
    // b.md#Body does NOT contain 'converged' → the declared check is false at the
    // pinned rev → class (c). The gate passes (no board dir); the pin refuses.
    let (_d, root) = ws_owned(&check_fixture("stale draft text"));

    let result = attest(&root, "a.md", &board_gate(), &real()).expect("attest runs");
    let AttestResult::Refused(AttestRefusal::Pin(refusal)) = result else {
        panic!("a false check_claim must refuse class (c): {result:?}");
    };
    assert!(!refusal.would_refuse, "a real (non-dry) refusal");
    let item = refusal
        .items
        .iter()
        .find(|i| i.declared_ref == "b.md#Body")
        .expect("Body item");
    let reason = item.reason.as_ref().expect("a reason");
    assert_eq!(
        reason.code, "check_claim",
        "row 4 — refuse-class, never staleness"
    );
    assert_eq!(reason.recovery, "fix");
    assert_eq!(
        reason.assert.as_deref(),
        Some("a.md#Predicate"),
        "the refusal names the failed assert"
    );

    // A refused attest writes nothing.
    assert!(!read(&root, "a.md").contains("^inputs"), "no lock written");
    assert_eq!(journal_rows(&root), 0, "no receipt minted");
}

// ---------------------------------------------------------------------------
// Scenario 4 — dry never refuses: reports would_refuse, writes nothing (b + c).
// ---------------------------------------------------------------------------

#[test]
fn dry_never_refuses_reports_would_refuse() {
    // (c) dry over a false claim → Pin would_refuse naming the assert.
    let (_dc, root_c) = ws_owned(&check_fixture("stale draft text"));
    let rc = attest(&root_c, "a.md", &board_gate(), &dry()).expect("attest runs");
    let AttestResult::Refused(AttestRefusal::Pin(refusal)) = rc else {
        panic!("dry over a false claim reports would_refuse: {rc:?}");
    };
    assert!(refusal.would_refuse, "class (c) dry is a would_refuse");
    let item = refusal
        .items
        .iter()
        .find(|i| i.declared_ref == "b.md#Body")
        .expect("Body item");
    assert_eq!(
        item.reason.as_ref().and_then(|r| r.assert.as_deref()),
        Some("a.md#Predicate"),
        "would_refuse names the assert"
    );
    assert!(
        !read(&root_c, "a.md").contains("^inputs"),
        "dry writes nothing"
    );
    assert_eq!(journal_rows(&root_c), 0, "dry mints no receipt");

    // (b) dry over a pending-agent claim → RealisedGate would_refuse.
    let (_db, root_b) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Body\n---\n\n# A\n\nbody\n"),
        ("b.md", "# Body\n\ncontent\n"),
        ("board/card.md", &pending_card("a.md#state", "r-000042")),
    ]);
    let rb = attest(&root_b, "a.md", &board_gate(), &dry()).expect("attest runs");
    let AttestResult::Refused(AttestRefusal::RealisedGate(gate)) = rb else {
        panic!("dry over a pending-agent claim reports would_refuse: {rb:?}");
    };
    assert!(gate.would_refuse, "class (b) dry is a would_refuse");
    assert_eq!(gate.claim, "a.md#state", "still names the claim");
    assert_eq!(journal_rows(&root_b), 0, "dry mints no receipt");
}

// ---------------------------------------------------------------------------
// Grey never refuses — the first pin is how grey turns green.
// ---------------------------------------------------------------------------

#[test]
fn grey_input_never_refuses() {
    // An out-of-corpus declared ref is grey (declared-unpinned) — it pins grey,
    // never refuses (d2 §2.5). The gate passes; attest records.
    let (_d, root) = ws(&[(
        "a.md",
        "---\ninputs:\n  - external/not-in-corpus.md#X\n---\n\n# A\n\nbody\n",
    )]);
    let result = attest(&root, "a.md", &board_gate(), &real()).expect("attest runs");
    let AttestResult::Attested(report) = result else {
        panic!("a grey input must never refuse: {result:?}");
    };
    let item = &report.pin.items[0];
    assert!(
        item.color.starts_with("grey"),
        "the input pins grey: {item:?}"
    );
    assert!(report.pin.wrote, "the first pin lands the grey lock");
}

// ---------------------------------------------------------------------------
// Taxonomy conformance — every attest refusal maps to its amendment row.
// ---------------------------------------------------------------------------

/// Each refusal class attest can mint carries the `(code, recovery)` pair its
/// `wire-contract-v2-refusal-amendment.md` row fixes: row 5 (b), rows 1-3 (a),
/// row 4 (c).
#[test]
fn taxonomy_conformance() {
    // Row 5 — realised-gate (class b).
    let (_d5, r5) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Body\n---\n\n# A\n"),
        ("b.md", "# Body\n\nc\n"),
        ("board/card.md", &pending_card("a.md#state", "r-1")),
    ]);
    let AttestResult::Refused(AttestRefusal::RealisedGate(g)) =
        attest(&r5, "a.md", &board_gate(), &real()).expect("attest runs")
    else {
        panic!("expected a realised-gate refusal");
    };
    assert_eq!((g.code, g.recovery), ("realised_gate", "retry"), "row 5");

    // Row 1 — ambiguity (class a, via the pin).
    let (_d1, r1) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Dup\n---\n\n# A\n"),
        ("b.md", "# Dup\n\n1\n\n# Dup\n\n2\n"),
    ]);
    assert_pin_reason(&r1, "b.md#Dup", "ambiguous_ref", "fix");

    // Row 2 — dangling anchor (class a).
    let (_d2, r2) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#^gone\n---\n\n# A\n"),
        ("b.md", "# B\n\nbody ^here\n"),
    ]);
    assert_pin_reason(&r2, "b.md#^gone", "ref_not_found", "refresh");

    // Row 3 — selector-unresolved (class a).
    let (_d3, r3) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Nope\n---\n\n# A\n"),
        ("b.md", "# Real\n\nbody\n"),
    ]);
    assert_pin_reason(&r3, "b.md#Nope", "no_match", "fix");

    // Row 4 — check_claim-false (class c).
    let (_d4, r4) = ws_owned(&check_fixture("stale draft text"));
    assert_pin_reason(&r4, "b.md#Body", "check_claim", "fix");
}

/// Assert the attest PIN refusal for `declared_ref` carries `(code, recovery)`.
fn assert_pin_reason(root: &fs::WorkspaceRoot, declared_ref: &str, code: &str, recovery: &str) {
    let result = attest(root, "a.md", &board_gate(), &real()).expect("attest runs");
    let AttestResult::Refused(AttestRefusal::Pin(refusal)) = result else {
        panic!("expected a pin refusal for {declared_ref}: {result:?}");
    };
    let item = refusal
        .items
        .iter()
        .find(|i| i.declared_ref == declared_ref)
        .unwrap_or_else(|| panic!("item {declared_ref} present"));
    let reason = item
        .reason
        .as_ref()
        .unwrap_or_else(|| panic!("{declared_ref} carries a reason"));
    assert_eq!(reason.code, code, "{declared_ref} code");
    assert_eq!(reason.recovery, recovery, "{declared_ref} recovery");
}

// ---------------------------------------------------------------------------
// BoardGate unit — the pure-read observation directly (no pin).
// ---------------------------------------------------------------------------

#[test]
fn board_gate_realised_when_no_board_dir() {
    let (_d, root) = ws(&[("a.md", "# A\n\nbody\n")]);
    let out = board_gate().evaluate(&root, "a.md").expect("gate reads");
    assert_eq!(out, GateOutcome::Realised, "no board dir ⇒ nothing pending");
}
