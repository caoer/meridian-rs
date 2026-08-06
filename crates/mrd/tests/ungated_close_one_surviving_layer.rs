//! U5.1 — the ungated close, and the ONE layer that still catches it (d2 §5.3).
//! The scenario is a bare status flip that freezes no verdict rev: the U4.4
//! `close-verdict` floor, evaluated through `policy::gate()` over the law
//! `policy::resolve_armed_law()` resolves at the writes own path — the REAL
//! armed change plane, not a second refusal path — refuses it with a `block`
//! violation. Positive control: a PROPER gated close (a distinct reviewer
//! verdict) passes that same armed gate.

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
/// resolves it. Never a hand-typed artifact — the arming act must approve it.
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
        // The once-armed marker: without it every leg below is a never-armed
        // no-op and the refusal assertions go vacuous.
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

/// A task page open, then closed as a BARE FLIP: `status: closed` with NO `verdict`.
fn bare_flip_before() -> String {
    "---\ntype: task\nstatus: open\nowner: worker-a\n---\n\n# Close\n\nclosing without a verdict.\n"
        .to_string()
}
fn bare_flip_after() -> String {
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Close\n\nclosing without a verdict.\n".to_string()
}

// ── the end-to-end wiring ─────────────────────────────────────────────────────

/// The ungated close, caught by the ONE surviving layer. The count IS the
/// claim, so the name follows the arity every time.
#[test]
fn ungated_close_caught_by_the_one_surviving_layer() {
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
}

/// Positive control: a PROPER gated close (reviewer ≠ owner, a real verdict)
/// passes the armed gate, so the refusal above is not a gate that refuses
/// everything.
#[test]
fn proper_close_passes_the_armed_gate() {
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
