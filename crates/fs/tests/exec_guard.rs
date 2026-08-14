//! On-disk gates for the exec-window detection bracket (`fs::guard`):
//! residual-compare, the config bracket, symlink refusal,
//! and the named accepted gaps. Steps are simulated — the guard is
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

/// The zero-descriptor cheat: a domain file changes with no governed edit
/// declared — named as altered.
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

/// The cheat a root-compare passes: one honest governed edit plus a rogue write
/// elsewhere. Residual-compare names exactly the rogue path.
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

/// The config-widening attack: the window rewrites `mdfs_config.yaml`
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

/// The same attack through the markdown surface: a bracket watching only
/// `mdfs_config.yaml` would compute the residual diff under the widened domain
/// — clean — and launder the rogue write.
#[test]
fn config_widening_attack_through_markdown_surface_refused() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"drafts/**\"\n---\n\n# Domain\n",
    );
    let guard = StepGuard::open(&root).unwrap();

    // Widen the domain mid-window, then write outside the ORIGINAL domain.
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"drafts/**\"\n  - \"notes/**\"\n---\n\n# Domain\n",
    );
    write(&root.0, "notes/rogue.md", "hidden by the new ignore?\n");

    assert!(
        matches!(guard.close(&[]), Err(GuardError::ConfigChanged)),
        "widening the markdown domain config mid-window must refuse, exactly as \
         widening the yaml one does"
    );
}

/// The guard must resolve the same domain the read path resolves: parsing only
/// the legacy surface would detect against the default domain while
/// `check`/`status` used the declared ignore list.
#[test]
fn guard_honours_the_markdown_domain_config() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"drafts/**\"\n---\n\n# Domain\n",
    );
    let guard = StepGuard::open(&root).unwrap();

    // An ignored path is outside detection: writing there is not a residual.
    write(
        &root.0,
        "drafts/scratch.md",
        "ignored by the declared domain\n",
    );

    assert!(
        guard.close(&[]).is_ok(),
        "a write under a path the markdown config ignores is outside the \
         detection domain, so the bracket must close clean"
    );
}

/// Present→absent: deleting the config mid-window is a domain change
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

/// The config bracket is mid-RUN, not just mid-step: a config change landing BETWEEN two
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

/// Naming discipline: the delta names the exec window and the paths, in the
/// exact required wording, never an author.
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

/// The residual-escape window: a write landing after close has no retroactive
/// verdict — the bracket boundary is the close snapshot. The escape is absorbed
/// into the next guard's baseline and shows only as a root delta between
/// brackets; naming it is the run layer's job.
#[test]
fn post_close_write_is_the_named_escape_window() {
    let (_tmp, root) = workspace();
    let g1 = StepGuard::open(&root).unwrap();
    let verified = g1.close(&[]).unwrap(); // clean window, verdict rendered

    // the escape: a straggler lands after the close snapshot
    write(&root.0, "notes/late.md", "after the bracket\n");

    // no retroactive verdict exists; the next guard absorbs it as baseline …
    let g2 = StepGuard::open(&root).expect("the escape is not a refusal at the primitive level");
    // … and the escape is visible ONLY as a root delta between brackets.
    assert_ne!(g2.pre_root(), verified);
    g2.close(&[]).unwrap();
}

/// `pre_root` equivalence: on a symlink-free tree the guard's observed baseline
/// folds to exactly the root [`fs::domain_snapshot`] computes, which is where
/// the flock-computed `root_after_phase1` comes from. A divergence would make
/// the run layer's computed-vs-observed comparison meaningless.
#[test]
fn pre_root_matches_domain_snapshot() {
    let (_tmp, root) = workspace();
    let (_, snap_root) = fs::domain_snapshot(&root).unwrap();
    let guard = StepGuard::open(&root).unwrap();
    assert_eq!(guard.pre_root(), snap_root);

    // and a clean zero-edit close returns that same root
    assert_eq!(guard.close(&[]).unwrap(), snap_root);
}

/// The accepted gap, stated as a test: non-md, `.meridian/`, and dot-path
/// writes during the window are undetected — the §12 hash domain is md-only
/// and dot-excluded. If this ever fails the gap has narrowed: update the docs,
/// not the assertion direction.
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

/// Symlink laundering, the motivating attack: the window symlinks an
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
        Err(GuardError::Symlink { count, first }) => {
            assert_eq!((count, first.as_str()), (1, "notes/x.md"));
        }
        other => panic!("expected Symlink refusal, got {other:?}"),
    }
}

/// The ignore set reaches the guarded walk: a link inside an IGNORED directory
/// does not refuse, while an identical link on a non-ignored path still does.
///
/// Both halves matter: without the first, one link anywhere in a vendored
/// subtree refuses the whole walk and no bracket can open; without the second,
/// the ignore set would switch the symlink refusal off.
#[cfg(unix)]
#[test]
fn links_under_an_ignored_dir_do_not_refuse_but_others_still_do() {
    let (tmp, root) = workspace();
    let secret = tmp.path().join("secret.md"); // OUT of tree, beside ws/
    std::fs::write(&secret, "out-of-tree secret\n").unwrap();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"vendor/**\"\n---\n\n# Domain\n",
    );

    // A link in the ignored subtree: outside detection by declaration.
    std::fs::create_dir_all(root.0.join("vendor/libs")).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("vendor/libs/x.md")).unwrap();
    let guard =
        StepGuard::open(&root).expect("a link under an ignored directory must not refuse the walk");
    assert!(
        guard.close(&[]).is_ok(),
        "and must not refuse at close either"
    );

    // The same link on a path the workspace DID NOT ignore still refuses —
    // the ignore set narrows the detection domain, it does not disarm the
    // symlink refusal.
    std::os::unix::fs::symlink(&secret, root.0.join("notes/x.md")).unwrap();
    match StepGuard::open(&root) {
        Err(GuardError::Symlink { count, first }) => {
            assert_eq!((count, first.as_str()), (1, "notes/x.md"));
        }
        other => panic!("expected Symlink refusal on the non-ignored path, got {other:?}"),
    }
}

/// A write aimed through a link in an ignored directory, whose real target is
/// an attested in-tree file, is caught at the target: detection compares domain
/// state, not the path a writer travelled.
#[cfg(unix)]
#[test]
fn a_write_through_an_ignored_link_is_caught_at_its_attested_target() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"vendor/**\"\n---\n\n# Domain\n",
    );
    write(&root.0, "notes/target.md", "original attested bytes\n");

    // The link lives in the ignored subtree; its target is attested.
    std::fs::create_dir_all(root.0.join("vendor")).unwrap();
    std::os::unix::fs::symlink(
        root.0.join("notes/target.md"),
        root.0.join("vendor/link.md"),
    )
    .unwrap();

    let guard = StepGuard::open(&root).expect("ignored-dir link must not refuse");

    // Write through the link — this really rewrites notes/target.md.
    std::fs::write(root.0.join("vendor/link.md"), "smuggled bytes\n").unwrap();

    match guard.close(&[]) {
        Err(GuardError::OutOfBand(delta)) => {
            assert_eq!(
                delta.altered,
                vec!["notes/target.md".to_string()],
                "the residual must name the attested TARGET, not the link"
            );
        }
        other => panic!("expected OutOfBand naming the target, got {other:?}"),
    }
}

/// At open: a pre-existing symlink (file or directory) means no
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

/// Dot-path symlinks sit in the named dot-path gap: outside detection, neither
/// walked nor refused.
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

/// The refusal covers the domain's own definition file: a symlinked
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

// ---- run-walk-real-roots (dogfood r2 D-USER F2): symlinks at DECLARED- ----
// ---- ignored paths are outside detection; a refusal is a count, not  ----
// ---- one mine per attempt.                                            ----

/// A symlink AT a custom-ignored path — not merely under an ignored real
/// directory — is outside the detection domain: skipped, never refused. All
/// three sessions-root forms on one workspace: a link under a real
/// scratch-named dir (the pruned-parent case that already held), a symlinked
/// DIRECTORY at an ignored name (the venv `bin` shape), and a FILE link whose
/// own path the ignore covers.
#[cfg(unix)]
#[test]
fn a_symlink_at_an_ignored_path_is_outside_detection() {
    let (tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"scratch*/\"\n  - \"bin/\"\n---\n\n# Domain\n",
    );
    // The sessions shape: a stranger's venv under a real scratch-named dir…
    let secret = tmp.path().join("secret.md");
    std::fs::write(&secret, "out-of-tree\n").unwrap();
    std::fs::create_dir_all(root.0.join("s1/scratch-r2-verify/venv")).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("s1/scratch-r2-verify/venv/link.md")).unwrap();
    // …a symlinked DIRECTORY named bin at a non-scratch path…
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(root.0.join("results/run-e1")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.0.join("results/run-e1/bin")).unwrap();
    // …and a FILE link whose own path matches the scratch shape directly.
    std::fs::create_dir_all(root.0.join("s2")).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("s2/scratch-notes.md")).unwrap();

    let guard = StepGuard::open(&root)
        .expect("a symlink at a domain-ignored path is outside detection — open must not refuse");
    assert!(
        guard.close(&[]).is_ok(),
        "and close must not refuse it either"
    );
}

/// The veto refusal is a count and a first offender — a claim a caller can
/// size a cleanup by — never one mine per attempt.
#[cfg(unix)]
#[test]
fn the_refusal_names_the_count_and_the_first_offender() {
    let (tmp, root) = workspace();
    let secret = tmp.path().join("secret.md");
    std::fs::write(&secret, "s\n").unwrap();
    for rel in [
        "year=2026/venv/x.md",
        "results/run-1/bin.md",
        "zeta/late.md",
    ] {
        std::fs::create_dir_all(root.0.join(rel).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&secret, root.0.join(rel)).unwrap();
    }
    // A dot-path link stays in the dot gap: walked around, never counted.
    std::fs::create_dir_all(root.0.join(".obsidian")).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join(".obsidian/link.md")).unwrap();

    let err = StepGuard::open(&root).expect_err("three stranger links must refuse");
    let text = err.to_string();
    assert!(
        text.contains("3 symlinked paths refused in exec-window snapshot"),
        "the count is the claim; got: {text}"
    );
    assert!(
        text.contains("first: results/run-1/bin.md"),
        "the first offender (sorted) is named; got: {text}"
    );
}

/// One link keeps the established single-path wording byte-identical — no
/// face, golden, or conformance churn for the common case.
#[cfg(unix)]
#[test]
fn a_single_symlink_keeps_the_established_wording() {
    let (tmp, root) = workspace();
    let secret = tmp.path().join("secret.md");
    std::fs::write(&secret, "s\n").unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("notes/x.md")).unwrap();

    let err = StepGuard::open(&root).expect_err("one link still refuses");
    assert_eq!(
        err.to_string(),
        "symlinked path refused in exec-window snapshot: notes/x.md",
    );
}

/// A `!` re-include lifts the ignore for the path it names, so a symlink AT a
/// re-included path is back inside the domain — refused, exactly as if the
/// ignore never covered it. The exclusion narrows detection; it cannot be
/// aimed to disarm the refusal for an attested file.
#[cfg(unix)]
#[test]
fn a_symlink_at_a_reincluded_path_still_refuses() {
    let (tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nignore:\n  - \"scratch/**\"\n  - \"!scratch/keep.md\"\n---\n\n# Domain\n",
    );
    let secret = tmp.path().join("secret.md");
    std::fs::write(&secret, "s\n").unwrap();
    std::fs::create_dir_all(root.0.join("scratch")).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("scratch/keep.md")).unwrap();

    assert!(
        matches!(StepGuard::open(&root), Err(GuardError::Symlink { .. })),
        "a re-included path is attested — its symlink must refuse"
    );
}

/// Reserved paths are engine substrate: an ignore list covering `meridian/**`
/// never turns a symlinked armed-rules artifact into a skippable stranger.
#[cfg(unix)]
#[test]
fn a_symlinked_reserved_path_never_skips() {
    let (tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nignore:\n  - \"meridian/**\"\n---\n\n# Domain\n",
    );
    let evil = tmp.path().join("evil-rules.md");
    std::fs::write(&evil, "# armed\n").unwrap();
    std::os::unix::fs::symlink(&evil, root.0.join("meridian/armed-rules.md")).unwrap();

    assert!(
        matches!(StepGuard::open(&root), Err(GuardError::Symlink { .. })),
        "the armed-rules artifact stays refused whatever the ignore list says"
    );
}
