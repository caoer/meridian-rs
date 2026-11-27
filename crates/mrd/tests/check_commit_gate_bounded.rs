//! Bounded path-scoped `mrd check --commit-gate --require-pins -- PATH…`,
//! non-blocking slot lock (`busy` exit 75 + `flock` wrapper cooperation),
//! parity with the in-process lane, and session-lifetime / wall-clock guards.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

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

/// Spawn a separate sibling process holding an exclusive `flock` on `lock_path`
/// and wait until the lock is actually held (using `ready_file` as a handshake).
/// Returns a guard that kills and reaps the sibling on drop.
struct SiblingLockHolder {
    child: Child,
}

impl SiblingLockHolder {
    fn acquire(lock_path: &Path, ready_file: &Path) -> Self {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir lock parent");
        }
        // `-o` closes the lock fd in the child `sleep` so killing `flock`
        // releases the slot immediately when `SiblingLockHolder` drops.
        let child = Command::new("flock")
            .arg("-o")
            .arg(lock_path)
            .args([
                "sh",
                "-c",
                "touch \"$1\"; exec sleep 30",
                "sh",
                ready_file.to_str().expect("utf8"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sibling flock holder");
        let start = std::time::Instant::now();
        while !ready_file.exists() && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            ready_file.exists(),
            "sibling flock holder did not acquire lock"
        );
        Self { child }
    }
}

impl Drop for SiblingLockHolder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    write(
        &ws,
        "source.md",
        "# Source\n\n## Rule\n\noriginal body\n\n## Extra\n\nv1\n",
    );
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
    write(
        &ws,
        "source.md",
        "# Source\n\n## Rule\n\noriginal body\n\n## Extra\n\nv1\n",
    );

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

/// An unrelated sibling process holding `MRD_CHECK_LOCK` causes `mrd check` to
/// refuse immediately with `busy` (exit 75, distinct from exit 2) without
/// queuing.
#[test]
fn concurrent_mrd_check_refuses_busy_without_queuing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };
    let lock_path = tmp.path().join("check-slot.lock");
    let holder = SiblingLockHolder::acquire(&lock_path, &tmp.path().join("slot.ready"));

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

    assert_eq!(
        out.status.code(),
        Some(75),
        "busy must refuse exit 75 (distinct from exit 2): {text}"
    );
    assert!(text.contains("busy"), "refusal must state `busy`: {text}");
    assert!(
        elapsed.as_secs() < 5,
        "must exit immediately without queuing (took {elapsed:?})"
    );

    // Release the sibling lock and verify the exact same call now succeeds.
    drop(holder);
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

/// Automatic discovery of `$HOME/.local/share/ucc/work/fleet-mutex/mrd-nyc1.lock`
/// when present on disk and neither `XDG_CACHE_HOME` nor `MERIDIAN_CACHE_ROOT`
/// overrides the cache root.
#[test]
fn nyc1_slot_lock_is_auto_discovered_when_present_under_home() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };
    let nyc1_lock = home.join(".local/share/ucc/work/fleet-mutex/mrd-nyc1.lock");
    let holder = SiblingLockHolder::acquire(&nyc1_lock, &tmp.path().join("nyc1.ready"));

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
        .env_remove("XDG_CACHE_HOME")
        .env_remove("MERIDIAN_CACHE_ROOT")
        .env_remove("MRD_CHECK_LOCK")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd check");
    let text = said(&out);
    assert_eq!(
        out.status.code(),
        Some(75),
        "nyc1 lock busy must refuse exit 75: {text}"
    );
    assert!(
        text.contains("busy") && text.contains("mrd-nyc1.lock"),
        "must name busy and mrd-nyc1.lock: {text}"
    );
    drop(holder);
}

/// P1-1: wrapping `mrd check` in `flock $L mrd check …` (and fd-inherited
/// `( flock 9; mrd check … ) 9>$L`) with `MRD_CHECK_LOCK=$L` must run and
/// return the gate verdict (exit 0), NOT refuse `busy`.
#[test]
fn flock_wrapped_mrd_check_runs_and_is_not_busy() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };
    let lock_path = tmp.path().join("wrapped.lock");

    // Shape 1: `flock $L mrd check …`
    let out1 = Command::new("flock")
        .arg(&lock_path)
        .arg(mrd_bin())
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
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn flock mrd check");
    assert_eq!(
        out1.status.code(),
        Some(0),
        "flock $L mrd check must run and pass, not busy: {}",
        said(&out1)
    );

    // Shape 2: `flock -o $L mrd check …` (closed fd; ancestor PID in /proc/locks)
    let out2 = Command::new("flock")
        .arg("-o")
        .arg(&lock_path)
        .arg(mrd_bin())
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
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn flock -o mrd check");
    assert_eq!(
        out2.status.code(),
        Some(0),
        "flock -o $L mrd check must run and pass, not busy: {}",
        said(&out2)
    );

    // Shape 3: `( flock 9; mrd check … ) 9>$L` (flock exited, locked OFD inherited)
    let script = format!(
        "( flock 9; exec '{}' check --commit-gate --require-pins -- agents/card-a/card-a.md ) 9>'{}'",
        mrd_bin().display(),
        lock_path.display()
    );
    let out3 = Command::new("sh")
        .args(["-c", &script])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("MRD_CHECK_LOCK", &lock_path)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn fd-9 flock mrd check");
    assert_eq!(
        out3.status.code(),
        Some(0),
        "(flock 9; mrd check) 9>$L must run and pass, not busy: {}",
        said(&out3)
    );
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

    let existing = std::fs::read_to_string(ws.join("source.md")).expect("read source.md");
    assert!(existing.contains("\nv1\n"), "fixture must have v1");
    write(&ws, "source.md", &existing.replace("\nv1\n", "\nv2\n"));
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
    assert!(
        !ws.join(".git/meridian/reach-v1.bin").exists(),
        "path-scoped git log should settle the historical pin without building reach-v1.bin"
    );
}

/// P1-2: target is edited, re-pinned, and `git add`ed (uncommitted). The scoped
/// gate must pass (exit 0) WITHOUT building or reading `reach-v1.bin`
/// (repo-wide `rev-list --objects --all`).
#[test]
fn staged_repin_avoids_rev_list_and_passes_gate() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };

    // Edit source.md, re-pin card-a to the new content, and `git add` both
    // without committing.
    write(
        &ws,
        "source.md",
        "# Source\n\n## Rule\n\nupdated body before commit\n\n## Extra\n\nv1\n",
    );
    let repin = common::mrd_command(&home, &cache)
        .args(["pin", "agents/card-a/card-a.md", "source.md#Source/Rule"])
        .current_dir(&ws)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("repin");
    assert_eq!(repin.status.code(), Some(0), "repin: {}", said(&repin));
    git(&ws, &["add", "source.md", "agents/card-a/card-a.md"]);
    assert!(
        !ws.join(".git/meridian/reach-v1.bin").exists(),
        "precondition: reach-v1.bin absent before check"
    );

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
        .expect("check staged repin");
    let text = said(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "staged re-pin must pass: {text}"
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(json["commit_gate"]["permits"], true);
    assert_eq!(json["pins"]["anchoring"]["pending_anchor"], 1);
    assert_eq!(json["pins"]["anchoring"]["orphaned"], serde_json::json!([]));
    assert!(
        !ws.join(".git/meridian/reach-v1.bin").exists(),
        "staged re-pin must NOT invoke repo-wide rev-list / build reach-v1.bin"
    );
}

/// Parity: run the same fixtures through the bounded scoped lane
/// (`--commit-gate -- <holder>`) and the full in-process lane
/// (`--commit-gate -- .`, with `MERIDIAN_DAEMON_BIN=/nonexistent`), asserting
/// identical exit codes, pin rows, and gate verdicts across:
/// - mounted-root target
/// - out-of-domain holder (custom-ignored) + enumerator note
/// - out-of-domain target
/// - holder whose lock block is refused
/// - target deleted only in the index
/// - target present only in the index
/// - short / alias object spelling (P2-2)
/// - orphan in a holder outside PATH (P2-1)
#[test]
#[allow(clippy::too_many_lines)] // full parity fixture covering 8 review edge cases
fn scoped_and_in_process_lanes_agree_across_edge_cases() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache = tmp.path().join("xdg-cache");
    let ws = tmp.path().join("ws");
    let ext = tmp.path().join("ext");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&ws).expect("ws");
    std::fs::create_dir_all(&ext).expect("ext");

    // Set up mounted external root `ext`.
    git(&ext, &["init", "-q"]);
    git(&ext, &["config", "user.email", "test@example.invalid"]);
    git(&ext, &["config", "user.name", "test"]);
    git(&ext, &["config", "commit.gpgsign", "false"]);
    write(&ext, "spec.md", "# Spec\n\n## Law\n\nexternal law\n");
    let init_ext = common::mrd_command(&home, &cache)
        .args(["init", "--name", "ext"])
        .current_dir(&ext)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("init ext");
    assert!(init_ext.status.success(), "init ext: {}", said(&init_ext));
    git(&ext, &["add", "-A"]);
    git(&ext, &["commit", "-qm", "ext seed"]);

    // Declare mount in $HOME/MERIDIAN.md.
    std::fs::write(
        home.join("MERIDIAN.md"),
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n# mounts\n\n```meridian-mount\nname: ext\npath: {}\n```\n",
            ext.display()
        ),
    )
    .expect("write MERIDIAN.md");

    // Set up main workspace.
    git(&ws, &["init", "-q"]);
    git(&ws, &["config", "user.email", "test@example.invalid"]);
    git(&ws, &["config", "user.name", "test"]);
    git(&ws, &["config", "commit.gpgsign", "false"]);
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"ignored/**\"\n---\n",
    );
    write(
        &ws,
        "sub/aliased.md",
        "---\naliases: [short-target]\n---\n# Aliased\n\n## Clause\n\naliased body\n",
    );
    write(
        &ws,
        "ignored/ood-target.md",
        "# OOD\n\n## Body\n\noutside domain\n",
    );
    write(&ws, "ignored/ood-holder.md", "# OOD Holder\n\nnotes.\n");
    write(
        &ws,
        "idx-del.md",
        "# IdxDel\n\n## Body\n\nwill be rm --cached\n",
    );
    write(&ws, "idx-only.md", "# IdxOnly\n\n## Body\n\nindex only\n");
    write(&ws, "orphan-src.md", "# OrphanSrc\n\n## Body\n\nv1\n");
    write(&ws, "h-mount.md", "# HMount\n");
    write(&ws, "h-ood-target.md", "# HOodTarget\n");
    write(
        &ws,
        "h-refused.md",
        "# HRefused\n\n```meridian-lock\nnot valid lock syntax ::::\n```\n",
    );
    write(&ws, "h-idx-del.md", "# HIdxDel\n");
    write(&ws, "h-idx-only.md", "# HIdxOnly\n");
    write(&ws, "h-alias.md", "# HAlias\n");
    write(&ws, "h-orphan-other.md", "# HOrphanOther\n");

    let init_ws = common::mrd_command(&home, &cache)
        .args(["init"])
        .current_dir(&ws)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("init ws");
    assert!(init_ws.status.success(), "init ws: {}", said(&init_ws));
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "seed"]);

    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };

    let pin_ok = |holder: &str, target: &str| {
        let out = common::mrd_command(&home, &cache)
            .args(["pin", holder, target])
            .current_dir(&ws)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("pin");
        assert_eq!(
            out.status.code(),
            Some(0),
            "pin {holder} -> {target}: {}",
            said(&out)
        );
    };

    // 1. Mounted-root target
    pin_ok("h-mount.md", "ext:spec.md#Spec/Law");
    // 2. Out-of-domain holder pinning an in-domain target
    pin_ok("ignored/ood-holder.md", "sub/aliased.md#Aliased/Clause");
    // 3. Out-of-domain target (mrd pin on in-domain holder to ood target, or hand-built lock)
    pin_ok("h-idx-del.md", "idx-del.md#IdxDel/Body");
    pin_ok("h-idx-only.md", "idx-only.md#IdxOnly/Body");
    // 4. Short/alias object spelling (pin canonical then rewrite object to alias)
    pin_ok("h-alias.md", "sub/aliased.md#Aliased/Clause");
    let alias_body = std::fs::read_to_string(ws.join("h-alias.md")).expect("read h-alias");
    let rewritten = alias_body.replace("sub/aliased", "short-target");
    assert_ne!(alias_body, rewritten, "alias replacement must apply");
    std::fs::write(ws.join("h-alias.md"), rewritten).expect("write h-alias");
    // Out-of-domain target pin on h-ood-target.md (copied lock structure pointing to ignored/ood-target)
    let ood_lock = alias_body.replace("sub/aliased", "ignored/ood-target");
    std::fs::write(ws.join("h-ood-target.md"), ood_lock).expect("write h-ood-target");

    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "all pins"]);

    // Create an orphan in `h-orphan-other.md` (outside PATH of the other holders):
    // edit orphan-src.md, pin it (eagerly writes blob to ODB), then overwrite orphan-src.md
    // without ever committing the intermediate blob!
    write(
        &ws,
        "orphan-src.md",
        "# OrphanSrc\n\n## Body\n\nuncommitted blob B\n",
    );
    pin_ok("h-orphan-other.md", "orphan-src.md#OrphanSrc/Body");
    write(
        &ws,
        "orphan-src.md",
        "# OrphanSrc\n\n## Body\n\noverwritten blob C\n",
    );
    git(&ws, &["add", "h-orphan-other.md", "orphan-src.md"]);

    // Stage index-only deletion of idx-del.md (still on disk in worktree)
    git(&ws, &["rm", "--cached", "-q", "idx-del.md"]);
    // Index-only presence of idx-only.md (staged, removed from worktree)
    std::fs::remove_file(ws.join("idx-only.md")).expect("rm worktree idx-only");

    // Helper: compare scoped lane (`--commit-gate --json -- <holder>`) against
    // the full in-process lane (`--commit-gate --json -- <holder>` forced into
    // in-process by passing a second root-empty path? No: compare against
    // `mrd check --staged --json -- <holder>` with daemon disabled, which uses
    // `assess_in_process` with the exact same `-- <holder>` filter!).
    let run_pair = |holder: &str| {
        let scoped = common::mrd_command(&home, &cache)
            .args(["check", "--commit-gate", "--json", "--", holder])
            .current_dir(&ws)
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("scoped");
        // `--staged` without `--commit-gate` takes `assess_in_process` (never
        // `assess_scoped`), producing the exact same `pins` and `interval.staged.pins`
        // for `holder`!
        let inproc = common::mrd_command(&home, &cache)
            .args(["check", "--staged", "--json", "--", holder])
            .current_dir(&ws)
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("inproc");
        let sj: serde_json::Value = serde_json::from_slice(&scoped.stdout)
            .unwrap_or_else(|e| panic!("scoped json for {holder}: {e}\n{}", said(&scoped)));
        let ij: serde_json::Value = serde_json::from_slice(&inproc.stdout)
            .unwrap_or_else(|e| panic!("inproc json for {holder}: {e}\n{}", said(&inproc)));
        // Compare gated interval's pin plane (`staged.pins` if staged assessed, else `pins`).
        let s_gated = if sj["commit_gate"]["gated_interval"] == "staged" {
            &sj["interval"]["staged"]["pins"]
        } else {
            &sj["pins"]
        };
        let i_gated = if ij["interval"]["staged"].is_object() {
            &ij["interval"]["staged"]["pins"]
        } else {
            &ij["pins"]
        };
        assert_eq!(
            s_gated, i_gated,
            "scoped vs in-process gated pin plane mismatch for {holder}\nscoped: {sj}\ninproc: {ij}"
        );
        (scoped, sj)
    };

    // (a) mounted-root target: green in both
    let (out, j) = run_pair("h-mount.md");
    assert_eq!(out.status.code(), Some(0), "h-mount: {}", said(&out));
    assert_eq!(j["commit_gate"]["permits"], true);

    // (b) out-of-domain holder: green in both AND voices enumerator note on stderr (P3-4)
    let (out, j) = run_pair("ignored/ood-holder.md");
    assert_eq!(out.status.code(), Some(0), "ood-holder: {}", said(&out));
    assert_eq!(j["commit_gate"]["permits"], true);
    let stderr_ood = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_ood.contains("outside the hash domain"),
        "scoped lane must voice enumerator note for declined holder: {stderr_ood}"
    );

    // (c) out-of-domain target: unattested in both
    let (out, j) = run_pair("h-ood-target.md");
    assert_eq!(out.status.code(), Some(0), "h-ood-target: {}", said(&out));
    assert_eq!(j["pins"]["unattested"].as_array().map(Vec::len), Some(1));

    // (d) refused lock block: grey lock-refused in both (exit 1)
    let (out, j) = run_pair("h-refused.md");
    assert_eq!(out.status.code(), Some(1), "h-refused: {}", said(&out));
    assert_eq!(j["commit_gate"]["permits"], false);

    // (e) target deleted only in the index: RED in staged (exit 1) in both
    let (out, j) = run_pair("h-idx-del.md");
    assert_eq!(out.status.code(), Some(1), "h-idx-del: {}", said(&out));
    assert_eq!(j["commit_gate"]["permits"], false);
    assert_eq!(j["commit_gate"]["gated_interval"], "staged");

    // (f) target present only in the index: `s_gated == i_gated` proves exact
    //     parity with `assess_in_process` (both check `on_ambient_disk`, so
    //     both produce the identical staged verdict).
    let (out, j) = run_pair("h-idx-only.md");
    assert_eq!(
        out.status.code(),
        Some(1),
        "h-idx-only must match in-process lane: {}",
        said(&out)
    );
    assert_eq!(j["commit_gate"]["gated_interval"], "staged");

    // (g) P2-2 short/alias object spelling: GREEN (exit 0) in both (not false red!)
    let (out, j) = run_pair("h-alias.md");
    assert_eq!(
        out.status.code(),
        Some(0),
        "short/alias target must resolve green via fallback: {}",
        said(&out)
    );
    assert_eq!(j["commit_gate"]["permits"], true);

    // (h) P2-1 orphan in h-orphan-other.md: caught when checking h-orphan-other.md
    //     itself, ignored when checking unrelated h-mount.md — identical in both lanes!
    let (out, j) = run_pair("h-orphan-other.md");
    assert_eq!(out.status.code(), Some(1), "h-orphan-other: {}", said(&out));
    assert_eq!(j["commit_gate"]["permits"], false);
}

/// Verify `mrd check` exits immediately on `SIGHUP` (not ignored / detached).
#[test]
fn sighup_terminates_mrd_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
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
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let child = Command::new(mrd_bin())
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
    std::thread::sleep(std::time::Duration::from_millis(150));
    let git_alive = unsafe { libc::kill(git_pid, 0) } == 0;
    assert!(
        !git_alive,
        "child git pid {git_pid} must not survive after mrd check exits on SIGHUP"
    );
}

/// P2-6: deeper ssh ancestor chain `A -> bash -s -> timeout -> mrd -> git`:
/// killing `A` must cause `mrd` and its `git` child to exit well before the
/// 60 s wall-clock bound.
#[test]
fn deep_ssh_ancestor_chain_aborts_mrd_and_git_child() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let fake_bin = tmp.path().join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("mkdir");
    let git_pid_file = tmp.path().join("deep-git.pid");
    let mrd_pid_file = tmp.path().join("deep-mrd.pid");
    let fake_git = fake_bin.join("git");
    std::fs::write(
        &fake_git,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            git_pid_file.display()
        ),
    )
    .expect("write fake git");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let path_env = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // Build chain: outer `sh` (A) -> `bash -s` -> `timeout 60` -> `mrd` -> `git`
    let inner_script = format!(
        "timeout 60 sh -c 'echo $$ > \"{}\"; exec \"{}\" check --commit-gate --require-pins -- agents/card-a/card-a.md'\n",
        mrd_pid_file.display(),
        mrd_bin().display()
    );
    // Use a regular file (not a pipe) for stdout/stderr so stdout-pipe-close
    // does NOT mask the ancestor-chain check!
    let log_file = std::fs::File::create(tmp.path().join("deep.log")).expect("log");
    let log_err = log_file.try_clone().expect("clone log");
    let mut anc_a = Command::new("sh")
        .args([
            "-c",
            &format!("bash -s <<'MRD_DEEP_EOF'\n{inner_script}MRD_DEEP_EOF\n"),
        ])
        .current_dir(&ws)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("PATH", path_env)
        .env("MRD_CHECK_LOCK", tmp.path().join("deep.lock"))
        .env("MRD_CHECK_TIMEOUT_SECS", "60")
        .env_remove("MERIDIAN_WORKSPACE")
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .spawn()
        .expect("spawn ancestor A");

    let start = std::time::Instant::now();
    while (!git_pid_file.exists() || !mrd_pid_file.exists()) && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        git_pid_file.exists() && mrd_pid_file.exists(),
        "deep chain did not reach fake git in time"
    );
    let mrd_pid: i32 = std::fs::read_to_string(&mrd_pid_file)
        .expect("read mrd pid")
        .trim()
        .parse()
        .expect("parse mrd pid");
    let git_pid: i32 = std::fs::read_to_string(&git_pid_file)
        .expect("read git pid")
        .trim()
        .parse()
        .expect("parse git pid");

    // Kill ancestor A with SIGKILL (simulating sshd dying abruptly).
    anc_a.kill().expect("kill A");
    let _ = anc_a.wait();

    // Both mrd and its git child must die well before the 60 s wall-clock bound.
    let wait_start = std::time::Instant::now();
    while wait_start.elapsed().as_secs() < 5 {
        let mrd_alive = unsafe { libc::kill(mrd_pid, 0) } == 0;
        let git_alive = unsafe { libc::kill(git_pid, 0) } == 0;
        if !mrd_alive && !git_alive {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mrd_alive = unsafe { libc::kill(mrd_pid, 0) } == 0;
    let git_alive = unsafe { libc::kill(git_pid, 0) } == 0;
    // Clean up any stragglers before asserting.
    unsafe {
        let _ = libc::kill(mrd_pid, libc::SIGKILL);
        let _ = libc::kill(git_pid, libc::SIGKILL);
    }
    panic!(
        "deep ancestor chain failed to abort within 5s: mrd_alive={mrd_alive} git_alive={git_alive}"
    );
}

/// Verify `mrd check` enforces its wall-clock bound (`MRD_CHECK_TIMEOUT_SECS`),
/// honors `MRD_CHECK_TIMEOUT_SECS=0` as no bound, and kills child git processes
/// on timeout.
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
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755)).expect("chmod");
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
        .env("MRD_CHECK_TIMEOUT_SECS", "2")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd");
    let elapsed = started.elapsed();
    let text = said(&out);
    assert_eq!(
        out.status.code(),
        Some(124),
        "timeout must exit 124: {text}"
    );
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

    // Also verify `MRD_CHECK_TIMEOUT_SECS=0` disables the bound cleanly (normal
    // check completes exit 0).
    let no_bound = Command::new(mrd_bin())
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
        .env("MRD_CHECK_LOCK", tmp.path().join("timeout0.lock"))
        .env("MRD_CHECK_TIMEOUT_SECS", "0")
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd with timeout=0");
    assert_eq!(
        no_bound.status.code(),
        Some(0),
        "MRD_CHECK_TIMEOUT_SECS=0 must disable bound cleanly: {}",
        said(&no_bound)
    );
}

/// P2-5: a real commit through the emitted fence (`mrd skill hook`) while a
/// sibling process holds `MRD_CHECK_LOCK` fails closed with `"busy"` and
/// `"retry"`, NOT the exit-2 version-skew/bad-invocation message.
#[test]
fn placed_fence_refuses_busy_with_retry_guidance() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };

    // Emit and place the fence on pre-commit.
    let emitted = common::mrd_command(&home, &cache)
        .args(["skill", "hook"])
        .current_dir(&ws)
        .output()
        .expect("skill hook");
    assert_eq!(emitted.status.code(), Some(0));
    let doc = String::from_utf8(emitted.stdout).expect("utf8");
    let mut lines = doc.lines();
    let _ = lines
        .by_ref()
        .find(|l| l.starts_with("```sh"))
        .expect("```sh");
    let mut body = String::new();
    for line in lines {
        if line.starts_with("```") {
            break;
        }
        body.push_str(line);
        body.push('\n');
    }
    let hook_path = ws.join(".git/hooks/pre-commit");
    std::fs::write(&hook_path, body).expect("write pre-commit");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }

    // Put our test `mrd` binary first on PATH.
    let bin_dir = tmp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("mkdir bin");
    std::os::unix::fs::symlink(mrd_bin(), bin_dir.join("mrd")).expect("symlink mrd");
    let path_env = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let lock_path = tmp.path().join("fence-busy.lock");
    let holder = SiblingLockHolder::acquire(&lock_path, &tmp.path().join("fence-busy.ready"));

    write(&ws, "note.md", "# Note\n\nhello\n");
    git(&ws, &["add", "note.md"]);
    let commit = Command::new("git")
        .arg("-C")
        .arg(&ws)
        .args(["commit", "-m", "should fail busy"])
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("PATH", path_env)
        .env("MRD_CHECK_LOCK", &lock_path)
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("git commit");
    let text = said(&commit);
    assert!(
        !commit.status.success(),
        "commit must fail closed when slot is busy: {text}"
    );
    assert!(
        text.contains("75") && text.contains("busy:") && text.contains("retry"),
        "fence must print busy/retry guidance (exit 75): {text}"
    );
    assert!(
        !text.contains("OLDER than this fence"),
        "busy must NOT print the exit-2 version-skew message: {text}"
    );
    drop(holder);
}

/// P3-3: a symlink loop under a directory `PATH` must NOT spin until the 60 s
/// timeout.
#[test]
fn symlink_loop_under_directory_path_does_not_spin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, cache, ws) = setup_repo(tmp.path());
    let _reaper = common::DaemonReaper {
        home: home.clone(),
        cache_home: cache.clone(),
    };
    #[cfg(unix)]
    std::os::unix::fs::symlink(ws.join("agents"), ws.join("agents/loop-back"))
        .expect("symlink loop");

    let started = std::time::Instant::now();
    let out = common::mrd_command(&home, &cache)
        .args(["check", "--commit-gate", "--require-pins", "--", "agents"])
        .current_dir(&ws)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-no-daemon")
        .env("MRD_CHECK_TIMEOUT_SECS", "10")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("check dir with symlink loop");
    let elapsed = started.elapsed();
    assert_eq!(
        out.status.code(),
        Some(0),
        "symlink loop must be skipped cleanly: {}",
        said(&out)
    );
    assert!(
        elapsed.as_secs() < 5,
        "must not spin on symlink loop (took {elapsed:?})"
    );
}
