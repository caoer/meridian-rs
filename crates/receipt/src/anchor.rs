//! The anchor law + two-badge freshness (d2 §2.3 v3 W-C1; d3 §1.5).
//!
//! # What the anchor is, and the one law it exists to hold
//! The freshness axis is two-sided. Tip equality — the object id of local
//! `HEAD` versus the local remote-tracking ref `origin/<branch>` — is
//! *mechanical and local* ([`TipPosition`], a source-2 fact). But the **anchor**
//! — the engine's local knowledge of origin's refs — carries its **own trust
//! state**, because a restored backup restores `.git` *including the local copy
//! of origin's refs*. A stale local `origin/<branch>` therefore makes the
//! mechanical compare say `at-tip` while the world has moved on.
//!
//! **The load-bearing law (W-C1): no surface may render a bare `at-tip` /
//! `behind` from the local remote-tracking ref.** Only an anchor the *current
//! run* verified against origin may render the tip axis bare. Every other
//! render carries a qualifier that names exactly how stale the knowledge is.
//! [`render_tip_axis`] is the sole renderer, and it produces a bare axis from
//! the [`AnchorState::Verified`] arm alone — the invariant is structural, not a
//! convention.
//!
//! # A "run" is one engine invocation
//! One verb execution or one wire call; the facts computed within it share one
//! moment ([`AnchorState::classify`] takes that moment as `now_unix`).
//!
//! # The three anchor states (qualifier mandatory unless verified)
//! - [`AnchorState::Verified`] — the run ITSELF observed origin. Exactly one
//!   door produces it: `realise` executing a fetch observe-class claim (net
//!   cap, customer-triggered — the engine never fetches implicitly). Verified
//!   is a **moment, never a stored state**: persisted evidence always decays to
//!   as-known, so this state is reachable only with [`AnchorFacts::run_observed`]
//!   set true for THIS run. Only Verified renders bare `at-tip` / `behind`;
//!   `status` is cap-free and so NEVER reaches it.
//! - [`AnchorState::AsKnownAged`] — the last origin observation is a journaled
//!   fetch-claim receipt: renders `at-tip (anchor as-known, observed <now>,
//!   ~<age>)`, the age sourced from the receipt's `now` (source 3,
//!   chain-protected). A restore replays it only with **visibly growing age**
//!   and cannot re-mint recency without a journaled write.
//! - [`AnchorState::AsKnownAgeless`] — the local origin ref is present but no
//!   journaled observation dates it (an out-of-engine `git fetch` is a world
//!   act the ledger cannot verify). Renders `at-tip (anchor as-known)` forever,
//!   plus the [`AGELESS_NUDGE`] hint to run the fetch through the engine door.
//!   `FETCH_HEAD` / reflog mtimes are dropped entirely — they are never a
//!   dater.
//! - [`AnchorState::Unverified`] — never fetched, or anchor facts absent (no
//!   local origin ref at all): renders `at-tip (anchor unverified)`.
//!
//! # The two-badge fusion (d3 §1.5, fused ON TOP)
//! A pin's color is computed per validity axis and the board badges the axis:
//! the **owned axis** ([`OwnedBadge`], content-equality — practical, not
//! cryptographic) and the **freshness axis** ([`FreshnessBadge`], the pointed
//! 5th axis). The board renders **two badges, never one merged color**
//! ([`TwoBadge`]). The anchor law fuses onto the freshness badge:
//! [`freshness_badge`] returns green ONLY for a verified, at-tip anchor — an
//! as-known / unverified / behind anchor is grey. This is the invariant both
//! probes agree on: **a local-only or restored pin never reads as pointed-fresh
//! green.** Where a single glyph is needed, freshness-grey dominates a pointed
//! claim (fail-safe — show the weakest asserted axis; [`TwoBadge::dominant`]).

use std::time::Duration;

use crate::journal::ParsedRow;

/// The mechanical tip position: the object id of local `HEAD` versus the local
/// remote-tracking ref `origin/<branch>` (a source-2 rev comparison).
///
/// This is LOCAL and MECHANICAL. It says **nothing** about whether the local
/// remote-tracking ref is fresh — that is the anchor's job. Rendering this
/// position without an anchor qualifier is the exact W-C1 violation this module
/// forbids; always pair it with an [`AnchorState`] through [`render_tip_axis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipPosition {
    /// Local `HEAD` equals the local `origin/<branch>` object id.
    AtTip,
    /// Local `HEAD` differs from the local `origin/<branch>` object id.
    Behind,
}

impl TipPosition {
    /// The bare axis word — the render's stem before any anchor qualifier.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            TipPosition::AtTip => "at-tip",
            TipPosition::Behind => "behind",
        }
    }
}

/// The last journaled fetch-claim observation the anchor is dated by. `now` is
/// the receipt's timestamp token, rendered verbatim (source-3, recorded exactly
/// as given). `unix` is that token parsed to epoch seconds — the age arithmetic
/// coordinate. The caller (the status surface / the realise fetch-claim path)
/// derives `unix` from `now`; this leaf crate never parses dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// The receipt's `now` token, rendered verbatim in the `observed <now>`
    /// clause.
    pub now: String,
    /// `now` parsed to epoch seconds — the age is `run_now_unix - unix`.
    pub unix: i64,
}

/// The origin-observation facts the status surface gathers for one run. The
/// anchor law is computed from THESE — never from the freshness of the local
/// remote-tracking ref (a restored backup carries a stale copy of it).
///
/// Checking whether the origin ref *exists* ([`AnchorFacts::origin_ref_present`])
/// is allowed and required — it splits as-known-ageless from unverified. What is
/// forbidden is trusting that ref's *recency*; that trust comes only from
/// [`AnchorFacts::run_observed`] or a journaled observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorFacts {
    /// True iff THIS run performed the origin observation — the one verified
    /// door (`realise` fetch observe-claim). `status`, being cap-free, always
    /// passes false.
    pub run_observed: bool,
    /// The most recent journaled fetch-claim observation, if any. The ONLY
    /// dater of the anchor.
    pub last_observation: Option<Observed>,
    /// Whether the local remote-tracking ref (`origin/<branch>`) is present at
    /// all. Absent ⇒ anchor facts absent ⇒ unverified. Present with no
    /// journaled observation ⇒ as-known AGELESS (an out-of-engine fetch).
    pub origin_ref_present: bool,
}

/// The three anchor states (d2 §2.3 v3). The qualifier is mandatory unless
/// [`AnchorState::Verified`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorState {
    /// The run itself observed origin — the only state that renders a bare tip
    /// axis. A moment, never stored.
    Verified,
    /// A journaled fetch-claim receipt dates the anchor. Age grows as the run
    /// moment advances past the receipt's `now`.
    AsKnownAged {
        /// The receipt's `now` token, rendered verbatim.
        observed: String,
        /// The age at the run moment (`run_now_unix - observed.unix`, clamped
        /// at zero — a receipt cannot be observed in the future).
        age: Duration,
    },
    /// The local origin ref is present but undated by any journaled
    /// observation (an out-of-engine `git fetch`). Renders `anchor as-known`
    /// forever, plus [`AGELESS_NUDGE`].
    AsKnownAgeless,
    /// Never fetched, or anchor facts absent (no local origin ref).
    Unverified,
}

impl AnchorState {
    /// Classify the anchor from the run's gathered [`AnchorFacts`] and its one
    /// moment `now_unix` (epoch seconds). The precedence is the state ladder:
    /// a run-observed anchor is verified; else a journaled observation dates it
    /// as-known-aged; else a present-but-undated origin ref is as-known-ageless;
    /// else unverified.
    ///
    /// The local remote-tracking ref's *object id* never enters this
    /// classification — only whether it *exists*. That is the W-C1 guard: a
    /// restored `.git` cannot promote the anchor past as-known.
    #[must_use]
    pub fn classify(facts: &AnchorFacts, now_unix: i64) -> AnchorState {
        if facts.run_observed {
            return AnchorState::Verified;
        }
        if let Some(obs) = &facts.last_observation {
            let secs = now_unix.saturating_sub(obs.unix).max(0);
            let age = Duration::from_secs(u64::try_from(secs).unwrap_or(0));
            return AnchorState::AsKnownAged {
                observed: obs.now.clone(),
                age,
            };
        }
        if facts.origin_ref_present {
            return AnchorState::AsKnownAgeless;
        }
        AnchorState::Unverified
    }

    /// Whether this state is allowed to render the tip axis BARE (no
    /// qualifier). True for [`AnchorState::Verified`] alone — the structural
    /// witness of the W-C1 invariant.
    #[must_use]
    pub fn renders_bare_axis(&self) -> bool {
        matches!(self, AnchorState::Verified)
    }
}

/// The agent-native nudge shown beside an as-known AGELESS anchor: an
/// out-of-engine `git fetch` cannot be dated, so run the fetch through the
/// engine door (`realise` fetch observe-claim) to mint a journaled, dated
/// observation.
pub const AGELESS_NUDGE: &str = "anchor as-known but undated — an out-of-engine `git fetch` cannot be dated; \
     run the fetch through the engine (realise fetch observe-claim) to mint a dated observation";

/// The one journal `op` a fetch observe-claim receipt records (the seam the
/// realise fetch-claim path writes through the strict writer, and that
/// [`last_observation`] reads back). Kept as one named constant so the observe
/// row and the anchor's reader never drift.
pub const OBSERVE_OP: &str = "observe";

/// Render the anchor-qualified tip axis (d2 §2.3 v3, the sole renderer).
///
/// A bare axis (`at-tip` / `behind`, no qualifier) is produced by the
/// [`AnchorState::Verified`] arm **alone** — this is the structural guarantee
/// of the W-C1 law: no as-known or unverified anchor, and therefore no local
/// remote-tracking ref, can ever mint a bare tip axis.
#[must_use]
pub fn render_tip_axis(tip: TipPosition, anchor: &AnchorState) -> String {
    let word = tip.word();
    match anchor {
        AnchorState::Verified => word.to_string(),
        AnchorState::AsKnownAged { observed, age } => {
            format!(
                "{word} (anchor as-known, observed {observed}, ~{})",
                human_age(*age)
            )
        }
        AnchorState::AsKnownAgeless => format!("{word} (anchor as-known)"),
        AnchorState::Unverified => format!("{word} (anchor unverified)"),
    }
}

/// The hint text to render beside a tip axis, if any. Only an as-known AGELESS
/// anchor carries one — the [`AGELESS_NUDGE`].
#[must_use]
pub fn nudge_hint(anchor: &AnchorState) -> Option<&'static str> {
    match anchor {
        AnchorState::AsKnownAgeless => Some(AGELESS_NUDGE),
        _ => None,
    }
}

/// Find the `now` token of the most recent journaled fetch observe-claim
/// receipt (`op == OBSERVE_OP`) in page-ordered journal rows — the anchor's
/// dater. Composes [`crate::journal::parse_rows`] output; it never reparses the
/// page. Returns `None` when the journal carries no observation (the anchor is
/// then as-known-ageless or unverified per [`AnchorFacts::origin_ref_present`]).
#[must_use]
pub fn last_observation(rows: &[ParsedRow]) -> Option<&str> {
    rows.iter()
        .rev()
        .find(|r| r.op == OBSERVE_OP)
        .and_then(|r| r.now.as_deref())
}

/// A compact, honest (`~`-prefixed at the call site) age: whole units, coarsest
/// that fits. Sub-minute reads in seconds; then minutes, hours, days. Never
/// claims sub-second precision — the anchor age is always approximate.
#[must_use]
pub fn human_age(age: Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

// ---------------------------------------------------------------------------
// two-badge freshness (d3 §1.5, fused on the anchor law)
// ---------------------------------------------------------------------------

/// The owned axis (content-equality) badge — green iff live bytes hash to the
/// pinned rev. Owned enforcement is practical, not cryptographic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedBadge {
    /// Live bytes hash to the pinned rev.
    Green,
    /// Live bytes drifted from the pinned rev.
    Red,
    /// The ledger cannot verify the owned axis (declared-only / unmanaged).
    Grey,
}

impl OwnedBadge {
    /// The render label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            OwnedBadge::Green => "green",
            OwnedBadge::Red => "red",
            OwnedBadge::Grey => "grey",
        }
    }
}

/// The freshness axis (pointed, 5th) badge — the anchor law fuses here. Green
/// ONLY when a verified anchor confirms on-origin; never red (nothing drifted),
/// grey for every unconfirmed anchor (the false-green invariant). Produced by
/// [`freshness_badge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessBadge {
    /// A verified, at-tip anchor confirms the pin is on origin.
    Green,
    /// The freshness claim is unconfirmed: as-known / unverified / behind, or a
    /// local-only (unpushed) pin.
    Grey,
}

impl FreshnessBadge {
    /// The render label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FreshnessBadge::Green => "green",
            FreshnessBadge::Grey => "grey",
        }
    }
}

/// Fuse the anchor law onto the freshness badge (d3 §1.5 fused on d2 §2.3 v3).
///
/// A green freshness badge REQUIRES a verified anchor confirming on-origin at
/// tip. Every other combination — an as-known / unverified anchor, OR a behind
/// tip even when verified — is grey. This is the false-green invariant: a
/// local-only, restored, or merely as-known pin never reads as pointed-fresh
/// green.
#[must_use]
pub fn freshness_badge(tip: TipPosition, anchor: &AnchorState) -> FreshnessBadge {
    match (anchor, tip) {
        (AnchorState::Verified, TipPosition::AtTip) => FreshnessBadge::Green,
        _ => FreshnessBadge::Grey,
    }
}

/// The single dominant glyph, worst-of the asserted axes (d3 merge law).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Drift on the owned axis — the strongest warning.
    Red,
    /// An unverifiable / unconfirmed axis dominates.
    Grey,
    /// Every asserted axis is green.
    Green,
}

impl Glyph {
    /// The render label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Glyph::Red => "red",
            Glyph::Grey => "grey",
            Glyph::Green => "green",
        }
    }
}

/// The two-badge result for one pin: the owned-axis badge and the freshness
/// badge, rendered side by side and NEVER merged into one color. `pointed`
/// records whether the pin asserts an origin-freshness claim — only a pointed
/// claim lets freshness-grey dominate the single glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoBadge {
    /// The owned-axis (content-equality) result.
    pub owned: OwnedBadge,
    /// The freshness-axis result (anchor-fused).
    pub freshness: FreshnessBadge,
    /// Whether the pin asserts a pointed (origin-freshness) claim. An owned-only
    /// pin (`false`) shows its owned color with the freshness badge as a
    /// passenger; a pointed pin (`true`) lets freshness-grey dominate.
    pub pointed: bool,
}

impl TwoBadge {
    /// Render both badges, side by side (never merged): `owned <c> · freshness
    /// <c>`.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "owned {} · freshness {}",
            self.owned.label(),
            self.freshness.label()
        )
    }

    /// The single dominant glyph when only one can be shown: the weakest
    /// asserted axis (d3 merge law). Owned red dominates always; then, for a
    /// pointed pin, freshness-grey dominates; otherwise the owned color shows.
    /// An owned-only pin's grey freshness badge never dominates its glyph.
    #[must_use]
    pub fn dominant(&self) -> Glyph {
        if self.owned == OwnedBadge::Red {
            return Glyph::Red;
        }
        if self.pointed && self.freshness == FreshnessBadge::Grey {
            return Glyph::Grey;
        }
        match self.owned {
            OwnedBadge::Green => Glyph::Green,
            OwnedBadge::Grey => Glyph::Grey,
            OwnedBadge::Red => Glyph::Red,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{JournalRow, render_row};

    /// A day, in seconds — the age unit the restore-replay scenario grows by.
    const DAY: i64 = 86_400;

    fn observed(now: &str, unix: i64) -> Observed {
        Observed {
            now: now.to_string(),
            unix,
        }
    }

    /// The verified door: a run that observed origin renders the tip axis BARE.
    /// This is the ONLY door to a bare axis.
    #[test]
    fn verified_run_renders_bare_axis() {
        let facts = AnchorFacts {
            run_observed: true,
            last_observation: None,
            origin_ref_present: true,
        };
        let state = AnchorState::classify(&facts, 1_000);
        assert_eq!(state, AnchorState::Verified);
        assert!(state.renders_bare_axis());
        assert_eq!(render_tip_axis(TipPosition::AtTip, &state), "at-tip");
        assert_eq!(render_tip_axis(TipPosition::Behind, &state), "behind");
        assert!(nudge_hint(&state).is_none());
    }

    /// **Gate — restore-replay scenario (load-bearing negative).** A restored
    /// `.git` carries a stale local `origin/<branch>` (so the origin ref is
    /// present) and replays an old journaled fetch-claim receipt. The run did
    /// NOT observe origin this moment. The anchor must render
    /// `at-tip (anchor as-known, observed …, ~<age>)` with a growing age and
    /// NEVER a bare `at-tip`.
    #[test]
    fn restore_replay_renders_as_known_growing_age_never_bare_at_tip() {
        // The old observation: journaled 2 days before the first render moment.
        let obs_unix = 0;
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: Some(observed("2026-07-20T00:00:00Z", obs_unix)),
            origin_ref_present: true, // the restore restored origin's local ref
        };

        // First render moment: 2 days after the observation.
        let render1 = AnchorState::classify(&facts, 2 * DAY);
        let line1 = render_tip_axis(TipPosition::AtTip, &render1);
        assert!(
            line1.starts_with("at-tip (anchor as-known, observed 2026-07-20T00:00:00Z, ~"),
            "as-known render carries the observed token and a ~age: {line1}"
        );
        assert_eq!(
            line1,
            "at-tip (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d)"
        );
        assert!(
            !render1.renders_bare_axis(),
            "a restored anchor must NOT render a bare tip axis"
        );

        // Replay LATER (5 days after the observation): the SAME receipt, but the
        // age has grown — a restore cannot re-mint recency.
        let render2 = AnchorState::classify(&facts, 5 * DAY);
        let line2 = render_tip_axis(TipPosition::AtTip, &render2);
        assert_eq!(
            line2,
            "at-tip (anchor as-known, observed 2026-07-20T00:00:00Z, ~5d)"
        );

        let (
            AnchorState::AsKnownAged { age: age1, .. },
            AnchorState::AsKnownAged { age: age2, .. },
        ) = (&render1, &render2)
        else {
            panic!("both renders must be as-known-aged");
        };
        assert!(
            age2 > age1,
            "the replayed age must grow: {age2:?} > {age1:?}"
        );

        // The invariant, stated as an assertion: the render is never bare.
        assert_ne!(line1, "at-tip");
        assert_ne!(line2, "at-tip");
    }

    /// **The fires-check (W-C1 guard).** A guard that classified the anchor
    /// from the local remote-tracking ref's *presence* (the restore-restored
    /// ref) would promote it to verified and render a bare `at-tip` — the exact
    /// violation. This test encodes the wrong guard inline and asserts it WOULD
    /// produce the bare axis the correct [`AnchorState::classify`] refuses, so
    /// the restore-replay gate above is not a tautology over a state that can
    /// never be reached.
    #[test]
    fn a_ref_reading_guard_would_render_bare_at_tip() {
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: Some(observed("2026-07-20T00:00:00Z", 0)),
            origin_ref_present: true,
        };

        // The WRONG guard: trust the local remote-tracking ref's presence as a
        // verified observation.
        let wrong = if facts.origin_ref_present {
            AnchorState::Verified
        } else {
            AnchorState::classify(&facts, 2 * DAY)
        };
        assert_eq!(
            render_tip_axis(TipPosition::AtTip, &wrong),
            "at-tip",
            "the ref-reading guard renders the bare axis the law forbids"
        );

        // The RIGHT guard never does.
        let right = AnchorState::classify(&facts, 2 * DAY);
        assert_ne!(render_tip_axis(TipPosition::AtTip, &right), "at-tip");
    }

    /// **Gate — out-of-engine fetch.** A `git fetch` run outside the engine
    /// leaves the local origin ref present but mints no journaled observation.
    /// The anchor renders as-known AGELESS (`at-tip (anchor as-known)`, no age)
    /// and carries the nudge hint.
    #[test]
    fn out_of_engine_fetch_is_as_known_ageless_with_nudge() {
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: None,
            origin_ref_present: true,
        };
        let state = AnchorState::classify(&facts, 9_999);
        assert_eq!(state, AnchorState::AsKnownAgeless);
        assert_eq!(
            render_tip_axis(TipPosition::AtTip, &state),
            "at-tip (anchor as-known)"
        );
        assert!(!state.renders_bare_axis());
        assert_eq!(nudge_hint(&state), Some(AGELESS_NUDGE));
    }

    /// Anchor facts absent entirely (no local origin ref, no observation):
    /// unverified. Distinct from as-known-ageless, which has the ref present.
    #[test]
    fn no_anchor_facts_is_unverified() {
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: None,
            origin_ref_present: false,
        };
        let state = AnchorState::classify(&facts, 42);
        assert_eq!(state, AnchorState::Unverified);
        assert_eq!(
            render_tip_axis(TipPosition::Behind, &state),
            "behind (anchor unverified)"
        );
        assert!(nudge_hint(&state).is_none());
    }

    /// A journaled observation dates the anchor even when the run did not
    /// observe — and it wins over the mere presence of the origin ref
    /// (as-known-aged, never ageless).
    #[test]
    // The age is the arithmetic `160 - 100 = 60` seconds — seconds are the unit
    // under test, so the second-grained `Duration` is the readable form here.
    #[allow(clippy::duration_suboptimal_units)]
    fn journaled_observation_beats_bare_ref_presence() {
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: Some(observed("2026-07-23T00:00:00Z", 100)),
            origin_ref_present: true,
        };
        let state = AnchorState::classify(&facts, 160);
        assert_eq!(
            state,
            AnchorState::AsKnownAged {
                observed: "2026-07-23T00:00:00Z".to_string(),
                age: Duration::from_secs(60),
            }
        );
    }

    /// A future-dated receipt (clock skew) clamps the age at zero — never a
    /// negative or wrapped age.
    #[test]
    fn future_observation_clamps_age_to_zero() {
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: Some(observed("2099-01-01T00:00:00Z", 1_000_000)),
            origin_ref_present: true,
        };
        let state = AnchorState::classify(&facts, 0);
        let AnchorState::AsKnownAged { age, .. } = state else {
            panic!("as-known-aged");
        };
        assert_eq!(age, Duration::ZERO);
    }

    /// [`last_observation`] reads the LAST observe-op `now` from parsed journal
    /// rows, composing the journal parser — never reparsing the page, and
    /// skipping ordinary write rows.
    #[test]
    fn last_observation_reads_last_observe_row() {
        use crate::journal::parse_rows;
        let splice = render_row(&JournalRow {
            seq: 1,
            op: "splice",
            path: "notes/plan.md",
            actor: Some("alice"),
            now: Some("2026-07-20T00:00:00Z"),
            root_before: "b3:0",
            root_after: "b3:1",
            edits: Vec::new(),
        });
        let observe_old = render_row(&JournalRow {
            seq: 2,
            op: OBSERVE_OP,
            path: "origin/main",
            actor: None,
            now: Some("2026-07-21T00:00:00Z"),
            root_before: "b3:1",
            root_after: "b3:2",
            edits: Vec::new(),
        });
        let observe_new = render_row(&JournalRow {
            seq: 3,
            op: OBSERVE_OP,
            path: "origin/main",
            actor: None,
            now: Some("2026-07-23T00:00:00Z"),
            root_before: "b3:2",
            root_after: "b3:3",
            edits: Vec::new(),
        });
        let page = format!("# journal\n{splice}\n{observe_old}\n{observe_new}\n");
        let rows = parse_rows(&page);
        assert_eq!(last_observation(&rows), Some("2026-07-23T00:00:00Z"));

        // No observe rows ⇒ None (the anchor is ageless or unverified).
        let only_splice = format!("# journal\n{splice}\n");
        assert_eq!(last_observation(&parse_rows(&only_splice)), None);
    }

    /// `human_age` picks the coarsest whole unit that fits.
    #[test]
    // Each assertion probes a second-grained THRESHOLD (59→60, 3599→3600,
    // 86399→86400); the raw second counts are the point of the test.
    #[allow(clippy::duration_suboptimal_units)]
    fn human_age_scales_units() {
        assert_eq!(human_age(Duration::from_secs(3)), "3s");
        assert_eq!(human_age(Duration::from_secs(59)), "59s");
        assert_eq!(human_age(Duration::from_secs(60)), "1m");
        assert_eq!(human_age(Duration::from_secs(3_599)), "59m");
        assert_eq!(human_age(Duration::from_secs(3_600)), "1h");
        assert_eq!(human_age(Duration::from_secs(86_399)), "23h");
        assert_eq!(human_age(Duration::from_secs(86_400)), "1d");
        assert_eq!(human_age(Duration::from_secs(3 * 86_400)), "3d");
    }

    /// The two-badge fusion: a verified, at-tip anchor mints the ONLY green
    /// freshness badge; the board shows both badges, never one merged color.
    #[test]
    fn freshness_badge_green_only_when_verified_at_tip() {
        assert_eq!(
            freshness_badge(TipPosition::AtTip, &AnchorState::Verified),
            FreshnessBadge::Green
        );
        // Verified but behind ⇒ not fresh-green.
        assert_eq!(
            freshness_badge(TipPosition::Behind, &AnchorState::Verified),
            FreshnessBadge::Grey
        );
        // As-known / unverified, even at-tip ⇒ grey (the false-green invariant).
        assert_eq!(
            freshness_badge(TipPosition::AtTip, &AnchorState::AsKnownAgeless),
            FreshnessBadge::Grey
        );
        assert_eq!(
            freshness_badge(
                TipPosition::AtTip,
                &AnchorState::AsKnownAged {
                    observed: "x".into(),
                    age: Duration::from_secs(1),
                }
            ),
            FreshnessBadge::Grey
        );
        assert_eq!(
            freshness_badge(TipPosition::AtTip, &AnchorState::Unverified),
            FreshnessBadge::Grey
        );
    }

    /// The two-badge merge law: two badges rendered, and the dominant single
    /// glyph is the weakest asserted axis.
    #[test]
    fn two_badge_render_and_dominant_glyph() {
        // An owned-only pin on uncommitted-but-matching content: green owned,
        // grey freshness — shows green (freshness-grey does not dominate an
        // owned-only pin).
        let owned_only = TwoBadge {
            owned: OwnedBadge::Green,
            freshness: FreshnessBadge::Grey,
            pointed: false,
        };
        assert_eq!(owned_only.render(), "owned green · freshness grey");
        assert_eq!(owned_only.dominant(), Glyph::Green);

        // A pointed pin with a grey freshness badge: freshness-grey dominates
        // (fail-safe — the weakest asserted axis).
        let pointed_grey = TwoBadge {
            owned: OwnedBadge::Green,
            freshness: FreshnessBadge::Grey,
            pointed: true,
        };
        assert_eq!(pointed_grey.dominant(), Glyph::Grey);

        // A pointed pin verified fresh: both green ⇒ green.
        let pointed_fresh = TwoBadge {
            owned: OwnedBadge::Green,
            freshness: FreshnessBadge::Green,
            pointed: true,
        };
        assert_eq!(pointed_fresh.dominant(), Glyph::Green);

        // Owned drift always dominates, pointed or not.
        let drifted = TwoBadge {
            owned: OwnedBadge::Red,
            freshness: FreshnessBadge::Green,
            pointed: true,
        };
        assert_eq!(drifted.dominant(), Glyph::Red);
    }
}
