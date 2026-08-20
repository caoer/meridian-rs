//! B-03 crash-phase × filesystem matrix — the pinned-old-binary process cells.
//!
//! The fence landed dormant (ZT 2026-08-15: not a cutover blocker; no
//! old-binary users; leftover bin = delete it; never activate on downgrade
//! grounds). These cells still prove the mechanism against the current `mrd`
//! (`CARGO_BIN_EXE_mrd`, sha256 printed per run) over its process boundary.
//! CLI writes route over IPC; the fence still holds because the daemon
//! publish path opens `.meridian/write.lock` (interim flock until
//! parallel-commits). When a test activates the fence past the commit point
//! the write cannot take `write.lock` and cannot mint; before that point it
//! still commits. Production doors activate nothing.
//!
//! The in-process half of the matrix lives in
//! `crates/fs/tests/downgrade_fence.rs`.
//!
//! Post-landing assertion: a workspace that took a REAL commit through this
//! binary reports `NotInstalled` — nothing on any door path activates a fence.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::fence::{self, ActivationPhase, FenceStatus};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// Print the pinned binary's identity once per process: path + sha256 (via
/// the host `sha256sum`/`shasum`, receipt-only — absence is recorded, never
/// a failure).
fn print_pin() {
    let bin = mrd_bin();
    let sha = Command::new("sha256sum")
        .arg(bin)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .or_else(|| {
            Command::new("shasum")
                .args(["-a", "256", bin])
                .output()
                .ok()
                .filter(|o| o.status.success())
        })
        .map_or_else(
            || "sha256 unavailable on this host".into(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        );
    println!("B03-PIN old-binary {sha}");
}

/// Name the filesystem backing `path`, measured (same instrument as the
/// fs-side matrix; duplicated because test crates cannot share helpers).
#[cfg(target_os = "linux")]
fn fs_name(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL in fixture path");
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: statfs writes into the zeroed struct; the CString outlives the call.
    if unsafe { libc::statfs(c.as_ptr(), &raw mut s) } != 0 {
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
    if unsafe { libc::statfs(c.as_ptr(), &raw mut s) } != 0 {
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

/// Fixture roots for the filesystem axis: default tempdir, `/dev/shm` when
/// writable, plus `MERIDIAN_FENCE_FS_ROOTS` (colon-separated extra mounts).
fn fixture_roots() -> Vec<(String, tempfile::TempDir)> {
    let mut roots = Vec::new();
    let default = tempfile::tempdir().expect("default tempdir");
    roots.push((fs_name(default.path()), default));
    if Path::new("/dev/shm").is_dir()
        && let Ok(shm) = tempfile::tempdir_in("/dev/shm")
    {
        roots.push((fs_name(shm.path()), shm));
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

const DOC: &str = "# Alpha\n\none two three\n";
const EDIT_FIRST: &str = r#"[{"target":{"hpath":[{"h":"Alpha"}]},"edit":{"match":{"old":"one two three","new":"first commit"}}}]"#;
const EDIT_FENCED: &str = r#"[{"target":{"hpath":[{"h":"Alpha"}]},"edit":{"match":{"old":"one two three","new":"MUST NEVER LAND"}}}]"#;

struct Sandbox {
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.cache_home);
    }
}

impl Sandbox {
    fn new(base: &Path) -> Self {
        let cache_home = base.join("xdg-cache");
        let home = base.join("home");
        std::fs::create_dir_all(&home).expect("home");
        Self { cache_home, home }
    }

    /// A marked workspace holding `doc.md`. Writes go through the live daemon
    /// (CLI is IPC); the fence still trips on the daemon's lock acquire.
    fn workspace(&self, base: &Path, name: &str) -> PathBuf {
        let ws = base.join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        let init = self.run(&ws, &["init"], None);
        assert!(
            init.status.success(),
            "init: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        ws
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin_bytes: Option<&str>) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE");
        let Some(body) = stdin_bytes else {
            return cmd.output().expect("spawn mrd");
        };
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, body.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }
}

/// Every visible byte of the workspace tree, for the no-root-minted proof:
/// path → contents. `.meridian` is excluded — the fence itself lives there.
fn visible_bytes(ws: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![ws.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".meridian") {
                continue;
            }
            if entry.file_type().expect("ft").is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("read");
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

/// The matrix, process-boundary cells: per filesystem, the real binary
/// commits normally on a pre-commit rung (Staged — no early fence), and past
/// the commit point (`Renamed`, `ParentSynced`) it cannot take `write.lock` nor
/// mint an old-law root.
#[test]
fn old_binary_matrix_across_fence_rungs() {
    print_pin();
    for (fs_kind, base) in fixture_roots() {
        let sb = Sandbox::new(base.path());

        // Pre-commit rung: staging is inert — the old binary still commits.
        let ws = sb.workspace(base.path(), "pre-commit");
        let root = fs::WorkspaceRoot(std::fs::canonicalize(&ws).expect("canonical"));
        fence::activate_until(&root, ActivationPhase::Staged).expect("stage");
        let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(EDIT_FIRST));
        assert!(
            out.status.success(),
            "pre-commit rung must not fence on {fs_kind}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let committed = std::fs::read_to_string(ws.join("doc.md")).expect("doc");
        assert!(
            committed.contains("first commit"),
            "the baseline commit landed"
        );
        println!("B03-CELL fs={fs_kind} phase=Staged old-binary commits-normally GREEN");

        // Post-commit rungs: the fence holds against the real binary.
        for phase in [ActivationPhase::Renamed, ActivationPhase::ParentSynced] {
            // Workspace basenames must stay inside init's [a-z0-9-] charset.
            let name = format!("post-{phase:?}").to_lowercase();
            let ws = sb.workspace(base.path(), &name);
            let root = fs::WorkspaceRoot(std::fs::canonicalize(&ws).expect("canonical"));
            fence::activate_until(&root, phase).expect("build the crash state");
            assert_eq!(fence::status(&root).expect("legal"), FenceStatus::Active);

            let before = visible_bytes(&ws);
            let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(EDIT_FENCED));
            assert!(!out.status.success(), "the fenced put must refuse");
            assert_eq!(
                out.status.code(),
                Some(1),
                "a typed engine refusal, not a crash: stderr={}",
                String::from_utf8_lossy(&out.stderr)
            );
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.contains("write lock"),
                "the refusal names the lock: {stderr}"
            );

            // No old-law root was minted: not one visible byte moved, no
            // receipt appeared, and the doc still holds the pre-fence text.
            let after = visible_bytes(&ws);
            assert_eq!(before, after, "the fenced workspace must not change");
            let doc = std::fs::read_to_string(ws.join("doc.md")).expect("doc");
            assert!(!doc.contains("MUST NEVER LAND"), "no fenced edit may land");
            assert_eq!(fence::status(&root).expect("legal"), FenceStatus::Active);
            println!("B03-CELL fs={fs_kind} phase={phase:?} old-binary fenced GREEN");
        }
    }
}

/// The post-landing no-active-fence assertion: a workspace that took a real
/// commit through the real binary reports `NotInstalled` — no door path
/// activates a fence. (The fleet half of this gate is the read-only scan
/// receipt on the card.)
#[test]
fn no_door_path_activates_a_fence() {
    let base = tempfile::tempdir().expect("tempdir");
    let sb = Sandbox::new(base.path());
    let ws = sb.workspace(base.path(), "clean");
    let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(EDIT_FIRST));
    assert!(
        out.status.success(),
        "put: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let root = fs::WorkspaceRoot(std::fs::canonicalize(&ws).expect("canonical"));
    assert_eq!(
        fence::status(&root).expect("legal state after a real commit"),
        FenceStatus::NotInstalled,
        "a normal commit must leave no fence behind"
    );
}
