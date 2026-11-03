//! The § A.3 retire law: `splice.pin.retire: true` drops one row of the
//! pinning page's own lock, named as stored — the target is never loaded, so
//! a row over a renamed or deleted section retires; a row the page does not
//! hold refuses naming the rows it does; a wire retire is a guarded write.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wire::{ErrorCode, Path as WPath, PinSpec, ReadSel, ResponseBody};
use wire_serve::guard::{Origin, Premise, PremiseValue};
use wire_serve::write::{SpliceArgs, scope_token, splice};

const PLAN: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";
const NOTES: &str = "# Notes\n\nalso draws from the guide.\n";
const GUIDE: &str =
    "# Guide\n\n## Source\n\nthe claim.\n\n## Other\n\nunrelated.\n\n## In/Out\n\na block. ^blk\n";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in [("plan.md", PLAN), ("notes.md", NOTES), ("guide.md", GUIDE)] {
        std::fs::write(dir.path().join(rel), body).expect("seed");
    }
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "retire@example.invalid"],
        vec!["config", "user.name", "retire"],
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

fn args(page: &str, selector: &str, retire: bool) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: Origin::InProcess,
        path: WPath(page.into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: ReadSel::parse(selector),
            vibe: None,
            fingerprint: None,
            sec_rev: None,
            retire: retire.then_some(true),
        }),
        fields: BTreeMap::default(),
    }
}

fn pin(root: &fs::WorkspaceRoot, page: &str, selector: &str) -> String {
    let out = splice(root, None, &args(page, selector, false), &[], None).expect("pin commits");
    let ResponseBody::Splice { pin, .. } = out.body else {
        panic!("splice body");
    };
    pin.expect("a pin fact").fingerprint
}

/// Every row of `page`'s lock, as `object §path`.
fn rows(root: &fs::WorkspaceRoot, page: &str) -> Vec<String> {
    let doc = fs::load(root, Path::new(page)).expect("load");
    lock::find(&doc)
        .expect("the lock parses")
        .map(|f| {
            f.lock
                .pins
                .iter()
                .map(|p| format!("{} §{}", p.object, p.selector.elements().join("/")))
                .collect()
        })
        .unwrap_or_default()
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// The stale-row case end to end: two pins, one section renamed, the retire
/// drops the stale row and keeps the live one; the response carries the
/// retired row's recorded fingerprint.
#[test]
fn a_retire_drops_the_row_of_a_renamed_section_and_keeps_its_sibling() {
    let (_dir, root) = workspace();
    let source_fp = pin(&root, "plan.md", "Guide/Source");
    pin(&root, "plan.md", "Guide/Other");
    let guide = read(&root, "guide.md").replace("## Source", "## Source renamed");
    std::fs::write(root.0.join("guide.md"), guide).expect("rename");

    let out = splice(
        &root,
        None,
        &args("plan.md", "Guide/Source", true),
        &[],
        None,
    )
    .expect("the retire commits");
    let ResponseBody::Splice {
        pin, pin_retired, ..
    } = out.body
    else {
        panic!("splice body");
    };
    assert!(pin.is_none(), "a retire mints nothing");
    let retired = pin_retired.expect("a retire answers with what it dropped");
    assert_eq!(retired.fingerprint, source_fp);
    assert_eq!(retired.target, WPath("guide.md".into()));
    assert_eq!(
        rows(&root, "plan.md"),
        vec!["guide §Guide/Other".to_owned()]
    );
    assert!(
        read(&root, "guide.md").contains("## Source renamed"),
        "the target is never written"
    );
}

/// A row the page does not hold refuses, nothing written, naming the rows
/// the page holds — and another page's row stays out of reach.
#[test]
fn a_retire_of_a_row_this_page_does_not_hold_refuses_and_names_its_rows() {
    let (_dir, root) = workspace();
    pin(&root, "plan.md", "Guide/Source");
    pin(&root, "notes.md", "Guide/Other");
    let plan = read(&root, "plan.md");
    let notes = read(&root, "notes.md");

    let e = splice(
        &root,
        None,
        &args("plan.md", "Guide/Other", true),
        &[],
        None,
    )
    .expect_err("plan.md holds no Guide/Other row");
    assert_eq!(e.code, ErrorCode::PinTargetMissing);
    let message = e.message.unwrap_or_default();
    assert!(message.contains("guide §Guide/Source"), "{message}");
    assert!(message.contains("another page"), "{message}");
    assert_eq!(read(&root, "plan.md"), plan, "nothing written");
    assert_eq!(
        read(&root, "notes.md"),
        notes,
        "the other page's row is untouched"
    );
}

/// The target is never loaded: rows over a deleted file retire, and the
/// last one leaves the canonical empty block, which parses back.
#[test]
fn rows_over_a_deleted_target_retire_down_to_the_empty_block() {
    let (_dir, root) = workspace();
    pin(&root, "plan.md", "Guide/Source");
    pin(&root, "plan.md", "Guide/Other");
    std::fs::remove_file(root.0.join("guide.md")).expect("delete the target");

    for selector in ["Guide/Source", "Guide/Other"] {
        splice(&root, None, &args("plan.md", selector, true), &[], None).expect("retire");
    }
    assert!(rows(&root, "plan.md").is_empty());
    assert!(
        read(&root, "plan.md").contains("```meridian-lock\nversion: 2\n```"),
        "{}",
        read(&root, "plan.md")
    );
}

/// A wire retire with no premise refuses `guard_required`; the page's own
/// premise admits the same request.
#[test]
fn a_wire_retire_is_a_guarded_write() {
    let (_dir, root) = workspace();
    pin(&root, "plan.md", "Guide/Source");
    let mut wire_args = args("plan.md", "Guide/Source", true);
    wire_args.origin = Origin::Wire;
    wire_args.actor = Some("agent:retire".into());

    let e = splice(&root, None, &wire_args, &[], None).expect_err("no premise");
    assert_eq!(e.code, ErrorCode::GuardRequired);
    assert_eq!(rows(&root, "plan.md").len(), 1, "nothing written");

    let token = scope_token(&root, None, Some(Path::new("plan.md")))
        .expect("mint")
        .expect("plan.md holds a node");
    wire_args.premises = vec![Premise {
        scope: Some(PathBuf::from("plan.md")),
        value: PremiseValue::Token(token),
    }];
    splice(&root, None, &wire_args, &[], None).expect("the covering premise admits it");
    assert!(rows(&root, "plan.md").is_empty());
}

/// The other row shapes `pin_row` mints retire by what a walk prints: a
/// sole-`^id` anchor row, and a heading that holds `/` (one element, named by
/// the walk's joined `§` text). A dry retire writes nothing and still reports.
#[test]
fn anchor_and_slash_heading_rows_retire_and_a_dry_retire_writes_nothing() {
    let (_dir, root) = workspace();
    pin(&root, "plan.md", "^blk");
    let mut slash = args("plan.md", "", false);
    slash.pin.as_mut().expect("pin").selector = ReadSel::Hpath {
        hpath: vec![
            wire::HpathSeg {
                h: "Guide".into(),
                n: None,
            },
            wire::HpathSeg {
                h: "In/Out".into(),
                n: None,
            },
        ],
    };
    splice(&root, None, &slash, &[], None).expect("the slash heading pins");
    assert_eq!(
        rows(&root, "plan.md"),
        vec!["guide §^blk".to_owned(), "guide §Guide/In/Out".to_owned()]
    );

    let before = read(&root, "plan.md");
    let mut dry = args("plan.md", "Guide/In/Out", true);
    dry.dry = true;
    let out = splice(&root, None, &dry, &[], None).expect("a dry retire rehearses");
    let ResponseBody::Splice { pin_retired, .. } = out.body else {
        panic!("splice body");
    };
    assert!(
        pin_retired.is_some(),
        "a dry retire reports the row it would drop"
    );
    assert_eq!(
        read(&root, "plan.md"),
        before,
        "a dry retire writes nothing"
    );

    splice(
        &root,
        None,
        &args("plan.md", "Guide/In/Out", true),
        &[],
        None,
    )
    .expect("the walk's joined text retires the slash heading");
    splice(&root, None, &args("plan.md", "^blk", true), &[], None).expect("the anchor row retires");
    assert!(rows(&root, "plan.md").is_empty());
}
