//! U6 (D9): write flock — in-process + two-process (A-C2; outside replay).
//!
//! `splice`/`create`/`remove` take exclusive `LOCK_NB` on `.meridian/write.lock`;
//! held → typed `workspace_busy`/retry; G2 lock I/O → `io_error{cause}`.

use std::time::{Duration, Instant};

use wire::{Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path as WPath, PutAt, Recovery, SecRef};
use wire_serve::write::{CreateArgs, RemoveArgs, SpliceArgs, create, remove, splice};

const PAGE: &str = "---\ntitle: Flock\n---\n# Log\n\nseed line\n";

fn splice_args(text: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
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

/// Splice with documented recovery: retry `workspace_busy` (2 s bound).
/// Fork+clone can briefly keep flock after close; retry is the contract.
fn splice_retrying(root: &fs::WorkspaceRoot, args: &SpliceArgs) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match splice(root, None, args, &[], None) {
            Ok(_) => return,
            Err(e) if e.code == ErrorCode::WorkspaceBusy && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("retrying splice must land within the deadline: {e:?}"),
        }
    }
}

/// Held lock refuses splice/create/remove busy; release → same splice lands.
#[test]
fn held_lock_refuses_all_write_ops_then_retry_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let held = fs::WriteLock::acquire(&root).expect("test holds the write lock");

    busy(
        &splice(&root, None, &splice_args("blocked\n"), &[], None)
            .expect_err("splice must refuse busy"),
    );
    busy(
        &create(
            &root,
            None,
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
            None,
            &RemoveArgs {
                id: None,
                path: WPath("log.md".into()),
                if_file_rev: Some(NodeRev("deadbeefdeadbeef".into())),
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

/// Dry splice also takes the lock and refuses busy.
#[test]
fn dry_splice_also_refuses_busy() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let _held = fs::WriteLock::acquire(&root).expect("held");
    let args = SpliceArgs {
        premises: Vec::new(),
        dry: true,
        ..splice_args("dry\n")
    };
    busy(&splice(&root, None, &args, &[], None).expect_err("dry splice must refuse busy"));
}

/// G2: unmakeable lock dir → typed `io_error{cause}` (not panic).
#[test]
fn lock_io_failure_is_typed_io_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");
    std::fs::write(dir.path().join(".meridian"), "not a dir").expect("squat the lock dir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let err = splice(&root, None, &splice_args("x\n"), &[], None)
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

/// Child holder: env `MERIDIAN_U6_HOLD_WS` → acquire, marker, wait for release.
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

/// Two-process (A-C2): remote holder → busy; release → same request lands.
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

    // Wait for child.locked (generous ceiling under cold parallel build).
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
        &splice(&root, None, &splice_args("cross\n"), &[], None)
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
