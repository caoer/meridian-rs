//! The `PATH` a spawned `git` — and the hook it runs — sees in the hook-fence
//! tests.
//!
//! These tests need a PATH with two properties: `git` is on it, and `mrd` is
//! NOT. The second is the whole point of the fence legs — a deployed `mrd`
//! found by accident makes a leg measure the wrong failure.
//!
//! It used to be the literal `/usr/bin:/bin:/usr/sbin:/sbin`, which encodes
//! "macOS" rather than "the system". On NixOS every one of those directories is
//! empty of `git` — it lives in the store — so the spawn failed with
//! `NotFound`, four directories deep in a `.expect("spawn git commit")`. The
//! panic names the test, not the platform, so it reads as a broken test rather
//! than a hardcoded assumption.
//!
//! Derive it instead: take the ambient `PATH` and drop any directory holding an
//! `mrd`. That keeps the no-`mrd` property by construction on every platform,
//! and keeps the rest of the machine's tools — a hook is a shell script and
//! reaches for more than `git`.

use std::path::{Path, PathBuf};

/// Directories from the ambient `PATH` that hold no `mrd`.
fn dirs() -> Vec<PathBuf> {
    let ambient = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&ambient)
        .filter(|d| !d.as_os_str().is_empty())
        .filter(|d| !d.join("mrd").exists())
        .collect()
}

/// The system `PATH` for a spawned `git`: has `git`, has no `mrd`.
///
/// Panics rather than returns an empty string when `git` is unreachable — a
/// PATH without `git` produces the same `NotFound` this module exists to
/// explain, and the panic should say so at the source.
pub(crate) fn system_path() -> String {
    let dirs = dirs();
    assert!(
        dirs.iter().any(|d| is_executable(&d.join("git"))),
        "no `git` on the ambient PATH once the directories holding an `mrd` are \
         dropped — the fence tests spawn a real `git`, and without one they \
         would report a spawn failure instead of a fence verdict"
    );
    std::env::join_paths(dirs)
        .expect("PATH entries came from PATH, so they rejoin")
        .into_string()
        .expect("PATH is valid unicode on the platforms these tests run on")
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
