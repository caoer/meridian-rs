//! End-to-end `check` fixtures over a REAL tmpdir workspace (U2.10 gates).
//!
//! Every fixture seeds the journal through the PRODUCTION write path (a guarded
//! `create` mints real journal rows with real tree roots — no in-memory double),
//! then exercises the `check` engine over the resulting on-disk state:
//! - [`spliced_journal_caught_through_check`] — the U2.1 spliced-row fixture
//!   caught THROUGH `check` itself (chain red, the forged row cited), not just at
//!   the library level;
//! - [`foreign_edit_caught_through_check`] — an out-of-writer edit reddens
//!   convention-free (last-receipt-vs-live);
//! - [`layer1_run_is_read_only`] — a layer-1 armed run over the tree mutates
//!   nothing (no rev, no journal row, no file byte).

use std::collections::BTreeMap;
use std::path::Path;

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use receipt::journal::{JournalRow, parse_rows, render_row};
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, create};

/// Birth `path` with `body` through the production guarded-create write path — the
/// real journal-row-minting edge (no in-memory double). Panics on refusal (the
/// fixtures never provoke one).
fn produce(root: &WorkspaceRoot, path: &str, body: &str) {
    let args = CreateArgs {
        id: None,
        path: WirePath(path.to_string()),
        body: body.to_string(),
        actor: Some("agent:alice".to_string()),
        now: None,
        if_root: None,
        dry: false,
    };
    create(root, 0, &args, &[])
        .unwrap_or_else(|e| panic!("production create {path} refused: {e:?}"));
}

/// Read the reserved journal page bytes.
fn read_journal(root: &WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(RESERVED_JOURNAL_PATH)).expect("journal page present")
}

/// The live tree merkle root (journal-excluded) — the quantity a receipt records.
fn live_root(root: &WorkspaceRoot) -> String {
    fs::domain_snapshot(root).expect("snapshot").1.0
}

/// **spliced-journal end-to-end** (task gate). Two honest writes chain; a forged
/// row is spliced BETWEEN them (the U2.1 fixture shape). `check`'s journal TRACE
/// recomputes the chain over the on-disk journal and reddens, citing the forged
/// row — the U2.1 primitive mounted end-to-end. The journal is root-EXCLUDED, so
/// the splice does not move the tree root: `foreign_edit` stays clear, isolating
/// the chain break as the signal.
#[test]
fn spliced_journal_caught_through_check() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());

    // Two honest writes → journal rows r-000001, r-000002 with real roots.
    produce(&root, "a.md", "# A\n\nalpha\n");
    produce(&root, "b.md", "# B\n\nbeta\n");
    let page = read_journal(&root);
    assert!(
        page.contains("^r-000001") && page.contains("^r-000002"),
        "two honest rows landed"
    );

    // Splice a forged row BETWEEN the two honest rows: its roots belong to no real
    // write, so it fails to continue the chain (the tamper the detector must catch).
    let forged = render_row(&JournalRow {
        seq: 99,
        op: "splice",
        path: "a.md",
        actor: Some("mallory"),
        now: None,
        root_before: "b3:FORGED_BEFORE",
        root_after: "b3:FORGED_AFTER",
        file: None,
        edits: Vec::new(),
    });
    let spliced: String = page
        .lines()
        .flat_map(|line| {
            if line.contains("^r-000002") {
                vec![forged.as_str(), line]
            } else {
                vec![line]
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(root.0.join(RESERVED_JOURNAL_PATH), format!("{spliced}\n"))
        .expect("rewrite journal");

    // Through check itself (not the bare check_chain library call).
    let trace = check::journal_trace(&root).expect("journal trace");
    assert!(trace.chain.is_red(), "the spliced row breaks the chain");
    assert_eq!(
        trace.chain.breaks[0].row_anchor, "r-000099",
        "the forged row is cited first"
    );
    let summary = trace.red_summary().expect("red render");
    assert!(
        summary.contains("^r-000099"),
        "the red render cites the forged row: {summary}"
    );
    assert!(
        trace.foreign_edit.is_none(),
        "the root-excluded journal splice does not move the tree root — no foreign_edit"
    );
}

/// **`foreign_edit` fixture** (task gate). One honest write records the tree root;
/// an out-of-writer edit (a raw `std::fs::write`, not the engine) moves the tree
/// with no journal row. `check`'s journal TRACE compares the live root against the
/// last receipt's recorded `root_after` and reddens `foreign_edit`, convention-free
/// (no convention armed). The chain stays green — one honest row, no break.
#[test]
fn foreign_edit_caught_through_check() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());

    produce(&root, "a.md", "# A\n\nalpha\n");
    let recorded = live_root(&root); // the root the receipt r-000001 recorded

    // An out-of-writer edit — bytes change with no governed write, no journal row.
    std::fs::write(root.0.join("a.md"), "# A\n\nEDITED OUT OF BAND\n").expect("raw edit");
    let now = live_root(&root);
    assert_ne!(recorded, now, "the out-of-band edit moved the tree root");

    let trace = check::journal_trace(&root).expect("journal trace");
    let fe = trace
        .foreign_edit
        .as_ref()
        .expect("an out-of-writer edit renders red convention-free");
    assert_eq!(
        fe.last_receipt, "r-000001",
        "cites the last governed receipt"
    );
    assert_eq!(
        fe.recorded_root, recorded,
        "the receipt's recorded root_after"
    );
    assert_eq!(fe.live_root, now, "the drifted live root");
    assert!(trace.is_red(), "foreign_edit reddens the core");
    assert!(
        trace.chain.is_green(),
        "one honest row, no chain break — foreign_edit is the isolated signal"
    );
}

/// **read-only** (task gate). A layer-1 armed run over the tree mutates nothing:
/// load a page from disk, derive a change, evaluate the armed seed convention, and
/// confirm every workspace byte (including the journal) and the tree root are
/// byte-identical before and after — no rev, no journal row, no projection write.
#[test]
fn layer1_run_is_read_only() {
    use policy::{ChangeOp, CheckLimits, Invocation, derive_change, load_seed_convention};

    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());

    // A real task in the seed's `tasks/**` scope, born through the write path.
    produce(
        &root,
        "tasks/t.md",
        "---\nowner: agent:alice\nstatus: open\n---\n# T\n\nx\n",
    );

    let before_bytes = snapshot_tree(dir.path());
    let before_root = live_root(&root);

    // The layer-1 run over the tree: load the doc, frame it as a change, evaluate
    // the armed convention. Owner == actor, so the convention FIRES — a firing
    // convention still writes nothing.
    let mut doc = fs::load(&root, Path::new("tasks/t.md")).expect("load doc");
    // The disk edge stamps the path model::build leaves empty (change.rs law) —
    // the convention's `tasks/**` scope matches on it.
    if let model::NodeKind::Document { path, .. } = &mut doc.root.kind {
        *path = "tasks/t.md".to_string();
    }
    let no_edges = |_: &str| None;
    let change = derive_change(
        &doc,
        &doc,
        &[],
        Invocation {
            op: ChangeOp::Splice,
            actor: Some("agent:alice"),
            force: false,
        },
        &[],
        &no_edges,
    );
    let armed = check::ArmedConvention {
        slug: "reviewer-not-owner".to_string(),
        enforcement: policy::Enforcement::Block,
        convention: load_seed_convention(CheckLimits::default()).expect("seed loads"),
    };
    let report = check::evaluate(std::slice::from_ref(&armed), &change);
    assert!(
        report.is_red(),
        "the armed convention fired (owner self-close)"
    );

    let after_bytes = snapshot_tree(dir.path());
    let after_root = live_root(&root);

    assert_eq!(
        before_bytes, after_bytes,
        "a layer-1 run mutates no workspace byte (no rev, no journal row, no projection)"
    );
    assert_eq!(before_root, after_root, "the tree root is unmoved");
}

/// Recursively snapshot every file under `dir` as `relative-path → bytes` — the
/// whole tree, so nothing (journal included) can change unseen.
fn snapshot_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    collect(dir, dir, &mut out);
    out
}

fn collect(base: &Path, cur: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    let entries = std::fs::read_dir(cur).expect("read_dir");
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect(base, &path, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .expect("under base")
                .to_string_lossy()
                .into_owned();
            out.insert(rel, std::fs::read(&path).expect("read file"));
        }
    }
}

/// A sanity assertion the fixtures rely on: a clean production write leaves a
/// green core (chain continuous, no foreign edit) — check does not cry wolf.
#[test]
fn clean_production_writes_leave_a_green_core() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    produce(&root, "a.md", "# A\n\nalpha\n");
    produce(&root, "b.md", "# B\n\nbeta\n");

    // Confirm the chain the honest writes built is actually continuous.
    let rows = parse_rows(&read_journal(&root));
    assert_eq!(rows.len(), 2, "two honest rows");

    let trace = check::journal_trace(&root).expect("journal trace");
    assert!(trace.chain.is_green(), "honest writes chain");
    assert!(trace.foreign_edit.is_none(), "no out-of-writer edit");
    assert!(!trace.is_red(), "a clean tree is green convention-free");
}
