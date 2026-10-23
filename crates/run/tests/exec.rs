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
        program: exec::Program::Inline(source),
        args: &[],
        env,
        scratch: scratch.path(),
        project_root: scratch.path(),
        timeout: Duration::from_secs(30),
        step_cwd: None,
        interpreter: exec::BASH,
        stdin: None,
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
        program: exec::Program::Inline(r#"printf '%s' "${MERIDIAN_PROJECT_ROOT:-unset}""#),
        args: &[],
        env: &env,
        scratch: scratch.path(),
        project_root: project.path(),
        timeout: Duration::from_secs(30),
        step_cwd: None,
        interpreter: exec::BASH,
        stdin: None,
    })
    .unwrap();
    assert_eq!(r.stdout, project.path().as_os_str().as_encoded_bytes());
}

/// Run-env law (run must not strip the daemon's
/// environment) — the inversion of the retired `env_clear` law: the
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
        program: exec::Program::Inline("sleep 30"),
        args: &[],
        env: &env,
        scratch: tmp.path(),
        project_root: tmp.path(),
        timeout: Duration::from_millis(300),
        step_cwd: None,
        interpreter: exec::BASH,
        stdin: None,
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

/// The child leads its own process group — the property `kill_group`
/// (`kill(-pid, SIGKILL)`) reaches the step's whole tree by.
#[test]
fn the_child_leads_its_own_process_group() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&spec_in(
        &tmp,
        r#"printf '%s %s' "$$" "$(ps -o pgid= -p $$)""#,
        &env,
    ))
    .unwrap();
    assert!(
        r.status.success(),
        "{:?}: {}",
        r.status,
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8(r.stdout).unwrap();
    let ids: Vec<&str> = out.split_whitespace().collect();
    assert_eq!(ids.len(), 2, "pid and pgid: {out:?}");
    assert_eq!(ids[0], ids[1], "the child is its own group leader");
}

/// The spawn takes `posix_spawn`, never `fork()`: its CPU cost does not grow
/// with the parent's address space. Self-calibrating — the same 20 spawns
/// through a forced-fork control (a no-op `pre_exec`) set the bar, and a plain
/// `Command` shows whether this libc gives std a `posix_spawn` path at all.
/// Linux-only: `CLOCK_THREAD_CPUTIME_ID` charges the spawning thread alone, so
/// the suite's other threads add no noise.
#[cfg(target_os = "linux")]
#[test]
fn the_spawn_cost_does_not_grow_with_the_parents_heap() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    const HEAP: usize = 512 << 20;
    const SPAWNS: u32 = 20;

    fn thread_cpu() -> Duration {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes one `timespec` through a valid
        // pointer; `CLOCK_THREAD_CPUTIME_ID` is a valid clock on Linux.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut ts) };
        assert_eq!(rc, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID)");
        Duration::new(
            u64::try_from(ts.tv_sec).unwrap(),
            u32::try_from(ts.tv_nsec).unwrap(),
        )
    }
    fn cpu_of(mut spawn: impl FnMut()) -> Duration {
        let before = thread_cpu();
        for _ in 0..SPAWNS {
            spawn();
        }
        thread_cpu()
            .checked_sub(before)
            .expect("the thread CPU clock is monotonic")
    }
    fn shaped() -> Command {
        let mut cmd = Command::new(exec::BASH);
        cmd.arg("-c")
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    }

    // Touched, so every page is mapped and a fork has its page tables to copy.
    let mut heap = vec![0u8; HEAP];
    for page in heap.chunks_mut(4096) {
        page[0] = 1;
    }
    std::hint::black_box(&mut heap);

    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let production = cpu_of(|| {
        let r = exec::exec(&spec_in(&tmp, "true", &env)).unwrap();
        assert!(r.status.success(), "{:?}", r.status);
    });
    let plain = cpu_of(|| assert!(shaped().status().unwrap().success()));
    let forked = cpu_of(|| {
        let mut cmd = shaped();
        // SAFETY: the closure does nothing; its presence alone puts std on
        // `fork()`, which is the control this gate measures against.
        unsafe {
            cmd.pre_exec(|| Ok(()));
        }
        assert!(cmd.status().unwrap().success());
    });
    drop(heap);

    eprintln!("spawn CPU x{SPAWNS}: production {production:?}, plain {plain:?}, forked {forked:?}");
    if plain * 4 > forked {
        eprintln!("spawn path: fork on this libc (plain std spawn costs as much as a fork); skip");
        return;
    }
    assert!(
        production * 4 <= forked,
        "the production spawn costs like a fork: {production:?} against the forked control's \
         {forked:?} (plain std spawn {plain:?})"
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
