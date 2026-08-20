//! `frontmatter.prop_rev` — the per-key CAS token on the SQL projection
//! (`docs/node-rev-merkle-spec.md` §2.1), driven through the real binary.
//!
//! Three gates, and the third is the reason the column exists:
//! - a multi-key document yields a DISTINCT `prop_rev` per key while its
//!   `node_rev` stays the one block-grain token every key shares;
//! - the column agrees byte-for-byte with `mrd read --json`'s
//!   `.read.props[].prop_rev` — the anti-drift gate, since both would serve 16
//!   plausible hex characters if the projector minted its own hash;
//! - a guarded `put` on an `fm_key` target COMMITS on `prop_rev` and REFUSES
//!   `cas_mismatch` on `node_rev`. Before this column the projection served
//!   only the second token, so a guarded frontmatter write refused
//!   deterministically and the unguarded path clobbered concurrent writes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

mod common;

/// Four top-level keys, so the block grain and the key grain cannot coincide.
const DOC: &str =
    "---\ntype: note\nstatus: seeded\nowner: zt\ntags: [a, b]\n---\n\n# Alpha\n\nbody\n";

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

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
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    /// Run with `stdin_bytes` piped — the `put` edits channel.
    fn run_stdin(&self, cwd: &Path, args: &[&str], stdin_bytes: &str) -> Output {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, stdin_bytes.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }

    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            stderr(out)
        )
    })
}

/// `[key, node_rev, prop_rev]` per row, document order.
fn rev_rows(sb: &Sandbox, ws: &Path) -> Vec<(String, String, String)> {
    let out = sb.run(
        ws,
        &[
            "sql",
            "--json",
            "SELECT key, node_rev, prop_rev FROM frontmatter WHERE path = 'doc.md' ORDER BY ord",
        ],
    );
    assert!(out.status.success(), "sql failed: {}", stderr(&out));
    json(&out)["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| {
            let cell = |i: usize| r[i].as_str().expect("text cell").to_owned();
            (cell(0), cell(1), cell(2))
        })
        .collect()
}

/// Gate 1 — the grain. Four keys, four distinct `prop_rev`, one shared
/// `node_rev`: the block token cannot address a single key, which is the whole
/// reason a second column had to land beside it.
#[test]
fn prop_rev_is_distinct_per_key_while_node_rev_stays_block_grain() {
    let sb = sandbox();
    let ws = sb.workspace();
    let rows = rev_rows(&sb, &ws);

    let keys: Vec<&str> = rows.iter().map(|(k, ..)| k.as_str()).collect();
    assert_eq!(keys, ["type", "status", "owner", "tags"], "document order");

    let block: std::collections::BTreeSet<&str> = rows.iter().map(|(_, n, _)| n.as_str()).collect();
    assert_eq!(
        block.len(),
        1,
        "node_rev is the whole `---`…`---` span — every key shares it: {rows:?}"
    );

    let per_key: std::collections::BTreeSet<&str> = rows.iter().map(|(.., p)| p.as_str()).collect();
    assert_eq!(
        per_key.len(),
        rows.len(),
        "each key mints its own prop_rev: {rows:?}"
    );
    for (key, _, prop_rev) in &rows {
        assert_eq!(
            prop_rev.len(),
            16,
            "prop_rev is 16 lowercase hex (§1 width) for {key}: {prop_rev}"
        );
        assert!(
            prop_rev
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "prop_rev is 16 lowercase hex (§1 width) for {key}: {prop_rev}"
        );
    }
}

/// Gate 2 — anti-drift. The projection SERVES the read face's token; it does
/// not mint one. Two derivations of one hash disagree silently, because both
/// spellings produce 16 plausible hex characters.
#[test]
fn the_column_equals_the_read_face_prop_rev_for_every_key() {
    let sb = sandbox();
    let ws = sb.workspace();

    let read = sb.run(&ws, &["read", "doc.md", "--json"]);
    assert!(read.status.success(), "read failed: {}", stderr(&read));
    let served: Vec<(String, String)> = json(&read)["read"]["props"]
        .as_array()
        .expect("props plane")
        .iter()
        .map(|p| {
            (
                p["key"].as_str().expect("key").to_owned(),
                p["prop_rev"].as_str().expect("prop_rev").to_owned(),
            )
        })
        .collect();

    let projected: Vec<(String, String)> = rev_rows(&sb, &ws)
        .into_iter()
        .map(|(key, _, prop_rev)| (key, prop_rev))
        .collect();

    assert_eq!(
        projected, served,
        "frontmatter.prop_rev must be the same token `mrd read --json` serves \
         at .read.props[].prop_rev — one owner (model::resolve on an fm_key), \
         never a second derivation"
    );
}

/// Gate 3 — the card's reason to exist. One guarded edit, twice: the key-grain
/// token commits it, the block-grain token refuses it `cas_mismatch`.
#[test]
fn a_guarded_fm_key_put_commits_on_prop_rev_and_refuses_on_node_rev() {
    let sb = sandbox();
    let ws = sb.workspace();
    let rows = rev_rows(&sb, &ws);
    let (_, node_rev, prop_rev) = rows
        .iter()
        .find(|(k, ..)| k == "status")
        .expect("status row")
        .clone();

    let edits = |guard: &str| {
        format!(
            r#"[{{"target":{{"fm_key":"status"}},"edit":{{"match":{{"old":"seeded","new":"done"}}}},"if_node_rev":"{guard}"}}]"#
        )
    };

    // The block token: refused, and the refusal names the key-grain token as
    // the actual — the diagnosis this column restores.
    let refused = sb.run_stdin(&ws, &["put", "doc.md", "--json"], &edits(&node_rev));
    assert_eq!(
        refused.status.code(),
        Some(1),
        "the block-grain token must refuse a key-grain write: {}",
        stderr(&refused)
    );
    let body = json(&refused);
    assert_eq!(
        body["error"]["code"], "cas_mismatch",
        "refusal is cas_mismatch: {body}"
    );
    assert_eq!(
        body["error"]["expected"], node_rev,
        "the refusal echoes the block token the caller pinned: {body}"
    );
    assert_eq!(
        body["error"]["actual"], prop_rev,
        "and names the key-grain token as the actual — the diagnosis the \
         column restores: {body}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("doc"),
        DOC,
        "a refused splice writes nothing"
    );

    // The key-grain token: committed.
    let ok = sb.run_stdin(&ws, &["put", "doc.md", "--json"], &edits(&prop_rev));
    assert_eq!(
        ok.status.code(),
        Some(0),
        "prop_rev is the token the fm_key write door compares against: {}",
        stderr(&ok)
    );
    assert!(
        std::fs::read_to_string(ws.join("doc.md"))
            .expect("doc")
            .contains("status: done"),
        "the guarded write landed"
    );
}
