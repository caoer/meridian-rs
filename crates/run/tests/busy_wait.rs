//! The CLI lane's waiting policy on `.meridian/write.lock` (card
//! `mrd-cli-lane-workspace-busy`, run-plane § Executor laws amendment
//! 2026-08-20).
//!
//! The engine still refuses fast — `LOCK_EX|LOCK_NB`, no queue, no engine
//! retry — because the wire contract puts waiting on the CALLER. The run plane
//! is such a caller: its birth lane retries a `workspace_busy` door refusal
//! until `MERIDIAN_BUSY_WAIT_MS` is spent. What this file pins is that BOTH
//! halves are real — the wait absorbs a transient holder, and the bound still
//! surfaces the typed refusal so a hung holder cannot make a caller hang
//! (review C4).
//!
//! ONE test function, deliberately: `MERIDIAN_BUSY_WAIT_MS` is process-global,
//! so two tests in one binary setting it would race. The legs run in order in
//! one thread instead.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError};

const PAGE: &str = "---\nstatus: todo\n---\n\n# Tasks\n";

/// Empty run-birth fields (the CLI-entry shape).
static EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

/// Under `target/`, not `$TMPDIR`: macOS's `/var`→`/private/var` symlink makes
/// the door's cache canonicalization disagree with the fixture root (the same
/// reason `birth_cap.rs` does it).
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn current_root(root: &fs::WorkspaceRoot) -> MerkleRoot {
    fs::domain_snapshot(root).unwrap().1
}

fn create_effect(path: &str, body: &str) -> Effect {
    Effect {
        kind: EffectKind::Create,
        rule_id: "t".to_owned(),
        seq: 0,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: [
            ("path".to_owned(), ArgValue::Str(path.to_owned())),
            ("body".to_owned(), ArgValue::Str(body.to_owned())),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    }
}

fn request<'a>(
    effects: &'a [Effect],
    authority: &'a Authority,
    observed: &'a MerkleRoot,
) -> ApplyRequest<'a> {
    ApplyRequest {
        page: "page.md",
        task: "t",
        task_rev: "cafecafecafecafe",
        invocation_id: "inv-1",
        now: None,
        effects,
        authority,
        observed_root: observed,
        receipt: None,
        exec: None,
        actor: None,
        depth: 0,
        delta: None,
        fields: &EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

#[test]
fn the_birth_lane_waits_out_a_transient_holder_and_still_refuses_past_the_bound() {
    // ── Leg 1: a transient holder is waited out, not refused. ───────────────
    //
    // A competing writer holds `write.lock` for 600 ms — the shape of a real
    // daemon commit (measured at 1.1–1.5 s on a 37 878-file corpus). The birth
    // must land anyway, and must have waited: an instant success would mean the
    // holder never held.
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/waited.md", "# Waited\n")];
    let authority = Authority::granted(CapSet::parse("md.create").unwrap());

    let hold = Duration::from_millis(600);
    let (locked_tx, locked_rx) = mpsc::channel();
    let holder_root = root.clone();
    let holder = std::thread::spawn(move || {
        let lock = fs::WriteLock::acquire(&holder_root).expect("the holder takes the flock");
        locked_tx.send(()).expect("announce the hold");
        std::thread::sleep(hold);
        drop(lock);
    });
    locked_rx
        .recv()
        .expect("the holder holds before the birth starts");

    let observed = current_root(&root);
    let start = Instant::now();
    let applied = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect("the birth waits the holder out instead of refusing");
    let waited = start.elapsed();
    holder.join().expect("holder thread");

    assert_eq!(applied.applied, 1, "the birth landed");
    assert!(
        tmp.path().join("tasks/waited.md").exists(),
        "the born file is on disk"
    );
    assert!(
        waited >= hold,
        "the birth must have WAITED for the holder, not raced it: {waited:?} < {hold:?}"
    );

    // ── Leg 2: the bound still surfaces the typed refusal. ──────────────────
    //
    // `MERIDIAN_BUSY_WAIT_MS=0` is the pure `LOCK_NB` behavior. A holder that
    // never releases must not hang the caller (review C4), and the refusal is
    // the door's own frame carried whole.
    let (tmp2, root2) = workspace();
    let effects2 = [create_effect("tasks/refused.md", "# Refused\n")];
    let _stuck = fs::WriteLock::acquire(&root2).expect("the stuck holder takes the flock");
    let observed2 = current_root(&root2);

    // SAFETY: this test binary is single-threaded at this point — leg 1's
    // holder thread is joined, and this file declares exactly one test.
    unsafe { std::env::set_var("MERIDIAN_BUSY_WAIT_MS", "0") };
    let start = Instant::now();
    let err = executor::apply(&root2, &request(&effects2, &authority, &observed2))
        .expect_err("a held flock past the bound refuses");
    let elapsed = start.elapsed();
    // SAFETY: same single-threaded window.
    unsafe { std::env::remove_var("MERIDIAN_BUSY_WAIT_MS") };

    let ExecError::BirthRefused { path, detail, .. } = err else {
        panic!("expected BirthRefused, got {err:?}");
    };
    assert_eq!(path, "tasks/refused.md");
    assert!(
        detail.contains("workspace_busy"),
        "the door's own code rides the detail: {detail}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "a zero budget must refuse immediately, not block: {elapsed:?}"
    );
    assert!(
        !tmp2.path().join("tasks/refused.md").exists(),
        "a refused birth writes nothing"
    );
}
