//! U5.1 — the ungated close, and the ONE layer that still catches it (d2 §5.3). The scenario is
//! an ungated close: a bare status flip that freezes no verdict rev. **Correction (R1.3), not a
//! new loss.** This line read "a bare status flip that DECLARES EVIDENCE but freezes no verdict
//! rev" until now. The declares-evidence half was proven by the colors layer and by nothing
//! else, so it died at `eb45f0a9` when that layer was deleted — see Lost 2. The line has been
//! describing a property nothing proved since that commit; removing the clause dates the loss
//! where it belongs rather than to this edit. WHAT THIS FILE PROVES NOW - **refusal = armed
//! CHECK** — the U4.4 `close-verdict` floor, evaluated through `policy::gate()` over the law
//! `policy::resolve_armed_law()` resolved at the writes own path (the REAL armed change plane,
//! not a second refusal path), REFUSES the bare flip with a `block` violation. The positive
//! control: a PROPER gated close (a distinct reviewer verdict) passes that same armed gate.
//! WHAT THIS FILE NO LONGER PROVES — two propositions, both lost, neither moved It was THREE
//! layers, then TWO, and is now ONE. The name has followed the arity each time, because the
//! count IS the claim and a file whose thesis implies "every surviving layer catches it at
//! once" is a lie about coverage the moment the layers stop being plural. **Lost 1 — traces =
//! `co_edit_trace`** (recorded at the β train, unchanged). Every column of that view came from
//! `receipt_journal`. U4 recorded the projection as deleted with no replacement . The lost
//! claim, stated plainly so it is not rediscovered as a gap: *"the close write is visible
//! convention-free, without reading the task pages own semantics."* Nothing projects that now.
//! It is NOT re-pointed at git: git records commits, and asserting over them would be testing
//! git rather than the engine — a different claim wearing this tests name (the same reasoning
//! U6 applied to `floors.rs`). **Lost 2 — colors = the board view** (recorded here, U9a, R4).
//! The board rendered an ungated close GREY `declared-unpinned`. **R4 makes that state
//! UNREPRESENTABLE**, and this is structural rather than a fixture problem: the old fixture was
//! a lock item carrying a ref and NO rev — declared, unpinned.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::collections::BTreeMap;
use std::path::PathBuf;

use policy::armed::{ArmRequest, ArmRoot, Mode, PageSource, arm};
use policy::{
    ArmedLaw, ChangeOp, CheckLimits, GateOutcome, GateRefusal, Invocation, PageRef, RuleId,
    RuleIndex, ScopeLayer, derive_change, gate, page_rev, resolve_armed_law,
};

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

/// The armed law governing `at_path` with one floor rule armed at `mode`, MINTED by
/// `policy::armed::arm` through the landed resolver and then resolved back the way the door
/// resolves it. Never a hand-typed artifact: an artifact the test wrote by hand is an artifact
/// the arming act never approved.
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
        // The once-armed marker: this fixture IS an armed workspace, so the pivot says so. Omitting it
        // would make every leg below a never-armed no-op and the refusal assertions vacuous.
        //
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

/// A task page open, then closed as a BARE FLIP: `status: closed` with NO `verdict`. **The
/// pages carried an `^inputs` block until R1.3 and it is now removed.
///
///
///
///
///
///
///
fn bare_flip_before() -> String {
    "---\ntype: task\nstatus: open\nowner: worker-a\n---\n\n# Close\n\nclosing without a verdict.\n"
        .to_string()
}
fn bare_flip_after() -> String {
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Close\n\nclosing without a verdict.\n".to_string()
}

// ── the end-to-end wiring ─────────────────────────────────────────────────────

/// The ungated close, caught by the ONE surviving layer. Renamed twice now —
/// `…all_three_layers` → `…both_surviving_layers` → this. The count IS the claim, so the name
/// follows the arity every time: keeping a plural name over a single layer would be the
/// quietest possible lie about coverage.
///
///
#[test]
fn ungated_close_caught_by_the_one_surviving_layer() {
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

    // **THE DELETED LAYERS, recorded rather than asserted.** Both `co_edit_trace` (U4, with the
    // journal) and the boards declared-unpinned colour (U9a, R4) are gone. Neither is asserted
    // here any more: the first because its table is deleted and the second because its INPUT is
    // unrepresentable under R4 — an assertion over a state that cannot be constructed passes by
    // construction. The module doc names both lost propositions and the surviving carrier for the
    // law behind the second.
    //
}

/// Positive control: a PROPER gated close (reviewer ≠ owner, a real verdict, a distinct
/// reviewer verdict) passes the armed gate — the positive control for the one surviving layer,
/// so its refusal above is not a gate that refuses everything.
///
#[test]
fn proper_close_passes_the_armed_gate() {
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
}
