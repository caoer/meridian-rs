//! r8 D2+D3 end to end over the process boundary: `mrd pin` on each of two
//! same-named siblings mints distinct anchors (`^dup`, `^dup-2`), and
//! `mrd walk` in the same session shows BOTH pins green at their occurrence —
//! a fresh mint never walks grey `ambiguous` (card pin-mint-occurrence-handling).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

const PINNER: &str = "# Plan\n\ndraws from both dups.\n";
const DUP_TARGET: &str = "# Guide\n\n## Dup\n\nfirst dup body.\n\n## Dup\n\nsecond dup body.\n";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

/// Hermetic per-test env: `XDG_CACHE_HOME` and `HOME` pinned to scratch, the
/// ambient workspace override removed (traps-reference §4).
fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(cwd)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A git-initialised workspace (R4 pins need blob oids) holding the pinner
    /// and the duplicate-sibling target, declared a root by `mrd init`.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("plan.md"), PINNER).expect("plan");
        std::fs::write(ws.join("guide.md"), DUP_TARGET).expect("guide");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        git_init(&ws);
        ws
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `git init` with a deterministic identity for fixture plumbing.
fn git_init(dir: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "e2e@example.invalid"],
        vec!["config", "user.name", "e2e"],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
}

/// Pin both siblings by their dewey ordinals (the string coat has no occurrence
/// spelling by design — the ordinal is how a human names one row of the toc),
/// then walk: both pins green, no grey, distinct anchors written to the target.
#[test]
fn a_mint_on_each_duplicate_sibling_walks_green_in_the_same_session() {
    let sb = sandbox();
    let ws = sb.workspace();

    // Dewey: `# Guide` = 1; its children in document order: Dup#1 = 1.1, Dup#2 = 1.2.
    let second = sb.run(&ws, &["pin", "plan.md", "guide.md#1.2"]);
    assert!(
        second.status.success(),
        "r8 D2: the SECOND sibling pins instead of refusing on the taken slug: {}{}",
        stdout(&second),
        stderr(&second)
    );
    assert!(
        stdout(&second).contains("^dup-2 (written into guide.md)"),
        "the de-collided anchor is minted and written: {}",
        stdout(&second)
    );

    let first = sb.run(&ws, &["pin", "plan.md", "guide.md#1.1"]);
    assert!(
        first.status.success(),
        "first sibling pins: {}{}",
        stdout(&first),
        stderr(&first)
    );
    assert!(
        stdout(&first).contains("^dup (written into guide.md)"),
        "the first occurrence keeps the bare title slug: {}",
        stdout(&first)
    );

    let guide = std::fs::read_to_string(ws.join("guide.md")).expect("guide");
    assert_eq!(
        guide, "# Guide\n\n## Dup\n^dup\n\nfirst dup body.\n\n## Dup\n^dup-2\n\nsecond dup body.\n",
        "each sibling carries its own marker"
    );

    // r8 D3: the walk in the SAME session — the receipt that used to read
    // `grey ambiguous` on the node the mint had just receipted.
    let walk = sb.run(&ws, &["walk", "plan.md"]);
    assert!(walk.status.success(), "walk: {}", stderr(&walk));
    let out = stdout(&walk);
    assert!(
        out.contains("green  guide.md §Guide/Dup#1"),
        "the first occurrence pin walks green at its stored occurrence: {out}"
    );
    assert!(
        out.contains("green  guide.md §Guide/Dup#2"),
        "the second occurrence pin walks green at its stored occurrence: {out}"
    );
    assert!(
        !out.contains("grey") && !out.contains("ambiguous"),
        "a fresh mint never walks grey in the session that minted it: {out}"
    );
}
