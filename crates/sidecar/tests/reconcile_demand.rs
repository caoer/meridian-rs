//! The DEMAND LAW gates — what a line costs, and what it still observes.
//!
//! The serve loop used to reconcile before AND after every dispatched line, so
//! a `cat` of a 1 KiB file paid two full corpus folds for a ring it never
//! reads. `sidecar::watch::observes_ring` prices the line instead. These gates
//! prove BOTH halves, because only one of them is easy:
//!
//! - **The budget is asserted as a COUNT** (`fs::fold_count`), never a
//!   stopwatch. A timing test proves a machine was fast once; a fold that
//!   creeps back into the read path would still pass it on a small fixture.
//! - **Freshness is unchanged**, arm by arm: every ring reader still reconciles
//!   immediately before reading the ring, a live subscription still receives
//!   its Delta on a `cat` line, and the write path still folds fresh — its
//!   `if_root` guard refuses a stale root that no reconcile ever looked at.
//!
//! The one tense that DOES move is pinned here as data, not left to drift: the
//! epoch baseline is primed at the first ring observation rather than the first
//! line, so a `diff` anchored on a root learned from a ring-blind op before
//! that point answers `root_unknown` → resync (the §7.1 late law's existing
//! category, degrade to re-derive, never to wrong data).

use std::cell::RefCell;
use std::io::{self, BufRead, Read, Write as _};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use serde_json::Value;

/// The s0 corpus root — the same standing oracle the F5-WATCH gates use.
const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

/// The E3 splice, guarded on R0 (`splice_e2e`'s frozen request, verbatim).
const E3_AT_R0: &str = r#"{"id":42,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"ship by September"}}}],"if_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}"#;

/// `fs::fold_count` is process-global, so EVERY test in this file takes this
/// guard — the counting gates measure differences, and a freshness gate folding
/// on another thread lands inside that difference. One process per
/// integration-test file, so the guard covers everything that can race here.
static COUNTER: Mutex<()> = Mutex::new(());

fn counter_guard() -> MutexGuard<'static, ()> {
    COUNTER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for rel in ["notes/plan.md", "receipts/2026-07-18.md"] {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(&abs, wsfix(&format!("s0/{rel}"))).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// The oracle fold: `model::merkle_root` DIRECTLY, never `fs::domain_snapshot`
/// — computing an expectation must not move the counter under test.
fn oracle(files: &[(&str, &str)]) -> String {
    let entries: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_bytes())).collect();
    model::merkle_root(&entries, 0).0
}

/// The out-of-band create used wherever a test needs the world to move WITHOUT
/// touching the splice target: a new domain file, so `notes/plan.md` keeps its
/// bytes (and its node revs) while the corpus root advances.
fn external_new_file(root: &fs::WorkspaceRoot) -> Box<dyn FnMut()> {
    let path = root.0.join("notes/other.md");
    Box::new(move || std::fs::write(&path, OTHER).expect("external create"))
}

const OTHER: &str = "# Other\n\nlanded out of band\n";

fn r_with_other() -> String {
    oracle(&[
        ("notes/other.md", OTHER),
        ("notes/plan.md", &wsfix("s0/notes/plan.md")),
        (
            "receipts/2026-07-18.md",
            &wsfix("s0/receipts/2026-07-18.md"),
        ),
    ])
}

// ---------------------------------------------------------------------------
// The scripted serve harness (F5-WATCH's, extended with a counter probe)
// ---------------------------------------------------------------------------

enum Step {
    Line(String),
    /// Runs at the exact between-lines boundary: an out-of-band mutation, or a
    /// fold-counter sample of the line that just finished.
    Mutate(Box<dyn FnMut()>),
}

struct Scripted {
    steps: std::collections::VecDeque<Step>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for Scripted {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Scripted {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        while self.pos >= self.buf.len() {
            match self.steps.pop_front() {
                None => return Ok(&[]),
                Some(Step::Mutate(mut f)) => f(),
                Some(Step::Line(l)) => {
                    self.buf = format!("{l}\n").into_bytes();
                    self.pos = 0;
                }
            }
        }
        Ok(&self.buf[self.pos..])
    }
    fn consume(&mut self, amt: usize) {
        self.pos += amt;
    }
}

fn line(s: &str) -> Step {
    Step::Line(s.to_string())
}

/// A fold-counter sample, taken between lines — after the previous line is
/// fully served (response written, post-dispatch reconcile done).
fn probe(log: &Rc<RefCell<Vec<u64>>>) -> Step {
    let log = Rc::clone(log);
    Step::Mutate(Box::new(move || log.borrow_mut().push(fs::fold_count())))
}

fn serve_scripted(root: &fs::WorkspaceRoot, steps: Vec<Step>) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let input = Scripted {
        steps: steps.into(),
        buf: Vec::new(),
        pos: 0,
    };
    sidecar::serve(root, input, &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| {
            (
                l.to_string(),
                serde_json::from_str(l).expect("frame parses"),
            )
        })
        .collect()
}

fn is_notification(v: &Value) -> bool {
    !v.as_object().expect("frame object").contains_key("id")
}

/// Consecutive differences of the probe log = the fold cost of each line.
fn costs(log: &Rc<RefCell<Vec<u64>>>) -> Vec<u64> {
    log.borrow().windows(2).map(|w| w[1] - w[0]).collect()
}

// ---------------------------------------------------------------------------
// The budget
// ---------------------------------------------------------------------------

/// **The counter gate.** One probe per line boundary prices every op against
/// the demand law. `cat`/`extract` read O(target) and pay NOTHING; the ops that
/// print a root fold their own; only the ring readers pay a reconcile — and a
/// line that never reaches an arm pays nothing at all, so malformed input is no
/// longer the most expensive frame a client can send.
#[test]
fn the_fold_budget_is_the_demand_law() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let log = Rc::new(RefCell::new(Vec::new()));
    let ops = [
        (
            r#"{"id":1,"op":"hello","proto":1}"#,
            1,
            "hello: its own root",
        ),
        (r#"{"id":2,"op":"cat","path":"notes/plan.md"}"#, 0, "cat"),
        (
            r#"{"id":3,"op":"extract","path":"notes/plan.md"}"#,
            0,
            "extract",
        ),
        (
            r#"{"id":4,"op":"resolve","from":"notes/plan.md","ref":"plan"}"#,
            0,
            "resolve: walk plane, not the fold",
        ),
        (
            r#"{"id":5,"op":"toc","path":"notes/plan.md"}"#,
            1,
            "toc: its own root, no reconcile",
        ),
        (r#"{"id":6,"op":"root"}"#, 2, "root: reconcile + its own"),
        (r#"{"id":7,"op":"root"}"#, 2, "root again: still 2"),
        (
            &format!(r#"{{"id":8,"op":"diff","from_root":"{R0}","to_root":"{R0}"}}"#),
            2,
            "diff: reconcile + its own",
        ),
        (
            r#"{"id":9,"op":"links"}"#,
            3,
            "links: reconcile + as_of + live (§10.1)",
        ),
        (r#"{"id":10,"op":"nope"}"#, 0, "unknown op: no arm, no fold"),
        ("not json at all", 0, "bad frame: no arm, no fold"),
    ];
    let mut steps = vec![probe(&log)];
    for (req, _, _) in &ops {
        steps.push(line(req));
        steps.push(probe(&log));
    }
    let frames = serve_scripted(&root, steps);
    assert_eq!(frames.len(), ops.len(), "one response per line: {frames:?}");
    let measured = costs(&log);
    let expected: Vec<u64> = ops.iter().map(|(_, n, _)| *n).collect();
    let labels: Vec<&str> = ops.iter().map(|(_, _, l)| *l).collect();
    assert_eq!(
        measured, expected,
        "fold budget per line (labels: {labels:?})"
    );
}

/// The card's own case: a `cat` is the WHOLE session. Under the old loop this
/// cost two full corpus folds — at a 63k-file root, seconds for a 1 KiB read.
/// It now costs zero, cold, with nothing primed.
#[test]
fn a_one_shot_cat_pays_no_fold_at_all() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let before = fs::fold_count();
    let frames = serve_scripted(
        &root,
        vec![line(r#"{"id":1,"op":"cat","path":"notes/plan.md"}"#)],
    );
    assert_eq!(fs::fold_count() - before, 0, "a cat folds NOTHING");
    assert_eq!(
        frames[0].1["ok"], true,
        "and still answers: {}",
        frames[0].1
    );
}

// ---------------------------------------------------------------------------
// Freshness — arm by arm
// ---------------------------------------------------------------------------

/// **Arm 1.** An external change lands between two `cat`s — neither of which
/// reconciles — and the following `root` still reports it, with the Delta
/// emitted at its observation point. Demand-driven reconcile loses no change.
#[test]
fn arm1_a_change_between_cats_is_seen_by_the_next_root() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let r1 = r_with_other();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"root"}"#), // primes the baseline at R0
            line(r#"{"id":2,"op":"cat","path":"notes/plan.md"}"#),
            Step::Mutate(external_new_file(&root)),
            line(r#"{"id":3,"op":"cat","path":"notes/plan.md"}"#),
            line(r#"{"id":4,"op":"root"}"#),
        ],
    );
    assert_eq!(frames[0].1["body"], serde_json::json!({"root":R0,"seq":0}));
    assert_eq!(
        frames[3].1["body"],
        serde_json::json!({"root":r1,"seq":1}),
        "the root observes the out-of-band change, and its Delta was emitted"
    );
}

/// **Arm 2.** A live subscription is a STANDING observer: with a `sub` open,
/// the external Delta is emitted and pushed on a `cat` line — the op that pays
/// nothing for itself still serves the subscriber, before the response (§4.7
/// order). Subscribers are not starved by the demand law.
#[test]
fn arm2_a_live_sub_is_served_on_a_cat_line() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"sub","from_seq":0}"#),
            Step::Mutate(external_new_file(&root)),
            line(r#"{"id":2,"op":"cat","path":"notes/plan.md"}"#),
        ],
    );
    assert_eq!(
        frames.len(),
        3,
        "ack, notification, cat response: {frames:?}"
    );
    assert!(
        is_notification(&frames[1].1),
        "the external Delta reached the subscriber on a cat line: {}",
        frames[1].1
    );
    assert_eq!(frames[1].1["delta"]["root_before"], R0);
    assert_eq!(frames[1].1["delta"]["root_after"], r_with_other());
    assert_eq!(frames[2].1["id"], 2, "and the cat still answered");
}

/// **Arm 3, correct-answer leg.** A `diff` whose range contains a change that
/// no line reconciled answers it CORRECTLY: the op reconciles at its own
/// observation point, so the batch is in the ring by the time it is read.
#[test]
fn arm3_diff_over_an_unreconciled_change_answers_correctly() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let r1 = r_with_other();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"root"}"#), // the client learns R0 from a ring op
            line(r#"{"id":2,"op":"cat","path":"notes/plan.md"}"#),
            Step::Mutate(external_new_file(&root)),
            line(r#"{"id":3,"op":"cat","path":"notes/plan.md"}"#),
            line(&format!(
                r#"{{"id":4,"op":"diff","from_root":"{R0}","to_root":"{r1}"}}"#
            )),
        ],
    );
    let batches = frames[3].1["body"]["batches"]
        .as_array()
        .unwrap_or_else(|| panic!("diff answers with batches: {}", frames[3].1));
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0]["delta"]["root_before"], R0);
    assert_eq!(batches[0]["delta"]["root_after"], r1);
    assert_eq!(batches[0]["delta"]["files"][0]["path"], "notes/other.md");
    assert_eq!(batches[0]["delta"]["files"][0]["change"], "created");
}

/// **Arm 3, degrade leg — PINNED AS DATA.** The one tense the demand law moves:
/// the epoch baseline is primed at the first RING observation, so a root the
/// client learned from a ring-blind op (`toc` here) before that point is not an
/// anchorable position. The answer is `root_unknown` → resync — the §7.1 late
/// law's existing category, and the ruled degrade direction. Never wrong data,
/// and never silent: if this ever answers batches instead, the baseline moved
/// and someone must re-rule it.
#[test]
fn arm3_diff_anchored_before_the_baseline_refuses_root_unknown() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let r1 = r_with_other();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"toc","path":"notes/plan.md"}"#), // learns R0, ring-blind
            Step::Mutate(external_new_file(&root)),
            line(&format!(
                r#"{{"id":2,"op":"diff","from_root":"{R0}","to_root":"{r1}"}}"#
            )),
        ],
    );
    assert_eq!(frames[0].1["body"]["root"], R0, "toc handed over R0");
    assert_eq!(frames[1].1["ok"], false, "{}", frames[1].1);
    assert_eq!(frames[1].1["error"]["code"], "root_unknown");
}

/// **The freshness arm.** An out-of-band edit is still observed by the ops that
/// observe world-grain, after a run of ring-blind lines that reconciled
/// nothing: `links` computes its §10.1 triple at its own fresh fold, and its
/// `changes_seq` counts the Delta the same line emitted.
#[test]
fn out_of_band_edits_stay_visible_to_the_world_grain_ops() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let r1 = r_with_other();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"root"}"#),
            line(r#"{"id":2,"op":"cat","path":"notes/plan.md"}"#),
            line(r#"{"id":3,"op":"extract","path":"notes/plan.md"}"#),
            Step::Mutate(external_new_file(&root)),
            line(r#"{"id":4,"op":"cat","path":"notes/plan.md"}"#),
            line(r#"{"id":5,"op":"links"}"#),
        ],
    );
    let body = &frames[4].1["body"];
    assert_eq!(body["as_of_root"], r1, "links folded the CHANGED world");
    assert_eq!(body["live_root"], r1);
    assert_eq!(body["changes_seq"], 1, "the Delta was emitted, not skipped");
    assert!(
        body["files"]
            .as_object()
            .expect("files")
            .contains_key("notes/other.md"),
        "the new file is in the edge map: {body}"
    );
}

// ---------------------------------------------------------------------------
// The line the card must not cross: verdict sites keep folding fresh
// ---------------------------------------------------------------------------

/// **The write-path guard, after a run of ring-blind lines.** A `splice`
/// pinned to R0 lands after an out-of-band change that no `cat` reconciled. It
/// must REFUSE `root_mismatch`: the §5.1 guard folds fresh inside the write
/// path and compares against disk, never against a root the read path happened
/// to be holding. The same splice re-pinned to the live root commits — so the
/// refusal is the guard biting, not the splice being broken.
#[test]
fn splice_root_guard_still_folds_fresh_after_ring_blind_lines() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let stale = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"cat","path":"notes/plan.md"}"#),
            Step::Mutate(external_new_file(&root)),
            line(r#"{"id":2,"op":"cat","path":"notes/plan.md"}"#),
            line(E3_AT_R0),
        ],
    );
    assert_eq!(stale[2].1["ok"], false, "{}", stale[2].1);
    assert_eq!(
        stale[2].1["error"]["code"], "root_mismatch",
        "the stale pin is refused against a FRESH fold: {}",
        stale[2].1
    );

    let (_d2, root2) = s0();
    let live = r_with_other();
    let fresh = serve_scripted(
        &root2,
        vec![
            line(r#"{"id":1,"op":"cat","path":"notes/plan.md"}"#),
            Step::Mutate(external_new_file(&root2)),
            line(&E3_AT_R0.replace(R0, &live)),
        ],
    );
    assert_eq!(
        fresh[1].1["ok"], true,
        "the same splice at the LIVE root commits: {}",
        fresh[1].1
    );
    assert_eq!(fresh[1].1["body"]["root_before"], live);
}

/// **The verdict fold is uncached** — the primitive `mrd check`
/// (`check_cmd.rs`), `realise` (`realise_cmd.rs`) and the splice guards all
/// call. Driven through the exact failure the spec rejected the stat cache for
/// and the verdict reproduced on APFS: a same-length rewrite with mtime
/// restored leaves `(size, mtime)` bit-identical while the bytes differ. The
/// fold reports the change because it compares BYTES, and it runs once per
/// call — this card added a counter to that primitive, never a cache.
#[test]
fn the_verdict_fold_is_uncached_in_the_lie_window() {
    let _g = counter_guard();
    let (_d, root) = s0();
    let plan = root.0.join("notes/plan.md");
    let before_meta = std::fs::metadata(&plan).expect("metadata");
    let mtime = before_meta.modified().expect("mtime");

    let calls = fs::fold_count();
    let (_, first) = fs::domain_snapshot(&root).expect("fold");

    // The lie window: same length, restored mtime, different bytes.
    let original = std::fs::read_to_string(&plan).expect("read");
    let rewritten = original.replacen("ship by August", "ship by Auguzt", 1);
    assert_eq!(rewritten.len(), original.len(), "same length, or no lie");
    assert_ne!(rewritten, original);
    let handle = std::fs::File::options()
        .write(true)
        .open(&plan)
        .expect("open");
    handle.set_len(0).expect("truncate");
    (&handle).write_all(rewritten.as_bytes()).expect("rewrite");
    handle.set_modified(mtime).expect("restore mtime");
    drop(handle);
    let after_meta = std::fs::metadata(&plan).expect("metadata");
    assert_eq!(before_meta.len(), after_meta.len(), "size unchanged");
    assert_eq!(
        mtime,
        after_meta.modified().expect("mtime"),
        "mtime restored — a stat-keyed cache would report UNCHANGED here"
    );

    let (_, second) = fs::domain_snapshot(&root).expect("fold");
    assert_ne!(first.0, second.0, "the fold sees bytes, not metadata");
    assert_eq!(
        fs::fold_count() - calls,
        2,
        "two calls, two real folds — no cache, no short-circuit"
    );
}
