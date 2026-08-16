//! The honest-durability gates, proven with the one bare instrument
//! (`fs::honest_sync_count`) rather than by reading the source.
//!
//! Gate 3 — no sync inside the linearized state-owner step.
//! Gate 2 — the content destination really is honest-synced.
//! Dedupe — a transaction issues its enumerated budget and no more.
//!
//! The counter is process-wide, so every test here takes `COUNTING` first:
//! a concurrent writer in the same process would otherwise inflate a delta.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use fs::WorkspaceRoot;
use fs::intent::Policy;
use wire::Root;
use wire_serve::authority::AuthorityEpoch;
use wire_serve::publish::{
    DestImage, Region, RegionKind, ReservationTable, StagedTxn, StateOwner, TxnId, publish,
};

static COUNTING: Mutex<()> = Mutex::new(());

fn counting() -> MutexGuard<'static, ()> {
    COUNTING.lock().unwrap_or_else(PoisonError::into_inner)
}

fn dest(root: &WorkspaceRoot, rel: &str, old: &[u8], new: &[u8]) -> DestImage {
    let dest = root.0.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&dest, old).unwrap();
    let tmp = dest.with_extension("tmp-pub");
    std::fs::write(&tmp, new).unwrap();
    DestImage {
        rel: PathBuf::from(rel),
        old: old.to_vec(),
        new: new.to_vec(),
        tmp,
    }
}

fn staged(dests: Vec<DestImage>) -> StagedTxn {
    let writes = dests
        .iter()
        .map(|d| Region::file("c", d.rel.to_str().unwrap().as_bytes(), RegionKind::Write))
        .collect();
    StagedTxn {
        txn: TxnId::mint().unwrap(),
        epoch: AuthorityEpoch(1),
        reads: Vec::new(),
        writes,
        dests,
        files: Vec::new(),
        actor: None,
        now: None,
        policy: Policy::Restore,
        drop_tmp_after_decide: false,
    }
}

/// Honest syncs one `publish` of `dests` issues.
fn syncs_for(root: &WorkspaceRoot, dests: Vec<DestImage>) -> u64 {
    let table = ReservationTable::new();
    let owner = StateOwner::new(Root("g0".into()));
    let before = fs::honest_sync_count();
    publish(root, &table, &owner, staged(dests), |r| {
        Root(format!("{}+x", r.0))
    })
    .expect("publish");
    fs::honest_sync_count() - before
}

/// Gate 3, proven mechanically: the ONE linearized step assigns a frame with
/// no disk I/O at all. A future edit that puts a sync back inside `apply` —
/// or inside a fold handed to it — moves the counter and fails this.
#[test]
fn the_linearized_state_owner_step_issues_no_honest_sync() {
    let _counting = counting();
    let owner = StateOwner::new(Root("g0".into()));

    let before = fs::honest_sync_count();
    let frame = owner.apply(Vec::new(), None, None, |r| Root(format!("{}+x", r.0)));
    let issued = fs::honest_sync_count() - before;

    assert_eq!(frame.delta.seq, 1, "the step really ran");
    assert_eq!(
        issued, 0,
        "the linearized state-owner step must issue no honest sync; it issued {issued}"
    );
}

/// Gate 2 and the within-transaction dedupe in one instrument.
///
/// The enumerated budget of `publish`, for `D` destinations across `P`
/// distinct parent directories, is `D + P + 4`:
///
/// - `D` — each destination's own content sync, on its own fd (gate 2).
/// - `2` — the `O_EXCL` decided record: its own sync, plus one sync of the
///   intents directory.
/// - `P` — one sync per DISTINCT destination parent, however many renames
///   landed in it.
/// - `2` — finalize's record rewrite and its intents-directory sync.
///
/// Both halves of the card are visible in the two cases below. Two dests in
/// ONE folder cost 7, not 8, which is the parent dedupe; and they cost
/// exactly one more than a single dest, which is the second content sync —
/// every other term is identical between the two cases, so the difference
/// can only be the content destination's own honest sync. Delete that sync
/// and this test fails; re-add a duplicate directory sync and it fails too.
#[test]
fn a_publish_issues_its_enumerated_honest_sync_budget() {
    let _counting = counting();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());

    let one = syncs_for(&root, vec![dest(&root, "notes/a.md", b"OLD-A", b"NEW-A")]);
    assert_eq!(one, 6, "one dest, one parent: 1 + 1 + 4");

    let two = syncs_for(
        &root,
        vec![
            dest(&root, "notes/b.md", b"OLD-B", b"NEW-B"),
            dest(&root, "notes/c.md", b"OLD-C", b"NEW-C"),
        ],
    );
    assert_eq!(two, 7, "two dests sharing one parent: 2 + 1 + 4");
    assert_eq!(
        two - one,
        1,
        "the only term that moved is the second destination's own content sync"
    );

    let split = syncs_for(
        &root,
        vec![
            dest(&root, "notes/d.md", b"OLD-D", b"NEW-D"),
            dest(&root, "receipts/e.md", b"OLD-E", b"NEW-E"),
        ],
    );
    assert_eq!(split, 8, "two dests, two parents: 2 + 2 + 4");
}
