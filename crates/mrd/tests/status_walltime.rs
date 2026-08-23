//! The `mrd status` wall-time budget — the perf lane's gate, not the PR
//! lane's. Wall-time numbers are never gated in the PR lane (shared-runner
//! noise): `required-features = ["perf-walltime"]` keeps
//! `cargo test --workspace` from gating a wall clock, `ci.yml` still compiles
//! and lints this target, and `perf.yml` runs it on the pinned runner.
//!
//! This sandbox sets `HOME` to a bare temp dir, so the machine mount table is
//! empty — deliberate isolation, which also means the eager mount loader has
//! no roots to walk here. The multi-root profile that does declare a table is
//! `status_multiroot_cpu.rs`; the two are complementary — this one bounds the
//! corpus-independence claim, that one the mount-table-independence claim.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

mod common;

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sb = Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        home: tmp.path().join("home"),
        tmp,
    };
    std::fs::create_dir_all(&sb.home).expect("home");
    sb
}

fn run(sb: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .current_dir(cwd)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd")
}

/// Best-effort reap of a resident auto-spawned by this sandbox. Pidfile path, never
/// process-table substring — same law as the multiroot fixture and the fleet reap script. Soft
/// (no panic) so Drop can call it; the control target `perf_daemon_teardown` is the asserted
/// path.
fn try_teardown_daemon(sb: &Sandbox) {
    let pidfile = common::child_daemon_pidfile(&sb.home, &sb.cache_home);
    let Ok(text) = std::fs::read_to_string(pidfile) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<i32>() else {
        return;
    };
    // SAFETY: kill(2) to a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        // SAFETY: signal 0 probes existence.
        if unsafe { libc::kill(pid, 0) } == -1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        try_teardown_daemon(self);
    }
}

/// A bare workspace with an `mrd init` marker so `status` resolves it.
fn workspace(sb: &Sandbox, name: &str) -> PathBuf {
    let ws = sb.tmp.path().join(name);
    std::fs::create_dir_all(&ws).expect("mkdir");
    let init = run(sb, &ws, &["init"]);
    assert!(
        init.status.success(),
        "init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    ws
}

/// A minimal loadable CHECK rule page for one id — the O(armed) re-hash subject.
fn rule_page(id: &str) -> String {
    format!(
        "---\ntags: [type/rule, rules/check]\nid: {id}\npaths:\n  - {id}/**\n---\n\n\
         # {id}\n\n```starlark\ndef check_change(change):\n    pass\n```\n"
    )
}

/// Arm `ids` into `meridian/armed-rules.md` at each pages live rev and plant the once-armed
/// marker. Every row is FRESH, so `status` does the full O(armed) re-hash — which is the work
/// this budget is measured over. The artifact is MINTED by `policy::armed::arm`, never
/// hand-typed: a fixture artifact the arming act never approved would measure a read production
/// cannot perform.
fn arm_rules(ws: &Path, ids: &[String]) {
    let pages: Vec<(String, String)> = ids
        .iter()
        .map(|id| (format!("rules/{id}.md"), rule_page(id)))
        .collect();
    for (page, bytes) in &pages {
        let full = ws.join(page);
        std::fs::create_dir_all(full.parent().expect("rules dir")).expect("rules dir");
        std::fs::write(full, bytes).expect("rule page");
    }
    let index = policy::RuleIndex::discover(pages.iter().map(|(page, bytes)| policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page,
        bytes,
    }));
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        pages.iter().map(|(_, bytes)| policy::armed::ArmRequest {
            id: policy::RuleId::parse(
                bytes
                    .lines()
                    .find_map(|l| l.strip_prefix("id: "))
                    .expect("the fixture page declares an id"),
            )
            .expect("a legal id"),
            mode: policy::armed::Mode::Block,
            attested_rev: policy::page_rev(bytes),
        }),
    )
    .expect("the fixture arms at each page's live rev");
    let artifact_path = ws.join(fs::domain::ARMED_RULES_PATH);
    std::fs::create_dir_all(artifact_path.parent().expect("meridian dir")).expect("meridian dir");
    std::fs::write(artifact_path, artifact.render()).expect("artifact");
    std::fs::write(ws.join(fs::domain::ATTESTED_MARKER_PATH), "").expect("once-armed marker");
}

/// The <1s wall-time gate. A 3k-doc corpus with a handful of armed rules:
/// `status` reads one artifact file + the once-armed marker + O(armed)
/// rule-page re-hashes + the journal + the git refs — never the 3k docs, so
/// its wall-time is independent of corpus size.
#[test]
fn status_wall_time_under_1s_on_3k_corpus() {
    let sb = sandbox();
    let ws = workspace(&sb, "corpus3k");

    // 3,000 ordinary docs — the corpus size status must NOT scale with.
    let docs = ws.join("docs");
    std::fs::create_dir_all(&docs).expect("docs dir");
    for i in 0..3_000u32 {
        std::fs::write(
            docs.join(format!("note-{i:04}.md")),
            format!("# Note {i}\n\nbody line for note {i}\n"),
        )
        .expect("write doc");
    }
    // A handful of armed rules (the O(armed) work).
    let ids: Vec<String> = (0..5u32).map(|i| format!("conv-{i}")).collect();
    arm_rules(&ws, &ids);

    // Warm the process/page cache with one throwaway run, then measure.
    let _ = run(&sb, &ws, &["status"]);
    let start = Instant::now();
    let out = run(&sb, &ws, &["status"]);
    let elapsed = start.elapsed();

    let code = out.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "status ran: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let so = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        so.contains("5 armed · 0 drifted"),
        "the armed set read: {so}"
    );

    let ms = elapsed.as_millis();
    eprintln!("status wall-time on the 3k-doc corpus: {ms} ms (hard budget 1000 ms)");
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "status must be O(armed), <1s on the 3k corpus — measured {ms} ms"
    );

    // Perf-lane hygiene: any auto-spawned resident dies with the sandbox.
    // `status` is pure-local today (no spawn); Drop still covers a future probe.
    try_teardown_daemon(&sb);
}
