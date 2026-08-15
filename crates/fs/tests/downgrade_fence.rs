//! B-03 crash-phase × filesystem matrix — the in-process cells.
//!
//! The §4.7 amendment-3 obligation (authority contract §2.4, carried as a
//! GATE): the downgrade fence proven per supported filesystem, through every
//! crash phase, against the pinned old binary. This file proves the LOCK-PATH
//! half in-process: [`fs::WriteLock::acquire`] in this tree IS the legacy
//! open §2.4 cites (create/write `OpenOptions` then flock — unchanged by this
//! card), so driving it against every activation rung drives the old binary's
//! exact lock path. The process-boundary half (the pinned `mrd` binary
//! refusing a real `put`) lives in `crates/mrd/tests/downgrade_fence_old_binary.rs`.
//!
//! Filesystem axis: every cell runs on each discovered fixture root — the
//! default tempdir (CI: the pipeline workspace; mac: APFS), `/dev/shm` when
//! writable (tmpfs), plus any colon-separated extra mounts named in
//! `MERIDIAN_FENCE_FS_ROOTS`. Each cell prints the measured filesystem so the
//! CI log carries the matrix, not a claim.
//!
//! NOTHING in this file leaves a fence active anywhere: every fixture is a
//! throwaway tempdir (the post-landing no-active-fence assertion rides the
//! mrd-side test and the fleet scan receipt).

use std::path::Path;

use fs::fence::{self, ActivationPhase, FenceStatus};
use fs::{WorkspaceRoot, WriteLock};

/// The three rungs BEFORE the commit point, in ladder order. A crash losing
/// the `Renamed` rename to a non-durable rename is byte-identical to
/// `Staged`, so the rename-lost crash state is covered by this list.
const PRE_COMMIT: [ActivationPhase; 3] = [
    ActivationPhase::TombstoneBuilt,
    ActivationPhase::MetaDirSynced,
    ActivationPhase::Staged,
];

/// The rungs AT and AFTER the commit point.
const POST_COMMIT: [ActivationPhase; 2] = [ActivationPhase::Renamed, ActivationPhase::ParentSynced];

/// Name the filesystem actually backing `path`, measured — never assumed.
#[cfg(target_os = "linux")]
fn fs_name(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL in fixture path");
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: statfs writes into the zeroed struct; the CString outlives the call.
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return "statfs-failed".into();
    }
    // The magic is a bit pattern, not an arithmetic value.
    #[allow(clippy::cast_sign_loss)]
    let t = s.f_type as u64;
    match t {
        0xEF53 => "ext2/3/4".into(),
        0x0102_1994 => "tmpfs".into(),
        0x794c_7630 => "overlayfs".into(),
        0x5846_5342 => "xfs".into(),
        0x9123_683E => "btrfs".into(),
        other => format!("f_type=0x{other:X}"),
    }
}

#[cfg(target_os = "macos")]
fn fs_name(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL in fixture path");
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: statfs writes into the zeroed struct; the CString outlives the call.
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return "statfs-failed".into();
    }
    let bytes: Vec<u8> = s
        .f_fstypename
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| u8::try_from(b).unwrap_or(b'?'))
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The fixture roots for the filesystem axis: (measured fs name, holder).
/// The `TempDir` handles ride along so fixtures die with the test.
fn fixture_roots() -> Vec<(String, tempfile::TempDir)> {
    let mut roots = Vec::new();
    let default = tempfile::tempdir().expect("default tempdir");
    roots.push((fs_name(default.path()), default));
    if Path::new("/dev/shm").is_dir() {
        if let Ok(shm) = tempfile::tempdir_in("/dev/shm") {
            roots.push((fs_name(shm.path()), shm));
        }
    }
    if let Ok(extra) = std::env::var("MERIDIAN_FENCE_FS_ROOTS") {
        for base in extra.split(':').filter(|s| !s.is_empty()) {
            match tempfile::tempdir_in(base) {
                Ok(dir) => roots.push((fs_name(dir.path()), dir)),
                Err(e) => panic!("MERIDIAN_FENCE_FS_ROOTS names an unusable base {base}: {e}"),
            }
        }
    }
    roots
}

/// A fresh workspace under `base` whose `write.lock` already exists as a
/// regular file — the realistic pre-cutover state (a prior writer created
/// it). `with_lockfile=false` gives the never-written workspace instead.
fn workspace(base: &Path, with_lockfile: bool) -> WorkspaceRoot {
    let ws = tempfile::tempdir_in(base).expect("workspace dir").keep();
    let root = WorkspaceRoot(ws);
    if with_lockfile {
        drop(WriteLock::acquire(&root).expect("seed the legacy lockfile"));
        assert!(root.0.join(".meridian/write.lock").is_file());
    }
    root
}

fn cell(fs: &str, phase: &str, verdict: &str) {
    println!("B03-CELL fs={fs} phase={phase} in-process {verdict}");
}

/// Every rung BEFORE the commit point leaves the fence inactive: the legacy
/// lock path still succeeds (no early activation — the no-rollback error
/// class), and activation re-runs to completion from the crash state.
#[test]
fn pre_commit_rungs_do_not_fence_and_complete() {
    for (fs_kind, base) in fixture_roots() {
        for phase in PRE_COMMIT {
            for with_lockfile in [true, false] {
                let root = workspace(base.path(), with_lockfile);
                fence::activate_until(&root, phase).expect("build the crash state");
                assert_eq!(
                    fence::status(&root).expect("legal state"),
                    FenceStatus::NotInstalled,
                    "rung {phase:?} on {fs_kind} must not fence (lockfile={with_lockfile})"
                );
                drop(
                    WriteLock::acquire(&root)
                        .expect("the legacy open must still succeed before the commit point"),
                );
                // Crash recovery: the cutover machine re-runs activation over
                // its own leftovers and must reach Active.
                let held = fence::activate(&root).expect("re-run completes from the crash state");
                assert_eq!(
                    fence::status(&root).expect("legal state"),
                    FenceStatus::Active
                );
                WriteLock::acquire(&root)
                    .expect_err("the completed fence must refuse the legacy open");
                drop(held);
            }
            cell(
                &fs_kind,
                &format!("{phase:?}"),
                "inactive+recoverable GREEN",
            );
        }
    }
}

/// Every rung AT or AFTER the commit point fences the legacy open with
/// `EISDIR` — before any flock, before any staging.
#[test]
fn post_commit_rungs_fence_the_legacy_open() {
    for (fs_kind, base) in fixture_roots() {
        for phase in POST_COMMIT {
            for with_lockfile in [true, false] {
                let root = workspace(base.path(), with_lockfile);
                fence::activate_until(&root, phase).expect("build the crash state");
                assert_eq!(
                    fence::status(&root).expect("legal state"),
                    FenceStatus::Active,
                    "rung {phase:?} on {fs_kind} must fence (lockfile={with_lockfile})"
                );
                let err = WriteLock::acquire(&root)
                    .expect_err("the legacy open must fail past the commit point");
                assert_eq!(
                    err.raw_os_error(),
                    Some(libc::EISDIR),
                    "the fence fails the OPEN as a directory, not the flock: got {err}"
                );
                // Idempotent completion over an already-active fence.
                drop(fence::activate(&root).expect("activate on Active is Ok"));
                assert_eq!(
                    fence::status(&root).expect("legal state"),
                    FenceStatus::Active
                );
            }
            cell(&fs_kind, &format!("{phase:?}"), "fenced EISDIR GREEN");
        }
    }
}

/// The counterexample cell: a DANGLING fence symlink (rename durable, target
/// directory lost — the state our ladder's dir-sync-before-rename makes
/// unreachable). The legacy `O_CREAT` open creates THROUGH the link and takes
/// the lock — the breach is real, which is exactly why the durability order
/// is load-bearing. `status` must refuse to bless the state in either form.
#[test]
fn dangling_symlink_counterexample_breaches_and_is_loud() {
    for (fs_kind, base) in fixture_roots() {
        let root = workspace(base.path(), true);
        fence::activate_until(&root, ActivationPhase::ParentSynced).expect("activate fully");
        let tomb = root.0.join(".meridian").join(fence::TOMBSTONE_DIR);
        std::fs::remove_file(tomb.join(fence::TOMBSTONE_MARKER)).expect("empty the tombstone");
        std::fs::remove_dir(&tomb).expect("lose the tombstone directory");

        let dangling = fence::status(&root).expect_err("a dangling fence must be loud");
        assert!(fence::is_fence_state_error(&dangling), "typed: {dangling}");

        drop(
            WriteLock::acquire(&root)
                .expect("the breach is real: O_CREAT creates through the dangling link"),
        );
        assert!(
            tomb.is_file(),
            "the legacy open materialized the target as a lockFILE through the link"
        );
        let after = fence::status(&root).expect_err("the materialized breach must stay loud");
        assert!(fence::is_fence_state_error(&after), "typed: {after}");
        cell(
            &fs_kind,
            "dangling-counterexample",
            "breach-demonstrated loud GREEN",
        );
    }
}

/// Foreign or half states never pass for a fence and never pass for absence:
/// `status` errors loud, and `activate` refuses to build over them.
#[test]
fn unrecognized_states_are_loud_never_guessed() {
    for (fs_kind, base) in fixture_roots() {
        // A directory sitting directly at the lock pathname.
        let root = workspace(base.path(), false);
        let dir = root.0.join(".meridian");
        std::fs::create_dir_all(dir.join("write.lock")).expect("plant a directory");
        let e = fence::status(&root).expect_err("directory-at-lock is loud");
        assert!(fence::is_fence_state_error(&e), "typed: {e}");
        fence::activate(&root).expect_err("activate refuses a foreign state");

        // A symlink to a foreign target.
        let root = workspace(base.path(), false);
        let dir = root.0.join(".meridian");
        std::fs::create_dir_all(&dir).expect("meta dir");
        std::os::unix::fs::symlink("somewhere-else", dir.join("write.lock")).expect("plant");
        let e = fence::status(&root).expect_err("foreign symlink is loud");
        assert!(fence::is_fence_state_error(&e), "typed: {e}");

        // A tombstone directory without its marker.
        let root = workspace(base.path(), true);
        fence::activate_until(&root, ActivationPhase::ParentSynced).expect("activate fully");
        std::fs::remove_file(
            root.0
                .join(".meridian")
                .join(fence::TOMBSTONE_DIR)
                .join(fence::TOMBSTONE_MARKER),
        )
        .expect("lose the marker");
        let e = fence::status(&root).expect_err("markerless tombstone is loud");
        assert!(fence::is_fence_state_error(&e), "typed: {e}");

        // A marker with foreign contents.
        let root = workspace(base.path(), true);
        fence::activate_until(&root, ActivationPhase::ParentSynced).expect("activate fully");
        std::fs::write(
            root.0
                .join(".meridian")
                .join(fence::TOMBSTONE_DIR)
                .join(fence::TOMBSTONE_MARKER),
            "not ours\n",
        )
        .expect("corrupt the marker");
        let e = fence::status(&root).expect_err("foreign marker is loud");
        assert!(fence::is_fence_state_error(&e), "typed: {e}");

        cell(&fs_kind, "unrecognized-states", "loud GREEN");
    }
}

/// Staging leftovers are inert (nothing opens the staged name) and a re-run
/// recovers them; a fresh or normally-used workspace reports `NotInstalled` —
/// the no-early-activation face of the post-landing assertion.
#[test]
fn staging_is_inert_and_normal_workspaces_report_not_installed() {
    for (fs_kind, base) in fixture_roots() {
        // Fresh: no .meridian at all.
        let fresh = WorkspaceRoot(tempfile::tempdir_in(base.path()).expect("ws").keep());
        assert_eq!(
            fence::status(&fresh).expect("legal"),
            FenceStatus::NotInstalled
        );

        // Normally used: a lock cycle ran; still NotInstalled.
        let used = workspace(base.path(), true);
        assert_eq!(
            fence::status(&used).expect("legal"),
            FenceStatus::NotInstalled
        );

        // Staged leftover beside a regular lockfile: inert, recoverable.
        let root = workspace(base.path(), true);
        fence::activate_until(&root, ActivationPhase::Staged).expect("stage");
        assert_eq!(
            fence::status(&root).expect("legal"),
            FenceStatus::NotInstalled
        );
        drop(WriteLock::acquire(&root).expect("staging must not fence"));
        drop(fence::activate(&root).expect("recover the stale staging"));
        assert_eq!(fence::status(&root).expect("legal"), FenceStatus::Active);
        cell(&fs_kind, "staging+baseline", "inert GREEN");
    }
}
