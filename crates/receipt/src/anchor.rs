//! The anchor law + two-badge freshness (d2 §2.3 v3 W-C1; d3 §1.5).
//!
//! Tip equality ([`TipPosition`]) is mechanical and local; the anchor — the
//! engine's local knowledge of origin's refs — carries its own trust state,
//! because a restored backup restores `.git` including the local copy of
//! origin's refs, so a stale `origin/<branch>` can read `at-tip`.
//!
//! W-C1: no surface may render a bare `at-tip` / `behind` from the local
//! remote-tracking ref. Only an anchor the current run verified against origin
//! renders the tip axis bare; every other render carries a qualifier naming
//! how stale the knowledge is. [`render_tip_axis`] is the sole renderer and
//! produces a bare axis from the [`AnchorState::Verified`] arm alone.
//!
//! A "run" is one engine invocation (one verb execution or wire call); its
//! facts share one moment ([`AnchorState::classify`] takes it as `now_unix`).
//!
//! The freshness badge fuses on top (d3 §1.5): [`freshness_badge`] is green
//! only for a verified, at-tip anchor, and the board renders two badges,
//! never one merged color ([`TwoBadge`]) — a local-only or restored pin never
//! reads as pointed-fresh green.
//!
//! [`ObjectAnchor`] answers the second, local question (S5): does the pinned
//! blob exist, and is it reachable from a commit? Same fact/classify split:
//! the `git` crate gathers [`ObjectAnchorFacts`], this module classifies.

use std::time::Duration;

/// The mechanical tip position: the object id of local `HEAD` versus the local
/// remote-tracking ref `origin/<branch>` (a source-2 rev comparison).
///
/// Says nothing about the ref's freshness — always pair with an
/// [`AnchorState`] through [`render_tip_axis`] (W-C1).
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
/// the receipt's timestamp token, rendered verbatim; `unix` is that token
/// parsed to epoch seconds. The caller derives `unix` from `now` — this leaf
/// crate never parses dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// The receipt's `now` token, rendered verbatim in the `observed <now>`
    /// clause.
    pub now: String,
    /// `now` parsed to epoch seconds — the age is `run_now_unix - unix`.
    pub unix: i64,
}

/// The origin-observation facts the status surface gathers for one run. The
/// anchor law is computed from these, never from the freshness of the local
/// remote-tracking ref. Checking that the origin ref *exists* is required (it
/// splits as-known-ageless from unverified); trusting its *recency* is not —
/// that comes only from [`AnchorFacts::run_observed`] or a journaled
/// observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorFacts {
    /// True iff this run performed the origin observation — the one verified
    /// door (`realise` fetch observe-claim). `status`, being cap-free, always
    /// passes false.
    pub run_observed: bool,
    /// The most recent journaled fetch-claim observation, if any. The only
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
    /// Classify the anchor from the run's [`AnchorFacts`] and its moment
    /// `now_unix` (epoch seconds): run-observed ⇒ verified; else a journaled
    /// observation ⇒ as-known-aged; else a present-but-undated origin ref ⇒
    /// as-known-ageless; else unverified.
    ///
    /// The origin ref's *object id* never enters this classification — only
    /// whether it *exists* — so a restored `.git` cannot promote the anchor
    /// past as-known (the W-C1 guard).
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

    /// Whether this state may render the tip axis bare (no qualifier). True
    /// for [`AnchorState::Verified`] alone — the structural witness of W-C1.
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

/// The one `op` a fetch observe-claim receipt records; a re-derived dater
/// must name the same op the claim writes.
pub const OBSERVE_OP: &str = "observe";

/// Render the anchor-qualified tip axis (d2 §2.3 v3, the sole renderer).
///
/// A bare axis (no qualifier) is produced by the [`AnchorState::Verified`]
/// arm alone — the structural guarantee of W-C1.
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

/// A compact age: whole units, coarsest that fits (seconds, then minutes,
/// hours, days). Never claims sub-second precision.
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
// object anchoring — the three states of a pinned blob (S5)
// ---------------------------------------------------------------------------

/// The git-object facts one anchoring check gathers about one blob. The
/// caller (the `git` crate's `Repo` handle) asks git; this crate never
/// computes an object id and never shells out.
///
/// Both facts must come from one object store in one pass (U13): an oid can
/// be reachable in root A and absent from root B, so a fact pair split across
/// two stores is a wrong answer, not a stale one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectAnchorFacts {
    /// Whether the object exists in the repository's object database.
    pub object_present: bool,
    /// Whether the object is reachable from a ref — i.e. some commit carries
    /// it, so `git gc` will keep it.
    pub reachable_from_commit: bool,
}

/// The three anchoring states of a pinned blob. Stage-2 grain (M1
/// anchor-is-a-line); the stage-3 anchor-after-fence receipt grain is not
/// modeled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectAnchor {
    /// Reachable from a ref: a commit carries the blob, so it survives `git gc`
    /// and any clone of that ref sees it. The only durable state.
    Anchored,
    /// The object is in the database but no ref reaches it — the vibe eager
    /// write (`git hash-object -w`) before the file is committed. Verifiable
    /// locally, durable only until the TTL: see [`PENDING_ANCHOR_TTL`].
    PendingAnchor,
    /// No such object: nothing was ever written (a read-only blob-sha compute),
    /// the repository was re-cloned, or `git gc` has already pruned a
    /// pending-anchor blob past its TTL. A pin over it can be verified against
    /// nothing.
    NeverAnchored,
}

/// Named residual G1 — the pending-anchor TTL is the repository's local
/// `gc.pruneExpire` (git default `2.weeks.ago`). The engine documents this
/// and does not prevent it: committing the file is the only durable anchor,
/// and a pruned blob re-classifies as [`ObjectAnchor::NeverAnchored`].
pub const PENDING_ANCHOR_TTL: &str = "pending-anchor durability is the repository's local `gc.pruneExpire` \
     (git default 2.weeks.ago): an uncommitted vibe blob is unreachable, so git may prune it — \
     commit the file to anchor it durably";

impl ObjectAnchor {
    /// Classify the anchoring state from the run's gathered facts.
    /// Reachability is the stronger fact and decides first: a reachable
    /// object is in the database, so it classifies as
    /// [`ObjectAnchor::Anchored`] even against a stale presence answer.
    #[must_use]
    pub fn classify(facts: &ObjectAnchorFacts) -> ObjectAnchor {
        if facts.reachable_from_commit {
            ObjectAnchor::Anchored
        } else if facts.object_present {
            ObjectAnchor::PendingAnchor
        } else {
            ObjectAnchor::NeverAnchored
        }
    }

    /// The render word for this state — one spelling shared by every surface.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            ObjectAnchor::Anchored => "anchored",
            ObjectAnchor::PendingAnchor => "pending-anchor",
            ObjectAnchor::NeverAnchored => "never-anchored",
        }
    }

    /// The hint to render beside this state, if any. Only
    /// [`ObjectAnchor::PendingAnchor`] carries one — [`PENDING_ANCHOR_TTL`].
    #[must_use]
    pub fn nudge(self) -> Option<&'static str> {
        match self {
            ObjectAnchor::PendingAnchor => Some(PENDING_ANCHOR_TTL),
            _ => None,
        }
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

/// The freshness axis (pointed, 5th) badge. Green only when a verified anchor
/// confirms on-origin; never red (nothing drifted), grey for every
/// unconfirmed anchor. Produced by [`freshness_badge`].
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

/// Fuse the anchor law onto the freshness badge (d3 §1.5).
///
/// Green requires a verified anchor confirming on-origin at tip; every other
/// combination is grey — the false-green invariant.
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
/// badge, rendered side by side and never merged into one color.
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

    /// A day, in seconds — the age unit the restore-replay scenario grows by.
    const DAY: i64 = 86_400;

    fn observed(now: &str, unix: i64) -> Observed {
        Observed {
            now: now.to_string(),
            unix,
        }
    }

    /// A run that observed origin renders the tip axis bare — the only door
    /// to a bare axis.
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

    /// Gate — restore-replay: a restored `.git` with a stale origin ref and a
    /// replayed fetch-claim receipt must render as-known with a growing age,
    /// never a bare `at-tip`.
    #[test]
    fn restore_replay_renders_as_known_growing_age_never_bare_at_tip() {
        let obs_unix = 0;
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: Some(observed("2026-07-20T00:00:00Z", obs_unix)),
            origin_ref_present: true, // the restore restored origin's local ref
        };

        // Render 2 days after the observation.
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

        // Replay the same receipt 5 days after the observation: age grows.
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

        assert_ne!(line1, "at-tip");
        assert_ne!(line2, "at-tip");
    }

    /// Fires-check (W-C1 guard): a guard that trusted the origin ref's
    /// presence would render the bare `at-tip` that [`AnchorState::classify`]
    /// refuses — so the restore-replay gate above is not a tautology.
    #[test]
    fn a_ref_reading_guard_would_render_bare_at_tip() {
        let facts = AnchorFacts {
            run_observed: false,
            last_observation: Some(observed("2026-07-20T00:00:00Z", 0)),
            origin_ref_present: true,
        };

        // The wrong guard: trust the ref's presence as a verified observation.
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

        // The right guard never does.
        let right = AnchorState::classify(&facts, 2 * DAY);
        assert_ne!(render_tip_axis(TipPosition::AtTip, &right), "at-tip");
    }

    /// Gate — out-of-engine fetch: origin ref present, no journaled
    /// observation ⇒ as-known ageless, with the nudge hint.
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

    /// A dated observation dates the anchor even when the run did not
    /// observe — and it wins over the mere presence of the origin ref
    /// (as-known-aged, never ageless).
    #[test]
    // Seconds are the unit under test.
    #[allow(clippy::duration_suboptimal_units)]
    fn dated_observation_beats_bare_ref_presence() {
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

    /// `human_age` picks the coarsest whole unit that fits.
    #[test]
    // Each assertion probes a second-grained threshold; the raw counts matter.
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

    /// The two-badge fusion: only a verified, at-tip anchor mints a green
    /// freshness badge.
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

    /// The three object-anchoring states (S5); the end-to-end proof against a
    /// real repository lives in the `git` crate's `tests/plumbing.rs`.
    #[test]
    fn object_anchor_classifies_three_states() {
        let anchored = ObjectAnchor::classify(&ObjectAnchorFacts {
            object_present: true,
            reachable_from_commit: true,
        });
        assert_eq!(anchored, ObjectAnchor::Anchored);
        assert_eq!(anchored.word(), "anchored");
        assert!(anchored.nudge().is_none());

        // The vibe eager write: in the database, no ref reaches it.
        let pending = ObjectAnchor::classify(&ObjectAnchorFacts {
            object_present: true,
            reachable_from_commit: false,
        });
        assert_eq!(pending, ObjectAnchor::PendingAnchor);
        assert_eq!(pending.word(), "pending-anchor");
        assert_eq!(pending.nudge(), Some(PENDING_ANCHOR_TTL));

        // Nothing was ever written — or gc pruned it past the G1 TTL.
        let never = ObjectAnchor::classify(&ObjectAnchorFacts {
            object_present: false,
            reachable_from_commit: false,
        });
        assert_eq!(never, ObjectAnchor::NeverAnchored);
        assert_eq!(never.word(), "never-anchored");
        assert!(never.nudge().is_none());
    }

    /// Reachability is the stronger fact: a reachable object classifies as
    /// anchored even if the presence fact says otherwise (it cannot honestly —
    /// a reachable object exists — so the classification must not invent
    /// `never-anchored` out of the contradiction).
    #[test]
    fn reachable_wins_over_a_contradictory_presence_fact() {
        assert_eq!(
            ObjectAnchor::classify(&ObjectAnchorFacts {
                object_present: false,
                reachable_from_commit: true,
            }),
            ObjectAnchor::Anchored
        );
    }

    /// G1 is named where a reader of the code finds it: the pending-anchor
    /// nudge says `gc.pruneExpire` in words.
    #[test]
    fn pending_anchor_ttl_names_gc_prune_expire() {
        assert!(
            PENDING_ANCHOR_TTL.contains("gc.pruneExpire"),
            "the G1 residual names the git config that is the TTL: {PENDING_ANCHOR_TTL}"
        );
    }

    /// The two-badge merge law: two badges rendered, and the dominant single
    /// glyph is the weakest asserted axis.
    #[test]
    fn two_badge_render_and_dominant_glyph() {
        // Owned-only pin: freshness-grey does not dominate.
        let owned_only = TwoBadge {
            owned: OwnedBadge::Green,
            freshness: FreshnessBadge::Grey,
            pointed: false,
        };
        assert_eq!(owned_only.render(), "owned green · freshness grey");
        assert_eq!(owned_only.dominant(), Glyph::Green);

        // Pointed pin with grey freshness: freshness-grey dominates.
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
