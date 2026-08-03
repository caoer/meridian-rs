//! U5.3 merge gate — the named preset + session-birth fixtures (plan Block 5
//! U5.3 Test: line), each driving the REAL U2.6 guarded create over an on-disk
//! workspace (no in-memory double — fidelity is the point):
//!
//! - Gate 1: `unfold_births_every_scaffold_file`
//!   (the sweep asserts the FULL declared scaffold set, not a sample).
//! - Gate 2: `def_invalid_new_refuses_naming_the_def_rule`
//!   (the refusal code conforms to the closed §8 taxonomy — row 17 `def_invalid`).
//! - Gate 3: `unfold_on_an_existing_path_refuses_via_cas_if_absent`
//!   (guarded-create semantics hold under unfold — no unguarded clobber).
//!
//! Plus: the preset pins the convention floor + the U2.11 block-sequence grain
//! round-trips the born root record's `inputs` pin.

use model::{Ref, resolve};
use preset::{
    BirthOptions, FileOutcome, NewOutcome, PruneOutcome, load_def, new_record, pins_floor,
    reconcile, reconcile_plan, unfold,
};

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

fn opts() -> BirthOptions {
    BirthOptions {
        actor: Some("preset:test".to_owned()),
        now: Some("2026-07-23T12:00:00Z".to_owned()),
        dry: false,
    }
}

// ---------------------------------------------------------------------------
// Gate 1: unfold births every scaffold file — the FULL declared set, swept,
//         not a sample.
//
// The sweep asserted a birth RECEIPT per file (an `op=create before=absent`
// journal row) until the journal was removed (ZT 2026-08-02). The receipt is
// gone; what the guarded create still owes — one born file per declared scaffold
// entry, materialized on disk — is asserted below and is what the gate now means.
// ---------------------------------------------------------------------------

#[test]
fn unfold_births_every_scaffold_file() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);

    let report = unfold(&root, "presets/session.md", &opts()).unwrap();

    // Every declared scaffold file was born — clean unfold, four births.
    assert!(
        report.is_clean(),
        "a fresh unfold births every file: {report:?}"
    );
    assert_eq!(report.files.len(), SCAFFOLD.len());

    // The report enumerates the FULL scaffold set — every declared entry born.
    let born: std::collections::BTreeSet<&str> = report.births().into_iter().collect();
    for path in SCAFFOLD {
        assert!(
            born.contains(path),
            "scaffold file {path} absent from the unfold report"
        );
    }
    assert_eq!(
        born.len(),
        SCAFFOLD.len(),
        "exactly one birth per scaffold file"
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

    // A refused birth writes NOTHING: no target file.
    assert!(
        !root.0.join(&refusal.target).exists(),
        "a refused birth left a file at {}",
        refusal.target
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
    // The other declared paths WERE born (reconcile materializes the missing delta).
    for path in ["SESSION.md", "agents/index.md", "results/notes.md"] {
        assert!(root.0.join(path).exists(), "{path} should still be born");
    }
}

// ---------------------------------------------------------------------------
// A valid `new` births the first rev through the guarded create (the happy
// path the def-invalid gate is the mirror of).
// ---------------------------------------------------------------------------

#[test]
fn valid_new_births_the_first_rev() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);

    let outcome = new_record(&root, "presets/session.md", "s1", &opts()).unwrap();
    let NewOutcome::Born(report) = &outcome else {
        panic!("a valid def must birth, got {outcome:?}");
    };
    assert_eq!(report.target, "session/s1.md");

    // The record exists with the filled template.
    let body = std::fs::read_to_string(root.0.join("session/s1.md")).unwrap();
    assert!(
        body.contains("id: s1"),
        "the template filled {{id}}: {body}"
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

// ---------------------------------------------------------------------------
// U3.5b reconcile-toward-scaffold (ZT ruling #3, the asymmetric reconcile law).
// The three fixture assertions (plan §4 Block 3 U3.5b Test: line):
//   1. a missing declared path is materialized;
//   2. an undeclared NON-empty path is untouched and rendered as a finding;
//   3. a declared-ephemeral file is pruned.
// ---------------------------------------------------------------------------

/// A reconcile preset: `# Unfold` declares a three-file scaffold, `# Ephemeral`
/// marks `*.lock` disposable (the prune allowlist).
const RECONCILE_PRESET: &str = r#"---
type: def
defines: session
root: SESSION.md
inputs:
  - "conventions/reviewer-not-owner/CHECK.md@rev-a"
---

# Unfold

- SESSION.md
- tasks/index.md
- results/plan.md

# Ephemeral

- *.lock
"#;

#[test]
fn reconcile_plan_is_asymmetric_materialize_by_diff_prune_by_allowlist() {
    // A PURE-fold gate: declared scaffold vs a live tree.
    let declared = vec![
        "SESSION.md".to_owned(),
        "tasks/index.md".to_owned(),
        "results/plan.md".to_owned(),
    ];
    let ephemeral = vec!["*.lock".to_owned()];
    let live = vec![
        "SESSION.md".to_owned(),       // declared + present → converged
        "tasks/index.md".to_owned(),   // declared + present → converged
        "results/notes.md".to_owned(), // UNDECLARED content → finding, never pruned
        "tasks/build.lock".to_owned(), // declared-ephemeral → prune
    ];

    let plan = reconcile_plan(&declared, &ephemeral, &live);

    // 1. the missing declared path is the additive set-difference.
    assert_eq!(plan.materialize, vec!["results/plan.md".to_owned()]);
    // 3. the declared-ephemeral file is the ONLY prune (allowlist, not diff).
    assert_eq!(plan.prune, vec!["tasks/build.lock".to_owned()]);
    // 2. the undeclared content file is a finding, NEVER a prune action.
    assert_eq!(plan.findings, vec!["results/notes.md".to_owned()]);
}

#[test]
fn reconcile_materializes_missing_untouches_undeclared_prunes_ephemeral() {
    // A live tree that exercises all three fold branches at once. `results/plan.md`
    // is declared but ABSENT; `results/notes.md` is undeclared NON-empty content;
    // `tasks/build.lock` is declared-ephemeral. SESSION.md + tasks/index.md already
    // exist (converged, left byte-untouched).
    const NOTES_BODY: &str = "---\ntype: note\n---\n\n# undeclared work product\n";
    let (_tmp, root) = workspace(&[
        ("presets/session.md", RECONCILE_PRESET),
        ("SESSION.md", "---\ntype: session\n---\n\n# S\n"),
        ("tasks/index.md", "---\ntype: index\n---\n\n# I\n"),
        ("results/notes.md", NOTES_BODY),
        ("tasks/build.lock", "stale-lock\n"),
    ]);

    let report = reconcile(&root, "presets/session.md", true, &opts()).unwrap();

    // 1. the missing declared path was MATERIALIZED through the guarded create.
    let materialized: std::collections::BTreeMap<&str, bool> = report
        .materialized
        .iter()
        .map(|f| match f {
            FileOutcome::Born { path } => (path.as_str(), true),
            FileOutcome::Occupied { path, .. } => (path.as_str(), false),
        })
        .collect();
    assert_eq!(
        materialized.get("results/plan.md"),
        Some(&true),
        "the missing declared path is materialized: {report:?}"
    );
    assert!(
        root.0.join("results/plan.md").exists(),
        "results/plan.md was not materialized on disk"
    );
    // The already-present declared paths were NOT re-materialized (occupied is not
    // in the plan; only the genuinely missing path is).
    assert_eq!(
        report.materialized.len(),
        1,
        "exactly one missing declared path"
    );

    // 2. the undeclared NON-empty path is UNTOUCHED and rendered as a FINDING.
    assert_eq!(
        report.findings,
        vec!["results/notes.md".to_owned()],
        "undeclared content is a finding, never a prune: {report:?}"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("results/notes.md")).unwrap(),
        NOTES_BODY,
        "the undeclared file's bytes were left untouched"
    );

    // 3. the declared-ephemeral file was PRUNED (guarded remove) and is gone.
    let pruned: Vec<&str> = report
        .pruned
        .iter()
        .filter_map(|p| match p {
            PruneOutcome::Removed { path, .. } => Some(path.as_str()),
            PruneOutcome::Refused { .. } => None,
        })
        .collect();
    assert_eq!(
        pruned,
        vec!["tasks/build.lock"],
        "the declared-ephemeral file is the only prune: {report:?}"
    );
    assert!(
        !root.0.join("tasks/build.lock").exists(),
        "the ephemeral lock file was not pruned"
    );
}

#[test]
fn reconcile_without_prune_leaves_ephemeral_in_place() {
    // Prune is opt-in: bare `mrd reconcile` materializes but removes nothing.
    let (_tmp, root) = workspace(&[
        ("presets/session.md", RECONCILE_PRESET),
        ("SESSION.md", "---\ntype: session\n---\n\n# S\n"),
        ("tasks/index.md", "---\ntype: index\n---\n\n# I\n"),
        ("tasks/build.lock", "stale-lock\n"),
    ]);

    let report = reconcile(&root, "presets/session.md", false, &opts()).unwrap();

    assert!(report.pruned.is_empty(), "no prune without --prune");
    assert!(
        root.0.join("tasks/build.lock").exists(),
        "the ephemeral file survives a no-prune reconcile"
    );
    // The missing declared path is still materialized (additive is unconditional).
    assert!(root.0.join("results/plan.md").exists());
}
