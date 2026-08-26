//! Card `engine-io-error-names-no-path`, **leg 2**: an unreadable directory
//! mid-corpus, gated on the two clauses the card's § Done binds together —
//! the refusal **names the path**, and recovery **needs no restart**.
//!
//! Receipt (2026-08-24 ≈01:39–01:46Z, field-notes-sessions): one mode-000
//! session directory made every face refuse `io_error: Permission denied (os
//! error 13)`, workspace-wide, naming nothing. The incident memo
//! (`agents/36336bf0/memos/transient-workspace-eacces-clean-tree.md`) records
//! that it "cleared with no intervention" — nobody restarted the daemon. Both
//! halves of that sentence are behaviour, and until this file neither was
//! gated at the daemon.
//!
//! **Why this gate lives beside the `crates/fs` one rather than inside it.**
//! `crates/fs/tests/listing_refusal_names_path.rs` pins the mint: every corpus
//! walk names the directory it could not list. It cannot pin "needs no
//! restart", because at that layer there is no resident to restart — a
//! `DomainCache` that refuses and then serves proves the memo re-stats, not
//! that a live daemon recovers. The clause is a claim about the RESIDENT, so
//! the gate has to hold one.
//!
//! **What makes the recovery clause bite rather than narrate.** One
//! [`RunningServer`], started once and never restarted, and one socket
//! connection carrying all three calls: serve → refuse → serve. On top of
//! that the workspace's `tree_instance` is read before the poisoning and
//! after the recovery and asserted IDENTICAL. That token is the RING's
//! identity, not the engine's — the wire contract mints it fresh per ring
//! epoch and changes it on a daemon restart or an idle reap
//! (`crates/wire/src/lib.rs`, `sub_dead_instance_teaching`).
//! Equality therefore rules out the two ways the resident can be REPLACED
//! under the assertions: a restarted daemon, and a reap-and-rebirth. Both
//! mint a fresh ring, and `tree_instance` is that ring's identity.
//!
//! **It does not discriminate an in-place engine rebuild — which is what this
//! test's own recovery performs.** `crates/registry/src/registry.rs` replaces
//! the engines map in place under a stable ring; only reap removes the ring;
//! and a `WorkspaceRing` is constructed solely on first use. So a pass here
//! pins "not restarted, not reaped". It does not pin "not rebuilt", and a
//! passing gate stays consistent with the engine having been rebuilt
//! underneath the assertions.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// A daemon config rooted under `tmp`, with every horizon pushed past the
/// test's lifetime. The reap horizons matter to THIS gate more than to most:
/// an idle reap would drop the warm engine and change `tree_instance`, and the
/// recovery clause would then fail for a reason that has nothing to do with
/// the unreadable directory.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// A workspace `tmp/ws` with markdown at three depths, so `notes/locked` is
/// genuinely MID-corpus: readable members sit both above it and beneath it.
fn write_ws(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("ws");
    for (rel, content) in [
        ("a.md", "# A\n"),
        ("notes/b.md", "# B\n"),
        ("notes/locked/c.md", "# C\n"),
        ("notes/locked/deeper/d.md", "# D\n"),
    ] {
        let path = ws.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    ws
}

struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    fn call(&mut self, request: &Value) -> Value {
        common::honour_retry(|| {
            let mut line = serde_json::to_string(request).unwrap();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.flush().unwrap();
            let mut response = String::new();
            self.reader.read_line(&mut response).unwrap();
            serde_json::from_str(&response).unwrap()
        })
    }

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }

    /// The incident's own face: a composed corpus read, which is what every
    /// refusing MCP `read` was.
    fn read(&mut self, rel: &str) -> Value {
        self.call(&json!({"op": "read", "path": rel}))
    }
}

/// The workspace's warm-tree identity token, taken on a THROWAWAY connection:
/// an accepted `sub` turns its own connection push-only, so reading the token
/// must not cost the connection the rest of the gate is driving.
fn tree_instance(socket: &Path, ws: &Path) -> String {
    let mut probe = Conn::open(socket);
    let hello = probe.hello(ws);
    assert_eq!(hello["ok"], json!(true), "the probe binds: {hello}");
    let ack = probe.call(&json!({"op": "sub"}));
    assert_eq!(ack["ok"], json!(true), "the probe subscribes: {ack}");
    ack["body"]["tree_instance"]
        .as_str()
        .unwrap_or_else(|| panic!("the sub ack teaches tree_instance: {ack}"))
        .to_owned()
}

/// An unreadable directory that becomes readable again however the test
/// leaves — returned value, failed assertion, or panic. Restoring only on the
/// happy path would let a failing assertion leak a mode-000 directory that
/// `TempDir`'s own cleanup then cannot remove, so a first failure would breed
/// a second, unrelated symptom in the next run.
struct Locked<'a>(&'a Path);

impl<'a> Locked<'a> {
    /// Take the permissions away, and PROVE the instrument bites before any
    /// assertion rests on it: `chmod 000` is a no-op for a privileged user,
    /// and a gate that passes because its precondition never held is a green
    /// log that means nothing.
    fn new(dir: &'a Path) -> Locked<'a> {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            fs::read_dir(dir).is_err(),
            "PRECONDITION FAILED: {} is still listable at mode 000 — this gate \
             cannot run as a privileged user, and passing here would test nothing",
            dir.display()
        );
        Locked(dir)
    }

    fn unlock(&self) {
        let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        self.unlock();
    }
}

/// Leg 2's gate, both clauses against ONE never-restarted resident.
#[test]
fn an_unreadable_dir_names_the_path_and_recovery_needs_no_restart() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let socket = server.socket_path().to_path_buf();

    let mut conn = Conn::open(&socket);
    let bound = conn.hello(&ws);
    assert_eq!(
        bound["ok"],
        json!(true),
        "the healthy corpus binds: {bound}"
    );
    let healthy = conn.read("a.md");
    assert_eq!(
        healthy["ok"],
        json!(true),
        "BASELINE: the corpus serves before the poisoning, so a later refusal \
         is caused by it and not inherited: {healthy}"
    );
    let before = tree_instance(&socket, &ws);

    // ---- clause (a): the refusal names the path -------------------------
    let locked_dir = ws.join("notes/locked");
    let locked = Locked::new(&locked_dir);

    let refusal = conn.read("a.md");
    assert_eq!(
        refusal["ok"],
        json!(false),
        "an unreadable directory mid-corpus refuses the whole walk: {refusal}"
    );
    assert_eq!(
        refusal["error"]["code"],
        json!("io_error"),
        "the incident's own code: {refusal}"
    );
    let cause = refusal["error"]["cause"]
        .as_str()
        .unwrap_or_else(|| panic!("io_error carries its cause: {refusal}"));
    assert!(
        cause.contains("notes/locked"),
        "CLAUSE (a): the refusal must name the directory it could not list, or \
         the caller hunts the whole tree from outside — the exact six-minute \
         cost this card was opened for: {cause:?}"
    );

    // ---- clause (b): recovery needs no restart --------------------------
    locked.unlock();

    let after_unlock = conn.read("a.md");
    assert_eq!(
        after_unlock["ok"],
        json!(true),
        "CLAUSE (b): the SAME resident, on the SAME connection, must serve \
         again once the directory is readable — nothing was restarted: \
         {after_unlock}"
    );
    let after = tree_instance(&socket, &ws);
    assert_eq!(
        before, after,
        "CLAUSE (b), structurally: the warm tree instance must be the one that \
         refused. A different token would mean the corpus healed by being \
         rebuilt — a restart or an idle reap wearing recovery's clothes"
    );

    drop(locked);
    server.shutdown();
}

/// Negative control for the gate above. Without it, both clauses would also
/// pass on an engine that never refuses at all: clause (a)'s assertion is only
/// reached through a refusal, and clause (b) is a claim that serving RESUMES —
/// which is vacuous if serving never stopped. This pins that the poisoning,
/// not the corpus, is what makes the difference.
#[test]
fn the_same_corpus_never_refuses_while_every_directory_is_readable() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let bound = conn.hello(&ws);
    assert_eq!(bound["ok"], json!(true), "healthy corpus binds: {bound}");
    for rel in ["a.md", "notes/b.md", "notes/locked/c.md"] {
        let answer = conn.read(rel);
        assert_eq!(
            answer["ok"],
            json!(true),
            "{rel} serves while every directory is readable: {answer}"
        );
    }

    server.shutdown();
}

/// Control for the identity half of clause (b). `before == after` is only
/// evidence if the token can come out otherwise — a `tree_instance` that were
/// derived from, say, the workspace path would be equal across a restart too,
/// and the assertion would pass on the very state it exists to reject. This
/// pins that a genuinely restarted resident answers with a DIFFERENT token.
#[test]
fn the_tree_instance_token_moves_across_a_restart_so_equality_discriminates() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);

    let first = RunningServer::start(test_config(&tmp)).unwrap();
    let before = tree_instance(first.socket_path(), &ws);
    first.shutdown();

    let second = RunningServer::start(test_config(&tmp)).unwrap();
    let after = tree_instance(second.socket_path(), &ws);
    second.shutdown();

    assert_ne!(
        before, after,
        "a restarted resident must answer with a new tree_instance, or the \
         equality assertion in the recovery gate is vacuous"
    );
}

/// The recovery clause is about the DIRECTORY becoming readable again, not
/// about time passing: while the directory stays unreadable the refusal must
/// persist. Without this, a gate could pass on an engine that simply forgot
/// the failure after one call — which would be a different bug wearing
/// recovery's clothes, and the one the card's "cache/state" wording feared.
#[test]
fn the_refusal_persists_while_the_directory_stays_unreadable() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws)["ok"], json!(true));

    let locked_dir = ws.join("notes/locked");
    let locked = Locked::new(&locked_dir);

    for attempt in 0..5 {
        let refusal = conn.read("a.md");
        assert_eq!(
            refusal["ok"],
            json!(false),
            "attempt {attempt}: the refusal is a fact about disk, and disk has \
             not changed: {refusal}"
        );
        assert_eq!(refusal["error"]["code"], json!("io_error"));
    }

    drop(locked);
    server.shutdown();
}
