//! End-to-end `check` fixtures over a REAL tmpdir workspace (U2.10 gates).
//!
//! Every fixture seeds the journal through the PRODUCTION write path (a guarded
//! `create` mints real journal rows with real tree roots — no in-memory double),
//! then exercises the `check` engine over the resulting on-disk state:
//! - [`spliced_journal_caught_through_check`] — the U2.1 spliced-row fixture
//!   caught THROUGH `check` itself (chain red, the forged row cited), not just at
//!   the library level;
//! - [`a_moved_tree_is_a_stale_baseline_whoever_moved_it`] — a tree that no longer
//!   matches the last receipt is grey, not red: an out-of-writer edit and a
//!   governed splice leave identical evidence (S3-R8, superseding the
//!   `foreign_edit` RED this file used to assert);
//! - [`layer1_run_is_read_only`] — a layer-1 armed run over the tree mutates
//!   nothing (no rev, no journal row, no file byte).

use std::collections::BTreeMap;
use std::path::Path;

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use receipt::journal::{JournalRow, parse_rows, render_row};
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, SpliceArgs, create, splice};

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

/// Edit `path`'s body through the PRODUCTION splice write path — a fully governed
/// write (flock, CAS, armed gate, receipt), which advances the tree root and
/// journals nothing. That asymmetry with [`produce`] is the mechanism under test.
fn splice_through_the_write_path(root: &WorkspaceRoot, path: &str, old: &str, new: &str) {
    let args = SpliceArgs {
        id: None,
        path: WirePath(path.to_string()),
        actor: Some("agent:alice".to_string()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![wire::Edit {
            target: wire::SecRef::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: "A".to_string(),
                    n: None,
                }],
            },
            edit: wire::EditShape::Match {
                old: old.to_string(),
                new: new.to_string(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    };
    splice(root, 0, &args, &[], None)
        .unwrap_or_else(|e| panic!("production splice on {path} refused: {e:?}"));
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

    // Through check itself (not the bare check_chain library call). The last
    // honest row still records the LIVE root — the journal is root-excluded, so
    // the forged line moved no tree byte — which keeps the baseline current and
    // the chain break the isolated signal.
    let trace = check::journal_trace(&root).expect("journal trace");
    let check::JournalTrace::Assessed { chain } = &trace else {
        panic!(
            "the journal's last receipt still accounts for the live tree — this TRACE is \
             assessed, not grey"
        );
    };
    assert!(chain.is_red(), "the spliced row breaks the chain");
    assert_eq!(
        chain.breaks[0].row_anchor, "r-000099",
        "the forged row is cited first"
    );
    let summary = trace.red_summary().expect("red render");
    assert!(
        summary.contains("^r-000099"),
        "the red render cites the forged row: {summary}"
    );
}

/// **The stale-baseline fixture and its green-path control** (S3-R8, then U32).
/// One honest write records the tree root; then the tree moves twice, and the two
/// movers must now be told apart.
///
/// This test supersedes `foreign_edit_caught_through_check`, which asserted RED on
/// the out-of-writer case. That red was measured on the deployed binary against a
/// fully governed corpus and it accused a legitimate write (finding-01, corrected),
/// so check states the mismatch as evidence and declines to name a cause.
///
/// **U32 changed the second arm's outcome, and that inversion is the unit.** A
/// governed splice used to land in the SAME stale state as an out-of-band edit —
/// it advanced the root and journaled nothing. It now journals its row, so the
/// governed corpus is ASSESSED and green while the out-of-band one stays grey.
/// Without this pairing a "fix" that greyed everything would read as success
/// (S3-R8(c)).
///
/// The accusation stays withdrawn even so: a stale baseline is still what ANY
/// byte-landing door that skips the journal leaves, and the tree cannot say who
/// moved it. What U32 bought is that governed work no longer produces the state.
#[test]
fn a_governed_splice_re_dates_the_tree_an_out_of_band_edit_does_not() {
    // ── cause 1: an out-of-writer edit ──────────────────────────────────────
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    produce(&root, "a.md", "# A\n\nalpha\n");
    let recorded = live_root(&root); // the root the receipt r-000001 recorded

    std::fs::write(root.0.join("a.md"), "# A\n\nEDITED OUT OF BAND\n").expect("raw edit");
    let now = live_root(&root);
    assert_ne!(recorded, now, "the out-of-band edit moved the tree root");

    let trace = check::journal_trace(&root).expect("journal trace");
    let check::JournalTrace::StaleBaseline(m) = &trace else {
        panic!("the live tree no longer matches the last receipt: that is a stale baseline");
    };
    assert_eq!(m.last_receipt, "r-000001", "cites the last journaled write");
    assert_eq!(
        m.recorded_root, recorded,
        "the receipt's recorded root_after"
    );
    assert_eq!(m.live_root, now, "the drifted live root");
    assert!(
        !trace.is_red(),
        "the evidence is real but it does not identify a culprit — grey, not red"
    );
    assert!(trace.cannot_assess(), "and it says so");

    // The grey names no culprit — the withdrawn accusation, still withdrawn.
    let grey = trace.grey_summary().expect("the grey carries its reason");
    assert!(
        grey.contains("r-000001") && grey.contains(&recorded) && grey.contains(&now),
        "the EVIDENCE is stated in full: {grey}"
    );
    assert!(
        !grey.contains("out-of-writer") && !grey.contains("splice"),
        "but no cause is named — the tree cannot say who moved it: {grey}"
    );

    // ── the green-path control: a fully GOVERNED splice, U32 ─────────────────
    let dir = tempfile::tempdir().expect("tmpdir");
    let governed = WorkspaceRoot(dir.path().to_path_buf());
    produce(&governed, "a.md", "# A\n\nalpha\n");
    let recorded = live_root(&governed);
    let rows_before = parse_rows(&read_journal(&governed)).len();

    splice_through_the_write_path(&governed, "a.md", "alpha", "beta");

    let rows = parse_rows(&read_journal(&governed));
    assert_eq!(
        rows.len(),
        rows_before + 1,
        "the mechanism, asserted not assumed: U32 — a governed splice journals a row"
    );
    assert_ne!(
        recorded,
        live_root(&governed),
        "it DOES move the tree root — which is what staleness WOULD have been"
    );
    let last = rows.last().expect("the splice row");
    assert_eq!(last.op, "splice", "and the row names the op that moved it");
    assert_eq!(
        last.root_after,
        live_root(&governed),
        "the row re-dates the tree: its root_after IS the live root"
    );

    let trace = check::journal_trace(&governed).expect("journal trace");
    let check::JournalTrace::Assessed { chain } = &trace else {
        panic!(
            "THE FIX: a corpus whose every write was governed is ASSESSED, not grey — \
             {trace:?}"
        );
    };
    assert!(
        chain.is_green(),
        "and it reads green: create → splice is one continuous chain: {chain:?}"
    );
    assert!(
        !trace.cannot_assess(),
        "a governed corpus is something check CAN assess"
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
    let check::JournalTrace::Assessed { chain } = &trace else {
        panic!("two governed creates left a current baseline — this TRACE is assessed, not grey");
    };
    assert!(chain.is_green(), "honest writes chain");
    assert!(!trace.is_red(), "a clean tree is green convention-free");
    assert!(
        !trace.cannot_assess(),
        "the last receipt accounts for the live tree — this green is earned, not vacuous"
    );
}

/// **S3-R5, at the engine surface.** A workspace with no governed write has no
/// journal row, so the TRACE is grey: it cannot assess, it is not red, and it
/// carries no green for a caller to render. The very next governed write gives it
/// a baseline and the same workspace becomes assessable — the grey is about the
/// evidence, not about the workspace being new.
#[test]
fn a_workspace_with_no_journal_row_cannot_be_assessed() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    std::fs::write(dir.path().join("a.md"), "# A\n\nalpha\n").expect("raw write");
    assert!(
        !root.0.join(RESERVED_JOURNAL_PATH).exists(),
        "nothing governed has written here"
    );

    let trace = check::journal_trace(&root).expect("journal trace");
    assert!(trace.cannot_assess(), "no rows, no verdict");
    assert!(!trace.is_red(), "grey is not red");
    assert_eq!(
        trace.red_summary(),
        None,
        "no lie was found — none was read"
    );

    // One governed write later, the same tree IS assessable.
    produce(&root, "b.md", "# B\n\nbeta\n");
    let trace = check::journal_trace(&root).expect("journal trace");
    assert!(
        !trace.cannot_assess(),
        "a journaled write is the baseline the detectors needed"
    );
    assert!(!trace.is_red(), "the governed write is attributable");
}
