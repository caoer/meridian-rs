//! **The perf lane's multi-root profile — the shape `status_walltime.rs` cannot see.**
//!
//! `status_walltime.rs` gates 1000 ms and passed green for the whole life of the
//! W2 defect, while the real verb took 20-22 s in the field. The reason is its
//! fixture, not its budget: it sets `HOME` to a bare temp dir, so the machine
//! mount table is EMPTY, so the eager loader had no roots to walk. **The gate
//! removed the input that dominated the cost and then reported a bound it had
//! never tested.** A perf gate whose fixture deletes the dominant input is not a
//! slow gate — it is a green light with no lamp behind it.
//!
//! So this target measures `mrd status` with a **populated** mount table: four
//! declared roots, each a dirent-heavy corpus, none of them named by any lock
//! address in the workspace under test. Post-W2 (`load_mounts_for` +
//! `lock_addressed_roots`, `a8fdb356`) status builds none of those four corpora
//! and pays the ambient workspace only. Pre-W2 it built all four.
//!
//! # The measure is CPU, not wall
//! The host this lane runs on is shared, and the W2 investigation measured the
//! consequence directly: `mrd read` burns 0.01 s of CPU and took 5.3 s of wall
//! under load 41, carrying no defect at all. Wall-clock on a contended host
//! measures the neighbours. The load-independent number in the same
//! investigation is CPU — 9.7 s → 1.4 s across the fix — and CPU is what the
//! defect moves, because 72 % of its samples are `getdirentries64`/`open`.
//! `getrusage(RUSAGE_CHILDREN)` reads exactly that, for the child and no one
//! else, so this file asserts on user+sys and prints wall only for the record.
//!
//! # Negative control — how to redden this gate
//! In `crates/mrd/src/status_cmd.rs`, replace
//!
//! ```text
//!     let mounts = crate::walk_cmd::load_mounts_for(&lock_addressed_roots(&docs));
//! ```
//!
//! with the pre-W2 eager call, `crate::walk_cmd::load_mounts()`, and run this
//! target. That is the one-line reintroduction of the defect class, and the
//! measured red/green pair is recorded on the card.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// Declared mount roots in the fixture's table. Four is the field shape (the W2
/// investigation measured four bound roots on the dogfood machine).
const ROOTS: usize = 4;
/// Directories per root. The defect is directory ENUMERATION, so the corpus is
/// shaped like the sharpest field case — `meridian-rs`, 200 markdown files
/// behind 20,178 directories — rather than like a document pile.
const DIRS_PER_ROOT: usize = 25_000;
/// Markdown pages per root, so the skipped work includes real parsing too.
const PAGES_PER_ROOT: usize = 2_000;

/// **The CPU budget.** See the module header for why it is CPU and not wall.
/// MEASURED, never copied forward from the 1000 ms wall budget next door — that
/// number bounds a different verb shape on a different clock, and inheriting it
/// would be the same unasked question this file exists to ask.
///
/// The two arms, debug profile, one host, the one-line negative control:
///
/// | arm | CPU |
/// |---|---:|
/// | narrowed (`load_mounts_for`) | 186 / 189 / 208 ms over three runs |
/// | eager (`load_mounts`, W2 restored) | 2733 ms |
///
/// The arms are 13× apart and 600 ms sits inside that gap: 3× of headroom over
/// the fix's spread, 4.6× under the defect. The narrowed arm is also INSENSITIVE
/// to the corpus constants — 186 ms measured at both 8,000 and 100,000
/// declared-root directories — which is the invariant the budget stands on, so
/// the margin does not shrink as the fixture grows.
const CPU_BUDGET: Duration = Duration::from_millis(600);

struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    config: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        config: home.join("MERIDIAN.md"),
        home,
        tmp,
    }
}

fn run(sb: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", &sb.home)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("MERIDIAN_CONFIG", &sb.config)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd")
}

/// One declared root: its canonical-name declaration (INV-5 — without it the
/// bind renders undeclared and the table under test is vacuous), a dirent-heavy
/// tree, and a scatter of real pages.
fn plant_root(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).expect("root dir");
    std::fs::write(
        dir.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
    )
    .expect("root declaration");

    // Two levels, so the walk recurses rather than reading one wide directory.
    let fanout = 100usize;
    let deep = DIRS_PER_ROOT / fanout;
    for a in 0..fanout {
        for b in 0..deep {
            std::fs::create_dir_all(dir.join(format!("d{a:02}/s{b:02}"))).expect("mkdir");
        }
    }
    let pages = dir.join("pages");
    std::fs::create_dir_all(&pages).expect("pages dir");
    for i in 0..PAGES_PER_ROOT {
        std::fs::write(
            pages.join(format!("page-{i:04}.md")),
            format!("# {name} page {i}\n\n## Body\n\nA paragraph of body text for page {i}.\n"),
        )
        .expect("page");
    }
}

/// The child's user+sys CPU, cumulative over every child this process has
/// reaped. A delta of it around one `.output()` call attributes to that child
/// **only while this process spawns nothing else concurrently**, which is why
/// this target holds exactly ONE `#[test]` and `perf.yml` invokes it alone.
/// `RUSAGE_CHILDREN` is per-process, not per-thread, so a second `#[test]` here
/// would silently fold its own children into this measurement — the count is
/// scoped by the target's shape, never by the test harness's scheduling.
fn children_cpu() -> Duration {
    // SAFETY: `getrusage` writes a fully-initialised `rusage` into the out
    // pointer and reads nothing else; `RUSAGE_CHILDREN` is a valid `who`.
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &raw mut usage) };
    assert_eq!(rc, 0, "getrusage(RUSAGE_CHILDREN)");
    let secs = |t: libc::timeval| {
        Duration::new(
            u64::try_from(t.tv_sec).expect("non-negative seconds"),
            u32::try_from(t.tv_usec).expect("microseconds fit") * 1_000,
        )
    };
    secs(usage.ru_utime) + secs(usage.ru_stime)
}

#[test]
fn status_cpu_under_budget_with_a_populated_mount_table() {
    let sb = sandbox();

    // Four declared roots — the input the sibling gate deletes.
    let names: Vec<String> = (0..ROOTS).map(|i| format!("root{i}")).collect();
    let mut table = String::from("---\ntype: meridian-config\nversion: 1\n---\n\n# Perf roots\n\n");
    for name in &names {
        let dir = sb.tmp.path().join(name);
        plant_root(&dir, name);
        writeln!(
            table,
            "```meridian-mount\nname: {name}\npath: {}\nkind: vault\nvault: {name}vault\n```\n",
            dir.display()
        )
        .expect("writing into a String cannot fail");
    }
    std::fs::write(&sb.config, &table).expect("mount table");

    // The workspace under test: ordinary, small, and its locks name NO root —
    // the common case, and the one the narrowing is supposed to make cheap.
    let ws = sb.tmp.path().join("ws");
    std::fs::create_dir_all(&ws).expect("ws");
    let init = run(&sb, &ws, &["init"]);
    assert!(
        init.status.success(),
        "init: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    // **The fixture's own anti-blindness assert.** This is the check whose
    // absence let the sibling gate measure an empty table for months: prove the
    // table under test is POPULATED and BOUND before trusting anything measured
    // through it. A fixture that quietly stops declaring roots must fail here,
    // loudly, rather than pass the budget below for the wrong reason.
    let cfg = run(&sb, &ws, &["config"]);
    let cfg_out = String::from_utf8_lossy(&cfg.stdout).into_owned();
    for name in &names {
        assert!(
            cfg_out.contains(name.as_str()),
            "the mount table under test must DECLARE {name} — it read:\n{cfg_out}"
        );
    }
    assert_eq!(
        cfg_out.matches("bound").count(),
        ROOTS,
        "all {ROOTS} declared roots must BIND, or the corpora this gate measures \
         the absence of were never buildable in the first place — it read:\n{cfg_out}"
    );

    // Warm the page cache with a throwaway run, then measure the next one.
    let _ = run(&sb, &ws, &["status"]);
    let cpu_before = children_cpu();
    let wall_start = Instant::now();
    let out = run(&sb, &ws, &["status"]);
    let wall = wall_start.elapsed();
    // `RUSAGE_CHILDREN` is cumulative and monotonic, so the later read can only
    // be the larger one — but the checked form says so rather than trusting it,
    // and an underflow here would print as a wildly under-budget PASS.
    let cpu = children_cpu()
        .checked_sub(cpu_before)
        .expect("children CPU is cumulative, so it never goes backwards");

    let code = out.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "status ran: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    eprintln!(
        "status over {ROOTS} declared roots ({} dirs, {} pages unread): \
         CPU {} ms (budget {} ms), wall {} ms (recorded, NOT gated)",
        ROOTS * DIRS_PER_ROOT,
        ROOTS * PAGES_PER_ROOT,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
        wall.as_millis(),
    );
    assert!(
        cpu < CPU_BUDGET,
        "status must build only the mount roots its own lock addresses name. \
         With {ROOTS} declared roots that no lock addresses, it burned {} ms of \
         CPU against a {} ms budget — the eager-loader shape (W2).",
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
    );
}
