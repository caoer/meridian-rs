//! Parallel-commits process gate: `kill -9` after the first rename of a
//! multi-file transaction. Recovery finishes or restores the COMPLETE
//! declared set; no receipt describes a partial set (merged-plan §7(e),
//! contract gate 12 / card quality gate 2).
//!
//! The child is this test binary re-exec'd (`publish_child_entry`), so it
//! runs the real decide + first-rename path — fork+exec, no inherited
//! temps-as-fds.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use fs::intent::Policy;
use fs::{WorkspaceRoot, honest_sync_path};
use wire::Root;
use wire_serve::authority::AuthorityEpoch;
use wire_serve::publish::{
    DestImage, RecoveryOutcome, Region, RegionKind, StagedTxn, StateOwner, TxnId, recover,
};

const CHILD_ROOT_ENV: &str = "MERIDIAN_PUBLISH_CHILD_ROOT";
const CHILD_UP_MARKER: &str = "child-up";

/// The child entry. Without the env it is a no-op that stays green.
#[test]
fn publish_child_entry() {
    let Some(root) = std::env::var_os(CHILD_ROOT_ENV) else {
        return;
    };
    let root = WorkspaceRoot(PathBuf::from(root));
    child_first_rename_then_park(&root);
}

fn child_first_rename_then_park(root: &WorkspaceRoot) -> ! {
    let content = dest(root, "notes/plan.md", b"OLD-C", b"NEW-C");
    let receipt = dest(root, "receipts/log.md", b"OLD-R", b"NEW-R");
    let staged = StagedTxn {
        txn: TxnId::mint().expect("txn"),
        epoch: AuthorityEpoch(1),
        reads: Vec::new(),
        writes: vec![
            Region::file("c", b"notes/plan.md", RegionKind::Write),
            Region::file("c", b"receipts/log.md", RegionKind::Write),
        ],
        dests: vec![content, receipt],
        files: Vec::new(),
        actor: None,
        now: None,
        policy: Policy::Restore,
        drop_tmp_after_decide: false,
    };
    wire_serve::publish::decide(root, &staged).expect("durable decision");
    std::fs::rename(&staged.dests[0].tmp, root.0.join(&staged.dests[0].rel)).expect("first rename");
    std::fs::write(root.0.join(CHILD_UP_MARKER), staged.txn.hex()).expect("up marker");
    park()
}

fn dest(root: &WorkspaceRoot, rel: &str, old: &[u8], new: &[u8]) -> DestImage {
    let dest = root.0.join(rel);
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(&dest, old).unwrap();
    let tmp = dest.with_extension("tmp-pub");
    std::fs::write(&tmp, new).unwrap();
    honest_sync_path(&tmp).unwrap();
    DestImage {
        rel: PathBuf::from(rel),
        old: old.to_vec(),
        new: new.to_vec(),
        tmp,
    }
}

fn park() -> ! {
    loop {
        std::thread::sleep(Duration::from_hours(1));
    }
}

struct HolderChild(Child);

impl HolderChild {
    fn spawn(root: &WorkspaceRoot) -> Self {
        let exe = std::env::current_exe().expect("test binary path");
        let child = Command::new(exe)
            .args(["publish_child_entry", "--exact", "--nocapture"])
            .env(CHILD_ROOT_ENV, &root.0)
            .spawn()
            .expect("spawn the publish child");
        HolderChild(child)
    }

    fn kill9_and_reap(mut self) {
        self.0.kill().expect("SIGKILL the child");
        self.0.wait().expect("reap the child");
    }
}

impl Drop for HolderChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(path: &Path, budget: Duration) {
    let deadline = Instant::now() + budget;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the child never landed {} — its spawn or decide failed",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Card quality gate 2: kill -9 after the first rename of a multi-file
/// transaction. Recovery restores the complete declared set.
#[test]
fn gate_kill9_after_first_rename_restores_the_complete_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());

    let child = HolderChild::spawn(&root);
    wait_for(&root.0.join(CHILD_UP_MARKER), Duration::from_secs(15));
    child.kill9_and_reap();

    // Mid-crash disk: content new, receipt old — the stated window.
    assert_eq!(
        std::fs::read(root.0.join("notes/plan.md")).unwrap(),
        b"NEW-C"
    );
    assert_eq!(
        std::fs::read(root.0.join("receipts/log.md")).unwrap(),
        b"OLD-R"
    );

    let _owner = StateOwner::new(Root("g0".into()));
    let report = recover(&root).expect("recover");
    assert_eq!(
        report.len(),
        1,
        "one intent, recovered independently: {report:?}"
    );
    assert_eq!(report[0].outcome, RecoveryOutcome::Restored);

    assert_eq!(
        std::fs::read(root.0.join("notes/plan.md")).unwrap(),
        b"OLD-C",
        "content restored — the complete set, not a partial landing"
    );
    assert_eq!(
        std::fs::read(root.0.join("receipts/log.md")).unwrap(),
        b"OLD-R",
        "receipt never described a partial set"
    );
}
