//! U6a gates — bash process supervision (#21/S3): scratch cwd, own process
//! group, wall-clock timeout → group SIGKILL, step-end background-child
//! reaping, env hygiene, and the shim-fd capture.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::{Duration, Instant};

use run::exec::{self, DEFAULT_TIMEOUT, ExecSpec, ExecStatus};
use run::shim::{self, ShimError};

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
        timeout: Duration::from_secs(30),
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
    assert!(r.shim.bytes.is_empty());
}

#[test]
fn the_block_runs_in_the_scratch_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&spec_in(&tmp, "pwd", &env)).unwrap();
    let reported = String::from_utf8(r.stdout).unwrap();
    assert_eq!(
        std::fs::canonicalize(reported.trim()).unwrap(),
        std::fs::canonicalize(tmp.path()).unwrap()
    );
}

#[test]
fn the_child_env_is_cleared_to_the_declared_set() {
    // The parent's HOME must NOT leak; the declared key must arrive.
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::from([("MRD_U6A_DECLARED".to_owned(), "yes".to_owned())]);
    let r = exec::exec(&spec_in(
        &tmp,
        r#"printf '%s:%s' "${HOME:-unset}" "${MRD_U6A_DECLARED:-unset}""#,
        &env,
    ))
    .unwrap();
    assert_eq!(r.stdout, b"unset:yes");
}

#[test]
fn the_shim_fd_captures_a_framed_stream() {
    // The documented emitter idiom: LC_ALL=C makes ${#p} count BYTES, and
    // $MD_EFFECT_FD names the fd.
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let src = r#"
p='{"op":"md.set_field","field":"status","value":"done"}'
printf '%s:%s\n' "${#p}" "$p" >&"$MD_EFFECT_FD"
printf 'end:1\n' >&3
"#;
    let r = exec::exec(&spec_in(&tmp, src, &env)).unwrap();
    assert!(r.status.success());
    let descriptors = shim::parse(&r.shim).unwrap();
    assert_eq!(descriptors.len(), 1);
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
        timeout: Duration::from_millis(300),
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
    // S3: the child spawns a background writer holding stdout AND the shim
    // fd, then exits. Without the group SIGKILL the readers would wait 15s
    // for the inherited pipe fds; with it the step ends now and the writer
    // never lands its post-step write.
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let started = Instant::now();
    let r = exec::exec(&spec_in(
        &tmp,
        "( sleep 15; echo leaked > leak.txt; echo late ) & exit 0",
        &env,
    ))
    .unwrap();
    assert!(r.status.success());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "step end must not wait for the background child: {:?}",
        started.elapsed()
    );
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
fn a_shim_flood_past_the_cap_flags_overflow_and_fails_closed() {
    // 9 MB > the 8 MiB store cap; the reader keeps draining (the child is
    // never deadlocked) but the stream fails closed at parse.
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let r = exec::exec(&spec_in(&tmp, "head -c 9000000 /dev/zero >&3", &env)).unwrap();
    assert!(r.status.success());
    assert!(r.shim.overflowed);
    assert!(matches!(shim::parse(&r.shim), Err(ShimError::Overflow)));
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
fn configured_timeout_reads_meridian_toml_and_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    // No file → the default.
    assert_eq!(
        exec::configured_timeout(tmp.path()).unwrap(),
        DEFAULT_TIMEOUT
    );
    // A file without the key → the default.
    std::fs::write(tmp.path().join(".meridian.toml"), "[run.caps]\n").unwrap();
    assert_eq!(
        exec::configured_timeout(tmp.path()).unwrap(),
        DEFAULT_TIMEOUT
    );
    // The key → the configured ceiling.
    std::fs::write(
        tmp.path().join(".meridian.toml"),
        "[run]\ntimeout_secs = 7\n",
    )
    .unwrap();
    assert_eq!(
        exec::configured_timeout(tmp.path()).unwrap(),
        Duration::from_secs(7)
    );
    // Malformed → loud, never a silent default.
    std::fs::write(
        tmp.path().join(".meridian.toml"),
        "[run]\ntimeout_secs = \"fast\"\n",
    )
    .unwrap();
    assert!(exec::configured_timeout(tmp.path()).is_err());
}
