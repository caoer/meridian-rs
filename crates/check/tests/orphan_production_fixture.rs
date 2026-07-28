//! The production fixture for the orphan lint (G3).
//!
//! A lint that passes synthetic fixtures and misses the known production
//! orphans is worthless, so this pins the lint against the REAL anchor sequence
//! from `field-notes/receipts/run.md` as it stood at mrd `1333af61` — the commit
//! that first wrote completion receipts.
//!
//! That page is the defect era's own record: five invocations authorised by
//! builds that could not write a completion for an effect-free run, then one
//! invocation from the build that can. The boundary is visible in the bytes,
//! which is exactly what the lint reads.
//!
//! The anchors are copied verbatim; the JSON bodies are elided to the fields
//! that matter, because the lint joins on the anchor and must not depend on
//! payload shape.

use check::orphan::{orphaned_runs, render};
use model::Document;
use std::collections::BTreeMap;

/// The real sequence, in page order, from field-notes 2026-07-28.
const PRODUCTION_RECEIPTS: &str = "\
- run {\"page\":\"health/runtime.md\",\"task\":\"check\"} ^p-run-1785255857862-35849
- run {\"page\":\"health/runtime.md\",\"task\":\"check\"} ^p-run-1785255875874-36556
- run {\"page\":\"health/runtime.md\",\"task\":\"check\"} ^p-run-1785255961018-38094
- run {\"page\":\"health/runtime.md\",\"task\":\"check\"} ^p-run-1785268790900-93356
- run {\"page\":\"health/runtime.md\",\"task\":\"check\"} ^p-run-1785268805357-94789
- run {\"page\":\"LLM_WIKI.md\",\"task\":\"load-skill\"} ^p-run-1785273287426-33159
- run {\"page\":\"LLM_WIKI.md\",\"task\":\"load-skill\"} ^r-run-1785273287426-33159
";

fn corpus() -> BTreeMap<String, Document> {
    let raw = PRODUCTION_RECEIPTS.to_owned();
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    BTreeMap::from([("receipts/run.md".to_owned(), doc)])
}

/// The gate: every pre-marker invocation is flagged, the post-marker one is
/// spared. Not "flag one and spare one" — the whole era, by the dated boundary.
#[test]
fn the_lint_flags_the_five_known_orphans_and_spares_the_completed_run() {
    let found = orphaned_runs(&corpus());
    let ids: Vec<&str> = found.iter().map(|o| o.invocation.as_str()).collect();

    assert_eq!(
        ids,
        vec![
            "1785255857862-35849",
            "1785255875874-36556",
            "1785255961018-38094",
            "1785268790900-93356",
            "1785268805357-94789",
        ],
        "the five pre-marker invocations are orphans"
    );
    assert!(
        !ids.contains(&"1785273287426-33159"),
        "the invocation with a completion receipt must never be flagged"
    );
}

/// One of those five GENUINELY SUCCEEDED — `1785268805357-94789` is the
/// `health/runtime.md` run that reported eight live toolchain drifts. It is
/// still flagged, and that is the correct verdict: the bytes cannot prove it
/// finished, and a lint that excused it would assert what the record does not
/// support.
#[test]
fn a_run_that_succeeded_without_recording_it_is_still_an_orphan() {
    let found = orphaned_runs(&corpus());
    let succeeded = found
        .iter()
        .find(|o| o.invocation == "1785268805357-94789")
        .expect("the successful pre-marker run is flagged like any other");
    assert!(
        succeeded.before_completions_existed,
        "it belongs to the era that could not record completion"
    );
}

/// All five predate the page's first completion receipt, so all five render as
/// unauditable-by-construction — and none render as a live incident, because
/// nothing on this page went missing after the writer could record it.
#[test]
fn the_whole_defect_era_renders_as_unauditable_not_as_live_incidents() {
    let found = orphaned_runs(&corpus());
    assert!(
        found.iter().all(|o| o.before_completions_existed),
        "every orphan here predates the first completion receipt"
    );

    let rendered = render(&found).expect("five orphans render");
    assert!(rendered.contains("5 orphaned"), "{rendered}");
    assert!(rendered.contains("5 unauditable-by-construction"), "{rendered}");
    assert!(rendered.contains("0 live"), "{rendered}");
    assert!(
        rendered.contains("will never clear"),
        "the reader is told these cannot be fixed, so a standing red is not \
         mistaken for neglect: {rendered}"
    );
    // The verdict does not soften: every invocation is still named.
    for id in [
        "1785255857862-35849",
        "1785268805357-94789",
    ] {
        assert!(rendered.contains(id), "{id} must be named in the render");
    }
}

/// The boundary is derived from the page, not from a build id — so a later
/// orphan, after completions exist, reads as a LIVE incident on the same page.
#[test]
fn an_orphan_after_the_boundary_is_a_live_incident() {
    let raw = format!("{PRODUCTION_RECEIPTS}- run {{}} ^p-run-1785299999999-00001\n");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let docs = BTreeMap::from([("receipts/run.md".to_owned(), doc)]);

    let found = orphaned_runs(&docs);
    let live: Vec<&str> = found
        .iter()
        .filter(|o| !o.before_completions_existed)
        .map(|o| o.invocation.as_str())
        .collect();
    assert_eq!(
        live,
        vec!["1785299999999-00001"],
        "an orphan appearing after a completion is a real incident"
    );

    let rendered = render(&found).expect("renders");
    assert!(rendered.contains("1 live"), "{rendered}");
}
