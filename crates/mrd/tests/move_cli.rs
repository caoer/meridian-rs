//! `mrd move` end to end (`docs/move.md`): the rename lands, every reference
//! class the door owns is rewritten in place, the refusals write nothing, the
//! `--dry` plan and the `--json` frame have one shape, and a rewritten lock
//! row keeps `mrd check` green. Every gate drives the real binary over its
//! process boundary; the daemon is spawn-impossible because the door never
//! needs it (`move.md` §8).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
mod common;

// ── harness ──────────────────────────────────────────────────────────────────

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
    /// Spawn-impossible daemon, isolated cache and home, the mount table of
    /// `two_roots` when one exists, no ambient workspace override.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env("MERIDIAN_CONFIG", self.home.join("MERIDIAN.md"))
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// The fixture corpus: a git-backed, `mrd init`-marked workspace holding
    /// every reference class the door rewrites, committed so the pin plane
    /// can answer for the pinned blob.
    fn corpus(&self) -> PathBuf {
        let ws = self.tmp.path().join("ws");
        std::fs::create_dir_all(&ws).expect("mkdir");
        git_init(&ws);
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", said(&init));

        write(&ws, "guide.md", GUIDE);
        write(&ws, "docs/notes.md", NOTES);
        write(&ws, "docs/other.md", OTHER);
        write(&ws, "data/duckdb/tips.md", TIPS);
        write(&ws, "data/duckdb/intro.md", INTRO);
        write(&ws, "far/ref.md", REF);
        write(&ws, "sources/rec.md", REC);
        let hash = git_in(&ws, &["hash-object", "docs/notes.md"]);
        write(
            &ws,
            "pinning.md",
            &format!(
                "# Pinning\n\n## Inputs\n\n{}",
                lock_block("docs/notes", hash.trim(), &live_fingerprint(NOTES))
            ),
        );
        commit_all(&ws, "fixture");
        ws
    }

    /// Two mounted roots, `alpha` and `beta`, declared in the sandbox's
    /// `MERIDIAN.md` — the rooted lane both operands take (`move.md` §1).
    fn two_roots(&self) -> (PathBuf, PathBuf) {
        let alpha = self.tmp.path().join("alpha");
        let beta = self.tmp.path().join("beta");
        for (dir, name) in [(&alpha, "alpha"), (&beta, "beta")] {
            std::fs::create_dir_all(dir).expect("mkdir");
            write(
                dir,
                "MERIDIAN.md",
                &format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
            );
        }
        write(&alpha, "people/zt.md", "# ZT\n\n## Bio\n\nbio\n");
        write(&alpha, "card.md", CARD);
        write(&beta, "notes.md", "# Beta notes\n");
        let table = format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: alpha\npath: {}\nvault: alpha\n```\n\n\
             ```meridian-mount\nname: beta\npath: {}\nvault: beta\n```\n",
            alpha.display(),
            beta.display()
        );
        std::fs::write(self.home.join("MERIDIAN.md"), table).expect("config");
        (alpha, beta)
    }
}

const GUIDE: &str = "# Guide\n\nsee [[notes]] and [[docs/notes#Setup|the setup]]\n\n![[notes]]\n";
const NOTES: &str = "# Notes\n\n## Setup\n\nthe setup body\n";
const OTHER: &str = "# Other\n\n[[notes]] again\n";
const TIPS: &str = "# Tips\n\n[[duckdb/intro]]\n";
const INTRO: &str = "---\nrelated: \"[[tips]]\"\nsee:\n  - \"[[data/duckdb/tips#Top]]\"\n---\n\
                     # Intro\n\n## Top\n\nintro body\n";
const REF: &str = "# Ref\n\n[[data/duckdb/tips]] · [[duckdb/tips|alias]]\n";
const REC: &str = "# Rec\n\n[[notes]] frozen\n";
const CARD: &str = "---\nowner: \"[[zt]]\"\nsource: \"alpha:people/zt.md#Bio\"\n---\n\
                    # Card\n\n[[people/zt|ZT]]\n";

/// A `meridian-lock` block carrying one whole-page pin, in the canonical
/// bytes the engine mints.
fn lock_block(object: &str, hash: &str, fingerprint: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{hash}\"\n    path: []\n    fingerprint: \"{fingerprint}\"\n```\n"
    )
}

/// The live whole-page fingerprint of `raw`, through the engine's own hasher.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "fixture `git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("git stdout is utf-8")
}

fn git_init(dir: &Path) {
    git_in(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
    git_in(dir, &["config", "user.name", "move fixture"]);
    git_in(dir, &["config", "user.email", "move@fixture.invalid"]);
    git_in(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_all(dir: &Path, message: &str) {
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

fn write(ws: &Path, rel: &str, body: &str) {
    let path = ws.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, body).expect("write fixture");
}

fn read(ws: &Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn said(out: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn frame(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is one JSON frame ({e}): {}", said(out)))
}

/// The pin plane's red and grey rows, as `mrd check --json` reports them.
fn pin_colours(sb: &Sandbox, ws: &Path) -> (Value, Value) {
    let out = sb.run(ws, &["check", "--json"]);
    let v = frame(&out);
    (v["pins"]["red"].clone(), v["pins"]["grey"].clone())
}

// ── the listing ──────────────────────────────────────────────────────────────

#[test]
fn the_verb_is_listed_and_a_bad_invocation_is_exit_2() {
    let sb = sandbox();
    let ws = sb.corpus();
    let help = sb.run(&ws, &["move", "--help"]);
    assert_eq!(code(&help), 0, "{}", said(&help));
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("mrd move <OLD> <NEW>"), "{text}");
    assert!(text.contains("--immutable PREFIX"), "{text}");

    let one = sb.run(&ws, &["move", "docs/notes.md"]);
    assert_eq!(
        code(&one),
        2,
        "a missing operand is a bad invocation: {}",
        said(&one)
    );
    let flag = sb.run(&ws, &["move", "docs/notes.md", "x.md", "--freeze", "a/"]);
    assert_eq!(
        code(&flag),
        2,
        "an unknown flag is a bad invocation: {}",
        said(&flag)
    );
    assert!(ws.join("docs/notes.md").exists());
}

// ── file rename: bare links, fragment + alias, embed, lock row, check green ──

#[test]
fn a_file_rename_rewrites_bare_links_keeps_fragment_and_alias_and_the_lock_row() {
    let sb = sandbox();
    let ws = sb.corpus();
    let before = pin_colours(&sb, &ws);
    assert_eq!(
        before,
        (Value::Array(vec![]), Value::Array(vec![])),
        "green before"
    );

    let out = sb.run(&ws, &["move", "docs/notes.md", "docs/notes-v2.md"]);
    assert_eq!(code(&out), 0, "{}", said(&out));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("moved."), "{text}");

    assert!(!ws.join("docs/notes.md").exists(), "OLD is gone");
    assert_eq!(
        read(&ws, "docs/notes-v2.md"),
        NOTES,
        "the moved bytes are the same bytes"
    );
    assert_eq!(
        read(&ws, "guide.md"),
        "# Guide\n\nsee [[notes-v2]] and [[docs/notes-v2#Setup|the setup]]\n\n![[notes-v2]]\n"
    );
    assert_eq!(
        read(&ws, "docs/other.md"),
        "# Other\n\n[[notes-v2]] again\n"
    );
    assert_eq!(
        read(&ws, "sources/rec.md"),
        "# Rec\n\n[[notes-v2]] frozen\n"
    );
    let pinning = read(&ws, "pinning.md");
    assert!(
        pinning.contains("object: \"[[docs/notes-v2]]\""),
        "the lock row follows the page: {pinning}"
    );
    assert!(
        pinning.contains(&live_fingerprint(NOTES)),
        "the fingerprint is a content fact and stays: {pinning}"
    );

    let after = pin_colours(&sb, &ws);
    assert_eq!(after, before, "the pin plane reads the same after the move");
    let check = sb.run(&ws, &["check", "--json"]);
    assert_eq!(frame(&check)["red"], Value::Bool(false), "{}", said(&check));
}

// ── file move: partial and full path links, frontmatter scalar and list ──────

#[test]
fn a_file_move_rewrites_partial_and_full_links_and_frontmatter_links() {
    let sb = sandbox();
    let ws = sb.corpus();
    let out = sb.run(&ws, &["move", "data/duckdb/tips.md", "data/tips.md"]);
    assert_eq!(code(&out), 0, "{}", said(&out));

    assert_eq!(
        read(&ws, "far/ref.md"),
        "# Ref\n\n[[data/tips]] · [[data/tips|alias]]\n",
        "full path follows; partial becomes the shortest unique suffix; alias kept"
    );
    assert_eq!(
        read(&ws, "data/duckdb/intro.md"),
        "---\nrelated: \"[[tips]]\"\nsee:\n  - \"[[data/tips#Top]]\"\n---\n\
         # Intro\n\n## Top\n\nintro body\n",
        "a bare frontmatter link keeps its unique basename; a list entry follows the path"
    );
    assert_eq!(
        read(&ws, "data/tips.md"),
        TIPS,
        "the moved page's own partial link still resolves"
    );
}

// ── directory move, into-form ────────────────────────────────────────────────

#[test]
fn a_directory_move_carries_the_subtree_and_rewrites_what_would_break() {
    let sb = sandbox();
    let ws = sb.corpus();
    let out = sb.run(&ws, &["move", "data/duckdb", "archive/", "--json"]);
    assert_eq!(code(&out), 0, "{}", said(&out));
    let v = frame(&out);
    assert_eq!(v["move"]["kind"], Value::String("directory".into()));
    assert_eq!(
        v["move"]["new"],
        Value::String("archive/duckdb".into()),
        "into-form lands under NEW"
    );
    assert_eq!(
        v["move"]["renames"].as_array().map(Vec::len),
        Some(2),
        "{v}"
    );

    assert!(!ws.join("data/duckdb").exists(), "the old tree is gone");
    assert_eq!(read(&ws, "archive/duckdb/tips.md"), TIPS);
    assert_eq!(
        read(&ws, "far/ref.md"),
        "# Ref\n\n[[archive/duckdb/tips]] · [[duckdb/tips|alias]]\n",
        "the full path follows; the partial still resolves and is left alone"
    );
    assert_eq!(
        read(&ws, "archive/duckdb/intro.md"),
        "---\nrelated: \"[[tips]]\"\nsee:\n  - \"[[archive/duckdb/tips#Top]]\"\n---\n\
         # Intro\n\n## Top\n\nintro body\n"
    );
}

// ── --immutable: skipped, reported, never written ────────────────────────────

#[test]
fn an_immutable_prefix_is_skipped_and_reported() {
    let sb = sandbox();
    let ws = sb.corpus();
    let out = sb.run(
        &ws,
        &[
            "move",
            "docs/notes.md",
            "docs/notes-v2.md",
            "--immutable",
            "sources/",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", said(&out));
    let v = frame(&out);
    assert_eq!(
        v["move"]["immutable"],
        serde_json::json!([{
            "path": "sources/rec.md", "line": 3, "kind": "wikilink", "old": "notes", "new": "notes-v2"
        }]),
        "{v}"
    );
    assert_eq!(v["move"]["counts"]["immutable_skips"], Value::from(1));
    assert_eq!(
        v["move"]["links"]["read_back"]["dangling"],
        Value::from(1),
        "{v}"
    );
    assert_eq!(
        read(&ws, "sources/rec.md"),
        REC,
        "a frozen page is never written"
    );
    assert_eq!(
        read(&ws, "docs/other.md"),
        "# Other\n\n[[notes-v2]] again\n"
    );

    let frozen = sb.run(
        &ws,
        &[
            "move",
            "sources/rec.md",
            "rec.md",
            "--immutable",
            "sources/",
        ],
    );
    assert_eq!(
        code(&frozen),
        1,
        "OLD under the prefix refuses: {}",
        said(&frozen)
    );
    assert!(ws.join("sources/rec.md").exists());
}

// ── refusals: ambiguity, cross-root ──────────────────────────────────────────

#[test]
fn a_bare_link_the_move_would_leave_ambiguous_refuses_with_the_pair() {
    let sb = sandbox();
    let ws = sb.tmp.path().join("amb");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    write(&ws, "a/x.md", "# X\n");
    write(&ws, "b/y.md", "# Y\n");
    write(&ws, "notes/fan.md", "see [[x]]\n");

    let out = sb.run(&ws, &["move", "a/x.md", "a/y.md", "--json"]);
    assert_eq!(code(&out), 1, "{}", said(&out));
    let v = frame(&out);
    assert_eq!(
        v["error"]["code"],
        Value::String("ambiguous_ref".into()),
        "{v}"
    );
    assert_eq!(
        v["move"]["ambiguous"],
        serde_json::json!([{"source": "notes/fan.md", "linkpath": "x", "candidates": ["a/y.md", "b/y.md"]}]),
        "{v}"
    );
    assert!(ws.join("a/x.md").exists(), "nothing moved");
    assert_eq!(
        read(&ws, "notes/fan.md"),
        "see [[x]]\n",
        "nothing rewritten"
    );

    let human = sb.run(&ws, &["move", "a/x.md", "a/y.md"]);
    assert_eq!(code(&human), 1);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("refused: nothing written."), "{text}");
}

#[test]
fn a_move_across_roots_refuses_and_a_rooted_move_rewrites_rooted_strings() {
    let sb = sandbox();
    let ws = sb.corpus();
    let (alpha, beta) = sb.two_roots();

    let cross = sb.run(&ws, &["move", "alpha:people/zt.md", "beta:zt.md", "--json"]);
    assert_eq!(code(&cross), 1, "{}", said(&cross));
    assert_eq!(
        frame(&cross)["error"]["code"],
        Value::String("bad_path".into())
    );
    assert!(alpha.join("people/zt.md").exists(), "nothing moved");
    assert!(!beta.join("zt.md").exists());

    let mixed = sb.run(&ws, &["move", "docs/notes.md", "beta:notes2.md", "--json"]);
    assert_eq!(code(&mixed), 1, "{}", said(&mixed));
    assert_eq!(
        frame(&mixed)["error"]["code"],
        Value::String("bad_path".into())
    );
    assert!(ws.join("docs/notes.md").exists());

    // Both operands in one root, from a cwd in another tree: the rooted lane
    // selects the workspace, and a rooted string naming that root follows.
    let out = sb.run(
        &ws,
        &["move", "alpha:people/zt.md", "alpha:people/zt-user.md"],
    );
    assert_eq!(code(&out), 0, "{}", said(&out));
    assert_eq!(
        read(&alpha, "card.md"),
        "---\nowner: \"[[zt-user]]\"\nsource: \"alpha:people/zt-user.md#Bio\"\n---\n\
         # Card\n\n[[people/zt-user|ZT]]\n"
    );
    assert!(alpha.join("people/zt-user.md").exists());
    assert!(
        ws.join("docs/notes.md").exists(),
        "the ambient tree was not touched"
    );
}

// ── --dry writes nothing; the --json frame has one shape ─────────────────────

#[test]
fn a_dry_run_prints_the_whole_plan_and_writes_nothing() {
    let sb = sandbox();
    let ws = sb.corpus();
    let snapshot: Vec<(String, String)> = [
        "guide.md",
        "docs/notes.md",
        "docs/other.md",
        "sources/rec.md",
        "pinning.md",
    ]
    .iter()
    .map(|rel| ((*rel).to_owned(), read(&ws, rel)))
    .collect();

    let human = sb.run(&ws, &["move", "docs/notes.md", "docs/notes-v2.md", "--dry"]);
    assert_eq!(code(&human), 0, "{}", said(&human));
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("dry run: would move docs/notes.md → docs/notes-v2.md"),
        "{text}"
    );
    assert!(
        text.contains("guide.md"),
        "the plan names every page it would rewrite: {text}"
    );
    assert!(text.contains("dry run: nothing written."), "{text}");

    let json = sb.run(
        &ws,
        &[
            "move",
            "docs/notes.md",
            "docs/notes-v2.md",
            "--dry",
            "--json",
        ],
    );
    assert_eq!(code(&json), 0, "{}", said(&json));
    assert_eq!(
        frame(&json)["move"]["links"]["read_back"],
        Value::Null,
        "a dry run reads nothing back"
    );

    for (rel, bytes) in &snapshot {
        assert_eq!(
            &read(&ws, rel),
            bytes,
            "{rel} is byte-identical after a dry run"
        );
    }
    assert!(ws.join("docs/notes.md").exists());
    assert!(!ws.join("docs/notes-v2.md").exists());
}

#[test]
fn the_json_frame_has_one_shape() {
    let sb = sandbox();
    let ws = sb.corpus();
    let out = sb.run(
        &ws,
        &[
            "move",
            "docs/notes.md",
            "docs/notes-v2.md",
            "--dry",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", said(&out));
    let v = frame(&out);
    assert!(v["workspace"].is_string(), "{v}");
    let m = &v["move"];
    let keys: Vec<&str> = m
        .as_object()
        .expect("move object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "ambiguous",
            "applied",
            "counts",
            "dry",
            "immutable",
            "kind",
            "links",
            "lock_unreadable",
            "new",
            "old",
            "renames",
            "rewrites"
        ],
        "{v}"
    );
    assert_eq!(m["dry"], Value::Bool(true));
    assert_eq!(m["applied"], Value::Bool(false));
    assert_eq!(m["kind"], Value::String("file".into()));
    assert_eq!(
        m["renames"],
        serde_json::json!([{"from": "docs/notes.md", "to": "docs/notes-v2.md"}])
    );
    let rewrites = m["rewrites"].as_array().expect("rewrites");
    let paths: Vec<&str> = rewrites
        .iter()
        .map(|r| r["path"].as_str().expect("path"))
        .collect();
    assert_eq!(
        paths,
        ["docs/other.md", "guide.md", "pinning.md", "sources/rec.md"],
        "{v}"
    );
    assert_eq!(
        rewrites[1],
        serde_json::json!({"path": "guide.md", "wikilinks": 2, "embeds": 1, "frontmatter": 0, "rooted": 0, "lock_rows": 0})
    );
    assert_eq!(rewrites[2]["lock_rows"], Value::from(1), "{v}");
    assert_eq!(
        m["counts"],
        serde_json::json!({
            "files_rewritten": 4, "links_rewritten": 5, "lock_rows_rewritten": 1,
            "immutable_skips": 0, "moved_outside_domain": 0
        }),
        "{v}"
    );
    assert_eq!(
        m["links"]["before"], m["links"]["after"],
        "link-neutral: {v}"
    );
    assert_eq!(
        m["links"]["read_back"],
        Value::Null,
        "a dry run reads nothing back"
    );
}
