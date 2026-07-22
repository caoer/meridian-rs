//! On-disk gates for the exec-window detection bracket (`fs::guard`, U6b):
//! residual-compare (#19), the config bracket (#20), symlink refusal (#25),
//! and the NAMED accepted gaps. Steps are simulated — the guard is
//! exec-independent, so no bash runs here; "the window" is the span between
//! `open` and `close` and the test plays both the executor and the attacker.

use std::path::Path;

use fs::WorkspaceRoot;
use fs::guard::{GovernedEdit, GuardError, StepGuard};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// A workspace nested one level under the tempdir, so tests have a real
/// OUT-OF-TREE location (`tmp/…` beside `tmp/ws/`) for symlink targets.
fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "notes/plan.md", "# Plan\n");
    write(&ws, "receipts/log.md", "# Receipts\n");
    let root = WorkspaceRoot(ws);
    (tmp, root)
}

fn edit(path: &str, bytes: &str) -> GovernedEdit {
    GovernedEdit {
        path: path.to_string(),
        bytes: bytes.as_bytes().to_vec(),
    }
}

/// A clean step: the governed edits are exactly what changed (one replace,
/// one create); close verifies and returns the post root — equal to an
/// independently folded merkle over pre + edits.
#[test]
fn clean_step_close_returns_verified_root() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    // simulate the executor's committed writes for this step
    write(&root.0, "notes/plan.md", "# Plan v2\n");
    write(&root.0, "notes/new.md", "born in the window\n");

    let post = guard
        .close(&[
            edit("notes/plan.md", "# Plan v2\n"),
            edit("notes/new.md", "born in the window\n"),
        ])
        .unwrap();

    let independent = model::merkle_root(
        &[
            ("notes/new.md", b"born in the window\n".as_slice()),
            ("notes/plan.md", b"# Plan v2\n".as_slice()),
            ("receipts/log.md", b"# Receipts\n".as_slice()),
        ],
        0,
    );
    assert_eq!(
        post, independent,
        "verified post root = fold of expected set"
    );
    assert_eq!(StepGuard::GUARANTEE_CLASS, "detected");
}

/// The zero-descriptor cheat (#14/#19): a domain file changes with NO
/// governed edit declared — named as altered.
#[test]
fn zero_descriptor_cheat_is_named() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    write(&root.0, "notes/plan.md", "sneaky rewrite\n");

    match guard.close(&[]) {
        Err(GuardError::OutOfBand(delta)) => {
            assert_eq!(delta.altered, vec!["notes/plan.md".to_string()]);
            assert!(delta.unexpected.is_empty());
            assert!(delta.missing.is_empty());
        }
        other => panic!("expected OutOfBand, got {other:?}"),
    }
}

/// THE #19 cheat a naive root-compare passes: one HONEST governed edit AND a
/// rogue write elsewhere. "Root changed? we changed it" would wave this
/// through; residual-compare names exactly the rogue path.
#[test]
fn honest_edit_plus_rogue_write_is_caught() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    write(&root.0, "notes/plan.md", "# Plan v2\n"); // governed, honest
    write(&root.0, "notes/rogue.md", "written elsewhere\n"); // not declared

    match guard.close(&[edit("notes/plan.md", "# Plan v2\n")]) {
        Err(GuardError::OutOfBand(delta)) => {
            assert_eq!(delta.unexpected, vec!["notes/rogue.md".to_string()]);
            assert!(delta.missing.is_empty());
            assert!(delta.altered.is_empty(), "the honest edit is not accused");
        }
        other => panic!("expected OutOfBand, got {other:?}"),
    }
}

/// A governed target whose on-disk bytes differ from the declared post-edit
/// bytes is altered — tampering after the governed write is not absorbed by
/// the declaration.
#[test]
fn tampered_governed_target_is_altered() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    write(&root.0, "notes/plan.md", "# Plan v2 — tampered\n");

    match guard.close(&[edit("notes/plan.md", "# Plan v2\n")]) {
        Err(GuardError::OutOfBand(delta)) => {
            assert_eq!(delta.altered, vec!["notes/plan.md".to_string()]);
        }
        other => panic!("expected OutOfBand, got {other:?}"),
    }
}

/// A governed edit that never landed on disk is missing — an executor that
/// claims a write it did not make is caught by the same compare.
#[test]
fn claimed_edit_never_landed_is_missing() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    match guard.close(&[edit("notes/ghost.md", "never written\n")]) {
        Err(GuardError::OutOfBand(delta)) => {
            assert_eq!(delta.missing, vec!["notes/ghost.md".to_string()]);
        }
        other => panic!("expected OutOfBand, got {other:?}"),
    }
}

/// An out-of-band DELETION of a pre-step domain file is missing — the
/// expected set carries every pre-step file forward.
#[test]
fn out_of_band_deletion_is_missing() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    std::fs::remove_file(root.0.join("receipts/log.md")).unwrap();

    match guard.close(&[]) {
        Err(GuardError::OutOfBand(delta)) => {
            assert_eq!(delta.missing, vec!["receipts/log.md".to_string()]);
        }
        other => panic!("expected OutOfBand, got {other:?}"),
    }
}

/// #20, the config-widening attack: the window rewrites `mdfs_config.yaml`
/// to ignore its rogue path. The config bracket refuses BEFORE the residual
/// diff, so the widened domain never gets to filter the rogue write out.
/// (Also covers absent→present: the fixture starts with no config.)
#[test]
fn config_widening_attack_refused() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    write(&root.0, "mdfs_config.yaml", "ignore:\n  - \"notes/**\"\n");
    write(&root.0, "notes/rogue.md", "hidden by the new ignore?\n");

    assert!(matches!(guard.close(&[]), Err(GuardError::ConfigChanged)));
}

/// #20, present→absent: deleting the config mid-window is a domain change
/// like any other — refused.
#[test]
fn config_deletion_refused() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        "mdfs_config.yaml",
        "version: 1\nignore:\n  - \"drafts/**\"\n",
    );
    let guard = StepGuard::open(&root).unwrap();

    std::fs::remove_file(root.0.join("mdfs_config.yaml")).unwrap();

    assert!(matches!(guard.close(&[]), Err(GuardError::ConfigChanged)));
}

/// #20 is "mid-RUN", not just mid-step: a config change landing BETWEEN two
/// clean step brackets is caught by pinning every later guard to the
/// run-initial config state.
#[test]
fn cross_step_config_continuity() {
    let (_tmp, root) = workspace();

    let step1 = StepGuard::open(&root).unwrap();
    let pinned = step1.config_state().clone();
    step1.close(&[]).unwrap();

    // between steps: the domain moves
    write(&root.0, "mdfs_config.yaml", "ignore:\n  - \"notes/**\"\n");

    let step2 = StepGuard::open(&root).unwrap();
    assert!(matches!(
        step2.verify_config(&pinned),
        Err(GuardError::ConfigChanged)
    ));
}

/// S4 naming discipline: the delta names the exec WINDOW and the paths —
/// the exact required wording — never an author.
#[test]
fn delta_wording_names_the_window_not_the_block() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();
    write(&root.0, "notes/rogue.md", "x\n");

    let err = guard.close(&[]).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("out-of-band change during exec window"),
        "S4 wording missing: {msg}"
    );
    assert!(
        msg.contains("notes/rogue.md"),
        "delta names the path: {msg}"
    );
}

/// #20 accepted gap, stated as a test: non-md, `.meridian/`, and dot-path
/// writes during the window are UNDETECTED — the §12 hash domain is md-only
/// and dot-excluded. Explicit and named, distinct from the out-of-tree
/// honor system. If this test ever fails, the gap has narrowed: update the
/// docs, not the assertion direction.
#[test]
fn non_md_and_dot_path_writes_are_the_named_undetected_gap() {
    let (_tmp, root) = workspace();
    let guard = StepGuard::open(&root).unwrap();

    write(&root.0, "notes/data.json", "{\"not\": \"markdown\"}\n");
    write(&root.0, ".meridian/runs/inv-1.log.md", "dot-path md\n");
    write(&root.0, "notes/.hidden.md", "dot segment at depth\n");

    guard
        .close(&[])
        .expect("outside the hash domain ⇒ undetected (named gap)");
}

/// A governed edit whose target the domain ignores is inert: written to
/// disk, invisible to the snapshot, filtered from the expected set — close
/// stays clean and the root excludes it.
#[test]
fn governed_edit_outside_domain_is_inert() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        "mdfs_config.yaml",
        "version: 1\nignore:\n  - \"drafts/**\"\n",
    );
    let guard = StepGuard::open(&root).unwrap();

    write(&root.0, "drafts/tmp.md", "scratch\n");

    let post = guard.close(&[edit("drafts/tmp.md", "scratch\n")]).unwrap();
    let independent = model::merkle_root(
        &[
            ("notes/plan.md", b"# Plan\n".as_slice()),
            ("receipts/log.md", b"# Receipts\n".as_slice()),
        ],
        1,
    );
    assert_eq!(post, independent, "ignored path never enters the root");
}

/// #25 symlink laundering, the motivating attack: the window symlinks an
/// out-of-tree secret to an in-domain md path. Refused, naming the link.
#[cfg(unix)]
#[test]
fn symlink_laundering_refused_at_close() {
    let (tmp, root) = workspace();
    let secret = tmp.path().join("secret.md"); // OUT of tree, beside ws/
    std::fs::write(&secret, "out-of-tree secret\n").unwrap();

    let guard = StepGuard::open(&root).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("notes/x.md")).unwrap();

    match guard.close(&[]) {
        Err(GuardError::Symlink { path }) => assert_eq!(path, "notes/x.md"),
        other => panic!("expected Symlink refusal, got {other:?}"),
    }
}

/// #25 at open: a pre-existing symlink (file or directory) means no
/// trustworthy baseline — open itself refuses.
#[cfg(unix)]
#[test]
fn symlink_refused_at_open() {
    // file link
    let (tmp, root) = workspace();
    std::fs::write(tmp.path().join("secret.md"), "s\n").unwrap();
    std::os::unix::fs::symlink(tmp.path().join("secret.md"), root.0.join("notes/x.md")).unwrap();
    assert!(matches!(
        StepGuard::open(&root),
        Err(GuardError::Symlink { .. })
    ));

    // directory link — laundering a whole subtree is refused the same way
    let (tmp2, root2) = workspace();
    std::fs::create_dir(tmp2.path().join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(tmp2.path().join("elsewhere"), root2.0.join("notes/sub")).unwrap();
    assert!(matches!(
        StepGuard::open(&root2),
        Err(GuardError::Symlink { .. })
    ));
}

/// A non-md symlink on a non-dot path is refused too: the refusal is
/// deliberately wider than md-laundering (a link is a hole in the walk's
/// ground truth; fail closed).
#[cfg(unix)]
#[test]
fn non_md_symlink_also_refused() {
    let (tmp, root) = workspace();
    std::fs::write(tmp.path().join("target.bin"), "b\n").unwrap();
    let guard = StepGuard::open(&root).unwrap();
    std::os::unix::fs::symlink(tmp.path().join("target.bin"), root.0.join("notes/data.bin"))
        .unwrap();
    assert!(matches!(guard.close(&[]), Err(GuardError::Symlink { .. })));
}

/// Dot-path symlinks sit in the NAMED dot-path gap (#20/#25): outside
/// detection, neither walked nor refused. Explicit, like the non-md gap.
#[cfg(unix)]
#[test]
fn dot_path_symlink_is_outside_detection() {
    let (tmp, root) = workspace();
    std::fs::write(tmp.path().join("secret.md"), "s\n").unwrap();
    std::fs::create_dir_all(root.0.join(".obsidian")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("secret.md"),
        root.0.join(".obsidian/link.md"),
    )
    .unwrap();

    let guard = StepGuard::open(&root).expect("dot-path link is the named gap, not a refusal");
    guard.close(&[]).unwrap();
}

/// #25 covers the domain's own definition file: a symlinked
/// `mdfs_config.yaml` is refused at open (the config read is no-follow).
#[cfg(unix)]
#[test]
fn symlinked_config_refused() {
    let (tmp, root) = workspace();
    std::fs::write(tmp.path().join("evil-config.yaml"), "ignore:\n  - \"**\"\n").unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("evil-config.yaml"),
        root.0.join("mdfs_config.yaml"),
    )
    .unwrap();

    assert!(matches!(
        StepGuard::open(&root),
        Err(GuardError::Symlink { .. })
    ));
}
