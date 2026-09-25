//! `mrd walk` answers from the resident daemon (§ A.10) when one answers and
//! from the in-process build when none does, and the two lanes print ONE
//! listing: stdout byte-identical, exit code identical, on both faces. The
//! daemon lane is proven to have served (no degrade voice on stderr) and the
//! in-process lane to have degraded (the degrade voice present), so neither
//! comparison can pass by both runs quietly taking the same lane.

use std::path::{Path, PathBuf};
use std::process::Output;

mod common;

/// The live fingerprint token of a page's document root — what a green pin
/// holds.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

/// One whole-body pin on `object` at `token`, in the bytes `lock::render`
/// emits.
fn lock_block(object: &str, token: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"9ae3f1deadbeef\"\n    path: []\n    fingerprint: \"{token}\"\n```"
    )
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
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
    fn run(&self, cwd: &Path, args: &[&str], no_daemon: bool) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(args)
            .current_dir(cwd)
            .env_remove("MERIDIAN_WORKSPACE");
        if no_daemon {
            // A binary that cannot exist: the spawn fails at once and the
            // verb degrades to the in-process lane.
            cmd.env(
                "MERIDIAN_DAEMON_BIN",
                self.tmp.path().join("no-such-daemon"),
            );
        }
        cmd.output().expect("spawn mrd")
    }

    /// A workspace declared by `mrd init`: `a.md` draws from `b.md` (green),
    /// `b.md` claims a page in a root no machine binds (grey, unmounted — the
    /// exit-1 leg), and `c.md` pins `a.md` so a down walk has a dependent.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        let b = format!(
            "# B\n\nleaf that claims across roots\n\n{}\n",
            lock_block("ghost:z", "fp1.span2.b3.00000000")
        );
        let a = format!(
            "# A\n\ndraws from b\n\n{}\n",
            lock_block("b", &live_fingerprint(&b))
        );
        let c = format!(
            "# C\n\ndraws from a\n\n{}\n",
            lock_block("a", &live_fingerprint(&a))
        );
        std::fs::write(ws.join("a.md"), &a).expect("a");
        std::fs::write(ws.join("b.md"), &b).expect("b");
        std::fs::write(ws.join("c.md"), &c).expect("c");
        let init = self.run(&ws, &["init"], true);
        assert!(init.status.success(), "init: {}", text(&init.stderr));
        ws
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const DEGRADE_VOICE: &str = "mrd: source: ephemeral";

#[test]
fn the_daemon_lane_and_the_in_process_lane_print_one_listing() {
    let sb = sandbox();
    let ws = sb.workspace();

    let invocations: [&[&str]; 4] = [
        &["walk", "a.md"],
        &["walk", "a.md", "--json"],
        &["walk", "a.md", "--down", "--depth", "1"],
        &["walk", "b.md", "--json"],
    ];
    for args in invocations {
        let warm = sb.run(&ws, args, false);
        let warm_err = text(&warm.stderr);
        assert!(
            !warm_err.contains(DEGRADE_VOICE),
            "{args:?}: the daemon lane must serve this run (no degrade voice), stderr: {warm_err}"
        );

        let cold = sb.run(&ws, args, true);
        // The daemon the warm run spawned still answers on this socket; the
        // spawn-impossible knob alone does not make the verb degrade. Reap it
        // between the two runs so the cold run finds no daemon at all.
        let _ = common::reap_daemon(&sb.home, &sb.cache_home);
        let cold = if text(&cold.stderr).contains(DEGRADE_VOICE) {
            cold
        } else {
            sb.run(&ws, args, true)
        };
        let cold_err = text(&cold.stderr);
        assert!(
            cold_err.contains(DEGRADE_VOICE),
            "{args:?}: the in-process lane must degrade on this run, stderr: {cold_err}"
        );

        assert_eq!(
            text(&warm.stdout),
            text(&cold.stdout),
            "{args:?}: one listing across lanes"
        );
        assert_eq!(
            warm.status.code(),
            cold.status.code(),
            "{args:?}: one exit code across lanes"
        );
    }

    // The walks above are not all-green by design: `b.md`'s grey(unmounted)
    // claim is the exit-1 leg, and it must have reached both lanes alike.
    let up = sb.run(&ws, &["walk", "a.md"], false);
    assert_eq!(up.status.code(), Some(1), "{}", text(&up.stderr));
    assert!(
        text(&up.stdout).contains("grey unmounted"),
        "{}",
        text(&up.stdout)
    );
}
