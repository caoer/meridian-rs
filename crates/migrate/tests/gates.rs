//! U3.2 migrate kit — the Test scenario set (the merge gate). Named results:
//!
//! - `renames_key_preserves_value_and_mints_one_r_row` — a `draws-from:` page
//!   becomes `inputs:` with the VALUE bytes byte-identical; ONE `op=migrate`
//!   `^r-NNNNNN` wire audit row is minted and ZERO attestation receipts.
//! - `value_containing_draws_from_survives` — only the KEY token renames; a ref
//!   VALUE that itself contains `draws-from` is byte-preserved.
//! - `dry_run_reports_but_writes_nothing` — a dry run reports the rename and
//!   touches neither the page nor the journal.
//! - `idempotent_re_run_mints_nothing` — a second run over the migrated corpus
//!   renames 0 pages and appends 0 rows.
//! - `resumable_re_run_migrates_only_the_remainder` — a page added after a run
//!   is the only one the next run renames (already-`inputs:` pages are skipped).
//! - `both_keys_is_conflict_refused` — a page carrying BOTH keys is reported a
//!   conflict, written nothing, minting no row.
//! - `migrate_rows_chain_and_carry_no_attestation_receipt` — every row is an
//!   `op=migrate` journal row, the chain stays continuous, and no `- splice`
//!   attestation-receipt line is written anywhere.

use migrate::{MigrateOptions, PageOutcome, migrate_inputs};

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

/// Write one more file into an existing workspace (for the resume gate).
fn add_file(root: &fs::WorkspaceRoot, rel: &str, content: &str) {
    let full = root.0.join(rel);
    if let Some(p) = full.parent() {
        std::fs::create_dir_all(p).expect("mkdir");
    }
    std::fs::write(&full, content).expect("write fixture");
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

fn dry() -> MigrateOptions {
    MigrateOptions {
        dry: true,
        actor: Some("alice".into()),
        now: None,
    }
}
fn real() -> MigrateOptions {
    MigrateOptions {
        dry: false,
        actor: Some("alice".into()),
        now: None,
    }
}

// ---------------------------------------------------------------------------
// Gate 1 — rename the key, preserve the value bytes, mint ONE ^r row, ZERO
// attestation receipts.
// ---------------------------------------------------------------------------

/// A `draws-from:` block-sequence page renames to `inputs:` with the VALUE bytes
/// byte-identical (only the key token changes). Exactly ONE `op=migrate`
/// `^r-NNNNNN` row is minted; no attestation receipt file is written.
#[test]
fn renames_key_preserves_value_and_mints_one_r_row() {
    let before = "---\ntitle: Effect X\ndraws-from:\n  - \"[[substrate#^b1]]\"\n  - results/round2/design-1.md@d8536666b42dc8fd\n---\n\n# X\n\nbody\n";
    let (_d, root) = ws(&[("effects/x.md", before)]);

    let report = migrate_inputs(&root, &real()).expect("migrate runs");
    assert_eq!(report.renamed(), 1, "one page renamed");
    assert_eq!(report.conflicts(), 0);

    // Byte-for-byte: only `draws-from:` → `inputs:`, everything else identical.
    let expected = before.replacen("draws-from:", "inputs:", 1);
    assert_eq!(
        read(&root, "effects/x.md"),
        expected,
        "value bytes preserved"
    );

    // ONE ^r wire audit row, and it is an op=migrate row (not a receipt).
    assert_eq!(journal_rows(&root), 1, "one ^r row minted");
    assert!(
        journal(&root).contains("op=migrate"),
        "the row is an op=migrate wire audit line"
    );
    // The report carries the minted anchor in `r-NNNNNN` form.
    let receipts = report.receipts();
    assert_eq!(receipts.len(), 1);
    assert!(
        receipts[0].starts_with("r-"),
        "anchor is r-NNNNNN: {}",
        receipts[0]
    );
}

// ---------------------------------------------------------------------------
// Gate 2 — only the KEY renames; a value containing `draws-from` survives.
// ---------------------------------------------------------------------------

/// The rename touches the key token ALONE: a ref value that literally contains
/// the substring `draws-from` is byte-preserved (the migration is not a blind
/// text replace).
#[test]
fn value_containing_draws_from_survives() {
    let before = "---\ndraws-from:\n  - notes/draws-from-history.md\n  - \"[[draws-from-note]]\"\n---\n\n# P\n";
    let (_d, root) = ws(&[("p.md", before)]);

    migrate_inputs(&root, &real()).expect("migrate runs");
    let after = read(&root, "p.md");

    // The KEY renamed exactly once; both value occurrences of `draws-from` remain.
    assert!(after.starts_with("---\ninputs:\n"), "key renamed: {after}");
    assert!(
        after.contains("notes/draws-from-history.md"),
        "value ref preserved: {after}"
    );
    assert!(
        after.contains("\"[[draws-from-note]]\""),
        "value wikilink preserved: {after}"
    );
    assert_eq!(
        after,
        before.replacen("draws-from:", "inputs:", 1),
        "exactly one key rename, value bytes untouched"
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — dry run reports but writes nothing.
// ---------------------------------------------------------------------------

/// A dry run reports the rename it WOULD make, but the page and the journal are
/// both untouched (no bytes, no row).
#[test]
fn dry_run_reports_but_writes_nothing() {
    let before = "---\ndraws-from: []\n---\n\n# Pin-only\n";
    let (_d, root) = ws(&[("effects/skill.md", before)]);

    let report = migrate_inputs(&root, &dry()).expect("migrate runs");
    assert!(report.dry);
    assert_eq!(report.renamed(), 1, "one page WOULD rename");
    assert!(report.receipts().is_empty(), "dry mints no receipt anchors");

    assert_eq!(read(&root, "effects/skill.md"), before, "page untouched");
    assert_eq!(journal_rows(&root), 0, "no journal row on a dry run");
}

// ---------------------------------------------------------------------------
// Gate 4 — idempotent: a re-run over the migrated corpus mints nothing.
// ---------------------------------------------------------------------------

/// After a full migration every page carries `inputs:`; a second run finds no
/// `draws-from:` target, renames 0 pages, and appends 0 new rows (idempotent).
#[test]
fn idempotent_re_run_mints_nothing() {
    let (_d, root) = ws(&[
        ("a.md", "---\ndraws-from:\n  - b.md\n---\n\n# A\n"),
        ("b.md", "---\ndraws-from: [\"c.md\"]\n---\n\n# B\n"),
        ("c.md", "# C\n"),
    ]);

    let first = migrate_inputs(&root, &real()).expect("first run");
    assert_eq!(first.renamed(), 2, "both draws-from pages renamed");
    let rows_after_first = journal_rows(&root);
    assert_eq!(rows_after_first, 2, "two ^r rows");

    let second = migrate_inputs(&root, &real()).expect("second run");
    assert_eq!(second.renamed(), 0, "re-run renames nothing");
    assert_eq!(
        journal_rows(&root),
        rows_after_first,
        "re-run appends no new row (idempotent)"
    );
}

// ---------------------------------------------------------------------------
// Gate 5 — resumable: a page added after a run is the only one re-run touches.
// ---------------------------------------------------------------------------

/// A run migrates the corpus; a NEW `draws-from:` page appears (a resumed
/// corpus); the next run renames ONLY the new page (already-`inputs:` pages are
/// skipped), appending exactly one more row.
#[test]
fn resumable_re_run_migrates_only_the_remainder() {
    let (_d, root) = ws(&[("a.md", "---\ndraws-from:\n  - x.md\n---\n\n# A\n")]);

    let first = migrate_inputs(&root, &real()).expect("first run");
    assert_eq!(first.renamed(), 1);
    assert_eq!(journal_rows(&root), 1);

    // A page not yet migrated (or newly added) shows up.
    add_file(
        &root,
        "late.md",
        "---\ndraws-from: [\"a.md\"]\n---\n\n# Late\n",
    );

    let second = migrate_inputs(&root, &real()).expect("resumed run");
    assert_eq!(second.renamed(), 1, "only the remainder renames");
    assert_eq!(
        second.pages.len(),
        1,
        "the already-migrated page is out of scope"
    );
    assert!(matches!(&second.pages[0], PageOutcome::Renamed { path, .. } if path == "late.md"));
    assert_eq!(journal_rows(&root), 2, "one more row appended");
    // The first page kept its migrated form.
    assert!(read(&root, "a.md").contains("inputs:"));
}

// ---------------------------------------------------------------------------
// Gate 6 — both keys present is a refused conflict (never a silent merge).
// ---------------------------------------------------------------------------

/// A page carrying BOTH `draws-from:` and `inputs:` is a synonym-window defect:
/// reported a conflict, written nothing, minting no row.
#[test]
fn both_keys_is_conflict_refused() {
    let before = "---\ndraws-from:\n  - old.md\ninputs:\n  - new.md\n---\n\n# Both\n";
    let (_d, root) = ws(&[("both.md", before)]);

    let report = migrate_inputs(&root, &real()).expect("migrate runs");
    assert_eq!(report.renamed(), 0, "a conflict page is not renamed");
    assert_eq!(report.conflicts(), 1);
    assert!(matches!(&report.pages[0], PageOutcome::Conflict { path } if path == "both.md"));

    assert_eq!(read(&root, "both.md"), before, "conflict page untouched");
    assert_eq!(journal_rows(&root), 0, "no row for a refused conflict");
}

// ---------------------------------------------------------------------------
// Gate 7 — the rows are op=migrate wire audit lines that CHAIN, and NO
// attestation receipt is written anywhere.
// ---------------------------------------------------------------------------

/// Every minted row is an `op=migrate` journal row; the chain-continuity
/// detector stays green across them; and no `- splice` attestation-receipt line
/// is written to any file (attest is the sole receipt minter).
#[test]
fn migrate_rows_chain_and_carry_no_attestation_receipt() {
    let (_d, root) = ws(&[
        ("a.md", "---\ndraws-from:\n  - z.md\n---\n\n# A\n"),
        ("b.md", "---\ndraws-from: []\n---\n\n# B\n"),
        ("c.md", "---\ndraws-from:\n  - a.md\n---\n\n# C\n"),
    ]);

    let report = migrate_inputs(&root, &real()).expect("migrate runs");
    assert_eq!(report.renamed(), 3);

    let text = journal(&root);
    let rows = receipt::journal::parse_rows(&text);
    assert_eq!(rows.len(), 3, "three journal rows");
    assert!(
        rows.iter().all(|r| r.op == "migrate"),
        "every row is op=migrate"
    );

    // Chain continuity: root_after(N) == root_before(N+1) for the migrate rows.
    let report_chain = receipt::journal::check_chain(&rows);
    assert!(
        report_chain.is_green(),
        "migrate rows chain: {:?}",
        report_chain.red_summary()
    );

    // ZERO attestation receipts: no file carries a `- splice ` receipt line
    // (the receipt::render_line shape). Only op=migrate journal rows exist.
    for rel in ["a.md", "b.md", "c.md", fs::domain::RESERVED_JOURNAL_PATH] {
        let body = read(&root, rel);
        assert!(
            !body.contains("- splice "),
            "no attestation receipt line in {rel}: {body}"
        );
    }
}
