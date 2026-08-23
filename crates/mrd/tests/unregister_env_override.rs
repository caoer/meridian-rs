//! `mrd unregister PATH` under a live `MERIDIAN_WORKSPACE` override.
//!
//! # The divergence
//!
//! `workspace::resolve_with_override` returns at rung 1 — the env override —
//! BEFORE it ever looks at the argument (`crates/workspace/src/lib.rs`: the
//! override branch returns `Answer::EnvOverride` at ~:270; the argument's
//! `canonicalize` is at ~:275). `mrd unregister` takes the LENIENT lane of that
//! same ladder (`resolve::resolve_runtime_lenient`), so the PATH the operator
//! typed is discarded and the env root is unregistered instead.
//!
//! Measured at main `7612b7e58` with the real binary — two registered git
//! roots, `victim` and `target`:
//!
//! ```text
//! $ MERIDIAN_WORKSPACE=…/victim  mrd unregister …/target
//! unregistered workspace …/victim
//!   drawer:  removed
//! $ mrd cache ls        # …/target still listed, …/victim gone
//! ```
//!
//! The operator named one tree and a different one was removed. It is not
//! literally silent — the report line names `victim` — but nothing tells the
//! caller that the argument was overruled, which is the half that turns a typo
//! into data loss.
//!
//! # Why no existing test sees it
//!
//! Not because the harness cannot: 14 files in this tree DO set
//! `MERIDIAN_WORKSPACE` on a child (`run_cli.rs`, `timing_mode.rs`,
//! `pin_cause.rs`, …). The gap is narrower and entirely accidental — every
//! fixture that exercises `unregister` (`e2e.rs`,
//! `outside_workspace_fast_exit.rs`) calls `env_remove("MERIDIAN_WORKSPACE")`,
//! and no fixture that sets the override ever reaches `unregister`. The two
//! halves have simply never met. This file is where they meet.
//!
//! # Status: a live gate — the ruling landed
//!
//! Card `19-20-mrd-statusd-integration/tasks/unregister-env-override-present-path-wrong-removal`
//! asked for the shape to be carded, NOT prescribed: should `unregister` REFUSE
//! when an explicit PATH disagrees with the override, or WARN and name the
//! divergence? (Same divergence family as PR 207's `answered-by` line.) While
//! that stayed open this test was `#[ignore]`d — turning a description of an
//! unruled defect into a gate would have prescribed the answer.
//!
//! **Ruled 2026-08-23**, `19-20-mrd-statusd-integration/decisions/unregister-env-override-vs-explicit-path.md`,
//! **D with C's shape**: an explicit PATH argument outranks the env override,
//! for the WHOLE ladder, fixed in `workspace::resolve_with_override` and never
//! as a per-verb bypass. The `#[ignore]` came off with that fix. Every
//! assertion below is unchanged from the description this file shipped as.
//!
//! Under D the test takes the `argument_won` leg of § "the one assertion".
//! (The retired header claimed the invariant held under EITHER ruling. That
//! was false for option B — warn-and-proceed still removes the victim — and
//! the decision record carries the correction. It is true for the ruling
//! actually taken.)

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    cache_root: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let cache_root = cache_home.join("meridian");
    Sandbox {
        tmp,
        cache_home,
        home,
        cache_root,
    }
}

impl Sandbox {
    /// Run `mrd` with the override UNSET — the rest of the suite's default, used
    /// here for the setup and read-back legs so they cannot be confounded.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Run `mrd` with `MERIDIAN_WORKSPACE` SET. This is the rung the rest of the
    /// suite removes; setting it is the entire point of this file.
    fn run_with_override(&self, cwd: &Path, override_root: &Path, args: &[&str]) -> Output {
        self.base()
            .env("MERIDIAN_WORKSPACE", override_root)
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    fn base(&self) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE")
            .env_remove("MERIDIAN_CONFIG");
        cmd
    }

    /// A registered git root: `.git` makes it a rung-2 root, `mrd init` gives it
    /// a drawer.
    fn registered_root(&self, rel: &str) -> PathBuf {
        let dir = self.tmp.path().join(rel);
        std::fs::create_dir_all(dir.join(".git")).expect("mkdir .git");
        let canonical = std::fs::canonicalize(&dir).expect("canonical");
        let out = self.run(&canonical, &["init"]);
        assert!(
            out.status.success(),
            "init {rel} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        canonical
    }

    /// Whether this workspace still holds a live drawer — the on-disk fact
    /// `unregister` removes.
    fn is_registered(&self, workspace: &Path) -> bool {
        let drawer = cache::drawer_dir(&self.cache_root, workspace);
        matches!(cache::probe(&drawer), cache::Probe::Hit(_))
    }
}

/// `mrd unregister TARGET` under `MERIDIAN_WORKSPACE=VICTIM` must not unregister
/// VICTIM.
///
/// # The one assertion
///
/// The hard assertion is that the tree the operator did NOT name survives. That
/// invariant is ruling-agnostic: it holds whether the ruling makes the explicit
/// argument win, or makes the command refuse the disagreement outright. What
/// the command does to TARGET is exactly the open question, so this test only
/// requires the outcome to be one of the two defensible shapes — never the
/// third, which is what ships today.
#[test]
fn unregister_with_an_explicit_path_must_not_remove_the_env_override_root() {
    let sb = sandbox();
    let victim = sb.registered_root("victim");
    let target = sb.registered_root("target");
    let elsewhere = sb.tmp.path().join("cwd-is-irrelevant");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");

    assert!(sb.is_registered(&victim), "setup: victim is registered");
    assert!(sb.is_registered(&target), "setup: target is registered");

    // The operator names TARGET explicitly, with the override pointing at VICTIM.
    let out = sb.run_with_override(
        &elsewhere,
        &victim,
        &["unregister", &target.to_string_lossy()],
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    // --- the invariant, true under either ruling ---------------------------
    assert!(
        sb.is_registered(&victim),
        "`mrd unregister {}` must NOT unregister {} — the caller named one tree \
         and the env override named another. Today the override wins at rung 1 \
         and the named path is discarded.\nstdout: {stdout}\nstderr: {stderr}",
        target.display(),
        victim.display(),
    );

    // --- the outcome shape: argument-wins OR refusal, never a wrong removal --
    let argument_won = !sb.is_registered(&target);
    let refused = !out.status.success()
        && (stderr.contains("MERIDIAN_WORKSPACE") || stderr.contains("override"));
    assert!(
        argument_won || refused,
        "with the invariant held, the command must either act on the path it was \
         given or refuse and name the divergence — got exit {:?}\nstdout: {stdout}\nstderr: {stderr}",
        out.status.code(),
    );
}
