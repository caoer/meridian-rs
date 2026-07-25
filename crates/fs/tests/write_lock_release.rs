//! The D9 write flock releases when its guard drops — even while this process
//! has subprocesses in flight.
//!
//! # The bug this pins (measured, stage-2 S7)
//! A `flock` lock belongs to the open file DESCRIPTION, and `fork` duplicates
//! every descriptor. So any thread spawning any subprocess — `git` under the pin
//! path, a bash task, anything — transiently holds a copy of the lock fd between
//! its fork and its exec, `FD_CLOEXEC` notwithstanding (CLOEXEC acts at exec).
//! While `WriteLock` released by closing its fd, dropping the guard inside that
//! window did NOT release the lock: the child's copy kept the description alive,
//! and unrelated writers refused `workspace_busy` for a critical section that had
//! already finished. Measured on the S7 pin suite: 12 of 60 unrelated writes.
//!
//! `WriteLock::drop` now unlocks EXPLICITLY (`LOCK_UN` acts on the description,
//! so one unlock releases every copy). The refusal was never wrong — it is
//! contractually the Retry class — but a door that closes when no writer is
//! there teaches the wrong thing.

use std::process::{Child, Command};

/// Drop-then-reacquire must always succeed, however many children are mid-fork.
///
/// Without the explicit unlock this fails as soon as one spawn lands in the
/// window; with it, the lock is free the instant the guard drops.
#[test]
fn dropping_the_lock_releases_it_while_subprocesses_are_in_flight() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let mut kids: Vec<Child> = Vec::new();

    for round in 0..200 {
        let lock = fs::WriteLock::acquire(&root).unwrap_or_else(|e| {
            panic!("round {round}: the lock must be free before we take it: {e}")
        });
        // Fork a child WHILE the lock is held. It takes no lock itself — it only
        // has to exist across the drop below.
        kids.push(
            Command::new("/bin/sh")
                .args(["-c", "exec sleep 0.05"])
                .spawn()
                .expect("spawn"),
        );
        drop(lock);
        // The critical section is over, so the door must be open — regardless of
        // any child still sitting between its fork and its exec.
        fs::WriteLock::acquire(&root).unwrap_or_else(|e| {
            panic!(
                "round {round}: the lock was NOT released by its guard's drop \
                 ({e}) — a forked child is holding the open file description"
            )
        });
    }

    for mut kid in kids {
        let _ = kid.kill();
        let _ = kid.wait();
    }
}

/// The other half, unchanged by the fix: a lock that is genuinely HELD refuses
/// the second acquire immediately (`WouldBlock`, never a wait) — in-process
/// contention included, since `flock` contends per open file description.
#[test]
fn a_held_lock_still_refuses_a_second_acquire_without_blocking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let held = fs::WriteLock::acquire(&root).expect("first acquire");
    let err = fs::WriteLock::acquire(&root).expect_err("a held lock refuses");
    assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
    drop(held);
    fs::WriteLock::acquire(&root).expect("free again once the holder drops");
}
