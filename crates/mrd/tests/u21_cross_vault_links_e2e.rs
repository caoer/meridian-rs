//! U21 — the LINK plane, driven through the real `mrd links`: a cross-vault
//! link must resolve into its bound root, and the answer must differ between a
//! bound and an unbound root. Each arm runs with a fresh `XDG_CACHE_HOME`, so a
//! stale daemon table cannot explain a result.
//!
//! # Harness hygiene (correctness-lane daemon leak)
//!
//! `mrd links` auto-spawns a detached resident (`ensure_daemon`) on first use, and
//! the sandboxed `XDG_CACHE_HOME` holding its pidfile dies with the test. Without
//! a reap-before-delete the resident outlives the suite, so soft [`Drop`] reaps by
//! the pid the daemon wrote to its OWN pidfile (never `pgrep -f`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The TRUE target, in the `sessions` root.
const TARGET: &str = "# Notes\n\nTARGET BYTES — this file lives in the `sessions` root.\n";
/// The DECOY: same basename, ambient root, DIFFERENT bytes. This is the file a
/// basename fallback would answer with (FINDING 03).
const DECOY: &str = "# Notes\n\nAMBIENT ROOT FILE — the wrong document.\n";

struct Sandbox {
    /// Held for its field-Drop — the tree is deleted when this falls. [`Sandbox`]'s
    /// own `Drop` reaps the resident FIRST, while the pidfile under `cache_home`
    /// still exists; only then does field-drop remove the tempdir.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    /// The ambient workspace.
    ws: PathBuf,
    /// The mounted root's directory.
    sessions: PathBuf,
    /// `MERIDIAN.md` — the mount table.
    config: PathBuf,
}

/// A `MERIDIAN.md` mount table binding `sessions` to `dir`, or binding nothing.
fn config_raw(sessions: Option<&Path>) -> String {
    use std::fmt::Write as _;
    let mut raw = "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n".to_string();
    if let Some(dir) = sessions {
        let _ = write!(
            raw,
            "```meridian-mount\nname: sessions\npath: {}\nvault: sessions\n```\n",
            dir.display()
        );
    }
    raw
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    for d in [&home, &ws, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    // The mounted root declares its own canonical name (INV-5) — without this
    // the bind renders grey(undeclared) and every acceptance below is vacuous.
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("root declaration");
    std::fs::write(sessions.join("notes.md"), TARGET).expect("target");
    std::fs::write(ws.join("notes.md"), DECOY).expect("decoy");
    std::fs::write(
        ws.join("local.md"),
        "# Local\n\nAn ordinary ambient page.\n",
    )
    .expect("local");

    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, config_raw(Some(&sessions))).expect("config");

    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        sessions,
        config,
    }
}

impl Sandbox {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_CONFIG", &self.config)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// Write the claim page carrying `body`'s links.
    fn page(&self, name: &str, body: &str) {
        std::fs::write(self.ws.join(name), body).expect("page");
    }

    /// Unmount `sessions` by rewriting the table with no mount entry.
    fn unmount(&self) {
        std::fs::write(&self.config, config_raw(None)).expect("rewrite config");
    }

    /// `mrd links <page> --json`, parsed. Returns the whole envelope so a gate
    /// can read `source` (which path served it) as well as the edge map.
    fn links(&self, page: &str) -> (Output, Value) {
        let out = self.run(&["links", page, "--json"]);
        let v: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("links json ({e}): {}", stdout(&out)));
        (out, v)
    }

    /// One page's edge entry out of a `links` envelope.
    fn edges(&self, page: &str) -> (Output, Value) {
        let (out, v) = self.links(page);
        let entry = v["links"]["files"][page].clone();
        assert!(
            entry.is_object(),
            "the map names the corpus, so the page has an entry: {}",
            stdout(&out)
        );
        (out, entry)
    }

    /// The resident daemon's pidfile under this sandbox's cache root.
    fn daemon_pidfile(&self) -> PathBuf {
        self.cache_home
            .join("meridian")
            .join("registry")
            .join("daemon.pid")
    }

    /// Best-effort reap: TERM → verify → KILL → verify. Never panics (Drop path).
    /// Returns the pid that was signalled, if any. Pidfile path only — never a
    /// process-table substring: `pgrep -f` matches agent prompts and misses live
    /// daemons.
    fn try_reap(&self) -> Option<i32> {
        let text = std::fs::read_to_string(self.daemon_pidfile()).ok()?;
        let pid = text.trim().parse::<i32>().ok()?;
        signal_pid(pid, libc::SIGTERM);
        if !wait_dead(pid, Duration::from_secs(2)) {
            signal_pid(pid, libc::SIGKILL);
            let _ = wait_dead(pid, Duration::from_secs(2));
        }
        Some(pid)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Best-effort only: a panicking test must not leave a resident behind,
        // and Drop itself must not panic. Runs before TempDir field-drop so the
        // pidfile is still on disk.
        let _ = self.try_reap();
    }
}

/// Send `signal` to a detached daemon we do not own as a child.
fn signal_pid(pid: i32, signal: libc::c_int) {
    // SAFETY: plain kill(2) to a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Poll until `pid` is gone (`kill(pid, 0)` → ESRCH).
fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: signal 0 probes existence without delivering a signal.
        if unsafe { libc::kill(pid, 0) } == -1 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // SAFETY: final probe.
    unsafe { libc::kill(pid, 0) == -1 }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The whole rendered answer — stdout and stderr together, since a refusal rides
/// stderr and the edge map rides stdout, and a two-arm diff must compare BOTH.
fn whole_answer(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}

// ── F3 — the cross-vault link RESOLVES, into the TARGET root ─────────────────

/// `sessions` is bound, readable, and holds `notes.md`; the link plane must
/// resolve `[[sessions:notes.md]]` INTO that root and say so — never report it
/// unresolved, and never answer with the ambient decoy of the same basename.
///
/// The ambient link in the same page is the POSITIVE CONTROL: it must keep
/// resolving, so a miss on the rooted link is attributable to the root prefix
/// and not to `mrd links` being broken.
#[test]
fn a_cross_vault_link_resolves_into_the_target_root() {
    let sb = sandbox();
    sb.page(
        "claim.md",
        "# Claim\n\nA cross-vault link: [[sessions:notes.md]]\nAn ambient link: [[local.md]]\n",
    );

    let (out, entry) = sb.edges("claim.md");

    // The CONTROL first: if this fails, nothing below is about cross-vault.
    assert_eq!(
        entry["resolved"]["local.md"],
        1,
        "the ambient control must still resolve: {}",
        stdout(&out)
    );

    // The rooted edge is reported UNDER ITS ROOT — never folded into a bare
    // path, which would read as the ambient decoy.
    assert_eq!(
        entry["resolved_rooted"]["sessions"]["notes.md"],
        1,
        "the cross-vault link resolves inside 'sessions': {}",
        stdout(&out)
    );

    assert!(
        entry["unresolved"]["sessions:notes.md"].is_null(),
        "the shipped defect: a bound, readable, present target reported \
         unresolved: {}",
        stdout(&out)
    );

    // Decoy control: the ambient root holds its own `notes.md`, so a bare
    // `notes.md` in the answer means the link plane resolved the wrong document.
    assert!(
        entry["resolved"]["notes.md"].is_null(),
        "the cross-vault link must never answer the ambient decoy: {}",
        stdout(&out)
    );
}

// ── the two-arm check — the instrument must MOVE when the world does ─────────

/// State the two worlds, run both, diff them: with `sessions` bound and unbound
/// the two answers must DIFFER, and each must be separately correct. Identical
/// bytes across the arms is the instrument being blind to the mount table.
#[test]
fn the_link_plane_distinguishes_a_bound_root_from_an_unbound_one() {
    let sb = sandbox();
    sb.page("claim.md", "# Claim\n\n[[sessions:notes.md]]\n");

    let (bound_out, bound_entry) = sb.edges("claim.md");
    let bound_answer = whole_answer(&bound_out);

    sb.unmount();
    let (unbound_out, unbound_entry) = sb.edges("claim.md");
    let unbound_answer = whole_answer(&unbound_out);

    assert_ne!(
        bound_answer, unbound_answer,
        "byte-identical answers in two different worlds means the instrument \
         is blind — this is the exact measurement that opened U21",
    );

    // Each arm, separately correct. Bound: resolved inside the root.
    assert_eq!(
        bound_entry["resolved_rooted"]["sessions"]["notes.md"], 1,
        "bound arm: {bound_answer}"
    );

    // Unbound: GREY `unmounted` (R-3). An unmounted root is outside sight, so
    // claiming absence there would be a false negative.
    assert_eq!(
        unbound_entry["refused"]["sessions:notes.md"]["color"], "grey",
        "unbound arm is grey, never red: {unbound_answer}"
    );
    assert_eq!(
        unbound_entry["refused"]["sessions:notes.md"]["reason"], "unmounted",
        "unbound arm names the unmounted class: {unbound_answer}"
    );
    assert!(
        unbound_entry["refused"]["sessions:notes.md"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("sessions")),
        "the refusal names WHICH root is missing, or it teaches nothing: \
         {unbound_answer}"
    );
}

// ── F4 — the refusal names the root, with its own reason word ────────────────

/// A cross-vault target absent inside a root the machine binds and reads is a
/// MEASURED ABSENCE: red `file-not-found`, scoped to that root. Not
/// `selector-unresolved` — that word asserts the page resolved and the selector
/// did not — and not grey, because the root is visible.
#[test]
fn an_absent_cross_vault_target_refuses_with_its_own_reason_word() {
    let sb = sandbox();
    sb.page("claim.md", "# Claim\n\n[[sessions:absent.md]]\n");

    let (out, entry) = sb.edges("claim.md");
    let answer = whole_answer(&out);
    let refusal = &entry["refused"]["sessions:absent.md"];

    assert_eq!(refusal["color"], "red", "the root is visible: {answer}");
    assert_eq!(
        refusal["reason"], "file-not-found",
        "its own reason word, never borrowing the selector plane's: {answer}"
    );
    assert_ne!(
        refusal["reason"], "selector-unresolved",
        "that word claims the PAGE resolved: {answer}"
    );

    let message = refusal["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("sessions"),
        "the refusal is SCOPED to the root that missed: {answer}"
    );
    assert!(
        message.contains("absent.md"),
        "and it names the path that is missing: {answer}"
    );

    // Q3(a) — a cross-vault dangling link is a believed mount relationship that
    // does not hold, so it rides exit 1.
    assert_eq!(
        out.status.code(),
        Some(1),
        "a cross-vault file-not-found rides exit 1: {answer}"
    );
}

/// The asymmetry: an ambient dangling link is an ordinary authoring state —
/// first-class, non-refusing, exit 0. Without it, Q3(a) could be satisfied by a
/// resolver that refuses everything.
#[test]
fn an_ambient_dangling_link_stays_first_class_and_does_not_refuse() {
    let sb = sandbox();
    sb.page("claim.md", "# Claim\n\n[[nowhere.md]]\n");

    let (out, entry) = sb.edges("claim.md");
    let answer = whole_answer(&out);

    assert_eq!(
        entry["unresolved"]["nowhere.md"], 1,
        "ambient dangling is first-class: {answer}"
    );
    assert!(
        entry["refused"]["nowhere.md"].is_null(),
        "and it is not a refusal: {answer}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "an ordinary authoring state never rides exit 1: {answer}"
    );
}

// ── GATE 3 — one address, one answer, whichever path served it ───────────────

/// `mrd links` is normally served by the resident daemon, whose warm state is
/// ONE ambient corpus: it holds no mounted corpora and cannot answer a
/// cross-root address, so the CLI degrades to in-process whenever the page may
/// carry one.
#[test]
fn a_page_carrying_a_rooted_spelling_is_never_served_by_the_daemon() {
    let sb = sandbox();
    sb.page("claim.md", "# Claim\n\n[[sessions:notes.md]]\n");

    // Warm the daemon first, on a page with no rooted spelling, so this test
    // measures the GATE and not the absence of a daemon.
    sb.page("plain.md", "# Plain\n\n[[local.md]]\n");
    let (plain_out, plain) = sb.links("plain.md");
    assert_eq!(
        plain["source"],
        "daemon",
        "the control: an ordinary page IS served warm, so a degrade below is \
         the gate firing and not a missing daemon: {}",
        whole_answer(&plain_out)
    );

    let (out, v) = sb.links("claim.md");
    assert_eq!(
        v["source"],
        "ephemeral",
        "a rooted spelling must degrade to in-process: {}",
        whole_answer(&out)
    );
}

/// The slip fixture: `Sessions:notes.md` — capital `S`, so `MountName` refuses
/// it and `Addr::parse` fails. A degrade gate built on parse-SUCCESS would let
/// the daemon serve the page and answer `unresolved` at exit 0.
#[test]
fn a_malformed_rooted_spelling_also_degrades_and_refuses() {
    let sb = sandbox();
    sb.page("claim.md", "# Claim\n\n[[Sessions:notes.md]]\n");

    let (out, v) = sb.links("claim.md");
    let answer = whole_answer(&out);
    assert_eq!(
        v["source"], "ephemeral",
        "an unparseable rooted spelling still degrades: {answer}"
    );

    let entry = v["links"]["files"]["claim.md"].clone();
    assert!(
        entry["refused"]["Sessions:notes.md"].is_object(),
        "and the address plane REFUSES it rather than reporting it unresolved \
         at exit 0: {answer}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a refusal rides exit 1: {answer}"
    );
}

// ── the acceptance half — the ambient plane is byte-unchanged ────────────────

/// An ordinary corpus of ambient links produces exactly its old edge map —
/// resolved edges resolved, dangling edges dangling, no new keys, exit 0.
/// Without it, every pin above is satisfied by a resolver that refuses
/// everything.
#[test]
fn an_ordinary_ambient_corpus_answers_exactly_as_before() {
    let sb = sandbox();
    sb.page(
        "claim.md",
        "# Claim\n\n[[local.md]]\n[[notes.md]]\n[[nowhere.md]]\n[[local.md]]\n",
    );

    let (out, entry) = sb.edges("claim.md");
    let answer = whole_answer(&out);

    assert_eq!(
        entry["resolved"]["local.md"], 2,
        "per-edge counts: {answer}"
    );
    assert_eq!(
        entry["resolved"]["notes.md"], 1,
        "the decoy IS ambient here, \
        and an ambient link to it resolves normally: {answer}"
    );
    assert_eq!(entry["unresolved"]["nowhere.md"], 1, "dangling: {answer}");
    assert!(
        entry["refused"].is_null()
            || entry["refused"]
                .as_object()
                .is_some_and(serde_json::Map::is_empty),
        "an ambient corpus mints no refusals: {answer}"
    );
    assert!(
        entry["resolved_rooted"].is_null()
            || entry["resolved_rooted"]
                .as_object()
                .is_some_and(serde_json::Map::is_empty),
        "and no rooted edges: {answer}"
    );
    assert_eq!(out.status.code(), Some(0), "exit 0: {answer}");
}

// ── the md-only teaching leg ─────────────────────────────────────────────────

/// v1 is markdown-only and the corpus the three rules search holds only `.md`,
/// so the refusal names that LIMIT: "no `media/logo.png` in that root" would be
/// true and misleading, since the file may well be there.
#[test]
fn a_non_markdown_cross_vault_target_names_the_v1_limit() {
    let sb = sandbox();
    std::fs::create_dir_all(sb.sessions.join("media")).expect("mkdir");
    std::fs::write(sb.sessions.join("media/logo.png"), b"\x89PNG").expect("png");
    sb.page("claim.md", "# Claim\n\n[[sessions:media/logo.png]]\n");

    let (out, entry) = sb.edges("claim.md");
    let answer = whole_answer(&out);
    let message = entry["refused"]["sessions:media/logo.png"]["message"]
        .as_str()
        .unwrap_or_default();

    assert!(
        message.contains("markdown-only in v1"),
        "the refusal names the limit rather than implying absence: {answer}"
    );
}
