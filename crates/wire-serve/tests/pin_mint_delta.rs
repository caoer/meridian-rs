//! The mint's Delta honesty (card pin-mint-sub-attribution, dogfood r8 D4):
//! a pin's anchor promotion is a real write to the TARGET page, so the commit
//! frame names it — file row, node rows, on the actor-carrying frame — and the
//! frame's `root_before` is the call's true pre-state, so the sub chain stays
//! contiguous across a promoting pin. Zero semantic change to the mint itself.
//!
//! Receipts this file answers: five physical mints, zero sub rows across
//! complete-history drains 616→683 (r8 § D4); the ghost "foreign changes"
//! warning on every first pin (r1 F13) shares the missing-attribution root.
//!
//! One row per changed file (§7.1): a self-pin folds the mint into the page's
//! one row (before tense = pre-promotion), a reused anchor mints nothing and
//! frames nothing.

use wire::{FileChange, Path as WPath, PinSpec, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// Production-arm read into the session store — the covering read the D16
/// gate demands of an actor-carrying pin.
fn session_read(
    root: &fs::WorkspaceRoot,
    store: &receipt::read_mint::ReadMintStore,
    actor: &str,
    rel: &str,
    selector: &str,
) {
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    let params = wire_serve::read::ReadParams {
        sections: Some(vec![wire::ReadSel::parse(selector)]),
        actor: Some(actor.to_owned()),
        ..Default::default()
    };
    wire_serve::read::composed_read(
        &doc,
        &WPath(rel.into()),
        &wire::Root("r0".into()),
        &params,
        Some(store),
        &wire_serve::read::NO_DECORATIONS,
    )
    .expect("the read serves");
}

/// Pinning page (no lock yet — first pin births one as file preamble).
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

/// Pinned page; `Leader's Guideline` → D15 slug `leaders-guideline`.
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

/// A pinnable workspace — git-initialised (R4 gives every pin row a `hash`).
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "mint@example.invalid"],
        vec!["config", "user.name", "mint"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// Pin-only splice from `pinner` onto guide.md, actor-attributed.
fn pin_args(pinner: &str, selector: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(pinner.into()),
        actor: Some("agent:pinner".into()),
        now: Some("2026-08-15T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: wire::ReadSel::parse(selector),
            vibe: None,
            fingerprint: None,
            sec_rev: None,
        }),
    }
}

/// Engine content id of raw bytes (names the tense a rev claims).
fn rev(bytes: &str) -> String {
    model::build(bytes.to_string(), syntax::parse(bytes))
        .root
        .node_rev
        .0
}

/// r8 D4 red→green: the promotion is this splice's own write, so the frame
/// names the target — one `modified` row with honest tenses and node grain —
/// and carries the caller's identity. `root_before` is the pre-call world,
/// so the frame joins the chain the previous frame left off (the silent
/// fold at watch.rs's internal-commit arm never sees an unnamed movement).
#[test]
fn a_promoting_pin_frames_the_targets_write() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    session_read(
        &root,
        &store,
        "agent:pinner",
        "guide.md",
        "Guide/Leader's Guideline",
    );
    let pre_call = wire_serve::ambient_root(&root).expect("ambient");

    let out = splice(
        &root,
        None,
        &pin_args("plan.md", "Guide/Leader's Guideline"),
        &[],
        Some(&store),
    )
    .expect("the promoting pin commits");
    let frame = out.committed.expect("a real pin commits a frame");

    assert_eq!(
        frame.delta.root_before, pre_call,
        "the frame's root_before is the call's pre-state — the promotion is \
         inside the frame, not folded silently under it"
    );
    assert_eq!(
        frame.delta.actor.as_deref(),
        Some("agent:pinner"),
        "the mint rides the actor-carrying frame"
    );

    let paths: Vec<&str> = frame
        .delta
        .files
        .iter()
        .map(|f| f.path.0.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["plan.md", "guide.md"],
        "content first (§7.1 print order), then the promotion target"
    );

    let mint = &frame.delta.files[1];
    assert_eq!(mint.change, FileChange::Modified);
    assert_eq!(
        mint.file_rev_before.as_ref().map(|r| r.0.as_str()),
        Some(rev(TARGET).as_str()),
        "before tense: the target as it was before the marker landed"
    );
    let on_disk = std::fs::read_to_string(root.0.join("guide.md")).expect("target");
    assert_eq!(
        mint.file_rev_after.as_ref().map(|r| r.0.as_str()),
        Some(rev(&on_disk).as_str()),
        "after tense: the committed target bytes"
    );
    assert!(
        !mint.nodes.is_empty(),
        "the mint carries node grain like every other write: {mint:?}"
    );
}

/// Reused anchor (any pinner): the target is byte-identical, so no target row
/// rides — a row for an unchanged file would be the opposite dishonesty.
#[test]
fn a_reused_anchor_pin_frames_no_target_row() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    session_read(
        &root,
        &store,
        "agent:pinner",
        "guide.md",
        "Guide/Leader's Guideline",
    );
    splice(
        &root,
        None,
        &pin_args("plan.md", "Guide/Leader's Guideline"),
        &[],
        Some(&store),
    )
    .expect("first pin mints");
    let minted = std::fs::read_to_string(root.0.join("guide.md")).expect("target");

    std::fs::write(root.0.join("second.md"), PINNER).expect("second pinner");
    session_read(
        &root,
        &store,
        "agent:pinner",
        "guide.md",
        "Guide/Leader's Guideline",
    );
    let out = splice(
        &root,
        None,
        &pin_args("second.md", "Guide/Leader's Guideline"),
        &[],
        Some(&store),
    )
    .expect("the re-pin reuses the anchor");
    let frame = out.committed.expect("the lock write commits");

    let paths: Vec<&str> = frame
        .delta
        .files
        .iter()
        .map(|f| f.path.0.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["second.md"],
        "a reused anchor writes nothing to the target, so nothing is framed"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("guide.md")).expect("target"),
        minted,
        "control: the target really is byte-identical across the re-pin"
    );
}

/// Self-pin (target IS the pinning page): one changed file, ONE row (§7.1) —
/// the mint folds into the page's row as its before tense, never a second row
/// and never a silently-narrowed tense.
#[test]
fn a_self_pin_folds_the_mint_into_the_pages_one_row() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    session_read(&root, &store, "agent:pinner", "plan.md", "Plan");
    let mut args = pin_args("plan.md", "Plan");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let out = splice(&root, None, &args, &[], Some(&store)).expect("self-pin commits");
    let frame = out.committed.expect("one delta");

    assert_eq!(
        frame.delta.files.len(),
        1,
        "one changed file is one row: {:?}",
        frame.delta.files
    );
    let row = &frame.delta.files[0];
    assert_eq!(row.path, WPath("plan.md".into()));
    assert_eq!(
        row.file_rev_before.as_ref().map(|r| r.0.as_str()),
        Some(rev(PINNER).as_str()),
        "before tense: the page before the promotion landed — the mint's \
         movement is inside the row, not folded under it"
    );
    let on_disk = std::fs::read_to_string(root.0.join("plan.md")).expect("page");
    assert_eq!(
        row.file_rev_after.as_ref().map(|r| r.0.as_str()),
        Some(rev(&on_disk).as_str()),
        "after tense: the committed page (promotion + lock)"
    );
    assert_eq!(
        frame.delta.root_before,
        {
            let ResponseBody::Splice { root_before, .. } = &out.body else {
                panic!("splice body");
            };
            root_before.clone()
        },
        "response and frame agree on the one pre-state (§6.4: same facts, one set)"
    );
}
