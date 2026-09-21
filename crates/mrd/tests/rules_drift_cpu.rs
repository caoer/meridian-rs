//! CPU budget for the rules drift column, isolated from parallel CLI tests.
//! Run only in the controlled performance lane (`perf-walltime`).

mod common;
mod rules_fixture;

use rules_fixture::{arm, rule_page, sandbox};

// The gate bounds child CPU and records wall time. A single test in this
// executable prevents concurrent test children from contaminating the counter;
// a controlled runner supplies the fixed toolchain and resource allocation
// needed to compare measurements across revisions.
//
// What is actually being bounded: the drift column adds ONE page read per
// DISTINCT pinned page (`ArmedArtifact::drift` memoises by page), joined once
// for the whole answer rather than per section. This fixture arms a ledger wide
// enough that a per-row or per-section re-read would show up as a multiple.

/// The CPU budget for one `mrd rules` over a `LEDGER_ROWS`-row ledger.
///
/// MEASURED at `1e72a731`, both lanes, not guessed: **nyc-2 (the CI lane, Linux)
/// CPU 12 ms / wall 12 ms; the mac lane CPU 303 ms / wall 308 ms** — the whole
/// invocation, resolution included, of which the drift join is a small part.
/// 1 500 ms is ~5× the mac number and ~125× the CI one.
///
/// ⚠️ **What this bound is and is not.** It is the PERF LAW's receipt — a
/// regression that changes the SHAPE (a domain snapshot, an unbounded walk)
/// moves it by an order of magnitude and trips here. It is a poor instrument for
/// the specific claim "one read per DISTINCT pinned page", because that claim's
/// cost is a small fraction of the total and 5× headroom would swallow a 3×
/// regression in it. That claim has its own deterministic, load-independent
/// gate: `policy::armed::tests::drift_reads_each_distinct_pinned_page_once`
/// counts the reads instead of timing them.
const DRIFT_CPU_BUDGET: std::time::Duration = std::time::Duration::from_millis(1_500);

/// Wide enough that a per-row re-read is a multiple, small enough to stay a unit
/// test.
const LEDGER_ROWS: usize = 40;

/// Process-wide child CPU. This target has exactly one test, so no concurrent
/// test can contribute another child to the measured interval.
fn children_cpu() -> std::time::Duration {
    // SAFETY: `getrusage` writes a fully-initialised `rusage` into the out
    // pointer and reads nothing else; `RUSAGE_CHILDREN` is a valid `who`.
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &raw mut usage) };
    assert_eq!(rc, 0, "getrusage(RUSAGE_CHILDREN)");
    let secs = |t: libc::timeval| {
        std::time::Duration::new(
            u64::try_from(t.tv_sec).expect("non-negative seconds"),
            u32::try_from(t.tv_usec).expect("microseconds fit") * 1_000,
        )
    };
    secs(usage.ru_utime) + secs(usage.ru_stime)
}

/// **The drift column is bounded on CPU and its wall time is recorded.** Half
/// the rows are `off` — the population the column exists for, and the one
/// `verify_rows` skips, so this also proves the added work is the drift join's
/// and not the gate's.
#[test]
fn the_drift_column_costs_one_read_per_pinned_page_not_one_per_row() {
    let s = sandbox();
    let mut requests: Vec<(String, String, String)> = Vec::new();
    for n in 0..LEDGER_ROWS {
        let id = format!("task.rule{n:03}");
        s.write(
            &format!("rules/rule{n:03}.md"),
            &rule_page("hook", &id, &format!("rule {n} body")),
        );
        // Half off, half armed: the off half is invisible to `verify_rows` and
        // visible to the drift join, which is the whole point of the column.
        let mode = if n % 2 == 0 { "off" } else { "armed" };
        requests.push((".".to_owned(), id, mode.to_owned()));
    }
    let borrowed: Vec<(&str, &str, &str)> = requests
        .iter()
        .map(|(root, id, mode)| (root.as_str(), id.as_str(), mode.as_str()))
        .collect();
    arm(&s, &borrowed);

    // Warm the page cache with a throwaway run, then measure the next one.
    let _ = s.run(&["rules"]);
    let cpu_before = children_cpu();
    let wall_start = std::time::Instant::now();
    let out = s.run(&["rules"]);
    let wall = wall_start.elapsed();
    // Cumulative and monotonic; the checked form says so rather than trusting
    // it — an underflow would print as an under-budget PASS.
    let cpu = children_cpu()
        .checked_sub(cpu_before)
        .expect("children CPU is cumulative, so it never goes backwards");

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(
        code,
        0,
        "a clean ledger of {LEDGER_ROWS} rows is no finding: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The measurement is only about the column if the column is actually there.
    let drift_cells = stdout.matches("drift=").count();
    assert_eq!(
        drift_cells, LEDGER_ROWS,
        "one drift cell per ledger row, no more and no fewer: {stdout}"
    );

    eprintln!(
        "rules over a {LEDGER_ROWS}-row ledger ({} off, {} armed): \
         CPU {} ms (budget {} ms), wall {} ms (recorded, NOT gated)",
        LEDGER_ROWS.div_ceil(2),
        LEDGER_ROWS / 2,
        cpu.as_millis(),
        DRIFT_CPU_BUDGET.as_millis(),
        wall.as_millis(),
    );
    assert!(
        cpu < DRIFT_CPU_BUDGET,
        "`mrd rules` over {LEDGER_ROWS} ledger rows burned {} ms of CPU against a \
         {} ms budget. At this magnitude the cause is a SHAPE change — a domain \
         snapshot, an unbounded walk — not the drift join's per-page read, which \
         is counted directly by \
         `policy::armed::tests::drift_reads_each_distinct_pinned_page_once`.",
        cpu.as_millis(),
        DRIFT_CPU_BUDGET.as_millis(),
    );
}
