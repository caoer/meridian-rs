//! U5.2 — **transcript cross-check**: a CORROBORATING (never authenticating)
//! detector over the journal's actor claims (d3 §3; checklist 13; security F6).
//!
//! # What this detects, and what it cannot
//! The journal (U2.1) records `actor` on every guarded write "exactly as given,
//! never invented" ([`receipt::journal::ParsedRow::actor`]). On a single-uid box
//! that actor is a **hygiene-grade** fact, not a security-grade one: a local
//! process holding the socket can assert a valid-looking session id (the L1
//! ceiling, d3 §3). Two forge classes, two answers:
//! - a **doctored verdict** (rev mismatch) is caught by board/replay — not this
//!   detector's job;
//! - a **well-formed actor forge** (a valid reviewer string, a clean receipt) is
//!   indistinguishable from a real close by board/replay alone. The ONLY handle
//!   is the claimed actor's immutable local session transcript
//!   (`~/.local/share/ucc/shared/projects/<slug>/<session-id>.jsonl`): a real
//!   write leaves the corresponding wire call in that JSONL; a lazy forge does
//!   not.
//!
//! # Corroborating, NOT authenticating — the stated ceiling
//! The transcript is **unauthenticated local JSONL**. A deliberate local forger
//! can fabricate a matching line, so a present-and-matching transcript raises the
//! journal claim only to [`Verdict::Corroborated`] — **never** "clean" or
//! "verified". That ceiling is load-bearing and structural: this crate's verdict
//! vocabulary ([`Verdict`]) has no clean/verified member to return. What the
//! detector catches is the **lazy/accidental forge** (a claim whose actor DID
//! keep a transcript, but that transcript has no matching wire call —
//! [`Verdict::Uncorroborated`]). No transcript at all is an honest degrade
//! ([`Verdict::AttributableNotDetectable`]): the row is attributable (it names an
//! actor) but not detectable (no immutable transcript to cross-check).
//!
//! # The corroboration contract (what a matching transcript line is)
//! A transcript line **corroborates** a journal row iff it is a JSON object that
//! records a wire call to the row's target: a string `op` equal to the row's op
//! AND a string `path` equal to the row's path ([`line_corroborates`]). The
//! transcript is that actor's own JSONL, so the `(op, path)` pair is the wire
//! call's identity within it. A non-JSON prose line never corroborates.
//!
//! # Shape
//! [`cross_check`] is a PURE fold over parsed journal rows and a [`Source`]
//! (transcript lookup abstracted, exactly as `attest`'s `RealisedGate` abstracts
//! the board read). [`DirSource`] is the built-in on-disk instance: it resolves
//! `<base>/<actor>.jsonl`, treating a missing file as [`Lookup::Absent`].

use std::path::PathBuf;

use receipt::journal::ParsedRow;

/// The cross-check verdict for one journal actor claim. There are exactly three
/// — and, deliberately, **no clean/verified member**: an unauthenticated local
/// transcript can never authenticate the claim, only corroborate it. That
/// absence is the security ceiling (F6), stated in the type itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A transcript line records the wire call the journal row claims. **THE
    /// CEILING** — the strongest verdict the detector can reach, and explicitly
    /// NOT "clean"/"verified": a deliberate local forger could have fabricated
    /// the very line that corroborates here (unauthenticated local JSONL).
    Corroborated,
    /// The claimed actor's transcript IS present, but records no matching wire
    /// call. Presence turns absence-of-the-line into a positive finding — the
    /// lazy/accidental forge this detector exists to catch.
    Uncorroborated,
    /// No transcript for the claimed actor — an honest degrade. The row is
    /// attributable (it names an actor) but not detectable (nothing immutable to
    /// cross-check). Never an error, never a false finding.
    AttributableNotDetectable,
}

impl Verdict {
    /// A positive finding an operator must act on — TRUE only for
    /// [`Verdict::Uncorroborated`]. Corroboration is a ceiling, not a finding;
    /// an absent transcript is an honest degrade, not a finding.
    #[must_use]
    pub fn is_finding(self) -> bool {
        matches!(self, Verdict::Uncorroborated)
    }

    /// The verdict's render label — the plan's wording verbatim.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Corroborated => "corroborated",
            Verdict::Uncorroborated => "uncorroborated",
            Verdict::AttributableNotDetectable => "attributable, not detectable",
        }
    }
}

/// What a [`Source`] yields for one actor: the actor's raw transcript JSONL, or
/// its honest absence. Absence is a verdict input, never an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// No transcript exists for this actor.
    Absent,
    /// The actor's transcript, as raw JSONL text (one wire event per line).
    Present(String),
}

/// A transcript lookup faulted — a PRESENT transcript that could not be read
/// (permissions, I/O). Distinct from a clean [`Lookup::Absent`], which is a
/// verdict input, not a fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    /// The underlying reason (the path and the I/O error).
    pub reason: String,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transcript lookup failed: {}", self.reason)
    }
}

impl std::error::Error for SourceError {}

/// The transcript lookup the detector reads — a PURE observation (no write, no
/// capability). The built-in [`DirSource`] is one instance; a test supplies its
/// own in-memory instance.
pub trait Source {
    /// Yield the transcript for `actor`, or its honest absence.
    ///
    /// # Errors
    /// [`SourceError`] when the lookup itself faults — a present transcript that
    /// cannot be read. A missing transcript is [`Lookup::Absent`], not an error.
    fn lookup(&self, actor: &str) -> Result<Lookup, SourceError>;
}

/// The built-in on-disk [`Source`]: resolves `<base>/<actor>.jsonl`. In the
/// deployed layout `base` is the session's project dir
/// (`.../shared/projects/<slug>/`) and the actor IS the joined-session id (d3
/// §3); the `<slug>` layer is the caller's, the detector keys by actor.
#[derive(Debug, Clone)]
pub struct DirSource {
    /// The directory holding the per-actor `<actor>.jsonl` transcripts.
    pub base: PathBuf,
}

impl DirSource {
    /// A source over the transcript directory `base`.
    #[must_use]
    pub fn new(base: impl Into<PathBuf>) -> Self {
        DirSource { base: base.into() }
    }
}

impl Source for DirSource {
    fn lookup(&self, actor: &str) -> Result<Lookup, SourceError> {
        // Path-safety: an actor is a session-id filename component. A value with
        // a separator, a `.`/`..` traversal, or emptiness cannot name a
        // transcript here — treat it as Absent (attributable, not detectable),
        // never as a read outside `base`.
        if !is_safe_actor(actor) {
            return Ok(Lookup::Absent);
        }
        let path = self.base.join(format!("{actor}.jsonl"));
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Lookup::Present(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lookup::Absent),
            Err(e) => Err(SourceError {
                reason: format!("read transcript '{}': {e}", path.display()),
            }),
        }
    }
}

/// Whether `actor` is a single safe filename component (no separator, no
/// `.`/`..` traversal, non-empty) — the [`DirSource`] path-safety guard.
fn is_safe_actor(actor: &str) -> bool {
    !actor.is_empty()
        && actor != "."
        && actor != ".."
        && !actor.contains('/')
        && !actor.contains('\\')
        && !actor.contains('\0')
}

/// One journal actor claim's cross-check outcome, citing the journal row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The journal row's block anchor (e.g. `r-000042`) — the citation.
    pub row_anchor: String,
    /// The claimed actor cross-checked.
    pub actor: String,
    /// The row's guarded op.
    pub op: String,
    /// The content target the write claimed to land on.
    pub path: String,
    /// The cross-check verdict.
    pub verdict: Verdict,
}

/// The cross-check report over a journal — one [`Finding`] per row that CARRIES
/// an actor claim (a row with no recorded actor has no claim to cross-check and
/// is skipped, never invented into one).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Every actor-claim finding, in journal order.
    pub findings: Vec<Finding>,
}

impl Report {
    /// The flagged findings — the lazy/accidental forges
    /// ([`Verdict::Uncorroborated`]). Empty ⇔ no claim was contradicted by a
    /// present transcript.
    #[must_use]
    pub fn flagged(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| f.verdict.is_finding())
            .collect()
    }

    /// At least one claim was flagged — a present transcript lacked the claimed
    /// wire call.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        self.findings.iter().any(|f| f.verdict.is_finding())
    }
}

/// Whether a transcript `line` corroborates a wire call to `path` under `op`
/// (the corroboration contract): the line parses as a JSON object carrying a
/// string `op` equal to `op` AND a string `path` equal to `path`. A non-JSON
/// prose line, or one missing either field, does not corroborate.
///
/// This is corroboration, never authentication: any local writer can emit a
/// line satisfying this predicate (the F6 ceiling) — matching raises the claim
/// to [`Verdict::Corroborated`], never past it.
#[must_use]
pub fn line_corroborates(line: &str, op: &str, path: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return false;
    };
    value.get("op").and_then(serde_json::Value::as_str) == Some(op)
        && value.get("path").and_then(serde_json::Value::as_str) == Some(path)
}

/// The per-claim verdict: look the actor's transcript up, then decide.
fn verdict_for(
    actor: &str,
    op: &str,
    path: &str,
    source: &dyn Source,
) -> Result<Verdict, SourceError> {
    match source.lookup(actor)? {
        Lookup::Absent => Ok(Verdict::AttributableNotDetectable),
        Lookup::Present(text) => {
            if text.lines().any(|l| line_corroborates(l, op, path)) {
                Ok(Verdict::Corroborated)
            } else {
                Ok(Verdict::Uncorroborated)
            }
        }
    }
}

/// **Cross-check the journal's actor claims** (d3 §3): for every parsed row that
/// carries an actor, corroborate it against that actor's local transcript and
/// cite the outcome. A row without a recorded actor carries no claim and is
/// skipped. This is a corroborating detector — its strongest verdict is
/// [`Verdict::Corroborated`]; it never returns a clean/verified verdict, because
/// [`Verdict`] has none to return.
///
/// # Errors
/// [`SourceError`] when a transcript lookup faults (a present transcript that
/// cannot be read). An absent transcript is not an error — it degrades to
/// [`Verdict::AttributableNotDetectable`].
pub fn cross_check(rows: &[ParsedRow], source: &dyn Source) -> Result<Report, SourceError> {
    let mut findings = Vec::new();
    for row in rows {
        let Some(actor) = row.actor.as_deref() else {
            continue;
        };
        let verdict = verdict_for(actor, &row.op, &row.path, source)?;
        findings.push(Finding {
            row_anchor: row.anchor.clone(),
            actor: actor.to_string(),
            op: row.op.clone(),
            path: row.path.clone(),
            verdict,
        });
    }
    Ok(Report { findings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// An in-memory [`Source`] — actor → transcript text, or absence when the
    /// actor is not a key. Keeps the pure-detector unit tests off the disk.
    struct MapSource(HashMap<String, String>);

    impl Source for MapSource {
        fn lookup(&self, actor: &str) -> Result<Lookup, SourceError> {
            Ok(match self.0.get(actor) {
                Some(text) => Lookup::Present(text.clone()),
                None => Lookup::Absent,
            })
        }
    }

    fn row(seq: u64, actor: Option<&str>, op: &str, path: &str) -> ParsedRow {
        ParsedRow {
            anchor: format!("r-{seq:06}"),
            op: op.to_string(),
            path: path.to_string(),
            actor: actor.map(str::to_string),
            now: None,
            root_before: "b3:0".to_string(),
            root_after: "b3:1".to_string(),
            edits: 0,
            line_no: usize::try_from(seq).expect("seq fits usize"),
        }
    }

    /// A wire-call transcript line the corroboration contract matches.
    fn wire_line(op: &str, path: &str) -> String {
        format!(r#"{{"op":"{op}","path":"{path}","now":"2026-07-23T12:00:00Z"}}"#)
    }

    #[test]
    fn contract_matches_op_and_path_only() {
        let line = wire_line("splice", "notes/plan.md");
        assert!(line_corroborates(&line, "splice", "notes/plan.md"));
        // Wrong op or wrong path does not corroborate.
        assert!(!line_corroborates(&line, "create", "notes/plan.md"));
        assert!(!line_corroborates(&line, "splice", "notes/other.md"));
    }

    #[test]
    fn prose_line_never_corroborates() {
        assert!(!line_corroborates(
            "the agent spliced notes/plan.md",
            "splice",
            "notes/plan.md"
        ));
        assert!(!line_corroborates("", "splice", "notes/plan.md"));
        // A JSON object missing a field does not corroborate.
        assert!(!line_corroborates(
            r#"{"op":"splice"}"#,
            "splice",
            "notes/plan.md"
        ));
    }

    #[test]
    fn present_and_matching_is_corroborated_not_verified() {
        let mut m = HashMap::new();
        m.insert(
            "alice".to_string(),
            format!(
                "{}\n{}\n",
                wire_line("splice", "notes/plan.md"),
                "some prose"
            ),
        );
        let report = cross_check(
            &[row(1, Some("alice"), "splice", "notes/plan.md")],
            &MapSource(m),
        )
        .expect("no source fault");
        assert_eq!(report.findings[0].verdict, Verdict::Corroborated);
        // The ceiling: the strongest verdict is Corroborated — the vocabulary has
        // no clean/verified member. `is_finding` is false (a ceiling, not a find).
        assert!(!report.findings[0].verdict.is_finding());
    }

    #[test]
    fn present_without_line_is_uncorroborated_and_flagged() {
        let mut m = HashMap::new();
        m.insert(
            "alice".to_string(),
            "unrelated prose\nno wire calls\n".to_string(),
        );
        let report = cross_check(
            &[row(1, Some("alice"), "splice", "notes/plan.md")],
            &MapSource(m),
        )
        .expect("no source fault");
        assert_eq!(report.findings[0].verdict, Verdict::Uncorroborated);
        assert!(report.has_findings());
        assert_eq!(report.flagged().len(), 1);
    }

    #[test]
    fn absent_transcript_degrades_honestly() {
        let report = cross_check(
            &[row(1, Some("ghost"), "splice", "notes/plan.md")],
            &MapSource(HashMap::new()),
        )
        .expect("absence is not a fault");
        assert_eq!(
            report.findings[0].verdict,
            Verdict::AttributableNotDetectable
        );
        assert!(!report.has_findings());
    }

    #[test]
    fn actorless_row_carries_no_claim() {
        let report = cross_check(
            &[row(1, None, "splice", "notes/plan.md")],
            &MapSource(HashMap::new()),
        )
        .expect("no source fault");
        assert!(
            report.findings.is_empty(),
            "a row with no actor has no claim to cross-check"
        );
    }

    #[test]
    fn dir_source_guards_path_traversal() {
        let src = DirSource::new("/tmp/does-not-matter");
        // A traversal actor cannot escape base — it degrades to Absent.
        assert_eq!(
            src.lookup("../../etc/passwd").expect("guarded"),
            Lookup::Absent
        );
        assert_eq!(src.lookup("a/b").expect("guarded"), Lookup::Absent);
        assert_eq!(src.lookup("").expect("guarded"), Lookup::Absent);
    }
}
