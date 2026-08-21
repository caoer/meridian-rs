//! U6a gates — bash process supervision (#21/S3): the invocation cwd (U16),
//! own process group, wall-clock timeout → group SIGKILL, step-end
//! background-child reaping, and env passthrough + overlay.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::{Duration, Instant};

use run::exec::{self, DEFAULT_TIMEOUT, ExecSpec, ExecStatus};

fn spec_in<'a>(
    scratch: &'a tempfile::TempDir,
    source: &'a str,
    env: &'a BTreeMap<String, String>,
) -> ExecSpec<'a> {
    ExecSpec {
        source,
        args: &[],
        env,
        scratch: scratch.path(),
        project_root: scratch.path(),
        timeout: Duration::from_secs(30),
        step_cwd: None,
    }
}

#[test]
fn exit_code_and_stdout_and_stderr_are_captured() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&spec_in(&tmp, "printf out; printf err >&2; exit 3", &env)).unwrap();
    assert_eq!(r.status, ExecStatus::Exited { code: 3 });
    assert!(!r.status.success());
    assert_eq!(r.stdout, b"out");
    assert_eq!(r.stderr, b"err");
}

/// U16: the block runs WHERE `mrd` RUNS ("DO NOT CHANGE THE RUNNING PATH") —
/// never chdir'd into scratch.
#[test]
fn the_block_runs_in_the_invocation_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&spec_in(&tmp, "pwd", &env)).unwrap();
    let reported = String::from_utf8(r.stdout).unwrap();
    assert_eq!(
        std::fs::canonicalize(reported.trim()).unwrap(),
        std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap()
    );
    assert_ne!(
        std::fs::canonicalize(reported.trim()).unwrap(),
        std::fs::canonicalize(tmp.path()).unwrap(),
        "scratch is the artifact location, never the cwd"
    );
}

/// P6: the project root reaches the step as `$MERIDIAN_PROJECT_ROOT` — the
/// convenience that replaces the relocation. It is a path, not an authority:
/// a write under it still refuses convergence (gated in `dispatch_bash.rs`).
#[test]
fn the_project_root_is_exported_to_the_step() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&ExecSpec {
        source: r#"printf '%s' "${MERIDIAN_PROJECT_ROOT:-unset}""#,
        args: &[],
        env: &env,
        scratch: scratch.path(),
        project_root: project.path(),
        timeout: Duration::from_secs(30),
        step_cwd: None,
    })
    .unwrap();
    assert_eq!(r.stdout, project.path().as_os_str().as_encoded_bytes());
}

/// Run-env ruling (2026-08-16, ZT: "run must not strip the daemon's
/// environment") — the inversion of the retired `env_clear` law: the
/// parent's env passes through undeclared, and the declared key arrives too.
#[test]
fn the_daemon_env_passes_through_to_the_child() {
    let tmp = tempfile::tempdir().unwrap();
    let home = std::env::var("HOME").expect("the test runner env carries HOME");
    let env = BTreeMap::from([("MRD_U6A_DECLARED".to_owned(), "yes".to_owned())]);
    let r = exec::exec(&spec_in(
        &tmp,
        r#"printf '%s:%s' "${HOME:-unset}" "${MRD_U6A_DECLARED:-unset}""#,
        &env,
    ))
    .unwrap();
    assert_eq!(r.stdout, format!("{home}:yes").into_bytes());
}

/// The declared contract env OVERLAYS the inherited environment — a declared
/// pair shadows the daemon's value for the same key.
#[test]
fn declared_env_overlays_the_inherited_value() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::from([("HOME".to_owned(), "/declared/home".to_owned())]);
    let r = exec::exec(&spec_in(&tmp, r#"printf '%s' "$HOME""#, &env)).unwrap();
    assert_eq!(r.stdout, b"/declared/home");
}

#[test]
fn timeout_sigkills_the_group_and_is_a_distinct_state() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let started = Instant::now();
    let r = exec::exec(&ExecSpec {
        source: "sleep 30",
        args: &[],
        env: &env,
        scratch: tmp.path(),
        project_root: tmp.path(),
        timeout: Duration::from_millis(300),
        step_cwd: None,
    })
    .unwrap();
    assert!(
        matches!(r.status, ExecStatus::TimedOut { .. }),
        "{:?}",
        r.status
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the ceiling must cut the wait: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_background_child_is_reaped_at_step_end() {
    // S3: the child spawns a background writer holding stdout, then exits.
    // Without the group SIGKILL the readers would wait 15s for the inherited
    // pipe fds; with it the step ends now and the writer
    // never lands its post-step write. Correctness is the EVENT (no leak file
    // after step end); the wall-clock budget lives in `exec_walltime.rs`.
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    // The leak path is ABSOLUTE: since U16 the step runs in the invocation
    // cwd, so a relative write would land beside the test binary's cwd and the
    // assertion below would pass without proving anything.
    let src = format!(
        "( sleep 15; echo leaked > '{}/leak.txt'; echo late ) & exit 0",
        tmp.path().display()
    );
    let r = exec::exec(&spec_in(&tmp, &src, &env)).unwrap();
    assert!(r.status.success());
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !tmp.path().join("leak.txt").exists(),
        "the background child must die with the step"
    );
}

#[test]
fn an_external_signal_is_the_signaled_state() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&spec_in(&tmp, "kill -KILL $$", &env)).unwrap();
    assert_eq!(r.status, ExecStatus::Signaled { signal: 9 });
}

#[test]
fn exec_streaming_hands_stdout_to_the_consumer() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let (r, streamed) = exec::exec_streaming(&spec_in(&tmp, "printf live", &env), |mut out| {
        let mut buf = String::new();
        out.read_to_string(&mut buf).unwrap();
        buf
    })
    .unwrap();
    assert_eq!(streamed, "live");
    assert!(r.stdout.is_empty(), "the consumer owns the bytes");
}

#[test]
fn configured_timeout_reads_the_root_declaration_and_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let declare = |body: &str| {
        std::fs::write(
            tmp.path().join("MERIDIAN.md"),
            format!("---\ntype: meridian-root\nversion: 1\nname: r\n{body}---\n"),
        )
        .unwrap();
    };

    // No declaring root at all → the default.
    assert_eq!(exec::configured_timeout(None).unwrap(), DEFAULT_TIMEOUT);
    // A root with no declaration → the default.
    assert_eq!(
        exec::configured_timeout(Some(tmp.path())).unwrap(),
        DEFAULT_TIMEOUT
    );
    // A declaration without the key → the default.
    declare("run.caps.fix-*: md.edit\n");
    assert_eq!(
        exec::configured_timeout(Some(tmp.path())).unwrap(),
        DEFAULT_TIMEOUT
    );
    // The key → the configured ceiling.
    declare("run.timeout_secs: 7\n");
    assert_eq!(
        exec::configured_timeout(Some(tmp.path())).unwrap(),
        Duration::from_secs(7)
    );
    // Malformed → loud, never a silent default.
    declare("run.timeout_secs: fast\n");
    assert!(exec::configured_timeout(Some(tmp.path())).is_err());
    // Zero is malformed too: a zero ceiling would kill every step instantly.
    declare("run.timeout_secs: 0\n");
    assert!(exec::configured_timeout(Some(tmp.path())).is_err());

    // A declaration that does not read as one → loud, never a silent default:
    // the same posture the convention table takes.
    std::fs::write(
        tmp.path().join("MERIDIAN.md"),
        "---\ntype: meridian-config\nversion: 1\n---\n",
    )
    .unwrap();
    assert!(exec::configured_timeout(Some(tmp.path())).is_err());
}
