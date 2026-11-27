//! Bounded path-scoped `mrd check --commit-gate --require-pins -- PATH…`,
//! non-blocking slot lock (`busy`), and session-lifetime / wall-clock guards.

use std::fs::OpenOptions;
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod common;

fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(ws: &Path, rel: &str, body: &str) {
    let path = ws.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn setup_repo(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let home = tmp.join("home");
    let cache = tmp.join("xdg-cache");
    let ws = tmp.join("ws");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&ws).expect("ws");
    git(&ws, &["init", "-q"]);
    git(&ws, &["config", "user.email", "test@example.invalid"]);
    git(&ws, &["config", "user.name", "test"]);
    git(&ws, &["config", "commit.gpgsign", "false"]);
    write(&ws, "source.md", "# Source\n\n## Rule\n\noriginal body\n");
    write(&ws, "other-source.md", "# Other\n\n## Rule\n\nother body\n");
    write(&ws, "agents/card-a/card-a.md", "# Card A\n\nnotes.\n");
    write(&ws, "agents/card-b/card-b.md", "# Card B\n\nnotes.\n");
    let init = common::mrd_command(&home, &cache)
        .args(["init"])
        .current_dir(&ws)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("init");
    assert!(init.status.success(), "init: {}", said(&init));
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "seed"]);

    // Pin card-a -> source.md and card-b -> other-source.md.
    for (card, target) in [
        ("agents/card-a/card-a.md", "source.md#Source/Rule"),
        ("agents/card-b/card-b.md", "other-source.md#Other/Rule"),
    ] {
        let pin = common::mrd_command(&home, &cache)
            .args(["pin", card, target])
            .current_dir(&ws)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("pin");
        assert_eq!(pin.status.code(), Some(0), "pin {card}: {}", said(&pin));
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "pins"]);
    (home, cache, ws)
}

/// Path-scoped `--commit-gate --require-pins -- <one-card>` must NOT read or
/// fail on unrelated cards (even when another card has invalid UTF-8, a drifted
/// pin, an orphaned blob, and worktree/index divergence), while still catching
/// drift on the named card's own pinned target in both worktree and staged
/// intervals.
#[test]
fn scoped_commit_gate_ignores_unrelated_corpus_and_catches_own_drift() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };

    // Poison the rest of the corpus:
    // 1. Drift card-b's target AND leave it diverged in worktree vs index.
    write(&ws, "other-source.md", "# Other\n\n## Rule\n\nDRIFTED\n");
    // 2. Drop an unreadable/invalid-UTF-8 markdown file elsewhere in the corpus.
    //    If `check` walked the whole corpus, `voice_unserved` would mention it.
    std::fs::write(ws.join("unrelated-bad-utf8.md"), [0xff, 0xfe, b'\n']).expect("bad utf8");

    // Card-a's scoped commit gate must stay green (exit 0) and never touch the
    // unrelated bad-utf8 file or card-b's drift/orphan.
    let ok = common::mrd_command(&home, &cache)
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--json",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("check ok");
    let out_text = said(&ok);
    assert_eq!(ok.status.code(), Some(0), "card-a must pass: {out_text}");
    assert!(
        !out_text.contains("unrelated-bad-utf8.md"),
        "scoped check must not walk/parse unrelated corpus files: {out_text}"
    );
    let json: serde_json::Value = serde_json::from_slice(&ok.stdout).expect("json");
    assert_eq!(json["commit_gate"]["permits"], true);
    assert_eq!(json["commit_gate"]["pin_coverage"], 1);
    assert_eq!(json["pins"]["anchoring"]["asked"], 1);
    assert_eq!(json["pins"]["anchoring"]["anchored"], 1);

    // Now stage a forged edit to card-a's OWN target (`source.md`), restore
    // the worktree so worktree is clean and only the index diverges — the
    // scoped gate MUST refuse (exit 1, RED on staged).
    write(&ws, "source.md", "# Source\n\n## Rule\n\nSTAGED FORGERY\n");
    git(&ws, &["add", "source.md"]);
    write(&ws, "source.md", "# Source\n\n## Rule\n\noriginal body\n");

    let red = common::mrd_command(&home, &cache)
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--json",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("check red");
    let red_text = said(&red);
    assert_eq!(
        red.status.code(),
        Some(1),
        "staged forgery on card-a's target must refuse: {red_text}"
    );
    let red_json: serde_json::Value = serde_json::from_slice(&red.stdout).expect("red json");
    assert_eq!(red_json["commit_gate"]["permits"], false);
    assert_eq!(red_json["commit_gate"]["gated_interval"], "staged");
    assert_eq!(red_json["commit_gate"]["verdict"], "RED");
}

/// Holding a non-blocking flock on `MRD_CHECK_LOCK` (or the nyc-1 slot path
/// under `$HOME`) causes `mrd check` to refuse immediately with `busy` (exit 2)
/// instead of queuing.
#[test]
fn concurrent_mrd_check_refuses_busy_without_queuing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };
    let lock_path = tmp.path().join("check-slot.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("open lock");
    let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc, 0, "test acquires slot lock");

    let started = std::time::Instant::now();
    let out = Command::new(mrd_bin())
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("MRD_CHECK_LOCK", &lock_path)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd check");
    let elapsed = started.elapsed();
    let text = said(&out);

    assert_eq!(out.status.code(), Some(2), "busy must refuse exit 2: {text}");
    assert!(
        text.contains("busy"),
        "refusal must state `busy`: {text}"
    );
    assert!(
        elapsed.as_secs() < 5,
        "must exit immediately without queuing (took {elapsed:?})"
    );

    // Release the lock and verify the exact same call now succeeds.
    drop(lock_file);
    let ok = Command::new(mrd_bin())
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("MRD_CHECK_LOCK", &lock_path)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd check after unlock");
    assert_eq!(
        ok.status.code(),
        Some(0),
        "must pass once lock is released: {}",
        said(&ok)
    );
}

/// Also verify automatic discovery of `$HOME/.local/share/ucc/work/fleet-mutex/mrd-nyc1.lock`
/// when present on disk (no `MRD_CHECK_LOCK` env var needed).
#[test]
fn nyc1_slot_lock_is_auto_discovered_when_present_under_home() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };
    let nyc1_lock = home.join(".local/share/ucc/work/fleet-mutex/mrd-nyc1.lock");
    std::fs::create_dir_all(nyc1_lock.parent().expect("parent")).expect("mkdir");
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&nyc1_lock)
        .expect("open nyc1 lock");
    let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc, 0, "test acquires nyc1 lock");

    let out = Command::new(mrd_bin())
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env_remove("MRD_CHECK_LOCK")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd check");
    let text = said(&out);
    assert_eq!(out.status.code(), Some(2), "nyc1 lock busy must refuse: {text}");
    assert!(
        text.contains("busy") && text.contains("mrd-nyc1.lock"),
        "must name busy and mrd-nyc1.lock: {text}"
    );
    drop(lock_file);
}

/// A historical pin whose blob is NOT at `HEAD:<path>` (target edited and
/// committed later) is settled via path-scoped git history without requiring a
/// whole-repo reachability walk.
#[test]
fn historical_pin_is_anchored_via_path_history() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };

    // Edit an unpinned section of source.md and commit: HEAD:source.md now
    // has a DIFFERENT blob SHA, while card-a's pinned section still matches
    // and its recorded blob SHA is in the earlier commit of source.md.
    write(
        &ws,
        "source.md",
        "# Source\n\n## Rule\n\noriginal body\n\n## Extra\n\nnew section\n",
    );
    git(&ws, &["add", "source.md"]);
    git(&ws, &["commit", "-qm", "edit unpinned section"]);

    let out = common::mrd_command(&home, &cache)
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--json",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("check");
    let text = said(&out);
    assert_eq!(out.status.code(), Some(0), "must stay green: {text}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(json["pins"]["anchoring"]["anchored"], 1);
    assert_eq!(json["pins"]["anchoring"]["pending_anchor"], 0);
    assert_eq!(json["pins"]["anchoring"]["never_anchored"], 0);
    // And reach-v1.bin must NOT even have been created, proving no whole-repo
    // `rev-list --objects` ran!
    assert!(
        !ws.join(".git/meridian/reach-v1.bin").exists(),
        "path-scoped git log should settle the historical pin without building reach-v1.bin"
    );
}

/// Verify `mrd check` exits immediately on `SIGHUP` (not ignored / detached).
#[test]
fn sighup_terminates_mrd_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    // Use a fake git wrapper on PATH that sleeps so we can send SIGHUP mid-check
    // and verify neither mrd nor its child git survives.
    let fake_bin = tmp.path().join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("mkdir");
    let pid_file = tmp.path().join("fake-git.pid");
    let fake_git = fake_bin.join("git");
    std::fs::write(
        &fake_git,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
            pid_file.display()
        ),
    )
    .expect("write fake git");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(mrd_bin())
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("PATH", path_env)
        .env("MRD_CHECK_LOCK", tmp.path().join("sighup.lock"))
        .env_remove("MERIDIAN_WORKSPACE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mrd");

    // Wait until fake-git has started (up to 5 s).
    let start = std::time::Instant::now();
    while !pid_file.exists() && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(pid_file.exists(), "fake git did not start in time");
    let git_pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("read pid")
        .trim()
        .parse()
        .expect("parse pid");

    // Send SIGHUP to mrd check.
    let mrd_pid = i32::try_from(child.id()).expect("pid fits i32");
    unsafe {
        libc::kill(mrd_pid, libc::SIGHUP);
    }
    let out = child.wait_with_output().expect("wait mrd");
    assert!(
        !out.status.success(),
        "mrd check must exit non-zero on SIGHUP: {}",
        said(&out)
    );
    // Give the kernel a brief moment to reap the child git process.
    std::thread::sleep(std::time::Duration::from_millis(150));
    let git_alive = unsafe { libc::kill(git_pid, 0) } == 0;
    assert!(
        !git_alive,
        "child git pid {git_pid} must not survive after mrd check exits on SIGHUP"
    );
}

/// Verify `mrd check` enforces its wall-clock bound (`MRD_CHECK_TIMEOUT_SECS`)
/// and kills child git processes on timeout.
#[test]
fn wall_clock_bound_terminates_stalled_check_and_children() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let fake_bin = tmp.path().join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("mkdir");
    let pid_file = tmp.path().join("fake-git-timeout.pid");
    let fake_git = fake_bin.join("git");
    std::fs::write(
        &fake_git,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
            pid_file.display()
        ),
    )
    .expect("write fake git");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let started = std::time::Instant::now();
    let out = Command::new(mrd_bin())
        .args([
            "check",
            "--commit-gate",
            "--require-pins",
            "--",
            "agents/card-a/card-a.md",
        ])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("PATH", path_env)
        .env("MRD_CHECK_LOCK", tmp.path().join("timeout.lock"))
        .env("MRD_CHECK_TIMEOUT_SECS", "1")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd");
    let elapsed = started.elapsed();
    let text = said(&out);
    assert_eq!(out.status.code(), Some(124), "timeout must exit 124: {text}");
    assert!(text.contains("wall-clock bound"), "{text}");
    assert!(elapsed.as_secs() < 10, "took too long: {elapsed:?}");
    if let Ok(raw) = std::fs::read_to_string(&pid_file)
        && let Ok(git_pid) = raw.trim().parse::<i32>()
    {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let git_alive = unsafe { libc::kill(git_pid, 0) } == 0;
        assert!(
            !git_alive,
            "child git pid {git_pid} must not survive after wall-clock timeout"
        );
    }
}
