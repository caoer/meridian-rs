//! **THE SHAPE CORPUS** — the classifier's disposition for every SHAPE a lock
//! fence can take, pinned per shape rather than per instance.
//!
//! # Why shapes and not the field
//! The complementary proposal was to pin the parser-visible archive set as
//! exactly the six. That is a claim about THE FIELD — which blocks exist out
//! there — and it needs ZT's vaults, so CI is structurally blind to it and the
//! pin degrades to an `#[ignore]` somebody remembers. An unrun pin is a
//! decoration.
//!
//! **This is a claim about THE CLASSIFIER instead**: no parser-visible block of
//! any shape becomes a migration candidate unless it really is a page lock.
//! That is a property of this code, so it runs in CI on every commit, and it
//! covers pages nobody has written yet because it is keyed on SHAPE.
//!
//! Each row is a real on-disk vault driven through the production `sweep`.

use lockmigrate::{Options, PageVerdict, sweep};

const FP: &str = "fp1.span2.b3.a8222f5a4daeff2df5ffd61fbb7cb4ea00df7af479ef747b0f14a58666a2444d";

/// A v1 lock block — the shape the field archive carries.
fn v1() -> String {
    format!(
        "```meridian-lock\nversion: 1\nobjects:\n  \"t.md\": \"deadbeef\"\npins:\n  - ref: \"t.md#A\"\n    fingerprint: \"{FP}\"\n```"
    )
}

fn vault(body: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("page.md"), body).expect("write");
    std::fs::write(dir.path().join("t.md"), "# T\n\n## A\n\nx\n").expect("write");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "u9b@example.invalid"],
        vec!["config", "user.name", "u9b"],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(&args)
                .status()
                .expect("git")
                .success()
        );
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// What the classifier did with `page.md`, as one word.
fn disposition(body: &str) -> &'static str {
    let (_d, root) = vault(body);
    let report = sweep(
        &root,
        &Options {
            dry: true,
            ..Options::default()
        },
    )
    .expect("sweep runs");
    match report.pages.iter().find(|p| p.path() == "page.md") {
        None => "INVISIBLE",
        Some(PageVerdict::Migrated { .. }) => "MIGRATE",
        Some(PageVerdict::NotEnginePlaced { .. }) => "not-engine-placed",
        Some(PageVerdict::MultipleBlocks { .. }) => "refused:multiple",
        Some(PageVerdict::Unparseable { .. }) => "refused:unparseable",
        Some(PageVerdict::Unconvertible { .. }) => "refused:unconvertible",
        Some(PageVerdict::AlreadyV2 { .. }) => "already-v2",
    }
}

/// **THE PIN: one row per SHAPE, and exactly one shape may MIGRATE.**
///
/// The claim this makes, and it is the one that survives pages arriving: a
/// parser-visible lock block becomes a migration candidate ONLY when it is the
/// page's own engine-minted lock — top-level, terminal, single. Every other
/// shape is excluded or refused, and each exclusion is a RULE with a reason.
#[test]
fn every_shape_has_a_pinned_disposition() {
    let b = v1();
    let rows: &[(&str, String, &str)] = &[
        // THE ONE THAT MIGRATES: the engine's own page lock.
        (
            "top-level, terminal, single — a real page lock",
            format!("# Page\n\nbody\n\n{b}\n"),
            "MIGRATE",
        ),
        // ── Placement excludes ──
        (
            "top-level, prose after — an illustration in a document",
            format!("# Doc\n\n{b}\n\nThree defects in five lines.\n"),
            "not-engine-placed",
        ),
        (
            "many top-level illustrations, prose after — the trace shape",
            format!("# Trace\n\n{b}\n\nand:\n\n{b}\n\n...discussion.\n"),
            "not-engine-placed",
        ),
        // ── Arity refuses ──
        (
            "two blocks, last one terminal — sole-writer says one",
            format!("# P\n\n{b}\nmid\n\n{b}\n"),
            "refused:multiple",
        ),
        // ── ENCLOSURE: rule 0. Invisible today; the rule is what keeps the
        //    disposition correct if the parser's reach ever changes.
        (
            "ENCLOSED in ````text, terminal and single — THE HOLE rule 0 closes",
            format!("# Doc\n\n````text\nexample:\n\n{b}\n````\n"),
            "INVISIBLE",
        ),
        (
            "ENCLOSED in ````text, prose after",
            format!("# Doc\n\n````text\nexample:\n\n{b}\n````\n\nprose.\n"),
            "INVISIBLE",
        ),
        // ── Other spellings that reach the parser differently ──
        (
            "INDENTED four spaces — an indented code block",
            format!(
                "# Doc\n\nexample:\n\n{}\n",
                b.lines()
                    .map(|l| format!("    {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            "INVISIBLE",
        ),
        (
            "BLOCKQUOTED, prose after — quoted from somewhere else",
            format!(
                "# Doc\n\nZT wrote:\n\n{}\n\nprose after.\n",
                b.lines()
                    .map(|l| format!("> {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            "not-engine-placed",
        ),
        (
            "BLOCKQUOTED, terminal and single — quoted, but in lock position",
            format!(
                "# Doc\n\nZT wrote:\n\n{}\n",
                b.lines()
                    .map(|l| format!("> {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            "not-engine-placed",
        ),
        // ── No lock at all: the acceptance floor. Without this row a
        //    classifier that answered INVISIBLE to everything would pass.
        (
            "no lock block anywhere",
            "# Page\n\njust prose.\n".to_string(),
            "INVISIBLE",
        ),
    ];

    let mut wrong = Vec::new();
    for (shape, body, want) in rows {
        let got = disposition(body);
        println!("  {got:<20} {shape}");
        if got != *want {
            wrong.push(format!("\n  SHAPE: {shape}\n    want {want}, got {got}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "a shape's disposition changed — decide whether that is intended, then \
         re-pin it deliberately:{}",
        wrong.join("")
    );

    // THE PROPERTY, stated over the table rather than left implicit: exactly ONE
    // shape migrates. If a second ever does, this fails even when every row
    // above was individually updated.
    let migrating = rows
        .iter()
        .filter(|(_, body, _)| disposition(body) == "MIGRATE")
        .count();
    assert_eq!(
        migrating, 1,
        "exactly one shape — the engine's own page lock — may be a migration candidate"
    );
}
