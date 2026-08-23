//! PATH outside every defined root refuses fast (exit 2) — the help's own
//! triad leg ("2 bad invocation or PATH outside workspace") — instead of
//! adopting the bare cwd as a workspace and walking whatever sits under it
//! (measured 2026-08-20: an unmarked 75-repo parent walked 46k files for
//! ~21 s to report zero rules).
//!
//! Defined roots are: the env override, a `.git` ancestor, an ancestor
//! `MERIDIAN.md` root declaration (`mrd init`), or a daemon-registered root.
//! `mrd init` and `mrd unregister` stay legitimate outside all of them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    /// Load-bearing: the tree is deleted when this drops.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    /// The tempdir's canonical path — the resolver answers canonical
    /// (`workspace::canonicalize`), so a fixture that compares `workspace
    /// <path>` must spell the path the way the engine will: on macOS
    /// `$TMPDIR` is `/var/folders/…` and `/var` → `/private/var` (card
    /// mac-devhost-snapshot-canonicalization).
    root: PathBuf,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

/// An isolated HOME (no machine mount table, no daemon socket) so the only
/// rungs in play are the ones each arm constructs.
fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonical tempdir");
    let home = root.join("home");
    let cache_home = root.join("xdg-cache");
    std::fs::create_dir_all(&home).expect("mkdir home");
    Sandbox {
        tmp,
        root,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn dir(&self, name: &str) -> PathBuf {
        let d = self.root.join(name);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE")
            .env_remove("MERIDIAN_CONFIG")
            .output()
            .expect("spawn mrd")
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The refusal: exit 2, the outside-workspace sentence, and no adopted
/// workspace in the output.
fn assert_refused(out: &Output, verb: &str, cwd: &Path) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "`mrd {verb}` in an unmarked tree must exit 2 — stdout: {} stderr: {}",
        stdout(out),
        stderr(out),
    );
    let err = stderr(out);
    assert!(
        err.contains("outside a declared meridian workspace"),
        "`mrd {verb}` refusal must name the cause — got: {err}",
    );
    assert!(
        !stdout(out).contains(&format!("workspace {}", cwd.display())),
        "`mrd {verb}` must not adopt {} as a workspace",
        cwd.display(),
    );
}

#[test]
fn corpus_verbs_refuse_fast_in_an_unmarked_tree() {
    let sb = sandbox();
    let ws = sb.dir("unmarked");
    // A page that a corpus walk WOULD have counted: rules serving here would
    // prove adoption, and a slow refusal would prove a walk before the refusal.
    std::fs::write(ws.join("note.md"), "# Note\n\nbody\n").expect("page");

    for args in [
        &["rules"][..],
        &["resolve"][..],
        &["read", "note.md"][..],
        &["links"][..],
        &["walk", "note.md"][..],
        &["status"][..],
    ] {
        let started = Instant::now();
        let out = sb.run_in(&ws, args);
        let elapsed = started.elapsed();
        assert_refused(&out, args[0], &ws);
        // Milliseconds, not a corpus walk. The bound is deliberately loose
        // (cold process spawn under a loaded CI box), still two orders of
        // magnitude under the measured 21 s adoption walk.
        assert!(
            elapsed.as_millis() < 2_000,
            "`mrd {}` refusal took {elapsed:?} — it walked something",
            args[0],
        );
    }
}

#[test]
fn a_child_git_repo_does_not_anchor_its_parent() {
    let sb = sandbox();
    // The production shape: a parent of repos. The `.git` sits in a CHILD;
    // the marker walk goes up, never down, so the parent stays unmarked.
    let parent = sb.dir("projects");
    let child = parent.join("some-repo");
    std::fs::create_dir_all(child.join(".git")).expect("child .git");
    std::fs::write(parent.join("note.md"), "# Note\n\nbody\n").expect("page");

    let out = sb.run_in(&parent, &["rules"]);
    assert_refused(&out, "rules", &parent);
}

#[test]
fn a_git_anchored_tree_still_serves() {
    let sb = sandbox();
    let ws = sb.dir("repo");
    std::fs::create_dir_all(ws.join(".git")).expect(".git");
    std::fs::write(ws.join("note.md"), "# Note\n\nbody\n").expect("page");

    let out = sb.run_in(&ws, &["rules"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "rules in a git-anchored tree must serve — stderr: {}",
        stderr(&out),
    );
    let text = stdout(&out);
    assert!(
        text.contains(&format!("workspace  {}", ws.display())),
        "rules must resolve the git root as the workspace — got: {text}",
    );
}

#[test]
fn a_declared_gitless_root_still_serves() {
    let sb = sandbox();
    let ws = sb.dir("declared");
    std::fs::write(
        ws.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: declared\n---\n\n# declared\n",
    )
    .expect("declaration");
    let sub = ws.join("sub");
    std::fs::create_dir_all(&sub).expect("mkdir sub");

    // From the root and from a descendant: the declaration anchors both.
    for cwd in [&ws, &sub] {
        let out = sb.run_in(cwd, &["resolve"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "resolve under a declared root must serve — stderr: {}",
            stderr(&out),
        );
        let text = stdout(&out);
        assert!(
            text.contains(&format!("workspace {}", ws.display()))
                && text.contains("source: declared"),
            "the declared root must be the workspace — got: {text}",
        );
    }
}

/// A registered workspace whose DIRECTORY has been deleted is unregisterable
/// through the door.
///
/// This is the stale-entry class the registry sweep exists to remove, and
/// before this gate it was the one class the door could not touch: the CLI
/// resolved (and so canonicalized) the path before the request, and a vanished
/// directory cannot be canonicalized — exit 2, `cannot canonicalize workspace
/// path … (No such file or directory)`, entry intact. The workaround in use was
/// to recreate the directory just to remove its registration.
///
/// `Registry::unregister` already matched "on the path as given" for exactly
/// this case; only the CLI's resolve-first order kept the request from being
/// made.
#[test]
fn unregister_removes_an_entry_whose_directory_is_gone() {
    let sb = sandbox();
    let ws = sb.dir("vanishing");

    // Register: init writes the declaration and the drawer sentinel.
    let out = sb.run_in(&ws, &["init"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "init must register the tree — stderr: {}",
        stderr(&out),
    );
    let listed = stdout(&sb.run_in(&sb.root, &["cache", "ls"]));
    assert!(
        listed.contains(&ws.display().to_string()),
        "the drawer must be registered before the tree is deleted — got: {listed}",
    );

    // The directory vanishes. Run from a cwd that still exists, naming the
    // gone path explicitly.
    std::fs::remove_dir_all(&ws).expect("delete the workspace");
    assert!(!ws.exists(), "the workspace directory must be gone");

    let out = sb.run_in(&sb.root, &["unregister", &ws.display().to_string()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a vanished workspace must unregister through the door — stdout: {} stderr: {}",
        stdout(&out),
        stderr(&out),
    );
    let text = stdout(&out);
    assert!(
        text.contains("drawer:  removed"),
        "the drawer keyed by that path must be gone — got: {text}",
    );

    // The receipt that matters: the drawer is no longer listed.
    let listed = stdout(&sb.run_in(&sb.root, &["cache", "ls"]));
    assert!(
        !listed.contains(&ws.display().to_string()),
        "the drawer must be gone from `cache ls` — got: {listed}",
    );
}

/// A non-existent path that is keyed by nothing refuses, naming the path.
///
/// The clean no-op is for a tree that is PRESENT and was never registered.
/// With no tree there, exit 0 would report a sweep that removed nothing, and a
/// typo would read back as success.
#[test]
fn unregister_refuses_a_vanished_path_that_matches_nothing() {
    let sb = sandbox();
    let never = sb.root.join("never-existed");
    assert!(!never.exists(), "the fixture path must not exist");

    let out = sb.run_in(&sb.root, &["unregister", &never.display().to_string()]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a vanished path keyed by nothing must refuse — stdout: {} stderr: {}",
        stdout(&out),
        stderr(&out),
    );
    let err = stderr(&out);
    assert!(
        err.contains(&never.display().to_string()),
        "the refusal must name the path — got: {err}",
    );
    assert!(
        err.contains("nothing to unregister"),
        "the refusal must say what did not happen — got: {err}",
    );
}

#[test]
fn init_and_unregister_still_run_outside_a_defined_root() {
    let sb = sandbox();
    let ws = sb.dir("fresh");

    // Unregister of a never-registered tree: the documented clean no-op.
    let out = sb.run_in(&ws, &["unregister"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "unregister outside a defined root must stay a clean no-op — stderr: {}",
        stderr(&out),
    );

    // Init declares the root; the same tree then serves.
    let out = sb.run_in(&ws, &["init"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "init outside a defined root must declare it — stderr: {}",
        stderr(&out),
    );
    let out = sb.run_in(&ws, &["rules"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "rules after init must serve — stderr: {}",
        stderr(&out),
    );
}
