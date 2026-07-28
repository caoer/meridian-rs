//! G2 gates — `mrd journal genesis`: the governed reset of the receipt journal.
//!
//! The card's own standing question is asked here as a test rather than as
//! prose: *does this instrument include itself in its own population?* The
//! second-genesis gate is that question's answer, and it is the one that proves
//! the archive chain is walkable.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// A workspace with a declared root and a journal carrying `rows` rows.
fn workspace(rows: usize) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("MERIDIAN.md"),
        "---\ntype: meridian-root\nname: g2\n---\n\n# g2\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("meridian")).unwrap();
    let mut journal = String::new();
    for i in 1..=rows {
        let _ = writeln!(
            journal,
            "- op=splice path=page{i}.md root_before=b3:r{i} root_after=b3:r{} edits=0 ^r-{i:06}",
            i + 1
        );
        std::fs::write(tmp.path().join(format!("page{i}.md")), format!("# p{i}\n")).unwrap();
    }
    std::fs::write(tmp.path().join("meridian/journal.md"), journal).unwrap();
    tmp
}

fn mrd(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("mrd runs")
}

fn journal_of(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("meridian/journal.md")).unwrap()
}

/// Every domain file's bytes — the "did anything attested move" question, asked
/// without depending on any engine output. (The lane has been burned by a green
/// assertion that compared two empty strings.)
fn domain_digest(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let root = fs::WorkspaceRoot(dir.to_path_buf());
    let domain = fs::domain::Domain::load(&root).unwrap();
    fs::hash_domain(&root, &domain)
        .unwrap()
        .into_iter()
        .map(|rel| {
            let bytes = std::fs::read(dir.join(&rel)).unwrap_or_default();
            (rel.to_string_lossy().into_owned(), bytes)
        })
        .collect()
}

/// The whole act: rows move to the archive, the journal truncates, and the new
/// chain opens with a row naming where they went.
#[test]
fn genesis_archives_every_row_and_opens_the_new_chain() {
    let tmp = workspace(3);
    let dir = tmp.path();
    let before = journal_of(dir);

    let out = mrd(dir, &["journal", "genesis", "--ruling", "Ruling A"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The new journal is EXACTLY one row, and it is the genesis.
    let after = journal_of(dir);
    let rows = receipt::journal::parse_rows(&after);
    assert_eq!(rows.len(), 1, "the new chain opens with one row: {after}");
    assert_eq!(rows[0].op, "genesis");
    assert_eq!(rows[0].anchor, "r-000001");

    // The pointer: path names the archive, and the archive exists.
    let archive_rel = rows[0].path.clone();
    assert!(
        archive_rel.starts_with("meridian/journal-archive-"),
        "{archive_rel}"
    );
    let archive = std::fs::read_to_string(dir.join(&archive_rel)).expect("archive written");

    // Rows are MOVED, never destroyed — set equality on the rendered rows, not
    // a count (a count passes for three copies of one row).
    let moved: Vec<_> = receipt::journal::parse_rows(&archive)
        .into_iter()
        .map(|r| r.anchor)
        .collect();
    let original: Vec<_> = receipt::journal::parse_rows(&before)
        .into_iter()
        .map(|r| r.anchor)
        .collect();
    assert_eq!(moved, original, "every row survives, in order");

    // The archive carries the justification the verb refused to invent.
    assert!(archive.contains("genesis.ruling: 'Ruling A'"), "{archive}");
    assert!(archive.contains("status: superseded"), "{archive}");

    // The roots bracket a REAL domain change: the archive page is in the hash
    // domain, so the two roots must differ. This is the assertion that would
    // catch the row becoming fiction.
    assert_ne!(
        rows[0].root_before, rows[0].root_after,
        "the archive's creation moves the root; equal roots would mean the row lies"
    );
}

/// The standing question, as a gate: a SECOND genesis archives the FIRST
/// genesis row. That is what makes the chain of archives walkable — and the
/// oldest archive, holding no genesis row, is the terminator.
#[test]
fn a_second_genesis_archives_the_first_and_links_the_chain() {
    let tmp = workspace(2);
    let dir = tmp.path();

    let first = mrd(dir, &["journal", "genesis", "--ruling", "R1"]);
    assert!(first.status.success());
    let archive1 = receipt::journal::parse_rows(&journal_of(dir))[0]
        .path
        .clone();

    // A second reset, onto its own archive (same day ⇒ the default path is
    // taken, which is exactly why --archive exists).
    let second = mrd(
        dir,
        &[
            "journal",
            "genesis",
            "--ruling",
            "R2",
            "--archive",
            "meridian/arch2.md",
        ],
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );

    // The live journal points at archive 2 …
    let live = receipt::journal::parse_rows(&journal_of(dir));
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].path, "meridian/arch2.md");

    // … archive 2 holds the FIRST genesis row, which points at archive 1 …
    let a2 = std::fs::read_to_string(dir.join("meridian/arch2.md")).unwrap();
    let a2_rows = receipt::journal::parse_rows(&a2);
    let genesis_in_a2: Vec<_> = a2_rows.iter().filter(|r| r.op == "genesis").collect();
    assert_eq!(genesis_in_a2.len(), 1, "exactly one genesis row: {a2}");
    assert_eq!(genesis_in_a2[0].path, archive1, "it names its predecessor");

    // … and archive 1 holds NO genesis row. The absence is the terminator.
    let a1 = std::fs::read_to_string(dir.join(&archive1)).unwrap();
    assert!(
        !receipt::journal::parse_rows(&a1)
            .iter()
            .any(|r| r.op == "genesis"),
        "the oldest archive terminates the chain by holding no genesis row: {a1}"
    );
}

/// The refusal that makes the justification structural rather than cultural.
#[test]
fn genesis_without_a_ruling_refuses_and_writes_nothing() {
    let tmp = workspace(2);
    let dir = tmp.path();
    let before = domain_digest(dir);
    let journal_before = journal_of(dir);

    let out = mrd(dir, &["journal", "genesis"]);
    assert_eq!(out.status.code(), Some(2), "an invocation fault");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--ruling is required"), "{stderr}");

    assert_eq!(
        before,
        domain_digest(dir),
        "a refusal moves no attested byte"
    );
    assert_eq!(journal_before, journal_of(dir), "and no journal byte");
}

/// Two resets onto one archive is a mistake, not a batch.
#[test]
fn genesis_refuses_an_existing_archive_and_leaves_it_alone() {
    let tmp = workspace(2);
    let dir = tmp.path();
    std::fs::write(dir.join("meridian/taken.md"), "# already here\n").unwrap();
    let before = domain_digest(dir);

    let out = mrd(
        dir,
        &[
            "journal",
            "genesis",
            "--ruling",
            "R",
            "--archive",
            "meridian/taken.md",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "the plane refused");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already exists"), "{stderr}");
    assert_eq!(before, domain_digest(dir), "the existing page is untouched");
}

/// An empty journal has nothing to archive, and a genesis over it would record
/// an act that did not happen.
#[test]
fn genesis_over_an_empty_journal_refuses() {
    let tmp = workspace(0);
    let dir = tmp.path();
    let out = mrd(dir, &["journal", "genesis", "--ruling", "R"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("nothing to archive"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `--dry` is the reviewable arm: it must describe the act and perform none of
/// it.
#[test]
fn a_dry_genesis_writes_nothing_at_all() {
    let tmp = workspace(3);
    let dir = tmp.path();
    let before = domain_digest(dir);
    let journal_before = journal_of(dir);

    let out = mrd(dir, &["journal", "genesis", "--ruling", "R", "--dry"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("nothing written"), "{stdout}");
    assert!(
        stdout.contains("3 row(s)"),
        "the plan names the count: {stdout}"
    );

    assert_eq!(before, domain_digest(dir), "dry writes no attested byte");
    assert_eq!(journal_before, journal_of(dir), "and truncates nothing");
    assert!(
        std::fs::read_dir(dir.join("meridian"))
            .unwrap()
            .filter_map(Result::ok)
            .all(|e| !e
                .file_name()
                .to_string_lossy()
                .starts_with("journal-archive-")),
        "and creates no archive"
    );
}

/// The reset renders GREY, not green — said in the verb's own output, because
/// anyone not pre-told will read grey as failure.
#[test]
fn the_output_says_the_chain_is_grey_not_green() {
    let tmp = workspace(1);
    let dir = tmp.path();
    let out = mrd(dir, &["journal", "genesis", "--ruling", "R"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("grey(no-baseline)"), "{stdout}");
    assert!(stdout.contains("NOT green"), "{stdout}");
}

/// `--json` carries the same facts as the text, including the pointer.
#[test]
fn the_json_surface_carries_the_archive_and_the_count() {
    let tmp = workspace(4);
    let dir = tmp.path();
    let out = mrd(dir, &["journal", "genesis", "--ruling", "R", "--json"]);
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one json object");
    assert_eq!(value["rows_archived"], 4);
    assert_eq!(value["ruling"], "R");
    assert_eq!(value["chain_after"], "grey(no-baseline)");
    assert!(
        value["archive"]
            .as_str()
            .unwrap()
            .starts_with("meridian/journal-archive-"),
        "{value}"
    );
}
