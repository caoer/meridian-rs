//! `Phase::stop_as` reports under the name given AT THE STOP, and a span that
//! is never stopped still reports NOTHING.
//!
//! **Why this is an integration test and not a unit test.** The sink is a
//! process-wide `OnceLock` resolved from `MRD_TIMING` on first use, so a test
//! that wants to read the emitted bytes has to win that resolution. A `tests/`
//! file is its own binary, hence its own process: set the variable before the
//! first timing call and the sink is ours. Everything that observes the sink
//! therefore lives in ONE `#[test]`, because two of them would race the
//! `OnceLock` and the loser would read someone else's file.
//!
//! **What this test deliberately does NOT claim.** It says nothing about
//! whether the mode being OFF reads a clock. That property is not observable
//! from the emitted bytes — `false.then(Instant::now)` and an eager read that
//! is thrown away both leave `started: None` and both emit nothing — so an
//! assertion here would be a tautology wearing the name of a guard.
//!
//! **That property IS guarded, and not here.** The unit test
//! `an_off_phase_reads_no_clock` in `src/lib.rs` counts the clock reads routed
//! through the crate's `now()` and asserts the count does not move while the
//! mode is off. It reaches the property by counting a COST instead of
//! inspecting a VALUE — which is precisely why the emitted bytes this file
//! reads cannot see it, and why the assertion belongs there rather than here.

use std::path::PathBuf;

/// The sink for this process: one file, named for the pid so a stale one from
/// another run can never be read as this run's evidence.
fn sink_path() -> PathBuf {
    std::env::temp_dir().join(format!("mrd-timing-stop-as-{}.log", std::process::id()))
}

/// The phase names on the lane, in emission order. The line grammar is
/// `mrd-timing cmd=<c> who=<w> phase=<p> us=<n>`, space-separated.
fn phases(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            line.split(' ')
                .find_map(|field| field.strip_prefix("phase="))
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn stop_as_reports_the_stop_site_name_and_an_unstopped_span_reports_nothing() {
    let sink = sink_path();
    let _ = std::fs::remove_file(&sink);
    // SAFETY: set before this process makes its first timing call, and this is
    // the only test in this binary that touches the sink — nothing else can be
    // reading the environment concurrently.
    unsafe {
        std::env::set_var(timing::SWITCH, &sink);
    }

    // The measurement under test: constructed as `a`, stopped as `b`.
    timing::phase("probe.constructed_a").stop_as("probe.stopped_b");

    // The CONTROL, in the same invocation: plain `stop` still names at
    // construction. Without it, an empty result for `probe.constructed_a`
    // below would be equally consistent with "the sink never opened" — the
    // control is what makes the absence a finding instead of a receipt about
    // the instrument.
    timing::phase("probe.control").stop();

    // The no-`Drop` contract, at the site this PR changes: a span that reaches
    // neither `stop` nor `stop_as` says nothing at all. This is what makes an
    // emitted line mean "this arm ran", so the refused arm has to be stopped
    // ON PURPOSE to be counted — it can never leak in by being dropped.
    let abandoned = timing::phase("probe.abandoned");
    drop(abandoned);

    let raw = std::fs::read_to_string(&sink).expect("the timing sink was written");
    let seen = phases(&raw);

    assert!(
        seen.contains(&"probe.control".to_owned()),
        "the control never reported, so this run proves nothing about the other two — the \
         sink did not open or the mode did not turn on. Lane:\n{raw}"
    );
    assert!(
        seen.contains(&"probe.stopped_b".to_owned()),
        "`stop_as` did not report under the name given at the stop. Phases seen: {seen:?}\n\
         Lane:\n{raw}"
    );
    assert!(
        !seen.contains(&"probe.constructed_a".to_owned()),
        "`stop_as` reported under the CONSTRUCTION name as well as, or instead of, the stop \
         name — the whole point of the widening is that the name is decided by the branch \
         actually taken. Phases seen: {seen:?}\nLane:\n{raw}"
    );
    assert!(
        !seen.contains(&"probe.abandoned".to_owned()),
        "a span that was never stopped reported anyway — the no-`Drop` contract is broken, and \
         with it the rule that a line means the arm ran. Phases seen: {seen:?}\nLane:\n{raw}"
    );

    let _ = std::fs::remove_file(&sink);
}
