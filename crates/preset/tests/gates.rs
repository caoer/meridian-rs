//! U5.3 merge gate — preset + session-birth fixtures, each driving the real
//! U2.6 guarded create over an on-disk workspace (no in-memory double).

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
// Gate 1: unfold births every scaffold file — the full declared set, swept.
// ---------------------------------------------------------------------------

#[test]
fn unfold_births_every_scaffold_file() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);

    let report = unfold(&root, "presets/session.md", &opts()).unwrap();

    assert!(
        report.is_clean(),
        "a fresh unfold births every file: {report:?}"
    );
    assert_eq!(report.files.len(), SCAFFOLD.len());

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

    for path in SCAFFOLD {
        assert!(root.0.join(path).exists(), "{path} was not materialized");
    }

    // The preset pins the convention floor, and the born root record pins the
    // preset as a block sequence read back through the U2.11 grain.
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
    // Law 6.2: the def pin rides FIRST, then every declared floor pin in
    // declared order — the floor is readable from the session itself, not only
    // through the def blob.
    for pin in &report.floor {
        assert!(
            grain_text.contains(&format!("\n  - \"{pin}\"")),
            "the born root record drops the declared floor pin {pin}: {grain_text:?}"
        );
    }
    assert!(
        grain_text.find("conventions/reviewer-not-owner")
            < grain_text.find("conventions/claim-cas"),
        "the floor pins ride in declared order: {grain_text:?}"
    );
    assert!(
        !session.contains("the convention floor page"),
        "Law 6.1: a floor pin is a pin, never a copy of the floor's content"
    );
    // Law 3.5: `preset:` names the DEF, never the record itself.
    assert!(
        session.contains("\npreset: presets/session.md\n"),
        "the root record names the def it was born from: {session:?}"
    );
    assert!(
        session.contains("# session — born from presets/session.md"),
        "the provenance heading names the def: {session:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: a def-invalid `new` refuses, naming the def rule, and writes nothing.
// ---------------------------------------------------------------------------

#[test]
fn def_invalid_new_refuses_naming_the_def_rule() {
    let (_tmp, root) = workspace(&[("presets/broken.md", BROKEN_PRESET)]);

    let outcome = new_record(&root, "presets/broken.md", "s1", &opts()).unwrap();

    let NewOutcome::Refused(refusal) = &outcome else {
        panic!("an invalid def must refuse, got {outcome:?}");
    };
    // Closed §8 taxonomy: row 17, recovery fix; the refusal names the rule.
    assert_eq!(refusal.reason.code, "def_invalid");
    assert_eq!(refusal.reason.recovery, "fix");
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

    // A refused birth writes nothing.
    assert!(
        !root.0.join(&refusal.target).exists(),
        "a refused birth left a file at {}",
        refusal.target
    );
}

// ---------------------------------------------------------------------------
// Gate 3: unfold on an existing path refuses via the if_absent CAS.
// ---------------------------------------------------------------------------

#[test]
fn unfold_on_an_existing_path_refuses_via_cas_if_absent() {
    const SENTINEL: &str = "---\ntype: hand-authored\n---\n\n# do not clobber me\n";
    let (_tmp, root) = workspace(&[
        ("presets/session.md", SESSION_PRESET),
        ("tasks/index.md", SENTINEL),
    ]);

    let report = unfold(&root, "presets/session.md", &opts()).unwrap();

    assert!(!report.is_clean(), "an occupied path is a finding");

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

    assert_eq!(
        std::fs::read_to_string(root.0.join("tasks/index.md")).unwrap(),
        SENTINEL,
        "the guarded create must not clobber an existing file"
    );
    // The other declared paths WERE born.
    for path in ["SESSION.md", "agents/index.md", "results/notes.md"] {
        assert!(root.0.join(path).exists(), "{path} should still be born");
    }
}

// ---------------------------------------------------------------------------
// A valid `new` births the first rev through the guarded create.
// ---------------------------------------------------------------------------

#[test]
fn valid_new_births_the_first_rev() {
    let (_tmp, root) = workspace(&[("presets/session.md", SESSION_PRESET)]);

    let outcome = new_record(&root, "presets/session.md", "s1", &opts()).unwrap();
    let NewOutcome::Born(report) = &outcome else {
        panic!("a valid def must birth, got {outcome:?}");
    };
    assert_eq!(report.target, "session/s1.md");

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
// The preset pins the convention floor (read-only check).
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
// U3.5b reconcile-toward-scaffold (ruling #3, the asymmetric reconcile law):
//   1. a missing declared path is materialized;
//   2. an undeclared non-empty path is untouched and rendered as a finding;
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

    // 1. missing declared path → additive set-difference.
    assert_eq!(plan.materialize, vec!["results/plan.md".to_owned()]);
    // 3. declared-ephemeral file → the only prune (allowlist, not diff).
    assert_eq!(plan.prune, vec!["tasks/build.lock".to_owned()]);
    // 2. undeclared content file → finding, never a prune action.
    assert_eq!(plan.findings, vec!["results/notes.md".to_owned()]);
}

#[test]
fn reconcile_materializes_missing_untouches_undeclared_prunes_ephemeral() {
    // A live tree exercising all three fold branches at once.
    const NOTES_BODY: &str = "---\ntype: note\n---\n\n# undeclared work product\n";
    let (_tmp, root) = workspace(&[
        ("presets/session.md", RECONCILE_PRESET),
        ("SESSION.md", "---\ntype: session\n---\n\n# S\n"),
        ("tasks/index.md", "---\ntype: index\n---\n\n# I\n"),
        ("results/notes.md", NOTES_BODY),
        ("tasks/build.lock", "stale-lock\n"),
    ]);

    let report = reconcile(&root, "presets/session.md", true, &opts()).unwrap();

    // 1. the missing declared path was materialized through the guarded create.
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
    // The already-present declared paths were not re-materialized.
    assert_eq!(
        report.materialized.len(),
        1,
        "exactly one missing declared path"
    );

    // 2. the undeclared non-empty path is untouched and rendered as a finding.
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

    // 3. the declared-ephemeral file was pruned (guarded remove) and is gone.
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

// ---------------------------------------------------------------------------
// The directory half of the prune allowlist (§5.3). Regression: deriving the
// candidate set from the declared paths made `pruned_dirs` empty on every
// input — these assertions were once vacuous.
// ---------------------------------------------------------------------------

#[test]
fn an_empty_undeclared_dir_under_the_scaffold_is_pruned() {
    let (_tmp, root) = workspace(&[
        ("presets/session.md", RECONCILE_PRESET),
        ("SESSION.md", "---\ntype: session\n---\n\n# S\n"),
        ("tasks/index.md", "---\ntype: index\n---\n\n# I\n"),
        ("results/plan.md", "---\ntype: plan\n---\n\n# P\n"),
    ]);
    // A nest of empty dirs beneath a scaffold directory collapses in one pass,
    // deepest-first.
    std::fs::create_dir_all(root.0.join("tasks/scratch/inner")).unwrap();

    let report = reconcile(&root, "presets/session.md", true, &opts()).unwrap();

    assert_eq!(
        report.pruned_dirs,
        vec!["tasks/scratch/inner".to_owned(), "tasks/scratch".to_owned()],
        "the empty nest is pruned deepest-first: {report:?}"
    );
    assert!(
        !root.0.join("tasks/scratch").exists(),
        "the empty undeclared dir is gone from disk"
    );
    assert!(
        root.0.join("tasks/index.md").exists(),
        "the scaffold directory itself is never pruned"
    );
}

#[test]
fn a_dir_holding_content_or_a_declared_path_is_never_pruned() {
    let (_tmp, root) = workspace(&[
        ("presets/session.md", RECONCILE_PRESET),
        ("SESSION.md", "---\ntype: session\n---\n\n# S\n"),
        ("tasks/index.md", "---\ntype: index\n---\n\n# I\n"),
        ("results/plan.md", "---\ntype: plan\n---\n\n# P\n"),
        // Undeclared content under an undeclared dir — the dir stands.
        ("tasks/keep/work.md", "---\ntype: note\n---\n\n# mine\n"),
    ]);

    let report = reconcile(&root, "presets/session.md", true, &opts()).unwrap();

    assert!(
        report.pruned_dirs.is_empty(),
        "a dir holding content is never pruned: {report:?}"
    );
    assert!(root.0.join("tasks/keep/work.md").exists());
    // Every declared directory survives, unconditionally.
    for declared_dir in ["tasks", "results"] {
        assert!(
            root.0.join(declared_dir).is_dir(),
            "{declared_dir} is a scaffold directory and must stand"
        );
    }
}

#[test]
fn the_workspace_root_is_never_walked_for_directory_candidates() {
    // A scaffold of top-level files creates no directory, so it offers no
    // candidate — otherwise every empty dir in the workspace would be a prune
    // target under `--prune`.
    const TOP_LEVEL_PRESET: &str = r#"---
type: def
defines: session
root: SESSION.md
inputs:
  - "conventions/reviewer-not-owner/CHECK.md@rev-a"
---

# Unfold

- SESSION.md

# Ephemeral

- *.lock
"#;
    let (_tmp, root) = workspace(&[
        ("presets/top.md", TOP_LEVEL_PRESET),
        ("SESSION.md", "---\ntype: session\n---\n\n# S\n"),
    ]);
    std::fs::create_dir_all(root.0.join("unrelated-empty")).unwrap();

    let report = reconcile(&root, "presets/top.md", true, &opts()).unwrap();

    assert!(
        report.pruned_dirs.is_empty(),
        "the workspace root is not this shape's territory: {report:?}"
    );
    assert!(
        root.0.join("unrelated-empty").is_dir(),
        "an empty dir outside the scaffold's territory must survive"
    );
}
