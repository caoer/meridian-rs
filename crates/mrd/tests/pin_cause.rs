//! `mrd pin` refuses WITH its cause — the R24 gate. The `--vibe`-without-git refusal is the one
//! pin leg whose entire content lives in the wire bodys `cause` (v2 §8: `io_error` carries the
//! underlying cause).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

/// A workspace that is deliberately NOT a git repository, anchored explicitly so the resolution
/// ladder never falls to the ancestor git search — `git hash-object -w` stays the only thing
/// that can fail. `MERIDIAN_WORKSPACE` is the ladders only way to anchor a NON-git tree, and it
/// is set per-subprocess in [`run`] (never on this process), so the suite stays parallel-safe.
fn sandbox() -> (Sandbox, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sb = Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        home: tmp.path().join("home"),
        tmp,
    };
    std::fs::create_dir_all(&sb.home).expect("home");
    let ws = sb.tmp.path().join("project");
    std::fs::create_dir_all(&ws).expect("mkdir");
    std::fs::write(ws.join("claim.md"), "# Claim\n\nA claim.\n").expect("claim");
    std::fs::write(ws.join("source.md"), "# Source\n\n## Note\n\ntext here\n").expect("source");
    (sb, ws)
}

fn run(sb: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .current_dir(cwd)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        // Spawn-impossible: no resident daemon ever starts.
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        // The explicit anchor: this tree is the workspace, and it is not git.
        .env("MERIDIAN_WORKSPACE", cwd)
        .output()
        .expect("spawn mrd")
}

/// Gate — `mrd pin --vibe` outside a git repository refuses exit 1 and NAMES the cause: which
/// flag asked, what it asked for, and what git said. The bare class alone is the failure this
/// pins.
#[test]
fn vibe_without_git_refuses_with_its_cause_not_a_bare_class() {
    let (sb, ws) = sandbox();
    let out = run(
        &sb,
        &ws,
        &["pin", "claim.md", "source.md#Source/Note", "--vibe"],
    );
    assert_eq!(out.status.code(), Some(1), "refused, not a tool failure");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();

    assert_ne!(
        err.trim(),
        "mrd: io_error",
        "the bare class with no reason is the defect"
    );
    for named in ["io_error", "--vibe", "source.md", "git"] {
        assert!(err.contains(named), "refusal names {named}: {err}");
    }
}
