//! On-disk gates for the U6b exec-window bracket (`run::snapshot`): the #19
//! pre-exec cross-check, the always-rendered detection verdict, and the
//! fail-closed gate phase 2 rides on. Windows are simulated — the bracket is
//! independent of the exec mechanics; the test plays the attacker between
//! `open` and `close`.

use std::path::Path;

use fs::WorkspaceRoot;
use model::MerkleRoot;
use run::snapshot::{Detection, ExecBracket, OpenRefusal};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// A workspace nested under the tempdir (out-of-tree space beside it).
fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "notes/plan.md", "# Plan\n");
    write(&ws, "receipts/log.md", "# Receipts\n");
    let root = WorkspaceRoot(ws);
    (tmp, root)
}

/// The flock-computed root the dispatch would hold (`root_after_phase1`).
fn computed_root(root: &WorkspaceRoot) -> MerkleRoot {
    fs::domain_snapshot(root).unwrap().1
}

/// A clean window: open against the computed root, nothing happens, close
/// verifies — and the verified root IS `root_after_phase1` (what phase 2
/// pins against).
#[test]
fn clean_window_verifies_the_computed_root() {
    let (_tmp, root) = workspace();
    let after_phase1 = computed_root(&root);

    let bracket = ExecBracket::open(&root, &after_phase1).unwrap();
    let detection = bracket.close();

    assert!(detection.is_clean());
    match detection {
        Detection::Clean { root: verified } => assert_eq!(verified, after_phase1),
        other => panic!("expected Clean, got {other:?}"),
    }
}

/// #19 addendum: the computed root is the authority. A tree that moved
/// between the phase-1 commit and the bracket opening refuses BEFORE exec —
/// distinct from the in-window delta (nothing ran; nothing to accuse).
#[test]
fn pre_exec_mismatch_refuses_before_exec() {
    let (_tmp, root) = workspace();
    let after_phase1 = computed_root(&root);

    // out-of-band write BETWEEN the flock window and the bracket opening
    write(&root.0, "notes/plan.md", "moved underfoot\n");

    match ExecBracket::open(&root, &after_phase1) {
        Err(OpenRefusal::PreExecMismatch { expected, observed }) => {
            assert_eq!(expected, after_phase1);
            assert_ne!(observed, after_phase1);
        }
        other => panic!("expected PreExecMismatch, got {other:?}"),
    }
}

/// An in-window ungoverned write renders `OutOfBand` with the S4 wording and
/// the exact path — and the verdict is not clean, so phase 2 gates shut.
#[test]
fn in_window_write_is_out_of_band() {
    let (_tmp, root) = workspace();
    let bracket = ExecBracket::open(&root, &computed_root(&root)).unwrap();

    write(&root.0, "notes/rogue.md", "written during the window\n");

    let detection = bracket.close();
    assert!(!detection.is_clean());
    let msg = detection.to_string();
    assert!(
        msg.contains("out-of-band change during exec window"),
        "S4 wording missing: {msg}"
    );
    assert!(msg.contains("notes/rogue.md"), "path named: {msg}");
}

/// #20: a window that rewrites `mdfs_config.yaml` renders `ConfigChanged` —
/// the widening never filters the residual, and phase 2 gates shut.
#[test]
fn in_window_config_change_is_detected() {
    let (_tmp, root) = workspace();
    let bracket = ExecBracket::open(&root, &computed_root(&root)).unwrap();

    write(&root.0, "mdfs_config.yaml", "ignore:\n  - \"notes/**\"\n");
    write(&root.0, "notes/rogue.md", "hidden by the new ignore?\n");

    let detection = bracket.close();
    assert!(matches!(detection, Detection::ConfigChanged));
    assert!(!detection.is_clean());
}

/// #25: a symlink appearing during the window renders `Symlink`, named.
#[cfg(unix)]
#[test]
fn in_window_symlink_is_detected() {
    let (tmp, root) = workspace();
    std::fs::write(tmp.path().join("secret.md"), "out of tree\n").unwrap();
    let bracket = ExecBracket::open(&root, &computed_root(&root)).unwrap();

    std::os::unix::fs::symlink(tmp.path().join("secret.md"), root.0.join("notes/x.md")).unwrap();

    match bracket.close() {
        Detection::Symlink { path } => assert_eq!(path, "notes/x.md"),
        other => panic!("expected Symlink, got {other:?}"),
    }
}

/// S3 escape window — WHERE it lands (gate F1): a write after close escapes
/// THAT bracket, and is caught at the NEXT bracket's open as
/// `PreExecMismatch` against the previously verified root. Detection holds
/// across brackets even though the window itself has closed.
#[test]
fn the_escape_window_lands_in_the_next_brackets_open() {
    let (_tmp, root) = workspace();
    let b1 = ExecBracket::open(&root, &computed_root(&root)).unwrap();
    let Detection::Clean { root: verified } = b1.close() else {
        panic!("clean window expected");
    };

    // the S3 escape: a straggler write AFTER the close snapshot
    write(&root.0, "notes/late.md", "after the bracket\n");

    match ExecBracket::open(&root, &verified) {
        Err(OpenRefusal::PreExecMismatch { expected, observed }) => {
            assert_eq!(expected, verified);
            assert_ne!(observed, verified);
        }
        other => panic!("the escape must land in the next open, got {other:?}"),
    }
}

/// #20 mid-RUN via the bracket (gate F2): step N opened with `open_pinned`
/// against step 1's config refuses when the config moved between steps —
/// even though each window's own bracket was clean and the new computed
/// root matches the new on-disk state.
#[test]
fn open_pinned_refuses_a_config_that_moved_between_steps() {
    let (_tmp, root) = workspace();
    let b1 = ExecBracket::open(&root, &computed_root(&root)).unwrap();
    let pinned = b1.config_state().clone();
    let step1 = b1.close();
    assert!(step1.is_clean());

    // unmoved config: open_pinned passes (the positive arm)
    let b2 = ExecBracket::open_pinned(&root, &computed_root(&root), &pinned).unwrap();
    assert!(b2.close().is_clean());

    // between steps: the domain moves — each window still clean on its own
    write(&root.0, "mdfs_config.yaml", "ignore:\n  - \"notes/**\"\n");

    match ExecBracket::open_pinned(&root, &computed_root(&root), &pinned) {
        Err(OpenRefusal::Guard(fs::guard::GuardError::ConfigChanged)) => {}
        other => panic!("expected ConfigChanged, got {other:?}"),
    }
}

/// The bracket's guarantee class is `detected` — the U7 labeler's source
/// (#23: the label must come from the bracket being wired, not the language).
#[test]
fn bracket_guarantee_class_is_detected() {
    assert_eq!(
        ExecBracket::GUARANTEE_CLASS,
        run::fence::GuaranteeClass::Detected
    );
    assert_eq!(ExecBracket::GUARANTEE_CLASS.as_str(), "detected");
}
