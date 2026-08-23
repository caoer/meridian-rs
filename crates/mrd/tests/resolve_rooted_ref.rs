//! The resolve door's rooted lane — `root:path` on `mrd resolve`, answering
//! WHERE THE REF LANDS (address-grammar §4.1, the colon law; the MCP resolve
//! face's answer, ported through the read door's seam).
//!
//! The measured leak this closes (card cli-resolve-rooted-ref): after the
//! read (#124) and mint (#126) doors grew their rooted lanes, `mrd resolve
//! meridian-rs:docs/laws.md` still treated the colon as a path character and
//! answered `cannot canonicalize …/<ambient>/meridian-rs:docs/laws.md` — an
//! ambient canonicalize of the literal string, from the one door whose whole
//! job is naming where a ref lands without reading it.
//!
//! The law under gate:
//! - §4.1 root-wins: a head colon is an address, never a literal path — a
//!   bound `root:path` answers the named root's landing, and an unbound root
//!   refuses as a ROOT problem (exit 1), never a canonicalize of the literal
//!   colon-bearing string;
//! - the answer is the MCP resolve face's, shape-for-shape: physical path,
//!   rooted lane, root row (state + workspace), canonical `root:path`;
//! - no existence check: a missing rel inside a BOUND root still names its
//!   landing (absence is the engine's question, §5.6, never the address
//!   plane's);
//! - path grain: a `#` fragment refuses with its own teaching;
//! - D3: a colon after the first `/` stays an ordinary path byte — the
//!   ambient ladder report survives beside the lane.
//!
//! Every leg is daemon-free by construction: the rooted lane answers from
//! the mount table alone, and the sandbox cwd is git-anchored so refusal
//! frames resolve purely.

use std::path::PathBuf;
use std::process::{Command, Output};
mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    /// Held for its Drop — the sandbox tree dies with it.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    /// The ambient workspace (git-anchored), holding the literal-string traps.
    ws: PathBuf,
    /// The mounted root's directory.
    sessions: PathBuf,
}

fn sandbox() -> Sandbox {
    sandbox_with(false)
}

fn sandbox_with(primary: bool) -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    for d in [&home, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    // The mounted root declares its own canonical name (INV-5) — without this
    // the bind renders grey(undeclared) and every acceptance below is vacuous.
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("root declaration");
    std::fs::write(sessions.join("notes.md"), "# Notes\n\nthe real note.\n").expect("target");

    let config = home.join("MERIDIAN.md");
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
         ```meridian-mount\nname: sessions\npath: {}\n{}vault: sessions\n```\n",
        sessions.display(),
        // Canonical field order is name, path, primary, vault, pin — the
        // config plane refuses a reordered block.
        if primary { "primary: true\n" } else { "" }
    );
    std::fs::write(&config, raw).expect("config");

    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        sessions,
    }
}

impl Sandbox {
    /// Run `mrd` from `ws`. Spawn-impossible daemon on purpose: every answer
    /// below must arrive without one, so anything that answers answered from
    /// the mount table and the pure ladder alone.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_CONFIG", self.home.join("MERIDIAN.md"))
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE")
            .args(args)
            .current_dir(&self.ws)
            .output()
            .expect("spawn mrd")
    }

    fn canonical_sessions(&self) -> PathBuf {
        std::fs::canonicalize(&self.sessions).expect("canonical sessions")
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

// ---------------------------------------------------------------------------
// The acceptance half — a bound `root:path` names where the ref lands
// (quality gate 1: never the leaked ambient canonicalize).
// ---------------------------------------------------------------------------

/// The card's gate, in the sandbox's frame: a bound `root:path` answers the
/// NAMED root's landing — physical path first, then lane, root row, and the
/// canonical spelling — and the ambient workspace appears nowhere in the
/// answer.
#[test]
fn a_bound_rooted_ref_names_where_it_lands() {
    let sb = sandbox();
    let out = sb.run(&["resolve", "sessions:notes.md"]);
    assert_eq!(
        code(&out),
        0,
        "the rooted resolve answers: {}",
        stderr(&out)
    );
    let text = stdout(&out);
    let landed = sb.canonical_sessions().join("notes.md");
    assert_eq!(
        text.lines().next().unwrap_or_default(),
        landed.display().to_string(),
        "the physical path leads the answer: {text:?}"
    );
    assert!(
        text.contains("lane: rooted — the ref names root sessions"),
        "the lane line names the root: {text:?}"
    );
    assert!(
        text.contains(&format!(
            "root: sessions (bound)  workspace:{}",
            sb.canonical_sessions().display()
        )),
        "the root row states bind and workspace: {text:?}"
    );
    assert!(
        text.contains("ref: sessions:notes.md"),
        "the canonical spelling closes the answer: {text:?}"
    );
    // The leak this card closes: the ambient workspace must appear NOWHERE —
    // its presence is the literal-string canonicalize wearing any spelling.
    let ambient = sb.ws.display().to_string();
    assert!(
        !text.contains(&ambient) && !stderr(&out).contains(&ambient),
        "the ambient workspace never enters a rooted answer: {text:?}"
    );
}

/// No existence check: a missing rel inside a BOUND root still names its
/// landing — absence is the engine's question (§5.6), never the address
/// plane's. (Semantics probed live on the MCP face 2026-08-16: a missing
/// file resolves identically to a present one.)
#[test]
fn a_missing_rel_inside_a_bound_root_still_names_its_landing() {
    let sb = sandbox();
    let out = sb.run(&["resolve", "sessions:nope.md"]);
    assert_eq!(code(&out), 0, "no existence check: {}", stderr(&out));
    let text = stdout(&out);
    assert_eq!(
        text.lines().next().unwrap_or_default(),
        sb.canonical_sessions()
            .join("nope.md")
            .display()
            .to_string(),
        "the landing is named for a file that is not there: {text:?}"
    );
    assert!(
        text.contains("ref: sessions:nope.md"),
        "the canonical spelling still closes the answer: {text:?}"
    );
}

/// The `--json` face carries the same facts in the door's house keys, and the
/// undesignated mount reports `primary: false`.
#[test]
fn the_json_face_carries_the_ported_answer() {
    let sb = sandbox();
    let out = sb.run(&["resolve", "sessions:notes.md", "--json"]);
    assert_eq!(code(&out), 0, "json face answers: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("json parses ({e}): {}", stdout(&out)));
    let canonical = sb.canonical_sessions();
    assert_eq!(
        v["workspace"].as_str().unwrap_or_default(),
        canonical.display().to_string(),
        "workspace is the bound root's path: {v}"
    );
    assert_eq!(
        v["source"], "rooted",
        "the lane rides the door's source key: {v}"
    );
    assert_eq!(v["root"], "sessions", "{v}");
    assert_eq!(v["primary"], false, "{v}");
    assert_eq!(
        v["path"].as_str().unwrap_or_default(),
        canonical.join("notes.md").display().to_string(),
        "{v}"
    );
    assert_eq!(v["ref"], "sessions:notes.md", "{v}");
}

/// The declared-primary designation crosses from the one table read: the
/// human row grows the marker, the json face flips the bool.
#[test]
fn a_primary_mount_marks_its_row() {
    let sb = sandbox_with(true);
    let human = sb.run(&["resolve", "sessions:notes.md"]);
    let json = sb.run(&["resolve", "sessions:notes.md", "--json"]);
    assert_eq!(code(&human), 0, "{}", stderr(&human));
    assert!(
        stdout(&human).contains("← primary"),
        "the primary marker rides the root row: {:?}",
        stdout(&human)
    );
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).expect("primary json parses");
    assert_eq!(v["primary"], true, "{v}");
}

/// D3: a colon after the first `/` is an ordinary path byte — the spelling
/// stays on the ambient ladder and the machine-bootstrap report survives.
#[test]
fn a_colon_after_the_first_slash_stays_on_the_ladder() {
    let sb = sandbox();
    std::fs::create_dir_all(sb.ws.join("dir")).expect("dir");
    std::fs::write(sb.ws.join("dir").join("a:b.md"), "# X\n\nliteral.\n").expect("member");
    let out = sb.run(&["resolve", "dir/a:b.md"]);
    assert_eq!(code(&out), 0, "the ladder still answers: {}", stderr(&out));
    assert!(
        stdout(&out).starts_with("workspace "),
        "the ambient lanes keep the ladder shape: {:?}",
        stdout(&out)
    );
}

// ---------------------------------------------------------------------------
// §4.1 root-wins — a root problem refuses as a root problem, never a
// canonicalize of the literal string (quality gate 2).
// ---------------------------------------------------------------------------

/// An unbound root refuses, NAMES the bound roots, and never canonicalizes —
/// even when a file literally named `sessionz:notes.md` sits in the ambient
/// workspace waiting to be misresolved.
#[test]
fn an_unbound_root_refuses_as_a_root_problem() {
    let sb = sandbox();
    // The trap: the literal file EXISTS. §4.1 forbids the fallback that
    // would name it — a wrong success of exactly the misresolve shape.
    std::fs::write(sb.ws.join("sessionz:notes.md"), "# decoy\n").expect("literal trap");

    let out = sb.run(&["resolve", "sessionz:notes.md"]);
    assert_eq!(
        code(&out),
        1,
        "an unbound root is an address refusal: {}",
        stderr(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("does not bind") && err.contains("bound roots: sessions"),
        "the refusal names the miss and enumerates what DOES bind: {err:?}"
    );
    assert!(
        !err.contains("cannot canonicalize") && !stdout(&out).contains("workspace "),
        "the literal string is never canonicalized and no ladder answer leaks: \
         {err:?} / {:?}",
        stdout(&out)
    );
}

/// The malformed heads of §4.5 refuse with the grammar's own teaching, and
/// the door composes its own §1 consequence clause.
#[test]
fn malformed_rooted_heads_refuse_with_the_grammar_teaching() {
    let sb = sandbox();
    for (spelling, phrase) in [
        ("Sessions:notes.md", "is not a canonical root name"),
        ("a:b:c.md", "more than one `:`"),
        (":notes.md", "names no root"),
        ("sessions:", "names no path"),
    ] {
        let out = sb.run(&["resolve", spelling]);
        assert_eq!(code(&out), 1, "{spelling}: a malformed head refuses");
        assert!(
            stderr(&out).contains(phrase),
            "{spelling}: the refusal teaches its own arm ({phrase:?}): {:?}",
            stderr(&out)
        );
    }
    let out = sb.run(&["resolve", "sessions:"]);
    assert!(
        stderr(&out).contains("Nothing was resolved."),
        "the refusal carries the resolve door's own consequence: {:?}",
        stderr(&out)
    );
}

/// The rel half obeys the §1 path law, and the confinement teaching speaks in
/// the resolve door's name.
#[test]
fn an_unconfined_rel_refuses_in_the_doors_name() {
    let sb = sandbox();
    let out = sb.run(&["resolve", "sessions:../escape.md"]);
    assert_eq!(code(&out), 1, "confinement refuses: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("resolve path") && err.contains("Nothing was resolved."),
        "the teaching names this door and its consequence: {err:?}"
    );
}

/// Path grain, the MCP face's own `#` door: a fragment refuses with its
/// teaching instead of silently resolving the path half.
#[test]
fn a_fragment_refuses_at_path_grain() {
    let sb = sandbox();
    let out = sb.run(&["resolve", "sessions:notes.md#Design"]);
    assert_eq!(code(&out), 1, "a fragment refuses: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("path grain") && err.contains("Nothing was resolved."),
        "the refusal teaches the grain and the consequence: {err:?}"
    );
}

/// The `--json` face keeps its `{workspace, error}` frame on the refusal leg
/// — a machine consumer cannot tell an absent frame from success with no
/// output.
#[test]
fn a_rooted_refusal_emits_the_json_error_frame() {
    let sb = sandbox();
    let out = sb.run(&["resolve", "sessionz:notes.md", "--json"]);
    assert_eq!(code(&out), 1);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("refusal frame parses ({e}): {}", stdout(&out)));
    assert!(v.get("workspace").is_some(), "frame carries workspace: {v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("does not bind")),
        "frame carries the teaching refusal: {v}"
    );
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
