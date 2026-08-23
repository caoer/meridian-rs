//! The lazy root-at-eval fold (`docs/run-plane.md` § The run plane): the run
//! leg folds the hash domain when, and only when, a produced effect will carry
//! the token.
//!
//! Why it exists: on a 37 800-member root `fs::domain_snapshot` was **99.5%**
//! of an effect-free `mrd run` — 545 of 547 instrumented ms, ~0.9 s of wall —
//! spent folding a corpus to stamp a token onto nothing (card
//! `mrd-run-lazy-snapshot`, `results/mrd-run-perf/investigation.md`). Nobody
//! could read it: the sandbox is not given `ctx.root_at_eval`
//! (`effects::RunCtx`), an effect-free block has no provenance to stamp, no
//! md.\* batch to hand it to and no receipt to attest it.
//!
//! The two claims gated here, and they are opposite halves of one rule:
//! **no effects ⇒ no fold**, and **any effect ⇒ exactly one fold, and its
//! token is the live `fs::domain_snapshot` of the same tree** — in both
//! tenses, dry and live, byte-identical to what the eager fold produced.
//!
//! This binary must be the only fold-asserting work in its process
//! (`fs::fold_count` is process-global; assert as a difference — the pattern
//! of `registry/tests/sub_detect_quiet.rs`).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use effects::{Effect, EvalLimits, Provenance};
use run::caps::{Authority, CapSet};
use run::dispatch_starlark::{self, StarlarkDispatch};
use run::executor::ReceiptAddr;

/// Serializes the tests in this binary: `fold_count` is process-global, so a
/// difference assertion is only sound while nothing else in the process folds.
static FOLDS: Mutex<()> = Mutex::new(());

fn fold_guard() -> MutexGuard<'static, ()> {
    // A failed sibling test poisons the guard; the count discipline survives.
    FOLDS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

const PAGE: &str = "\
---
status: todo
---

# Tasks
";

/// A block that emits nothing at all — the shape ZT's complaint was about.
const PASS: &str = "def run(ctx):\n    pass\n";
/// A block that emits ONE non-md effect: no batch, no receipt, still stamped.
const NOTICE: &str = "def run(ctx):\n    notice(message = \"advisory only\")\n";
/// A block that emits an md.\* effect: the batch carries `observed_root`.
const SET_FIELD: &str = "def run(ctx):\n    set_field(field = \"status\", value = \"done\")\n";

const RECEIPT: &str = "receipts/2026-07-22.md";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn dispatch_of<'a>(source: &'a str, authority: &'a Authority) -> StarlarkDispatch<'a> {
    StarlarkDispatch {
        page: "page.md",
        task: "fix-x",
        task_rev: "b3:proc-star",
        source,
        args: vec![],
        env: BTreeMap::new(),
        invocation_id: "inv-1",
        now: Some("2026-07-22T02:00:00Z"),
        authority,
        receipt: Some(ReceiptAddr {
            path: RECEIPT.to_owned(),
            anchor: "r-000001".to_owned(),
        }),
        limits: EvalLimits::default(),
        actor: None,
        delta: None,
        fields: &TEST_EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

/// Every effect's stamped observation, deduplicated by inspection.
fn stamped(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .map(|e| match &e.provenance {
            Provenance::Run { root_at_eval, .. } => root_at_eval.as_str(),
            Provenance::Change { .. } => panic!("run plane emits Run provenance"),
        })
        .collect()
}

/// **No effects ⇒ no fold**, in the dry tense (`evaluate` +
/// `observe_if_emitted`, the pair `runner::rehearse` runs) and in the live one
/// (`dispatch`). The saving IS this assertion: a fold that does not happen.
#[test]
fn an_effect_free_block_folds_nothing_in_either_tense() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();
    let caps = Authority::granted(CapSet::parse("md.edit").unwrap());

    // Dry: the rehearsal's own pair.
    let before = fs::fold_count();
    let mut effects = dispatch_starlark::evaluate(&dispatch_of(PASS, &caps)).unwrap();
    assert!(effects.is_empty(), "the fixture emits nothing");
    let observed = dispatch_starlark::observe_if_emitted(&root, &mut effects).unwrap();
    assert!(observed.is_none(), "nothing to stamp, so nothing observed");
    assert_eq!(
        fs::fold_count(),
        before,
        "an effect-free rehearsal walked the corpus"
    );

    // Live: the same claim through the whole dispatch.
    let before = fs::fold_count();
    let out = dispatch_starlark::dispatch(&root, &dispatch_of(PASS, &caps)).unwrap();
    assert!(out.effects.is_empty());
    assert!(out.applied.is_none());
    assert_eq!(
        fs::fold_count(),
        before,
        "an effect-free live run walked the corpus"
    );

    // And it stayed a no-op on disk: no write, no receipt.
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    assert!(!root.0.join(RECEIPT).exists());
}

/// **Any effect ⇒ exactly one fold, carrying the live token.** A `notice`
/// emits no md.\* batch and writes no receipt, so this is the arm where the
/// ONLY consumer of the token is the reported provenance — the arm a
/// "fold when we are about to write" rule would get wrong.
#[test]
fn a_non_md_effect_still_folds_once_and_is_stamped_with_it() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();
    let caps = Authority::granted(CapSet::none());
    // The comparison token, taken BEFORE the measured window and on the same
    // unchanged tree: a `notice` writes nothing, so the domain cannot move.
    let live = fs::domain_snapshot(&root).unwrap().1;

    let before = fs::fold_count();
    let mut effects = dispatch_starlark::evaluate(&dispatch_of(NOTICE, &caps)).unwrap();
    let observed = dispatch_starlark::observe_if_emitted(&root, &mut effects).unwrap();
    assert_eq!(
        fs::fold_count() - before,
        1,
        "one emitted effect buys exactly one fold"
    );

    assert_eq!(observed.as_ref(), Some(&live), "the fold is the live root");
    assert_eq!(stamped(&effects), vec![live.0.as_str()]);
    assert!(!live.0.is_empty(), "a real token, not the placeholder");
}

/// The live md.\* arm: one fold, every effect stamped with it, and the SAME
/// token reaches the executor as `observed_root` — which the receipt row
/// attests as `root_pin`. The fold is taken after the eval and before the
/// write, so it is the domain the effects were produced against.
#[test]
fn an_md_effect_stamps_the_batch_and_the_receipt_with_one_fold() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();
    let caps = Authority::granted(CapSet::parse("md.edit").unwrap());
    // Taken before the run: the apply moves the domain afterwards, so this is
    // the only tense in which the two are comparable.
    let live = fs::domain_snapshot(&root).unwrap().1;

    let before = fs::fold_count();
    let out = dispatch_starlark::dispatch(&root, &dispatch_of(SET_FIELD, &caps)).unwrap();
    let folds = fs::fold_count() - before;

    let applied = out.applied.expect("md.* applied");
    assert_eq!(applied.applied, 1);
    assert_eq!(stamped(&out.effects), vec![live.0.as_str()]);
    assert_eq!(
        folds, 1,
        "the run leg's own fold, once — a second means the lazy seam ran twice"
    );

    // The receipt attests the same observation the effects carry.
    let receipt = std::fs::read_to_string(root.0.join(RECEIPT)).expect("receipt written");
    assert!(
        receipt.contains(&live.0),
        "receipt root_pin is the observed root:\n{receipt}"
    );

    // The write landed — this arm is the real one, not a rehearsal.
    assert!(
        std::fs::read_to_string(root.0.join("page.md"))
            .unwrap()
            .contains("status: done")
    );
}

/// A corpus change between two runs moves the stamp. Without this, "the token
/// is stamped" passes just as well on a frozen constant.
#[test]
fn the_stamp_follows_the_corpus() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();
    let caps = Authority::granted(CapSet::none());

    let mut first = dispatch_starlark::evaluate(&dispatch_of(NOTICE, &caps)).unwrap();
    dispatch_starlark::observe_if_emitted(&root, &mut first).unwrap();

    std::fs::write(root.0.join("second.md"), "# Second\n").unwrap();

    let mut second = dispatch_starlark::evaluate(&dispatch_of(NOTICE, &caps)).unwrap();
    dispatch_starlark::observe_if_emitted(&root, &mut second).unwrap();

    assert_ne!(
        stamped(&first),
        stamped(&second),
        "a new domain member must move the observed root"
    );
}
