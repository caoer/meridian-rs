//! 0025 socket law — version mismatch = refuse (session ruling, 2026-08-12).
//!
//! Measured defect (receipt `839fdb38`): a caller got an answer computed by an
//! engine it did not build, with no error and no way to tell — a foreign
//! resident daemon adopted a brand-new workspace and served `read` with wording
//! that does not exist in the caller's tree. One cache root ⇒ one socket ⇒ one
//! daemon, whatever binary bound it first; no client read `hello.identity`.
//!
//! The law under test: a LOCAL client on its own cache root compares
//! `hello.identity.build` (already on the hello frame it receives) against its
//! own baked `MRD_BUILD_SHA`, whole token. Equal → serve. Anything else —
//! a different token, or no identity published — refuses, naming both builds,
//! the reason, and fitted suggestions (ZT ruling 2026-08-14: a teaching
//! explains WHY and offers suggestions by applicability, never one demanded
//! command). Zero extra round trips: the comparison is in-memory on a frame
//! the single dial already parsed.
//!
//! Harness: an in-process `RunningServer` bound at the sandbox's DERIVED socket
//! path (`$XDG_CACHE_HOME/meridian/registry/daemon.sock`), with `build_sha`
//! injected per case — foreign, equal (the test and the binary carry the same
//! crate stamp), or absent.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use registry::{Config, RunningServer};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The identity this test's own compilation carries — the same crate stamp the
/// `mrd` binary under test bakes, because one `build.rs` run feeds every target
/// of the crate in one cargo invocation.
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

/// A build token no real tree produces — the injected foreign identity.
const FOREIGN_BUILD: &str = "feedfacefeedfacefeedfacefeedfacefeedface";

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

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
    /// The cache root the client derives from this sandbox's env.
    fn cache_root(&self) -> PathBuf {
        self.cache_home.join("meridian")
    }

    /// Start an in-process daemon at the DERIVED socket path — exactly where
    /// the client under test will dial — publishing `build_sha` as its
    /// identity (`None` publishes none).
    #[allow(clippy::duration_suboptimal_units)]
    fn daemon(&self, build_sha: Option<&str>) -> RunningServer {
        let forever = Duration::from_secs(365 * 24 * 60 * 60);
        let mut config = Config::for_cache_root(self.cache_root());
        config.idle_threshold = forever;
        config.reap_interval = forever;
        config.prewarm_interval = forever;
        config.prewarm_quiet_max = forever;
        config.idle_exit = None;
        config.build_sha = build_sha.map(ToOwned::to_owned);
        RunningServer::start(config).expect("in-process daemon binds the derived socket")
    }

    /// An anchored workspace holding the fixture doc.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            // No auto-spawn may hide a test bug: a wrong socket path must fail
            // loud (degrade voice), not quietly start a real daemon.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Mismatch → refuse, and the refusal speaks BOTH identities and the remedy.
/// Nothing is served: the poison of `839fdb38` was a correct-looking answer.
#[test]
fn read_refuses_on_foreign_identity_and_names_both_builds() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon(Some(FOREIGN_BUILD));

    let out = sb.run(&ws, &["read", "doc.md"]);
    let err = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a build-skewed daemon is an environment refusal (exit 2), got {:?}\nstdout: {}\nstderr: {err}",
        out.status.code(),
        stdout_of(&out),
    );
    assert!(
        stdout_of(&out).is_empty(),
        "nothing is served across builds — stdout must be empty, got: {}",
        stdout_of(&out)
    );
    assert!(
        err.contains(FOREIGN_BUILD),
        "the refusal names the daemon's build: {err}"
    );
    assert!(
        err.contains(OWN_BUILD),
        "the refusal names this client's build: {err}"
    );
    assert!(err.contains("SKEW"), "the one skew voice: {err}");
    assert!(
        err.to_lowercase().contains("restart"),
        "the refusal carries a restart suggestion: {err}"
    );
}

/// ZT ruling (2026-08-14): a refusal teaching explains WHY and offers fitted
/// suggestions — never a single demanded command, because one imperative does
/// not apply to all callers (a caller who does not own the resident, or is on
/// a foreign cache root, must not kill it; a managed install owns its own
/// restart). Teachings address users; suggestions carry their applicability
/// ("when you own …"), never authority ("only the owner may …").
#[test]
fn teaching_explains_why_and_offers_fitted_suggestions_never_one_imperative() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon(Some(FOREIGN_BUILD));

    let out = sb.run(&ws, &["read", "doc.md"]);
    let err = stderr_of(&out);

    // Reason first: the teaching explains the keying that makes builds skew
    // before it suggests anything.
    assert!(
        err.contains("Why:"),
        "the teaching leads with the reason: {err}"
    );
    assert!(
        err.contains("cache root"),
        "the reason names the socket's cache-root keying: {err}"
    );
    // Fitted suggestions, each opening with its applicability condition.
    assert!(
        err.contains("when you own"),
        "own-the-resident case, applicability phrasing: {err}"
    );
    assert!(
        err.contains("daemon.pid"),
        "the owner's restart suggestion still names the pidfile: {err}"
    );
    assert!(
        err.to_lowercase().contains("rerun"),
        "managed-install case suggests rerunning the step that owns the restart duty: {err}"
    );
    assert!(
        err.to_lowercase().contains("report"),
        "not-the-owner case suggests reporting to the daemon's operator: {err}"
    );
    // The old demand register is gone.
    assert!(
        !err.contains("Restart the daemon and rerun"),
        "no single demanded command: {err}"
    );
}

/// Equality → serve, warm. The check must not tax the healthy path.
#[test]
fn read_serves_on_equal_identity() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon(Some(OWN_BUILD));

    let out = sb.run(&ws, &["read", "doc.md"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "an in-step daemon serves\nstderr: {}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("Alpha"),
        "the answer is the corpus content: {}",
        stdout_of(&out)
    );
    assert!(
        !stderr_of(&out).contains("ephemeral"),
        "the warm path served — no degrade voice: {}",
        stderr_of(&out)
    );
}

/// A local daemon publishing NO identity is the stale-resident shape the law
/// exists for (a build predating the identity token). Local scope: refuse.
/// `hello.identity` stays optional ON THE WIRE — that binds remote peers, not
/// the caller's own cache root.
#[test]
fn a_daemon_publishing_no_identity_is_refused_on_the_local_socket() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon(None);

    let out = sb.run(&ws, &["read", "doc.md"]);
    let err = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(2),
        "no identity on the local socket refuses, got {:?}\nstdout: {}\nstderr: {err}",
        out.status.code(),
        stdout_of(&out),
    );
    assert!(err.contains("SKEW"), "the one skew voice: {err}");
    assert!(
        err.contains(OWN_BUILD),
        "the refusal names this client's build: {err}"
    );
    assert!(
        err.contains("no identity") || err.contains("published no"),
        "the refusal names the absence rather than inventing a token: {err}"
    );
}

/// The script CLI lane refuses on skew at connect (`SocketDoor`), before any
/// entry work: no fingerprint, no eval, no splice. Pinned e2e beside the
/// read/links lanes because the § A.7 in-process serve rides the same door —
/// this lane's connect-time law must survive that change byte-for-byte.
#[test]
fn script_refuses_on_foreign_identity_before_any_entry_work() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon(Some(FOREIGN_BUILD));

    let out = Command::new(mrd_bin())
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .env_remove("MERIDIAN_WORKSPACE")
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .args(["script", "--json"])
        .current_dir(&ws)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            common::feed_stdin(&mut child, b"t = read(\"doc.md\")\n");
            child.wait_with_output()
        })
        .expect("spawn mrd script");
    let err = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a build-skewed daemon refuses the script lane (exit 2), got {:?}\nstdout: {}\nstderr: {err}",
        out.status.code(),
        stdout_of(&out),
    );
    assert!(
        stdout_of(&out).is_empty(),
        "exit 2 + empty stdout is the pre-entry controlled exit: {}",
        stdout_of(&out)
    );
    assert!(err.contains("SKEW"), "the one skew voice: {err}");
    assert!(
        err.contains(FOREIGN_BUILD) && err.contains(OWN_BUILD),
        "both identities named: {err}"
    );
}

/// The links lane degrades on daemon-path FAILURE; skew is not a failure of
/// the path but a refusal of the serve — it must surface loud, never melt
/// into the in-process degrade (which would hide the stale resident forever).
#[test]
fn links_refuses_on_skew_rather_than_degrading() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon(Some(FOREIGN_BUILD));

    let out = sb.run(&ws, &["links"]);
    let err = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(2),
        "links refuses on skew, got {:?}\nstdout: {}\nstderr: {err}",
        out.status.code(),
        stdout_of(&out),
    );
    assert!(err.contains("SKEW"), "the one skew voice: {err}");
    assert!(
        !err.contains("source: ephemeral"),
        "skew is a refusal, not a degrade: {err}"
    );
}
