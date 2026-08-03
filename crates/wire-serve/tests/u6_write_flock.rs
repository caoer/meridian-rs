//! U6 (M1 D9): DIRECT two-process + in-process coverage for the cross-process
//! write flock, OUTSIDE the replay harness (A-C2: a single-threaded replay
//! corpus cannot contend for a lock).
//!
//! The law under test: every cooperating meridian writer — sidecar, resident
//! registry daemon, `mrd` — flows through `wire_serve::write::{splice,create,
//! remove}`, and each acquires the exclusive `LOCK_NB` flock on
//! `.meridian/write.lock` across its whole critical section. A held lock is
//! the fast typed `workspace_busy` refusal (retry — transient; the SAME
//! request succeeds once the holder exits), never a wait and never a panic.
//! G2: lock-file I/O failure maps to the typed `io_error{cause}` frame.

use std::time::{Duration, Instant};

use wire::{Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path as WPath, PutAt, Recovery, SecRef};
use wire_serve::write::{CreateArgs, RemoveArgs, SpliceArgs, create, remove, splice};

const PAGE: &str = "---\ntitle: Flock\n---\n# Log\n\nseed line\n";

fn splice_args(text: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::Cli,
        path: WPath("log.md".into()),
        actor: Some("locker".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![HpathSeg {
                    h: "Log".into(),
                    n: None,
                }],
            },
            edit: EditShape::Put {
                at: PutAt::End,
                text: text.to_string(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    }
}

fn busy(e: &wire::ErrorBody) {
    assert_eq!(
        e.code,
        ErrorCode::WorkspaceBusy,
        "typed busy: {:?}",
        e.message
    );
    assert_eq!(e.recovery, Recovery::Retry, "workspace_busy → retry");
}

/// Drive one splice by the DOCUMENTED recovery contract: retry on
/// `workspace_busy` (bounded — 2 s), fail loud on anything else.
///
/// Why a loop and not a single shot: `flock` release can outlive `close()` by
/// a moment when another thread `fork`s while the lock fd is open — the
/// child's cloned fd table keeps the description (and its lock) alive until
/// its `exec` closes the CLOEXEC fds. `LOCK_NB` callers see that as a
/// transient `workspace_busy`; retry IS the contract (recovery class), so the
/// test exercises exactly what a production caller does.
fn splice_retrying(root: &fs::WorkspaceRoot, args: &SpliceArgs) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match splice(root, 0, args, &[], None) {
            Ok(_) => return,
            Err(e) if e.code == ErrorCode::WorkspaceBusy && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("retrying splice must land within the deadline: {e:?}"),
        }
    }
}

/// In-process determinism: flock contends across independent acquires even
/// within one process, so holding the engine lock refuses `splice`, `create`,
/// AND `remove` with the typed `workspace_busy` — and releasing it lets the
/// SAME splice succeed (the retry recovery is real).
#[test]
fn held_lock_refuses_all_write_ops_then_retry_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let held = fs::WriteLock::acquire(&root).expect("test holds the write lock");

    busy(
        &splice(&root, 0, &splice_args("blocked\n"), &[], None)
            .expect_err("splice must refuse busy"),
    );
    busy(
        &create(
            &root,
            0,
            &CreateArgs {
                id: None,
                path: WPath("born.md".into()),
                body: "# Born\n".into(),
                actor: None,
                now: None,
                if_root: None,
                dry: false,
            },
            &[],
        )
        .expect_err("create must refuse busy"),
    );
    busy(
        &remove(
            &root,
            0,
            &RemoveArgs {
                id: None,
                path: WPath("log.md".into()),
                if_file_rev: NodeRev("deadbeefdeadbeef".into()),
                actor: None,
                now: None,
                if_root: None,
                dry: false,
            },
            &[],
        )
        .expect_err("remove must refuse busy"),
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("log.md")).unwrap(),
        PAGE,
        "a busy refusal lands nothing"
    );

    drop(held);
    splice_retrying(&root, &splice_args("landed\n"));
    assert!(
        std::fs::read_to_string(dir.path().join("log.md"))
            .unwrap()
            .contains("landed"),
        "the SAME request lands after the holder releases (retry recovery)"
    );
}

/// A dry run takes the lock too — the rehearsal refuses `workspace_busy`
/// exactly where the real write would (never a silent "would-succeed").
#[test]
fn dry_splice_also_refuses_busy() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let _held = fs::WriteLock::acquire(&root).expect("held");
    let args = SpliceArgs {
        dry: true,
        ..splice_args("dry\n")
    };
    busy(&splice(&root, 0, &args, &[], None).expect_err("dry splice must refuse busy"));
}

/// G2: lock-file I/O failure (here: `.meridian` exists as a regular FILE, so
/// the lock dir cannot be made) maps to the TYPED `io_error{cause}` frame —
/// never a panic, never an unwrap.
#[test]
fn lock_io_failure_is_typed_io_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    std::fs::write(dir.path().join(".meridian"), "not a dir").expect("squat the lock dir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let err = splice(&root, 0, &splice_args("x\n"), &[], None)
        .expect_err("an unmakeable lock dir must refuse typed");
    assert_eq!(
        err.code,
        ErrorCode::IoError,
        "G2: typed io_error, not a panic"
    );
    assert!(
        err.cause
            .as_deref()
            .is_some_and(|c| c.contains("write lock")),
        "the cause names the lock: {:?}",
        err.cause
    );
}

/// The child half of the two-process test: env-gated (a plain test run is a
/// no-op). Under `MERIDIAN_U6_HOLD_WS` it acquires the workspace write lock IN
/// THIS PROCESS, drops a `child.locked` marker, and holds until the parent
/// drops `child.release` (10 s ceiling — the test never hangs).
#[test]
fn hold_write_lock_helper() {
    let Ok(ws) = std::env::var("MERIDIAN_U6_HOLD_WS") else {
        return;
    };
    let root = fs::WorkspaceRoot(ws.clone().into());
    let _lock = fs::WriteLock::acquire(&root).expect("child acquires the write lock");
    std::fs::write(format!("{ws}/child.locked"), "1").expect("marker");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline
        && !std::path::Path::new(&format!("{ws}/child.release")).exists()
    {
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// TWO-PROCESS gate (A-C2): a SECOND OS process holds the flock; this
/// process's `splice` refuses the typed `workspace_busy` (cross-process
/// serialization is real, not thread-local), and after the holder exits the
/// SAME request succeeds.
#[test]
fn cross_process_holder_refuses_busy_then_retry_lands() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let mut child = std::process::Command::new(std::env::current_exe().expect("current_exe"))
        .args(["hold_write_lock_helper", "--exact", "--test-threads=1"])
        .env("MERIDIAN_U6_HOLD_WS", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the holder process");

    // Wait until the child actually holds the lock (marker). The ceiling is
    // generous (30 s): under a cold parallel build the child test binary can
    // start slowly, and a short ceiling turns machine load into a flake.
    let locked_marker = dir.path().join("child.locked");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !locked_marker.exists() {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        locked_marker.exists(),
        "the child process did not take the lock within 30s (spawn stalled?)"
    );

    busy(
        &splice(&root, 0, &splice_args("cross\n"), &[], None)
            .expect_err("a cross-process holder must refuse busy"),
    );

    // Release the child; once it exits, the same request lands (retry class).
    std::fs::write(dir.path().join("child.release"), "1").expect("release marker");
    let status = child.wait().expect("child exits");
    assert!(status.success(), "the holder process exits clean");
    splice_retrying(&root, &splice_args("cross\n"));
    assert!(
        std::fs::read_to_string(dir.path().join("log.md"))
            .unwrap()
            .contains("cross"),
        "the SAME request lands after the cross-process holder exits"
    );
}
