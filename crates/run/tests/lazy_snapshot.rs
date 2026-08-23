//! The lazy root-at-eval fold (`docs/run-plane.md` § The run plane): the
//! starlark leg folds the hash domain when, and only when, THAT TENSE'S output
//! will put the token in front of a reader.
//!
//! Why it exists: on a 37 800-member root `fs::domain_snapshot` was **99.5%**
//! of an effect-free `mrd run` — 545 of 547 instrumented ms, ~0.9 s of wall —
//! spent folding a corpus to stamp a token onto nothing (card
//! `mrd-run-lazy-snapshot`, `results/mrd-run-perf/investigation.md`). Nobody
//! could read it: the sandbox is not given `ctx.root_at_eval`
//! (`effects::RunCtx`), an effect-free block has no provenance to stamp, no
//! md.\* batch to hand it to and no receipt to attest it.
//!
//! The gate is per tense because the readers differ:
//!
//! | tense | folds when | reader |
//! |---|---|---|
//! | `Observation::Rehearsal` | any effect | the `--dry` report, which serializes whole effects |
//! | `Observation::Live` | an md.\* effect | the receipt's `root_pin`, via `observed_root` |
//!
//! The live `notice`-only case is the one that pays: the live report is
//! `kind` + `domain` only, so that run's token has no reader anywhere — and
//! that is the shape the hook plane fires, live, once per event.
//!
//! This binary must be the only fold-asserting work in its process
//! (`fs::fold_count` is process-global; assert as a difference — the pattern
//! of `registry/tests/sub_detect_quiet.rs`).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use effects::{Effect, EvalLimits, Provenance};
use run::caps::{Authority, CapSet};
use run::dispatch_starlark::{self, Observation, StarlarkDispatch};
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
/// A block that emits ONE non-md effect — the hook shape.
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
        // No Delta sink: `executor::delta_pre_facts` takes a SECOND fold when
        // one is armed ("the CLI pays no fold"), so an exact count over a
        // whole `dispatch` is only the lazy seam's while this stays `None`.
        delta: None,
        fields: &TEST_EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

/// Every effect's stamped observation.
fn stamped(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .map(|e| match &e.provenance {
            Provenance::Run { root_at_eval, .. } => root_at_eval.as_str(),
            Provenance::Change { .. } => panic!("run plane emits Run provenance"),
        })
        .collect()
}

/// Fold the given source in the given tense; answer `(folds taken, stamps)`.
/// The count brackets the SEAM only — `observe_if_emitted` — so no other
/// fold can be miscounted into it.
fn observe(root: &fs::WorkspaceRoot, source: &str, tense: Observation) -> (u64, Vec<String>) {
    let caps = Authority::granted(CapSet::parse("md.edit").unwrap());
    let evaluated = dispatch_starlark::evaluate(&dispatch_of(source, &caps)).unwrap();
    let before = fs::fold_count();
    let (effects, _) = dispatch_starlark::observe_if_emitted(root, evaluated, tense).unwrap();
    let folds = fs::fold_count() - before;
    (
        folds,
        stamped(&effects).into_iter().map(str::to_owned).collect(),
    )
}

/// **No effect ⇒ no fold, in either tense.** The saving IS this assertion: a
/// fold that does not happen.
#[test]
fn an_effect_free_block_folds_nothing_in_either_tense() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();

    for tense in [Observation::Rehearsal, Observation::Live] {
        let (folds, stamps) = observe(&root, PASS, tense);
        assert_eq!(folds, 0, "an effect-free block folded in {tense:?}");
        assert!(stamps.is_empty(), "the fixture emits nothing");
    }
}

/// **The live gate is md-only, and the `notice` case is why it exists.** A
/// live block that emits one `proto.notice` has no reader for the token:
/// no batch, no `observed_root`, no receipt, and `report::EffectLine` renders
/// kind + domain only. So it must not fold — this is the hook shape, fired
/// once per event, and folding here would spend the whole run on nothing.
#[test]
fn a_live_notice_only_block_does_not_fold() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();

    let (folds, stamps) = observe(&root, NOTICE, Observation::Live);
    assert_eq!(folds, 0, "the live gate folded for a token with no reader");
    assert_eq!(
        stamps,
        vec![String::new()],
        "unobserved is spelled empty, not with a stale token"
    );

    // And the whole live dispatch agrees — nothing applied, nothing written.
    let caps = Authority::granted(CapSet::none());
    let before = fs::fold_count();
    let out = dispatch_starlark::dispatch(&root, &dispatch_of(NOTICE, &caps)).unwrap();
    assert_eq!(
        fs::fold_count(),
        before,
        "dispatch folded for a live notice"
    );
    assert!(out.applied.is_none());
    assert_eq!(out.unexecuted.len(), 1);
    assert!(!root.0.join(RECEIPT).exists());
}

/// The SAME block in the rehearsal tense DOES fold: `--dry` serializes whole
/// effects, so the token has a reader there. One rule, two tenses, and the
/// difference is the reader — not a special case.
#[test]
fn the_same_notice_folds_once_in_the_rehearsal_tense() {
    let _guard = fold_guard();
    let (_tmp, root) = workspace();
    // The comparison token, taken outside the measured bracket and on the same
    // unchanged tree: a `notice` writes nothing, so the domain cannot move.
    let live = fs::domain_snapshot(&root).unwrap().1;

    let (folds, stamps) = observe(&root, NOTICE, Observation::Rehearsal);
    assert_eq!(folds, 1, "one emitted effect buys exactly one fold");
    assert_eq!(stamps, vec![live.0.clone()], "the fold is the live root");
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
    // One fold across the whole dispatch — the lazy seam's. `delta: None`
    // above is what keeps `executor::delta_pre_facts` out of this count; with
    // a Delta sink armed the honest expectation is two.
    let folds = fs::fold_count() - before;

    let applied = out.applied.expect("md.* applied");
    assert_eq!(applied.applied, 1);
    assert_eq!(stamped(&out.effects), vec![live.0.as_str()]);
    assert_eq!(folds, 1, "the lazy seam folded more than once");

    // The receipt attests the same observation the effects carry. Matched as
    // the FIELD, not as a substring of the file: the row is compact JSON
    // (`executor::render_receipt` → `serde_json::to_string`), and a bare
    // `contains(token)` would also pass on a token that landed in some other
    // field.
    let receipt = std::fs::read_to_string(root.0.join(RECEIPT)).expect("receipt written");
    assert!(
        receipt.contains(&format!("\"root_pin\":\"{}\"", live.0)),
        "receipt root_pin is not the observed root:\n{receipt}"
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

    let (_, first) = observe(&root, NOTICE, Observation::Rehearsal);
    std::fs::write(root.0.join("second.md"), "# Second\n").unwrap();
    let (_, second) = observe(&root, NOTICE, Observation::Rehearsal);

    assert_ne!(
        first, second,
        "a new domain member must move the observed root"
    );
}
