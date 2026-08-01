//! The `mrd status` wall-time budget — the PERF lane's gate, not the PR lane's.
//!
//! `ci.yml` states the law this file obeys: *"Wall-time numbers are NEVER gated
//! in this lane (shared-runner noise); that's perf.yml's job on the pinned fleet
//! runner."* While this assert lived in `status_e2e.rs` that law was
//! contradicted — `cargo test --workspace` gated a wall-clock number on whatever
//! machine happened to run it, so "workspace suite green" meant "green on a quiet
//! machine". A/B interleave on one host at matched load, three pairs: the shipped
//! `main` at `64d761b1` blew the same 1000 ms budget **3 for 3** (2012 / 1244 /
//! 2711 ms) against a candidate tree's 2387 / 1278 / 1477 ms. The assert was
//! measuring contention, not code.
//!
//! So it MOVED, and **the 1000 ms budget did not move with it.** The mechanism is
//! `required-features = ["perf-walltime"]` in `crates/mrd/Cargo.toml`:
//! `cargo test --workspace` skips this target entirely (no wall-clock gate, and
//! no vacuous pass either), `ci.yml` still compiles and lints it so it cannot
//! bit-rot, and `perf.yml` runs it for real on the pinned runner.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

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

/// Write an attested INDEX row pinning the convention's REAL `page_rev`, plus
/// its `CHECK.md` — the row is FRESH, so `status` does the full O(armed) re-hash.
fn arm_convention(ws: &Path, slug: &str, severity: &str, check: &str) -> String {
    let dir = ws.join("conventions").join(slug);
    std::fs::create_dir_all(&dir).expect("conv dir");
    std::fs::write(dir.join("CHECK.md"), check).expect("check");
    let pinned_rev = policy::page_rev(check);
    format!(
        "- [x] **{slug}** · {severity} · `{pinned_rev}` · [[conventions/{slug}/CHECK.md]] · `{slug}/**`"
    )
}

/// Assemble a valid INDEX page (title + preamble + rows) that `parse_index_strict`
/// accepts. Verified to parse by the caller's `status` read.
fn write_index(ws: &Path, rows: &[String]) {
    let dir = ws.join("conventions");
    std::fs::create_dir_all(&dir).expect("conventions dir");
    let page = format!(
        "# Attested conventions INDEX\n\nSwept from `conventions/`.\n\n{}\n",
        rows.join("\n")
    );
    std::fs::write(dir.join("INDEX.md"), page).expect("index");
}

/// **The <1s wall-time gate (the merge budget, U3.6).** A 3k-doc corpus with a
/// handful of armed conventions: `status` reads ONE index file + O(armed)
/// `CHECK.md` re-hashes + the journal + the git refs — NEVER the 3k docs. So its
/// wall-time is independent of corpus size and stays well under the 1s hard
/// budget. The measured milliseconds are printed for the card record.
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
    // A handful of armed conventions (the O(armed) work).
    let rows: Vec<String> = (0..5u32)
        .map(|i| arm_convention(&ws, &format!("conv-{i}"), "block", &format!("law {i} v1\n")))
        .collect();
    write_index(&ws, &rows);

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
}
