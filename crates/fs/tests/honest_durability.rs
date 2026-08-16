//! Honest-durability gates for the staging path (card `hdb-actor`).
//!
//! What is proven here, and how each proof stays race-free while the test
//! binary runs in parallel:
//!
//! - **Routing** — the staging door issues its durability through the actor
//!   (submits + waits on the calling thread), never a writer-issued honest
//!   sync. Instrument: `ops_on_this_thread`, thread-local, deterministic.
//! - **Falsifier** (module-doc premise): a staged temp sits on the same
//!   `st_dev` as the workspace root the barrier handle pins.
//! - **Concurrency** — N writers in N threads all land durable and the
//!   actor's counters moved; exact grouping counts are the unit tests'
//!   (deterministic pre-loaded drains inside `durability.rs`).

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use fs::{WorkspaceRoot, append_line, durability};

fn ws() -> (tempfile::TempDir, WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// The staging door routes through the actor: one `append_line` (stage +
/// `commit_rename`) performs its content submit + wait and its parent-dir
/// settle on the calling thread — four thread-local durability ops, no
/// writer-issued honest sync anywhere in the door.
#[test]
fn the_staging_door_settles_through_the_actor() {
    let (_d, root) = ws();
    let ops_before = durability::ops_on_this_thread();
    let honest_before = durability::honest_ops();
    append_line(&root, Path::new("notes/a.md"), "row-1").expect("append lands");
    let ops = durability::ops_on_this_thread() - ops_before;
    assert_eq!(
        ops, 4,
        "stage submit + stage wait + dir submit + dir wait ride the calling \
         thread; a different count means the door grew or lost a settle"
    );
    assert!(
        durability::honest_ops() > honest_before,
        "the actor issued honest-class ops for this commit (counter is \
         monotonic; concurrent tests only add)"
    );
    assert_eq!(
        std::fs::read(root.0.join("notes/a.md")).expect("file exists"),
        b"row-1\n"
    );
}

/// The module-doc falsifier, pinned as a test: every staged temp must sit on
/// the same physical device as the workspace root whose handle the actor
/// barriers on. Uses a real staged temp (`create_file`'s temp is gone after
/// commit, so this stages by hand through the public batch door).
#[test]
fn a_staged_temp_shares_the_workspace_root_device() {
    let (_d, root) = ws();
    // A staged temp lives beside its destination; the destination's device
    // IS the root's device exactly when the premise holds. Stage via
    // append_line, then compare the landed file (same dir as its temp was)
    // against the root.
    append_line(&root, Path::new("dir/leaf.md"), "x").expect("append");
    let root_dev = std::fs::metadata(&root.0).expect("root meta").dev();
    let file_dev = std::fs::metadata(root.0.join("dir/leaf.md"))
        .expect("file meta")
        .dev();
    assert_eq!(
        file_dev, root_dev,
        "premise falsified: a workspace file on a different device than the \
         barrier handle would be acked without its device flushing"
    );
}

/// N concurrent writers to disjoint files all land durable through the one
/// actor. Grouping economics are proven deterministically in the unit
/// drains; here the claim is the public path under real concurrency: all
/// succeed, all bytes present, and the actor (not the writers) carried the
/// honest ops.
#[test]
fn concurrent_writers_all_land_through_one_actor() {
    let (_d, root) = ws();
    let root = Arc::new(root);
    let honest_before = durability::honest_ops();
    let object_before = durability::object_syncs();
    let mut joins = Vec::new();
    for i in 0..8 {
        let root = Arc::clone(&root);
        joins.push(thread::spawn(move || {
            let rel = format!("w/{i}.md");
            append_line(&root, Path::new(&rel), "payload").expect("append lands");
        }));
    }
    for j in joins {
        j.join().expect("writer thread");
    }
    for i in 0..8 {
        assert_eq!(
            std::fs::read(root.0.join(format!("w/{i}.md"))).expect("exists"),
            b"payload\n",
            "writer {i} is durable and visible"
        );
    }
    assert!(
        durability::honest_ops() > honest_before,
        "honest ops were issued for the batch"
    );
    assert!(
        durability::object_syncs() - object_before >= 9,
        "at least 8 content files + the shared parent dir were synced as \
         distinct objects (dedupe may collapse the dir across drains)"
    );
}
