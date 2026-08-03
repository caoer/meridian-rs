//! U18 (docket row F2) — the board's status-move guard, driven through the door.
//!
//! The guard is a PAGE (`tests/rules/status-move.md`), armed and evaluated through
//! the same `RuleIndex::discover` → `arm` → `resolve_armed_law` → `gate` path the
//! write door runs. That is the whole point of the unit: verdict row 9.12
//! (gate-as-rev) is SUPERSEDED — a status move is guarded by an armed starlark
//! CHECK, never by a new engine surface.
//!
//! The refusal TEXT is asserted, not merely the refusal: a guard whose message
//! stops naming the move, the legal moves, or the way forward regresses silently.

use std::collections::BTreeMap;

use model::{Document, NodeKind};
use policy::armed::{ArmRequest, ArmRoot, Mode, PageSource, arm};
use policy::{
    Change, ChangeOp, CheckLimits, GateOutcome, GateRefusal, Invocation, PageRef, RuleId,
    RuleIndex, ScopeLayer, derive_change, gate, page_rev, resolve_armed_law,
};

/// The rule page under test — the real file, read as the disk edge would.
const STATUS_MOVE: &str = include_str!("rules/status-move.md");

/// The card the moves are made on.
const CARD: &str = "tasks/fix-parser.md";

/// The workspace's pages, as a `path → bytes` map.
struct MemPages(BTreeMap<String, String>);

impl PageSource for MemPages {
    fn read(&self, page: &str) -> std::io::Result<String> {
        self.0.get(page).cloned().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {page}"))
        })
    }
}

fn doc_of(path: &str, md: &str) -> Document {
    let nodes = syntax::parse(md);
    let mut doc = model::build(md.to_string(), nodes);
    if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    doc
}

/// The armed law governing a write at the card, with `status-move` armed `block`
/// at the workspace root.
fn armed_law() -> policy::ArmedLaw {
    let page = "rules/status-move.md";
    let index = RuleIndex::discover(std::iter::once(PageRef {
        layer: ScopeLayer::Workspace,
        page,
        bytes: STATUS_MOVE,
    }));
    let artifact = arm(
        &index,
        &ArmRoot::workspace(),
        std::iter::once(ArmRequest {
            id: RuleId::parse("status-move").expect("a legal id"),
            mode: Mode::Block,
            attested_rev: page_rev(STATUS_MOVE),
        }),
    )
    .expect("the page arms")
    .render();
    let pages = MemPages(BTreeMap::from([(page.to_string(), STATUS_MOVE.to_string())]));
    resolve_armed_law(Some(&artifact), true, CARD, &pages, CheckLimits::default())
}

/// A card written from `before` frontmatter to `after` frontmatter.
fn change_between(before_fm: &str, after_fm: &str) -> Change {
    let before = doc_of(CARD, &format!("---\n{before_fm}---\n# Fix\n\nbody\n"));
    let after = doc_of(CARD, &format!("---\n{after_fm}---\n# Fix\n\nbody\n"));
    derive_change(
        &before,
        &after,
        &[],
        Invocation {
            op: ChangeOp::Splice,
            actor: Some("worker-a"),
            force: false,
        },
        &[],
        &|_| None,
    )
}

/// A pure status move `from` → `to`.
fn status_move(from: &str, to: &str) -> Change {
    change_between(
        &format!("type: task\nstatus: {from}\n"),
        &format!("type: task\nstatus: {to}\n"),
    )
}

/// The single violation the gate refused with, or a panic naming what happened.
fn refusal_of(change: &Change) -> policy::GateViolation {
    match gate(change, &armed_law()) {
        GateOutcome::Refusal(GateRefusal::Blocked { mut violations }) => {
            assert_eq!(violations.len(), 1, "one guard, one refusal: {violations:?}");
            violations.remove(0)
        }
        other => panic!("the move must refuse: {other:?}"),
    }
}

// ── the refused move: the message TEXT is the contract ────────────────────────

/// An illegal move (`review` → `todo`) refuses, and the refusal NAMES the card,
/// the move, the legal moves out of the current status, and the way forward.
#[test]
fn an_illegal_move_refuses_with_the_authored_message() {
    let violation = refusal_of(&status_move("review", "todo"));
    assert_eq!(violation.rule, "status-move");
    assert_eq!(violation.passing_scenario, "advance-to-next-status");
    assert_eq!(
        violation.message,
        "status-move: tasks/fix-parser.md moved `review` -> `todo`, which is not a \
         legal board move. From `review` the legal moves are: `done`, `in-progress`. \
         Move the card one step along the board (`todo` -> `in-progress` -> `review` \
         -> `done`), or bounce it back from `review` to `in-progress` for rework."
    );
}

/// A move out of `done` refuses in its own words — the board has no move out of a
/// finished card, so the generic "legal moves are: …" sentence would name nothing.
#[test]
fn a_move_out_of_done_refuses_naming_the_terminal_status() {
    let violation = refusal_of(&status_move("done", "in-progress"));
    assert_eq!(
        violation.message,
        "status-move: tasks/fix-parser.md moved `done` -> `in-progress`. The board \
         has no move out of `done` — a finished card stays finished. Open a new card \
         for the follow-on work."
    );
}

/// A card sitting at an off-vocabulary status refuses with the board named, so the
/// operator can put the card on the board before moving it.
#[test]
fn an_off_vocabulary_source_status_refuses_naming_the_board() {
    let violation = refusal_of(&status_move("parked", "review"));
    assert_eq!(
        violation.message,
        "status-move: tasks/fix-parser.md sits at `parked`, which is not a board \
         status. The board is `todo` -> `in-progress` -> `review` -> `done`. Set the \
         card to the board status it really occupies, then move it one step."
    );
}

// ── the permitted moves ───────────────────────────────────────────────────────

/// Every forward step, plus the `review` → `in-progress` bounce, lands.
#[test]
fn the_legal_moves_land() {
    for (from, to) in [
        ("todo", "in-progress"),
        ("in-progress", "review"),
        ("review", "done"),
        ("review", "in-progress"),
    ] {
        assert_eq!(
            gate(&status_move(from, to), &armed_law()),
            GateOutcome::Ok(Vec::new()),
            "`{from}` -> `{to}` is a legal board move"
        );
    }
}

// ── the no-ops: what the guard must never judge ───────────────────────────────

/// A write that leaves `status:` alone lands, even when it changes other fields —
/// the guard judges the MOVE, never the card.
#[test]
fn a_write_that_does_not_move_the_status_is_never_judged() {
    let change = change_between(
        "type: task\nstatus: review\nowner: worker-a\n",
        "type: task\nstatus: review\nowner: worker-b\n",
    );
    assert_eq!(
        change.fields_changed,
        vec!["owner".to_string()],
        "only `owner` moved"
    );
    assert_eq!(gate(&change, &armed_law()), GateOutcome::Ok(Vec::new()));
}

/// A card's BIRTH status is not a move: with no `status:` before, any starting
/// status lands.
#[test]
fn a_cards_birth_status_is_not_a_move() {
    let change = change_between("type: task\n", "type: task\nstatus: review\n");
    assert_eq!(gate(&change, &armed_law()), GateOutcome::Ok(Vec::new()));
}
