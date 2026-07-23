//! U2.4 pin — the Test scenario set (the merge gate). Named results:
//!
//! - `ambiguous_ref_refuses_whole_pin_atomically` — an ambiguous ref refuses the
//!   WHOLE pin with per-item reasons; no partial pin lands (the resolvable
//!   sibling is NOT pinned). Falsification: making the resolve partial-tolerant
//!   makes this test FAIL (documented in the test).
//! - `re_pin_after_rename_confirms_rebind` — a rename surfaces a rebind
//!   CANDIDATE (never a verdict); re-pinning the updated ref confirms it.
//! - `false_check_claim_refuses_atomically` + `dry_reports_would_refuse` — a
//!   false `check_claim` refuses atomically naming the assert; `dry` reports
//!   `would_refuse`; a refused pin writes nothing.
//! - `idempotent_re_run_mints_nothing` — a second pin over unchanged inputs
//!   writes no bytes and mints no second receipt.
//! - `taxonomy_conformance` — every refusal maps to its amendment row (code +
//!   recovery class).

use pin::{PinOptions, PinResult, pin};

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

/// The reserved journal's text (empty when never written).
fn journal(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

/// Read a workspace file's bytes.
fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// Count `^r-` journal rows.
fn journal_rows(root: &fs::WorkspaceRoot) -> usize {
    journal(root).lines().filter(|l| l.contains("^r-")).count()
}

fn dry() -> PinOptions {
    PinOptions {
        dry: true,
        actor: Some("alice".into()),
        now: None,
    }
}
fn real() -> PinOptions {
    PinOptions {
        dry: false,
        actor: Some("alice".into()),
        now: None,
    }
}

// ---------------------------------------------------------------------------
// Gate 1 — ambiguous ref refuses the WHOLE pin, atomically, per-item reasons.
// ---------------------------------------------------------------------------

/// A pin whose manifest declares a resolvable ref AND an ambiguous ref refuses
/// the whole pin: the refusal names the ambiguous item (code `ambiguous_ref`,
/// recovery `fix`) with candidates, the resolvable sibling carries `reason:
/// None`, and NOTHING lands — no `^inputs` lock is written (atomicity).
///
/// FALSIFICATION (verified out-of-band): if `resolve_all` were partial-tolerant
/// — pinning `b.md#Good` and skipping the ambiguous `b.md#Dup` — then `a.md`
/// WOULD gain an `^inputs` block and `assert !written` below FAILS. Atomicity is
/// what this asserts.
#[test]
fn ambiguous_ref_refuses_whole_pin_atomically() {
    let (_d, root) = ws(&[
        (
            "a.md",
            "---\ninputs:\n  - b.md#Good\n  - b.md#Dup\n---\n\n# A\n\nbody\n",
        ),
        ("b.md", "# Good\n\ng\n\n# Dup\n\nd1\n\n# Dup\n\nd2\n"),
    ]);

    let result = pin(&root, "a.md", &real()).expect("pin runs");
    let PinResult::Refused(refusal) = result else {
        panic!("an ambiguous ref must refuse the whole pin: {result:?}");
    };
    assert!(!refusal.would_refuse, "a real (non-dry) refusal");

    // Per-item reasons: Dup is ambiguous, Good resolved (reason None).
    let dup = refusal
        .items
        .iter()
        .find(|i| i.declared_ref == "b.md#Dup")
        .expect("Dup item present");
    let reason = dup.reason.as_ref().expect("Dup carries a reason");
    assert_eq!(reason.code, "ambiguous_ref");
    assert_eq!(reason.recovery, "fix");
    assert_eq!(reason.candidates.len(), 2, "both duplicates named");
    let good = refusal
        .items
        .iter()
        .find(|i| i.declared_ref == "b.md#Good")
        .expect("Good item present");
    assert!(
        good.reason.is_none(),
        "the resolvable sibling has no reason"
    );

    // ATOMICITY: nothing landed — a.md has no ^inputs lock.
    assert!(
        !read(&root, "a.md").contains("^inputs"),
        "a refused pin writes NO lock (atomic — the resolvable ref is not pinned either)"
    );
    assert_eq!(journal_rows(&root), 0, "no receipt minted");
}

// ---------------------------------------------------------------------------
// Gate 2 — re-pin after rename confirms rebind (candidate, never verdict).
// ---------------------------------------------------------------------------

/// Pin `a.md → b.md#Old` (green), rename `b.md`'s heading, then re-pin: the
/// declared ref no longer resolves, so the pin refuses and SURFACES a rebind
/// CANDIDATE (the renamed heading) — never a silent rebind. Updating the
/// manifest to the new ref and re-pinning CONFIRMS the rebind (green again).
#[test]
fn re_pin_after_rename_confirms_rebind() {
    let (_d, root) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Old\n---\n\n# A\n\nbody\n"),
        ("b.md", "# Old\n\nlaw text\n"),
    ]);

    // First pin: b.md#Old resolves → green lock.
    let first = pin(&root, "a.md", &real()).expect("pin runs");
    assert!(
        matches!(first, PinResult::Pinned(ref r) if r.wrote),
        "first pin writes the lock: {first:?}"
    );
    assert!(
        read(&root, "a.md").contains("b.md#Old"),
        "lock pins b.md#Old"
    );

    // Rename b.md's heading Old -> New.
    std::fs::write(root.0.join("b.md"), "# New\n\nlaw text\n").expect("rename heading");

    // Re-pin: b.md#Old no longer resolves → refuse with a rebind candidate.
    let after = pin(&root, "a.md", &real()).expect("pin runs");
    let PinResult::Refused(refusal) = after else {
        panic!("a renamed ref must refuse (candidate, never silent rebind): {after:?}");
    };
    let item = &refusal.items[0];
    let reason = item.reason.as_ref().expect("a reason");
    assert_eq!(reason.code, "no_match", "selector-unresolved (row 3)");
    assert!(
        reason.candidates.iter().any(|c| c.contains("New")),
        "the rebind CANDIDATE names the renamed heading: {:?}",
        reason.candidates
    );
    // Never a verdict: the manifest/lock were NOT auto-rewritten to New.
    assert!(
        read(&root, "a.md").contains("b.md#Old"),
        "the engine never auto-rebinds — the stale pin stays until re-pinned"
    );

    // The author confirms the rebind by updating the manifest + re-pinning.
    std::fs::write(
        root.0.join("a.md"),
        "---\ninputs:\n  - b.md#New\n---\n\n# A\n\nbody\n",
    )
    .expect("author updates the manifest");
    let confirm = pin(&root, "a.md", &real()).expect("pin runs");
    assert!(
        matches!(confirm, PinResult::Pinned(ref r) if r.items.iter().all(|i| i.color == "green")),
        "re-pinning the updated ref confirms the rebind (green): {confirm:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — false check_claim refuses atomically; dry reports would_refuse.
// ---------------------------------------------------------------------------

/// The a.md fixture with a `check:` predicate over `b.md#Body`'s content.
fn check_fixture(body: &str) -> [(&'static str, String); 2] {
    let a = "---\ninputs:\n  - {ref: 'b.md#Body', check: 'a.md#Predicate'}\n---\n\n# Predicate\n\n```starlark\ndef check_claim(t):\n    return 'converged' in t.target\n```\n".to_string();
    let b = format!("# Body\n\n{body}\n");
    [("a.md", a), ("b.md", b)]
}

fn ws_owned(files: &[(&'static str, String)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    ws(&refs)
}

#[test]
fn false_check_claim_refuses_atomically() {
    // b.md#Body does NOT contain 'converged' → the claim is false.
    let (_d, root) = ws_owned(&check_fixture("stale draft text"));

    let result = pin(&root, "a.md", &real()).expect("pin runs");
    let PinResult::Refused(refusal) = result else {
        panic!("a false check_claim must refuse: {result:?}");
    };
    let item = refusal
        .items
        .iter()
        .find(|i| i.declared_ref == "b.md#Body")
        .expect("Body item");
    let reason = item.reason.as_ref().expect("a reason");
    assert_eq!(reason.code, "check_claim", "refusal-amendment row 4");
    assert_eq!(reason.recovery, "fix");
    assert_eq!(
        reason.assert.as_deref(),
        Some("a.md#Predicate"),
        "the refusal names the failed assert"
    );
    // A refused pin writes nothing.
    assert!(!read(&root, "a.md").contains("^inputs"), "no lock written");
    assert_eq!(journal_rows(&root), 0, "no receipt minted");
}

#[test]
fn dry_reports_would_refuse_naming_the_assert() {
    let (_d, root) = ws_owned(&check_fixture("stale draft text"));
    let result = pin(&root, "a.md", &dry()).expect("pin runs");
    let PinResult::Refused(refusal) = result else {
        panic!("dry over a false claim must report would_refuse: {result:?}");
    };
    assert!(refusal.would_refuse, "dry reports would_refuse");
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
        !read(&root, "a.md").contains("^inputs"),
        "dry writes nothing"
    );
}

#[test]
fn true_check_claim_pins() {
    // b.md#Body now contains 'converged' → the claim holds → the pin lands.
    let (_d, root) = ws_owned(&check_fixture("now converged content"));
    let result = pin(&root, "a.md", &real()).expect("pin runs");
    assert!(
        matches!(result, PinResult::Pinned(ref r) if r.wrote),
        "a holding claim pins: {result:?}"
    );
    let lock = read(&root, "a.md");
    assert!(lock.contains("check: 'a.md#Predicate'"), "check recorded");
    assert!(lock.contains("check_rev:"), "check_rev recorded");
}

// ---------------------------------------------------------------------------
// Gate 4 — idempotent re-run mints nothing.
// ---------------------------------------------------------------------------

#[test]
fn idempotent_re_run_mints_nothing() {
    let (_d, root) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Body\n---\n\n# A\n\nbody\n"),
        ("b.md", "# Body\n\ncontent\n"),
    ]);

    let first = pin(&root, "a.md", &real()).expect("pin runs");
    assert!(
        matches!(first, PinResult::Pinned(ref r) if r.wrote && r.receipt.is_some()),
        "first pin writes + mints a receipt: {first:?}"
    );
    let after_first = read(&root, "a.md");
    assert_eq!(journal_rows(&root), 1, "one receipt after the first pin");

    // Second pin over unchanged inputs: no bytes, no second receipt.
    let second = pin(&root, "a.md", &real()).expect("pin runs");
    let PinResult::Pinned(report) = second else {
        panic!("idempotent re-pin is still a Pinned verdict: {second:?}");
    };
    assert!(!report.wrote, "an unchanged re-pin writes nothing");
    assert!(report.receipt.is_none(), "no second receipt");
    assert_eq!(
        read(&root, "a.md"),
        after_first,
        "byte-identical (no rev churn)"
    );
    assert_eq!(journal_rows(&root), 1, "still exactly one receipt");
}

// ---------------------------------------------------------------------------
// Gate 5 — taxonomy conformance: every refusal maps to its amendment row.
// ---------------------------------------------------------------------------

/// Each refusal reason the pin can mint carries a `(code, recovery)` pair that
/// matches its `wire-contract-v2-refusal-amendment.md` row exactly.
#[test]
fn taxonomy_conformance() {
    // Row 1 — ambiguity.
    let (_d1, r1) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Dup\n---\n\n# A\n"),
        ("b.md", "# Dup\n\n1\n\n# Dup\n\n2\n"),
    ]);
    assert_reason(&r1, "b.md#Dup", "ambiguous_ref", "fix");

    // Row 2 — dangling anchor (a pinned ^block-id that vanished).
    let (_d2, r2) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#^gone\n---\n\n# A\n"),
        ("b.md", "# B\n\nbody ^here\n"),
    ]);
    assert_reason(&r2, "b.md#^gone", "ref_not_found", "refresh");

    // Row 3 — selector-unresolved (a heading that resolves to nothing).
    let (_d3, r3) = ws(&[
        ("a.md", "---\ninputs:\n  - b.md#Nope\n---\n\n# A\n"),
        ("b.md", "# Real\n\nbody\n"),
    ]);
    assert_reason(&r3, "b.md#Nope", "no_match", "fix");

    // Row 4 — check_claim-false.
    let (_d4, r4) = ws_owned(&check_fixture("stale draft text"));
    assert_reason(&r4, "b.md#Body", "check_claim", "fix");
}

/// Assert the refusal for `declared_ref` carries `(code, recovery)`.
fn assert_reason(root: &fs::WorkspaceRoot, declared_ref: &str, code: &str, recovery: &str) {
    let result = pin(root, "a.md", &real()).expect("pin runs");
    let PinResult::Refused(refusal) = result else {
        panic!("expected a refusal for {declared_ref}: {result:?}");
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
