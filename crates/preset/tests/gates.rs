//! U5.3 merge gate — the named preset + session-birth fixtures (plan Block 5
//! U5.3 Test: line), each driving the REAL U2.6 guarded create over an on-disk
//! workspace (no in-memory double — fidelity is the point):
//!
//! - Gate 1: `unfold_births_every_scaffold_file_with_a_birth_receipt`
//!   (the sweep asserts the FULL declared scaffold set, not a sample).
//! - Gate 2: `def_invalid_new_refuses_naming_the_def_rule`
//!   (the refusal code conforms to the closed §8 taxonomy — row 17 `def_invalid`).
//! - Gate 3: `unfold_on_an_existing_path_refuses_via_cas_if_absent`
//!   (guarded-create semantics hold under unfold — no unguarded clobber).
//!
//! Plus: the preset pins the convention floor + the U2.11 block-sequence grain
//! round-trips the born root record's `inputs` pin.

use model::{Ref, resolve};
use preset::{BirthOptions, FileOutcome, NewOutcome, load_def, new_record, pins_floor, unfold};

/// A valid session preset: `inputs` pin the convention floor, `^properties` are
/// satisfiable by the `^template`, and `# Unfold` declares a four-file scaffold.
const SESSION_PRESET: &str = r#"---
type: def
defines: session
root: SESSION.md
inputs:
  - "conventions/reviewer-not-owner/CHECK.md@rev-a"
  - "conventions/claim-cas/CHECK.md@rev-b"
---

# Properties ^properties

- `type` required
- `preset` required

# Template ^template

```record
---
type: session
preset: presets/session.md
status: todo
id: {{id}}
---

# {{id}}
```

# Unfold

- SESSION.md
- tasks/index.md
- agents/index.md
- results/notes.md
"#;

/// An INVALID session preset: `^properties` requires `owner`, but the `^template`
/// frontmatter never declares it — no record this def can birth satisfies its own
/// `^properties`, so `mrd new` refuses `def_invalid` naming the `owner` rule.
const BROKEN_PRESET: &str = r#"---
type: def
defines: session
inputs:
  - "conventions/reviewer-not-owner/CHECK.md@rev-a"
---

# Properties ^properties

- `type` required
- `owner` required

# Template ^template

```record
---
type: session
preset: presets/broken.md
id: {{id}}
---

# {{id}}
```

# Unfold

- SESSION.md
"#;

/// The four files [`SESSION_PRESET`]'s `# Unfold` declares — the full scaffold
/// set the birth-receipt sweep must cover.
const SCAFFOLD: [&str; 4] = [
    "SESSION.md",
    "tasks/index.md",
    "agents/index.md",
    "results/notes.md",
];

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    for (name, body) in files {
        let path = tmp.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn journal_text(root: &fs::WorkspaceRoot) -> String {
    let path = root.0.join(fs::domain::RESERVED_JOURNAL_PATH);
    std::fs::read_to_string(path).unwrap_or_default()
}

fn opts() -> BirthOptions {
    BirthOptions {
        actor: Some("preset:test".to_owned()),
        now: Some("2026-07-23T12:00:00Z".to_owned()),
        dry: false,
    }
}

// ---------------------------------------------------------------------------
// Gate 1: unfold births every scaffold file, and every one carries a birth
//         receipt (the FULL declared set, swept — not a sample).
// ---------------------------------------------------------------------------

#[test]
fn unfold_births_every_scaffold_file_with_a_birth_receipt() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);

    let report = unfold(&root, "presets/session.md", &opts()).unwrap();

    // Every declared scaffold file was born — clean unfold, four births.
    assert!(
        report.is_clean(),
        "a fresh unfold births every file: {report:?}"
    );
    assert_eq!(report.files.len(), SCAFFOLD.len());

    // The report enumerates the FULL scaffold set, each with a birth receipt.
    let born: std::collections::BTreeMap<&str, Option<&str>> =
        report.births().into_iter().collect();
    for path in SCAFFOLD {
        let receipt = born
            .get(path)
            .unwrap_or_else(|| panic!("scaffold file {path} absent from the unfold report"));
        assert!(
            receipt.is_some(),
            "scaffold file {path} carries no birth receipt"
        );
    }

    // The birth receipt is REAL: the reserved journal carries a guarded `op=create`
    // (before=absent) row for EVERY scaffold file — the load-bearing sweep.
    let journal = journal_text(&root);
    for path in SCAFFOLD {
        let row = format!("op=create path={path} ");
        assert!(
            journal.contains(&row),
            "no birth receipt row for {path} in the journal:\n{journal}"
        );
    }
    assert_eq!(
        journal.matches("op=create").count(),
        SCAFFOLD.len(),
        "exactly one birth row per scaffold file"
    );
    // Every birth is the guarded shape (before=absent) — a real create door.
    assert_eq!(
        journal.matches(" before=absent ").count(),
        SCAFFOLD.len(),
        "every scaffold birth is a guarded create (before=absent):\n{journal}"
    );

    // The files exist on disk with the born bytes.
    for path in SCAFFOLD {
        assert!(root.0.join(path).exists(), "{path} was not materialized");
    }

    // Deliverable 1: the preset pins the convention floor, and the born root
    // record pins the PRESET as a multi-line block sequence read back through the
    // U2.11 whole-value grain (byte-stable, no orphan).
    assert_eq!(
        report.floor,
        vec![
            "conventions/reviewer-not-owner/CHECK.md@rev-a".to_owned(),
            "conventions/claim-cas/CHECK.md@rev-b".to_owned(),
        ]
    );
    let session = std::fs::read_to_string(root.0.join("SESSION.md")).unwrap();
    let doc = model::build(session.clone(), syntax::parse(&session));
    let grain = resolve(&doc, &Ref::FmKey("inputs".to_owned())).expect("root record pins inputs");
    let grain_text = &doc.raw[grain.span.clone()];
    assert!(
        grain_text.starts_with("inputs:\n  - \"presets/session.md@"),
        "the U2.11 grain spans the whole block-sequence pin: {grain_text:?}"
    );
    assert!(
        !grain_text.contains("---"),
        "the grain stops before the fence — no orphan: {grain_text:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: a def-invalid `new` refuses, naming the def rule, and writes nothing
//         (the refusal code conforms to the closed taxonomy — row 17).
// ---------------------------------------------------------------------------

#[test]
fn def_invalid_new_refuses_naming_the_def_rule() {
    let (_tmp, root) = workspace(&[("presets/broken.md", BROKEN_PRESET)]);

    let outcome = new_record(&root, "presets/broken.md", "s1", &opts()).unwrap();

    let NewOutcome::Refused(refusal) = &outcome else {
        panic!("an invalid def must refuse, got {outcome:?}");
    };
    // The refusal code conforms to the closed §8 taxonomy (row 17, recovery fix).
    assert_eq!(refusal.reason.code, "def_invalid");
    assert_eq!(refusal.reason.recovery, "fix");
    // The refusal NAMES the violated def rule (the `owner` requirement).
    let rule = refusal.reason.rule.as_deref().expect("names the def rule");
    assert!(
        rule.contains("owner"),
        "the refusal names the owner rule: {rule}"
    );
    assert!(
        refusal.reason.message.contains("owner"),
        "the message teaches the rule: {}",
        refusal.reason.message
    );

    // A refused birth writes NOTHING: no target file, no journal row.
    assert!(
        !root.0.join(&refusal.target).exists(),
        "a refused birth left a file at {}",
        refusal.target
    );
    assert!(
        !journal_text(&root).contains("op=create"),
        "a refused birth wrote a journal row"
    );
}

// ---------------------------------------------------------------------------
// Gate 3: unfold on an existing path refuses via the if_absent CAS — the
//         guarded-create semantics hold under unfold (no unguarded clobber).
// ---------------------------------------------------------------------------

#[test]
fn unfold_on_an_existing_path_refuses_via_cas_if_absent() {
    const SENTINEL: &str = "---\ntype: hand-authored\n---\n\n# do not clobber me\n";
    let (_tmp, root) = workspace(&[
        ("presets/session.md", SESSION_PRESET),
        ("tasks/index.md", SENTINEL),
    ]);

    let report = unfold(&root, "presets/session.md", &opts()).unwrap();

    // The unfold is NOT clean — one declared path already existed.
    assert!(!report.is_clean(), "an occupied path is a finding");

    // The occupied path refused via the if_absent CAS (guarded-create semantics).
    let occupied = report
        .files
        .iter()
        .find_map(|f| match f {
            FileOutcome::Occupied { path, reason } if path == "tasks/index.md" => Some(reason),
            _ => None,
        })
        .expect("tasks/index.md refused via the CAS");
    assert_eq!(occupied.code, "cas_mismatch");
    assert_eq!(occupied.recovery, "refresh");

    // The pre-existing bytes are UNTOUCHED — the guard prevented a clobber.
    assert_eq!(
        std::fs::read_to_string(root.0.join("tasks/index.md")).unwrap(),
        SENTINEL,
        "the guarded create must not clobber an existing file"
    );
    // No birth receipt was minted for the occupied path.
    let journal = journal_text(&root);
    assert!(
        !journal.contains("op=create path=tasks/index.md "),
        "no birth row for the occupied path:\n{journal}"
    );
    // The other declared paths WERE born (reconcile materializes the missing delta).
    for path in ["SESSION.md", "agents/index.md", "results/notes.md"] {
        assert!(root.0.join(path).exists(), "{path} should still be born");
        assert!(
            journal.contains(&format!("op=create path={path} ")),
            "{path} birth receipt missing"
        );
    }
}

// ---------------------------------------------------------------------------
// A valid `new` births the first rev through the guarded create (the happy
// path the def-invalid gate is the mirror of).
// ---------------------------------------------------------------------------

#[test]
fn valid_new_births_the_first_rev_with_a_receipt() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);

    let outcome = new_record(&root, "presets/session.md", "s1", &opts()).unwrap();
    let NewOutcome::Born(report) = &outcome else {
        panic!("a valid def must birth, got {outcome:?}");
    };
    assert_eq!(report.target, "session/s1.md");
    assert!(
        report.receipt.is_some(),
        "born record carries a birth receipt"
    );

    // The record exists with the filled template, and the journal carries its
    // guarded birth row.
    let body = std::fs::read_to_string(root.0.join("session/s1.md")).unwrap();
    assert!(
        body.contains("id: s1"),
        "the template filled {{id}}: {body}"
    );
    assert!(
        journal_text(&root).contains("op=create path=session/s1.md "),
        "birth receipt row missing"
    );

    // A second birth at the same target refuses via the if_absent CAS.
    let again = new_record(&root, "presets/session.md", "s1", &opts()).unwrap();
    let NewOutcome::Refused(refusal) = &again else {
        panic!("a re-birth must refuse, got {again:?}");
    };
    assert_eq!(refusal.reason.code, "cas_mismatch");
}

// ---------------------------------------------------------------------------
// The preset pins the convention floor (deliverable 1, read-only check).
// ---------------------------------------------------------------------------

#[test]
fn session_preset_pins_the_convention_floor() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);
    let def = load_def(&root, "presets/session.md").unwrap();
    assert!(
        pins_floor(&def),
        "the preset inputs pin the conventions/ floor"
    );
    assert_eq!(def.defines, "session");
    assert_eq!(def.scaffold.len(), 4);
}
