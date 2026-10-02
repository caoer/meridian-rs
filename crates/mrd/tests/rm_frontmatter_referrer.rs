//! `mrd rm` end to end over a frontmatter citation (`wire-contract.md` § A.3):
//! a page cited only in a frontmatter value refuses `remove_refused`, the
//! `--json` frame names the citing page with kind `frontmatter` and the key,
//! and with the citation gone the same remove passes.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};
mod common;

struct Sandbox {
    /// Load-bearing: the tree is deleted when this drops.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    ws: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    std::fs::write(
        home.join("MERIDIAN.md"),
        "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n",
    )
    .expect("config");
    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
    }
}

impl Sandbox {
    fn run(&self, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(&self.ws)
            .env("MERIDIAN_CONFIG", self.home.join("MERIDIAN.md"))
            .env("MERIDIAN_DAEMON_BIN", env!("CARGO_BIN_EXE_mrd"))
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let out = self.run(args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v = serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "{args:?} answered no JSON ({e}):\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap_or(-1), v)
    }
}

#[test]
fn rm_refuses_a_page_cited_in_frontmatter_and_names_the_key() {
    let sb = sandbox();
    std::fs::write(sb.ws.join("a.md"), "# A\n").expect("a");
    std::fs::write(sb.ws.join("b.md"), "---\nsource: \"[[a]]\"\n---\n# B\n").expect("b");

    let (code, read) = sb.json(&["read", "a.md", "--json"]);
    assert_eq!(code, 0, "{read}");
    let rev = read["read"]["file_rev"]
        .as_str()
        .unwrap_or_else(|| panic!("the read serves a file_rev: {read}"))
        .to_owned();

    let (code, refused) = sb.json(&["rm", "a.md", "--rev", &rev, "--dry", "--json"]);
    assert_eq!(code, 1, "a refusal exits 1: {refused}");
    assert_eq!(refused["error"]["code"], "remove_refused", "{refused}");
    assert_eq!(
        refused["error"]["referrers"],
        json!([{"path": "b.md", "kind": "frontmatter", "key": "source", "count": 1}])
    );

    std::fs::write(sb.ws.join("b.md"), "---\nsource: plain text\n---\n# B\n").expect("b");
    let (code, passed) = sb.json(&["rm", "a.md", "--rev", &rev, "--dry", "--json"]);
    assert_eq!(
        code, 0,
        "with the citation gone the rehearsal passes: {passed}"
    );
    assert!(sb.ws.join("a.md").exists(), "a dry run touches nothing");
}
