//! The in-process CLI plane's unserved-member voice. `fs::build_corpus`
//! degrades per-file (node-rev-merkle-spec §3): a non-UTF-8 member serves no
//! spans/nodes but does not poison the corpus, and it comes back in the third
//! slot as member → condition. The daemon faces mint per-file `invalid_utf8`
//! refusals from that slot; these gates cover the CLI faces that build the
//! corpus in-process — walk, status, sql, check, repair, links — which must
//! TELL the operator, on stderr, which members their scan never saw. A scan
//! over a partial corpus that says nothing reads as a scan of the whole vault.
//!
//! `mrd retire` is deliberately NOT here: for a sweep that certifies absence,
//! an unserved member is a REFUSAL, gated in `u23_retire.rs` Pin 9.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

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
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            // Spawn-impossible: every face answers in-process, which is the
            // plane under test.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE");
        cmd.output().expect("spawn mrd")
    }

    fn workspace(&self, files: &[(&str, &str)]) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        for (path, body) in files {
            let full = ws.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("mkdir parent");
            }
            std::fs::write(&full, body).expect("write");
        }
        let init = self.run(&ws, &["init"]);
        assert!(
            init.status.success(),
            "init: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        ws
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// `repair` reads git; the other faces ignore it. One fixture serves all six.
fn git_init(dir: &Path) {
    for args in [
        &["-c", "init.defaultBranch=main", "init", "-q"][..],
        &["config", "user.name", "unserved-voice fixture"][..],
        &["config", "user.email", "fixture@fixture.invalid"][..],
        &["config", "commit.gpgsign", "false"][..],
    ] {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .expect("run git")
            .success();
        assert!(ok, "fixture git {args:?} failed");
    }
}

const GUIDE: &str = "# Guide\n\n## Usage\n\nA healthy page beside the poison.\n";

/// The six faces under gate, with an invocation each answers in-process.
const FACES: [&[&str]; 6] = [
    &["walk", "guide.md"],
    &["status"],
    &["sql", "SELECT 1"],
    &["check"],
    &["repair", "--dry"],
    &["links"],
];

fn poisoned_workspace(sb: &Sandbox) -> PathBuf {
    let ws = sb.workspace(&[("guide.md", GUIDE)]);
    git_init(&ws);
    let mut bytes = b"# P\n\n## Body\n\nprose the scan never sees\n".to_vec();
    bytes.extend_from_slice(b"\xFF\n");
    std::fs::write(ws.join("poison.md"), bytes).expect("write poison member");
    ws
}

/// Every in-process face names the unserved member AND its condition on
/// stderr — machine stdout stays clean, and the face still serves (per-file
/// degradation, not corpus refusal). *Mutation:* discard the third
/// `build_corpus` slot at any one site, and that face goes silently blind to
/// the member its scan skipped.
#[test]
fn every_in_process_face_voices_the_unserved_member() {
    let sb = sandbox();
    let ws = poisoned_workspace(&sb);

    for args in FACES {
        let out = sb.run(&ws, args);
        let err = stderr(&out);
        assert!(
            err.contains("poison.md") && err.contains("not UTF-8"),
            "`mrd {}` names the unserved member and its condition on stderr \
             (exit {}): stderr was: {err}",
            args.join(" "),
            code(&out)
        );
    }
}

/// VACUITY CONTROL: a fully served corpus voices nothing — the warning fires
/// on the member, never on the plane.
#[test]
fn a_fully_served_corpus_voices_nothing() {
    let sb = sandbox();
    let ws = sb.workspace(&[("guide.md", GUIDE)]);
    git_init(&ws);

    for args in FACES {
        let out = sb.run(&ws, args);
        let err = stderr(&out);
        assert!(
            !err.contains("not UTF-8") && !err.contains("unserved"),
            "`mrd {}` stays silent on a healthy corpus: stderr was: {err}",
            args.join(" ")
        );
    }
}
