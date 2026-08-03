//! U5.1 — the collaboration layers wired end-to-end on ONE scenario (d2 §5.3):
//! an ungated close (a bare status flip that declares evidence but freezes no
//! verdict rev) is caught by every surviving layer at once —
//!
//! - **refusal = armed CHECK** — the U4.4 `close-verdict` floor, evaluated through
//!   `policy::gate()` over the law `policy::resolve_armed_law()` resolved at the
//!   write's own path (the REAL armed change plane, not a second refusal path),
//!   REFUSES the bare flip with a `block` violation.
//! - **colors = board view** — U2.9's locked read face renders the close's
//!   declared-unpinned `^inputs` edge GREY (never green, never silently clean).
//!
//! The positive control: a PROPER gated close (a distinct reviewer verdict + a
//! pinned, matching evidence rev) passes the armed gate AND renders green.
//!
//! # THIS FILE WAS THREE LAYERS AND IS NOW TWO — the traces layer is DELETED
//! **Layer 1 was `co_edit_trace`**: the reserved journal's mechanical write-facts,
//! which made a co-edit visible convention-free. Every column of that view came
//! from the `receipt_journal` table. U4 recorded the projection as **deleted with
//! no replacement** (ZT 2026-08-02, remove-no-replacement), and
//! `view::read_face`'s own module doc carries the tombstone.
//!
//! So the scenario is unchanged and the two surviving layers still catch it — but
//! **one of the three propositions this file existed to make is gone, not moved.**
//! The lost claim, stated plainly so it is not rediscovered as a gap: *"the close
//! write is visible convention-free, without reading the task page's own
//! semantics."* Nothing projects that now. It is NOT re-pointed at git: git records
//! commits, and asserting over them would be testing git rather than the engine —
//! a different claim wearing this test's name (the same reasoning U6 applied to
//! `floors.rs`'s "Ungated is not UNRECORDED").
//!
//! The layer was not weakened into passing. Its subject was removed, and the
//! removal is recorded here and on the β train's one lost-power accounting.

use std::collections::BTreeMap;
use std::path::PathBuf;

use policy::armed::{ArmRequest, ArmRoot, Mode, PageSource, arm};
use policy::{
    ArmedLaw, ChangeOp, CheckLimits, GateOutcome, GateRefusal, Invocation, PageRef, RuleId,
    RuleIndex, ScopeLayer, derive_change, gate, page_rev, resolve_armed_law,
};
use view::read_face::open_board;

// ── floor rule PAGES on disk, armed through the SAME acts the door reads ──────

fn floors(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/floors")
        .join(sub)
}

/// The workspace-relative page a floor rule is mounted at for this fixture.
fn floor_page(id: &str) -> String {
    format!("rules/{id}.md")
}

/// The floor rule page's committed bytes.
fn floor_bytes(id: &str) -> String {
    std::fs::read_to_string(floors(&format!("rules/{id}.md"))).expect("the floor page is readable")
}

/// The armed pages, keyed by workspace path — `policy` is I/O-free, so the law
/// resolver reads bytes through this rather than off disk.
struct MemPages(BTreeMap<String, String>);

impl PageSource for MemPages {
    fn read(&self, page: &str) -> std::io::Result<String> {
        self.0
            .get(page)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, page.to_owned()))
    }
}

/// The armed law governing `at_path` with one floor rule armed at `mode`, MINTED
/// by `policy::armed::arm` through the landed resolver and then resolved back the
/// way the door resolves it. Never a hand-typed artifact: an artifact the test
/// wrote by hand is an artifact the arming act never approved.
fn armed_law(id: &str, mode: Mode, at_path: &str) -> ArmedLaw {
    let page = floor_page(id);
    let bytes = floor_bytes(id);
    let index = RuleIndex::discover([PageRef {
        layer: ScopeLayer::Workspace,
        page: &page,
        bytes: &bytes,
    }]);
    let artifact = arm(
        &index,
        &ArmRoot::workspace(),
        [ArmRequest {
            id: RuleId::parse(id).expect("a legal id"),
            mode,
            attested_rev: page_rev(&bytes),
        }],
    )
    .expect("the floor arms at its live rev");
    let pages = MemPages(BTreeMap::from([(page, bytes)]));
    resolve_armed_law(
        Some(&artifact.render()),
        // The once-armed marker: this fixture IS an armed workspace, so the pivot
        // says so. Omitting it would make every leg below a never-armed no-op and
        // the refusal assertions vacuous.
        true,
        at_path,
        &pages,
        CheckLimits::default(),
    )
}

// ── fixtures ────────────────────────────────────────────────────────────────

fn doc(path: &str, md: &str) -> model::Document {
    let mut d = model::build(md.to_string(), syntax::parse(md));
    if let model::NodeKind::Document { path: p, .. } = &mut d.root.kind {
        *p = path.to_string();
    }
    d
}

const CLOSE_PATH: &str = "tasks/close.md";

/// A task page open, then closed as a BARE FLIP: `status: closed` with NO
/// `verdict`, declaring `subject.md#^claim` as an input but pinning no rev.
fn bare_flip_before() -> String {
    "---\ntype: task\nstatus: open\nowner: worker-a\n---\n\n# Close\n\n```yaml ^inputs\nitems:\n  - {ref: 'subject.md', to: 'subject.md#^claim', claim: 'ungated — no rev'}\n```\n".to_string()
}
fn bare_flip_after() -> String {
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Close\n\n```yaml ^inputs\nitems:\n  - {ref: 'subject.md', to: 'subject.md#^claim', claim: 'ungated — no rev'}\n```\n".to_string()
}

// ── the end-to-end wiring ─────────────────────────────────────────────────────

/// The ungated close, caught by both surviving layers at once.
///
/// Renamed from `ungated_close_caught_by_all_three_layers`: the count was the
/// claim, so keeping the old name over two layers would have been the quietest
/// possible lie about coverage.
#[test]
fn ungated_close_caught_by_both_surviving_layers() {
    // Layer 3 — refusal = armed CHECK: the close-verdict floor, armed and run
    // through the REAL gate(), refuses the bare flip.
    let armed = armed_law("close-verdict", Mode::Block, CLOSE_PATH);
    let before = doc(CLOSE_PATH, &bare_flip_before());
    let after = doc(CLOSE_PATH, &bare_flip_after());
    let change = derive_change(
        &before,
        &after,
        &[],
        Invocation {
            op: ChangeOp::Splice,
            actor: Some("worker-a"),
            force: false,
        },
        &[],
        &|_: &str| None,
    );
    let outcome = gate(&change, &armed);
    let GateOutcome::Refusal(GateRefusal::Blocked { violations }) = &outcome else {
        panic!("the armed close-verdict floor must refuse the bare flip: {outcome:?}");
    };
    assert!(
        violations.iter().any(|v| v.rule == "close-verdict"),
        "the refusal names the close-verdict floor by its rule id: {violations:?}",
    );

    // Layer 2 — colors = board view: the SAME closed page renders GREY (its
    // declared-unpinned `^inputs` edge), never green, never silently clean.
    let mut docs = BTreeMap::new();
    docs.insert(CLOSE_PATH.to_string(), doc(CLOSE_PATH, &bare_flip_after()));
    docs.insert(
        "subject.md".to_string(),
        doc("subject.md", "# Subject\n\nbody. ^claim\n"),
    );
    // The traces layer stood HERE: a journal row was handed to `open_board` beside
    // the docs, and the close write was then read back out of `co_edit_trace`. Both
    // the row type and the view are deleted, so `open_board` takes docs alone.
    let conn = open_board(&docs).expect("open board");

    let (color, reason): (String, String) = conn
        .query_row(
            "SELECT color, reason FROM board WHERE src_path = ? AND to_path='subject.md'",
            [CLOSE_PATH],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the ungated close must appear on the board");
    assert_eq!(color, "grey", "an ungated close is grey, never green");
    assert_eq!(reason, "declared-unpinned");

    // **THE DELETED LAYER, asserted as deleted.** `co_edit_trace` is gone with the
    // journal it projected. This is not a repair of the old assert — it is the
    // opposite assert, and it exists so that anyone who rebuilds the view rebuilds
    // it deliberately: this line fails the moment the table comes back, and
    // whoever brings it back has to come here and say why.
    let trace = conn.prepare("SELECT 1 FROM co_edit_trace LIMIT 1");
    assert!(
        trace.is_err(),
        "the traces layer is DELETED WITH NO REPLACEMENT (U4). A co-edit is no \
         longer visible convention-free, and this file no longer claims it is."
    );
}

/// Positive control: a PROPER gated close (reviewer ≠ owner, a real verdict, a
/// pinned matching evidence rev) passes the armed gate AND renders green — the
/// three layers agree the close is clean.
#[test]
fn proper_close_passes_gate_and_renders_green() {
    // Refusal layer: a close carrying a verdict passes the close-verdict floor.
    let armed = armed_law("close-verdict", Mode::Block, CLOSE_PATH);
    let before = doc(
        CLOSE_PATH,
        "---\ntype: task\nstatus: open\nowner: worker-a\n---\n\n# Close\n\nbody\n",
    );
    let after = doc(
        CLOSE_PATH,
        "---\ntype: task\nstatus: closed\nverdict: approve\nowner: worker-a\n---\n\n# Close\n\nbody\n",
    );
    let change = derive_change(
        &before,
        &after,
        &[],
        Invocation {
            op: ChangeOp::Splice,
            actor: Some("reviewer-b"),
            force: false,
        },
        &[],
        &|_: &str| None,
    );
    assert_eq!(
        gate(&change, &armed),
        GateOutcome::Ok(Vec::new()),
        "a close carrying a verdict passes the armed close-verdict floor",
    );

    // Colors layer: a verdict pinning its subject at the live rev renders green.
    let subject = "# Subject\n\nbody. ^claim\n";
    let mut probe = BTreeMap::new();
    probe.insert("subject.md".to_string(), doc("subject.md", subject));
    let probe_conn = open_board(&probe).expect("probe board");
    let rev: String = probe_conn
        .query_row(
            "SELECT node_rev FROM node WHERE path='subject.md' AND selector='^claim'",
            [],
            |r| r.get(0),
        )
        .expect("subject ^claim rev");
    let review = format!(
        "---\ntype: verdict\nreviewer: reviewer-b\n---\n\n# Verdict\n\napprove\n\n```yaml ^inputs\nitems:\n  - {{ref: 'subject.md', to: 'subject.md#^claim', rev: '{rev}', rev_class: 'content'}}\n```\n"
    );
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc("subject.md", subject));
    docs.insert(
        "verdicts/close.md".to_string(),
        doc("verdicts/close.md", &review),
    );
    let conn = open_board(&docs).expect("open board");
    let color: String = conn
        .query_row(
            "SELECT color FROM board WHERE src_path='verdicts/close.md'",
            [],
            |r| r.get(0),
        )
        .expect("the gated close appears on the board");
    assert_eq!(
        color, "green",
        "a proper gated close with a matching pin is green"
    );
}
