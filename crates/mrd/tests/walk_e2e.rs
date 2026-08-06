//! End-to-end gates for `mrd walk` (U2.3), driving the REAL binary (`CARGO_BIN_EXE_mrd`) over
//! its process boundary against a three-doc chain fixture on disk.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The live document-root rev of `raw` — the rev the binary computes for the
/// file on disk (raw bytes → the same parse the corpus builder runs).
fn doc_rev(raw: &str) -> String {
    model::build(raw.to_string(), syntax::parse(raw))
        .root
        .node_rev
        .0
}

/// The three-doc chain `a.md -> b.md -> c.md`, all pins GREEN. Returns the three
/// file bodies (a, b, c) and their live revs.
struct Chain {
    a: String,
    b: String,
    c: String,
    /// The containing pages' doc revs — what `revs-read:` cites.
    a_rev: String,
    b_rev: String,
    c_rev: String,
    /// The PINNED tokens — what each entrys `rev=` renders. Not the citations
    /// above: the citation says which bytes were read, the token what was claimed.
    b_pin: String,
    c_pin: String,
}

/// The live fingerprint token of a pages document root — what a CORRECT R4 pin holds. Minted
/// through the engines own mint over the same parse the corpus builder runs.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

/// One whole-body R4 pin on `object` at `token`, hand-written in the exact bytes `lock::render`
/// emits. `path: []` is the body without frontmatter — the document root the token was minted
/// over.
fn lock_block(object: &str, token: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"9ae3f1deadbeef\"\n    path: []\n    fingerprint: \"{token}\"\n```"
    )
}

fn chain() -> Chain {
    let c = "# C\n\nleaf body\n".to_string();
    let c_rev = doc_rev(&c);
    let c_pin = live_fingerprint(&c);

    let b = format!("# B\n\ndraws from c\n\n{}\n", lock_block("c", &c_pin));
    let b_rev = doc_rev(&b);
    let b_pin = live_fingerprint(&b);

    let a = format!("# A\n\ndraws from b\n\n{}\n", lock_block("b", &b_pin));
    let a_rev = doc_rev(&a);

    Chain {
        a,
        b,
        c,
        a_rev,
        b_rev,
        c_rev,
        b_pin,
        c_pin,
    }
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
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A workspace with the three-doc chain written, declared a root by
    /// `mrd init`.
    fn workspace_with_chain(&self, chain: &Chain) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("a.md"), &chain.a).expect("a");
        std::fs::write(ws.join("b.md"), &chain.b).expect("b");
        std::fs::write(ws.join("c.md"), &chain.c).expect("c");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A stable digest of the workspace's `.md` file set + bytes — to prove the walk
/// wrote nothing (never-stored at the verb level).
fn md_snapshot(ws: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(ws).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "md") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read(&path).expect("read")));
        }
    }
    out.sort();
    out
}

/// Gate 1 — the three-doc chain `walk` up-output is byte-expected: depth-tagged
/// `{color, selector, rev}` entries and the rev citations (§2.4 honesty).
#[test]
fn walk_up_is_byte_expected() {
    let sb = sandbox();
    let ch = chain();
    let ws = sb.workspace_with_chain(&ch);

    let out = sb.run(&ws, &["walk", "a.md"]);
    assert!(out.status.success(), "walk up: {}", stderr(&out));

    let expected = format!(
        "walk up a.md\n  \
         depth 1  green  b.md  rev={bp}\n  \
         depth 2  green  c.md  rev={cp}\n\
         revs-read:\n  \
         a.md @ {a}\n  \
         b.md @ {b}\n  \
         c.md @ {c}\n",
        a = ch.a_rev,
        b = ch.b_rev,
        c = ch.c_rev,
        bp = ch.b_pin,
        cp = ch.c_pin,
    );
    assert_eq!(stdout(&out), expected);
}

/// Gate 2 — `walk --down --depth 1` returns EXACTLY the direct dependents: c's
/// only direct dependent is b, never the transitive a.
#[test]
fn walk_down_depth_one_is_direct_dependents_only() {
    let sb = sandbox();
    let ch = chain();
    let ws = sb.workspace_with_chain(&ch);

    let out = sb.run(&ws, &["walk", "c.md", "--down", "--depth", "1", "--json"]);
    assert!(out.status.success(), "walk down: {}", stderr(&out));

    let value: Value = serde_json::from_slice(&out.stdout).expect("json");
    let walk = &value["walk"];
    assert_eq!(walk["direction"], "down");
    assert_eq!(walk["root"], "c.md");
    assert_eq!(walk["depth_bound"], 1);

    let entries = walk["entries"].as_array().expect("entries");
    assert_eq!(
        entries.len(),
        1,
        "exactly the direct dependent, no transitive leak"
    );
    assert_eq!(entries[0]["selector"], "b.md");
    assert_eq!(entries[0]["depth"], 1);
    assert_eq!(entries[0]["color"], "green");
    // `rev` is the PINNED value — bs pin OF c, which under R4 is cs fingerprint
    // token, NOT cs doc rev.
    assert_eq!(entries[0]["rev"], ch.c_pin);

    // Unbounded down reaches the transitive a.md too — the bound above dropped it.
    let full = sb.run(&ws, &["walk", "c.md", "--down", "--json"]);
    let full_value: Value = serde_json::from_slice(&full.stdout).expect("json");
    let selectors: Vec<&str> = full_value["walk"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["selector"].as_str().unwrap())
        .collect();
    assert_eq!(selectors, vec!["b.md", "a.md"]);
}

/// Gate 3 — never-stored: a walk writes nothing. The workspace `.md` set is
/// byte-identical after the walk (no journal page, no listing persisted).
#[test]
fn walk_writes_nothing() {
    let sb = sandbox();
    let ch = chain();
    let ws = sb.workspace_with_chain(&ch);

    let before = md_snapshot(&ws);
    let out = sb.run(&ws, &["walk", "a.md"]);
    assert!(out.status.success(), "walk: {}", stderr(&out));
    let up_down = sb.run(&ws, &["walk", "c.md", "--down"]);
    assert!(up_down.status.success(), "walk down: {}", stderr(&up_down));

    assert_eq!(
        md_snapshot(&ws),
        before,
        "the walk stored nothing — the corpus is byte-identical"
    );
}

/// A drifted pin is a finding: exit 1, the red edge rendered with its reason.
#[test]
fn walk_exits_one_on_a_red_edge() {
    let sb = sandbox();
    let ch = chain();
    let ws = sb.workspace_with_chain(&ch);
    // Rewrite c.md so b's pin of it drifts (live rev no longer matches).
    std::fs::write(ws.join("c.md"), "# C\n\nEDITED — drift\n").expect("edit c");

    let out = sb.run(&ws, &["walk", "a.md"]);
    assert_eq!(out.status.code(), Some(1), "a red edge is a finding");
    assert!(
        stdout(&out).contains("red content-drifted  c.md"),
        "the red edge renders with its reason: {}",
        stdout(&out)
    );
}

/// A missing root is a tool failure (exit 2), never an empty walk.
#[test]
fn walk_missing_root_exits_two() {
    let sb = sandbox();
    let ch = chain();
    let ws = sb.workspace_with_chain(&ch);

    let out = sb.run(&ws, &["walk", "gone.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("walk root not in the corpus: gone.md"));
}
