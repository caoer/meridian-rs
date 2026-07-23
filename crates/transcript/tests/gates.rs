//! U5.2 transcript cross-check — the Test scenario set (the merge gate). The
//! detector is **corroborating, NOT authenticating**: it cross-checks the
//! journal's actor claims against the actor's local, UNAUTHENTICATED session
//! JSONL. Each scenario drives the built-in [`transcript::DirSource`] over REAL
//! on-disk `.jsonl` fixtures in a tempdir, so a missing transcript is a genuinely
//! absent file and a fabricated transcript is a genuinely present forged file.
//! Named results:
//!
//! - `missing_transcript_degrades_honestly` — no transcript for the claimed
//!   actor ⇒ [`Verdict::AttributableNotDetectable`]. No error, no false finding.
//! - `forged_actor_without_transcript_line_flagged` — the actor's transcript IS
//!   present but records NO matching wire call ⇒ [`Verdict::Uncorroborated`], a
//!   flagged finding. The lazy/accidental forge is caught.
//! - `fabricated_transcript_does_not_return_clean_or_verified` — a fabricated
//!   transcript line matching the claim ⇒ [`Verdict::Corroborated`], the stated
//!   ceiling, and NEVER a clean/verified verdict. LOAD-BEARING negative.
//!   FALSIFICATION verified: making the detector return a verified-class verdict
//!   here makes this test FAIL (documented inline).

use std::path::Path;

use receipt::journal::{JournalRow, ParsedRow, parse_rows, render_row};
use transcript::{DirSource, Verdict, cross_check};

/// A tempdir holding per-actor `<actor>.jsonl` transcripts — the [`DirSource`]
/// base. A scenario writes the files it wants present; a MISSING file is simply
/// one never written (honest absence on disk).
fn transcripts(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, content) in files {
        std::fs::write(dir.path().join(name), content).expect("write transcript fixture");
    }
    dir
}

/// One journal row as the engine renders it (U2.1), then parsed back the way the
/// detector reads it — a real journal actor claim, not a hand-built struct.
fn claim(seq: u64, actor: &str, op: &str, path: &str) -> ParsedRow {
    let line = render_row(&JournalRow {
        seq,
        op,
        path,
        actor: Some(actor),
        now: Some("2026-07-23T12:00:00Z"),
        root_before: "b3:aaa",
        root_after: "b3:bbb",
        file: None,
        edits: Vec::new(),
    });
    parse_rows(&line).pop().expect("one parsed row")
}

/// A wire-call transcript line the corroboration contract matches: a JSON object
/// carrying `op` and `path`. This is the shape both an HONEST write and a
/// FABRICATED forge emit — the detector cannot tell them apart (the F6 ceiling).
fn wire_call_line(op: &str, path: &str) -> String {
    format!(r#"{{"session":"alice","op":"{op}","path":"{path}","now":"2026-07-23T12:00:00Z"}}"#)
}

// ---------------------------------------------------------------------------
// Gate 1 — missing transcript degrades honestly ("attributable, not detectable").
// ---------------------------------------------------------------------------

#[test]
fn missing_transcript_degrades_honestly() {
    // The journal claims `alice` spliced notes/plan.md, but NO alice.jsonl exists
    // in the transcript dir — the honest-degrade case.
    let dir = transcripts(&[]);
    let source = DirSource::new(dir.path());
    let rows = [claim(1, "alice", "splice", "notes/plan.md")];

    let report = cross_check(&rows, &source).expect("absent transcript is not a fault");

    let finding = &report.findings[0];
    assert_eq!(
        finding.verdict,
        Verdict::AttributableNotDetectable,
        "no transcript ⇒ attributable, not detectable"
    );
    assert_eq!(finding.verdict.label(), "attributable, not detectable");
    // Honest degrade: not a flagged finding, and the claim is still attributed.
    assert!(
        !finding.verdict.is_finding(),
        "an absent transcript is no false finding"
    );
    assert!(!report.has_findings());
    assert_eq!(finding.actor, "alice", "the row stays attributable");
}

// ---------------------------------------------------------------------------
// Gate 2 — forged actor whose present transcript LACKS the wire call ⇒ flagged.
// ---------------------------------------------------------------------------

#[test]
fn forged_actor_without_transcript_line_flagged() {
    // alice.jsonl EXISTS (alice is a real session with real activity), but it
    // records no wire call to notes/plan.md — the journal row attributing that
    // splice to alice is a LAZY forge. Presence of the transcript turns the
    // missing line into a positive finding.
    let dir = transcripts(&[(
        "alice.jsonl",
        // Real activity, but to a DIFFERENT path — the claimed call is absent.
        &format!(
            "{}\n{}\n",
            wire_call_line("splice", "notes/other.md"),
            "user: hi"
        ),
    )]);
    let source = DirSource::new(dir.path());
    let rows = [claim(7, "alice", "splice", "notes/plan.md")];

    let report = cross_check(&rows, &source).expect("no source fault");

    let finding = &report.findings[0];
    assert_eq!(
        finding.verdict,
        Verdict::Uncorroborated,
        "present transcript without the wire call ⇒ flagged (uncorroborated)"
    );
    assert!(
        finding.verdict.is_finding(),
        "the lazy forge is a positive finding"
    );
    assert!(report.has_findings());
    let flagged = report.flagged();
    assert_eq!(flagged.len(), 1);
    assert_eq!(
        flagged[0].row_anchor, "r-000007",
        "the finding cites the journal row"
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — fabricated transcript ⇒ "corroborated" at the ceiling, NEVER verified.
// LOAD-BEARING. This is the unit's reason to exist.
// ---------------------------------------------------------------------------

/// A deliberate local forger fabricates BOTH the journal row AND a matching
/// transcript line (unauthenticated local JSONL — nothing stops them). The
/// detector finds the matching line and returns [`Verdict::Corroborated`] — its
/// STATED CEILING. It must NOT return any clean/verified verdict: corroboration
/// is never authentication (security F6). The whole detector exists to hold this
/// line honestly rather than over-claim.
///
/// FALSIFICATION (perform by hand, then restore — evidence on the card):
/// add a `Verified` variant to `transcript::Verdict` and make `verdict_for`
/// return it on a present-and-matching transcript. This test then FAILS at the
/// `assert_eq!(finding.verdict, Verdict::Corroborated)` below — the detector
/// would have over-claimed verification over a forgeable local file. Restore →
/// the assertion passes again. That failure is the proof this negative is
/// load-bearing.
#[test]
fn fabricated_transcript_does_not_return_clean_or_verified() {
    // The forger fabricates alice.jsonl WITH a line matching the (forged) claim.
    let dir = transcripts(&[(
        "alice.jsonl",
        &format!("{}\n", wire_call_line("splice", "notes/plan.md")),
    )]);
    let source = DirSource::new(dir.path());
    let rows = [claim(9, "alice", "splice", "notes/plan.md")];

    let report = cross_check(&rows, &source).expect("no source fault");

    let finding = &report.findings[0];
    // The STATED CEILING — corroborated, and only corroborated.
    assert_eq!(
        finding.verdict,
        Verdict::Corroborated,
        "a fabricated matching line reaches corroborated, never past it"
    );
    // NEVER a clean/verified verdict. The vocabulary structurally cannot express
    // one (three variants, none verified); this asserts the reached verdict is
    // the corroborating ceiling, not a finding and not a clean claim.
    assert_eq!(finding.verdict.label(), "corroborated");
    assert!(
        !finding.verdict.is_finding(),
        "corroborated is a ceiling, not a finding — but it is NOT clean/verified either"
    );
}

// ---------------------------------------------------------------------------
// Composite — the three verdicts coexist over one journal, cited per row.
// ---------------------------------------------------------------------------

#[test]
fn one_journal_yields_all_three_verdicts_cited_per_row() {
    let dir = transcripts(&[
        // bob really did the write — corroborated.
        (
            "bob.jsonl",
            &format!("{}\n", wire_call_line("splice", "notes/bob.md")),
        ),
        // carol has a transcript but not this call — flagged.
        ("carol.jsonl", "user: unrelated\n"),
        // dave has no transcript at all — attributable, not detectable.
    ]);
    let source = DirSource::new(dir.path());
    let rows = [
        claim(1, "bob", "splice", "notes/bob.md"),
        claim(2, "carol", "splice", "notes/carol.md"),
        claim(3, "dave", "splice", "notes/dave.md"),
    ];

    let report = cross_check(&rows, &source).expect("no source fault");
    assert_eq!(report.findings.len(), 3, "one finding per actor-claim row");
    assert_eq!(report.findings[0].verdict, Verdict::Corroborated);
    assert_eq!(report.findings[1].verdict, Verdict::Uncorroborated);
    assert_eq!(
        report.findings[2].verdict,
        Verdict::AttributableNotDetectable
    );
    // Only the middle row is a flagged finding.
    assert_eq!(report.flagged().len(), 1);
    assert_eq!(report.flagged()[0].actor, "carol");
    // The tempdir path never leaked into DirSource resolution.
    assert!(Path::new(dir.path()).exists());
}
