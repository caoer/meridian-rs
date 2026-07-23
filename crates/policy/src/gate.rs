//! The blocking gate at the armed change plane (U4.2) — the pure decision half.
//!
//! # What this is (laws.md § the policy gate; refusal-amendment §11.1)
//! `crates/policy` once owned advisory verdicts only. This module is the
//! blocking `gate()` seam: after CAS and before bytes land, the write door
//! evaluates a [`Change`](crate::Change) through a workspace's OWN armed law and
//! either lets the write stand ([`GateOutcome::Ok`]) or REFUSES it
//! ([`GateOutcome::Refusal`]) — the bytes never land. It replaces the deferred
//! `authorize` stub (the Go-era caller-supplies shape dies with Go).
//!
//! # Two halves, one seam
//! The seam splits along the I/O line the crate's charter draws (policy stays
//! I/O-free, as `model` is):
//! - [`resolve_armed_set`] — the LOAD + VERIFY half. Given the attested INDEX
//!   bytes (U1.4), the once-armed marker (U2.5 read-contract), and an injected
//!   [`ConventionSource`] to read convention folders, it resolves the workspace's
//!   armed law into an [`ArmedSet`], failing CLOSED on a missing-once-armed,
//!   corrupt, un-loadable, or DRIFTED law. All I/O is injected — the caller (the
//!   trusted write path in `wire-serve`/`run`) does the disk reads.
//! - [`gate`] — the DECISION half. `gate(change, armed_set)`: never-armed is a
//!   no-op; a fault refuses fail-closed; otherwise every in-scope armed
//!   convention runs over the change and its refusals STACK (conjunction), a
//!   `block`-severity firing refusing the write, a `warn` firing rendering as an
//!   advisory finding.
//!
//! # ATTACK-034 scoping (laws.md)
//! Refusal makes violations "unrepresentable through an armed change plane" —
//! never a stronger claim. A never-armed workspace is a bit-for-bit no-op; the
//! genesis epoch renders grey, never green. Out-of-band mutation is caught by
//! the git witness plus the receipt-engine-only write restriction, never by this
//! gate.

use crate::change::Change;
use crate::check_eval::CheckLimits;
use crate::convention::{Convention, ConventionFiles, load_convention};
use crate::index::{ArmedRef, Enforcement, evidence_rev, parse_index_strict};

/// A per-slug accessor for a convention folder, injected so [`resolve_armed_set`]
/// stays I/O-free: given a slug, hand back a [`ConventionFiles`] reading that
/// convention's `conventions/<slug>/` folder from wherever the caller keeps it
/// (disk in production, an in-memory map in tests).
pub trait ConventionSource {
    /// A files accessor for the convention folder named `slug`.
    fn files_for<'a>(&'a self, slug: &str) -> Box<dyn ConventionFiles + 'a>;
}

/// One armed convention resolved from an INDEX `[x]` row and drift-verified: its
/// live evidence rev equalled the pinned armed rev at load. Construction is
/// sealed to [`resolve_armed_set`] — a `Convention` reaches `block`/`warn`
/// enforcement only through the drift gate.
#[derive(Debug, Clone)]
pub struct ArmedConvention {
    slug: String,
    enforcement: Enforcement,
    convention: Convention,
}

impl ArmedConvention {
    /// The convention's slug.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The armed enforcement level (`warn` or `block`).
    #[must_use]
    pub fn enforcement(&self) -> Enforcement {
        self.enforcement
    }
}

/// The verified armed law of a workspace, resolved inside the trusted write path
/// from its attested INDEX (U1.4) and once-armed marker (U2.5). The gate's input.
#[derive(Debug, Clone)]
pub enum ArmedSet {
    /// The workspace has NEVER been armed (no once-armed marker). The gate is a
    /// no-op — writes land bit-for-bit as with no gate at all.
    NeverArmed,
    /// A resolved, drift-verified armed set — zero or more in-scope conventions.
    Armed(Vec<ArmedConvention>),
    /// Fail-CLOSED: the armed law itself is broken and cannot be honored. The
    /// gate refuses every write until the law is repaired.
    Faulted(GateFault),
}

/// Why the armed law could not be resolved — the fail-closed causes. Bound to
/// the closed §8 taxonomy at the wire seam (`convention_fault` env /
/// `armed_drift` refresh).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFault {
    /// The armed INDEX is absent on a once-armed workspace, is corrupt, or an
    /// armed convention cannot load. Refusal-amendment row 6 (`convention_fault`,
    /// env). `detail` names the fault (the INDEX/convention).
    ConventionFault { detail: String },
    /// An armed convention's live evidence rev no longer equals its pinned armed
    /// rev. Refusal-amendment row 7 (`armed_drift`, refresh).
    ArmedDrift {
        /// The armed convention's slug.
        slug: String,
        /// The reviewer-approved rev the INDEX pins (`armed-rev`).
        armed_rev: String,
        /// The rev the convention's evidence reads NOW (`report-rev`).
        report_rev: String,
    },
}

/// The outcome of gating one change through a workspace's armed law:
/// `Ok(verdicts) | Refusal(violations)` (laws.md § the policy gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The change stands. `verdicts` are advisory `warn`-severity findings — the
    /// §11.1 advisory shape, which renders but never blocks. Empty on a
    /// never-armed workspace (the no-op) or when nothing fired.
    Ok(Vec<GateFinding>),
    /// The change is REFUSED — the bytes must not land. The mount maps this to a
    /// wire `{code, recovery}` from the closed §8 taxonomy.
    Refusal(GateRefusal),
}

/// One advisory finding a `warn`-armed convention emitted, OR a `--force`-escaped
/// refusal (U4.3) — both render on the write response but never refuse (§11.1
/// advisory shape). A `forced` finding is the loud record of a skip: it renders,
/// and the mount journals it (the sanctioned bypass, decision #6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFinding {
    /// The convention (or `binding-break:<side>`) that emitted it.
    pub slug: String,
    /// The teaching message.
    pub message: String,
    /// The legal path the refusal cites (the passing scenario).
    pub passing_scenario: String,
    /// `true` when this finding is a `--force`-escaped refusal — a skip the
    /// mount must journal (never a plain `warn` advisory).
    pub forced: bool,
}

/// One blocking violation a `block`-armed convention emitted — the change
/// violates armed law and the write refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateViolation {
    /// The convention that refused.
    pub slug: String,
    /// The teaching message.
    pub message: String,
    /// The legal path the refusal cites (the passing scenario).
    pub passing_scenario: String,
}

/// Why the gate refused a write — each arm carries the facts the wire refusal
/// names. `Blocked` and `ConventionFault` both mint the `convention_fault` wire
/// code in U4.2 (U4.4's floor conventions add per-rule codes); `ArmedDrift`
/// mints `armed_drift`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateRefusal {
    /// One or more `block`-armed conventions fired over the change (conjunction
    /// stacking). Every violation names its convention and cites the passing
    /// scenario — the refusal teaches the legal path.
    Blocked { violations: Vec<GateViolation> },
    /// The armed law is broken (missing-once-armed / corrupt / un-loadable / a
    /// convention that cannot evaluate). Fail-closed `convention_fault`.
    ConventionFault { detail: String },
    /// An armed convention drifted off its attested rev. `armed_drift`.
    ArmedDrift {
        slug: String,
        armed_rev: String,
        report_rev: String,
    },
    /// U4.3 taxonomy row 9: a one-sided file↔index change stopped at the door (a
    /// checkbox flip on the INDEX, or a direct edit of an armed convention's
    /// `CHECK.md`). Force-escapable — a `--force` write converts this to a
    /// journaled + rendered finding instead. Mints `binding_break`.
    BindingBreak {
        /// Which side the one-sided change touched (`index` / `file`).
        side: crate::binding::BindingSide,
        /// The engine-managed file the write targeted.
        path: String,
        /// The teaching message naming the break.
        teaching: String,
        /// The legal path the refusal cites (the ONE-act proper path).
        legal_path: String,
    },
    /// U4.3 taxonomy row 10: deletion/rename of the INDEX or the once-armed
    /// marker, refused by the INDEX-integrity floor convention. NOT
    /// force-escapable (security F2). Mints `index_integrity`.
    IndexIntegrity {
        /// The protected file (the INDEX or the marker).
        target: String,
        /// The teaching message citing the floor convention.
        teaching: String,
    },
}

/// Resolve a workspace's armed law — the LOAD + VERIFY half of the gate seam.
///
/// `index` is the attested INDEX page bytes (`None` when the INDEX file is
/// absent); `ever_armed` is the once-armed marker's presence; `source` reads
/// convention folders (injected — policy does no I/O); `limits` bound each
/// convention's load gate.
///
/// The fail-closed ladder (refusal-amendment rows 6/7; laws.md § the policy
/// gate):
/// - **never-armed** (`!ever_armed`) — [`ArmedSet::NeverArmed`], a bit-for-bit
///   no-op. A stray INDEX cannot arm a workspace (arming is drift-gated and sets
///   the marker), so the INDEX is not even read.
/// - **missing INDEX on a once-armed workspace** — `Faulted(ConventionFault)`.
/// - **corrupt INDEX** — `Faulted(ConventionFault)`, naming the corruption. A
///   corrupt page never reads as an empty (gate-disabling) armed set.
/// - **an armed convention cannot load** — `Faulted(ConventionFault)`.
/// - **an armed convention drifted** (report-rev ≠ armed-rev) —
///   `Faulted(ArmedDrift)`.
/// - otherwise — `Armed(convs)`, every convention drift-verified.
#[must_use]
pub fn resolve_armed_set(
    index: Option<&str>,
    ever_armed: bool,
    source: &dyn ConventionSource,
    limits: CheckLimits,
) -> ArmedSet {
    // Never armed: the gate is OFF. The INDEX is not consulted — a workspace can
    // only be armed by an attested arm (which sets the marker), so any INDEX on
    // a never-armed workspace is a stray file, ignored for a bit-for-bit no-op.
    if !ever_armed {
        return ArmedSet::NeverArmed;
    }

    // Once armed: an attested INDEX MUST be present and valid, or fail CLOSED.
    let Some(index_src) = index else {
        return ArmedSet::Faulted(GateFault::ConventionFault {
            detail: "attested INDEX is missing on a workspace that has been armed \
                     (once-armed marker present) — failing closed"
                .to_string(),
        });
    };
    let rows = match parse_index_strict(index_src) {
        Ok(rows) => rows,
        Err(corrupt) => {
            return ArmedSet::Faulted(GateFault::ConventionFault {
                detail: format!("attested INDEX is corrupt: {}", corrupt.detail),
            });
        }
    };

    let mut convs = Vec::with_capacity(rows.len());
    for row in rows {
        match resolve_one(&row, source, limits) {
            Ok(ac) => convs.push(ac),
            Err(fault) => return ArmedSet::Faulted(fault),
        }
    }
    ArmedSet::Armed(convs)
}

/// Load one armed row's convention and drift-verify it, or fail closed.
fn resolve_one(
    row: &ArmedRef,
    source: &dyn ConventionSource,
    limits: CheckLimits,
) -> Result<ArmedConvention, GateFault> {
    let files = source.files_for(&row.slug);
    let convention =
        load_convention(&row.slug, &*files, limits).map_err(|e| GateFault::ConventionFault {
            detail: format!("armed convention `{}` cannot load: {e}", row.slug),
        })?;
    // The drift gate: the live evidence rev must still equal the pinned armed
    // rev, or the attested law changed out from under its approval.
    let check_md = files
        .read("CHECK.md")
        .map_err(|e| GateFault::ConventionFault {
            detail: format!("armed convention `{}` CHECK.md unreadable: {e}", row.slug),
        })?;
    let report_rev = evidence_rev(&check_md);
    if report_rev != row.armed_rev {
        return Err(GateFault::ArmedDrift {
            slug: row.slug.clone(),
            armed_rev: row.armed_rev.clone(),
            report_rev,
        });
    }
    Ok(ArmedConvention {
        slug: row.slug.clone(),
        enforcement: row.enforcement,
        convention,
    })
}

/// Gate one change through a resolved armed law — the DECISION half.
/// `gate(change, armed_set) → Ok(verdicts) | Refusal(violations)` (laws.md).
///
/// - [`ArmedSet::NeverArmed`] — `Ok([])`, the no-op.
/// - [`ArmedSet::Faulted`] — `Refusal` (fail-closed): the armed law is broken.
/// - [`ArmedSet::Armed`] — every in-scope armed convention runs over the change
///   and its refusals STACK (conjunction). A `block` firing refuses the write; a
///   `warn` firing renders as an advisory finding. A convention that cannot
///   EVALUATE the change fails closed (`convention_fault`) — never a silent pass.
///
/// `--force` (amendment pt 3): a forced change escapes a `block` refusal — the
/// skip is the mount's to journal + render. The wire `force` field plumbing is
/// carried on U4.3; here the pure escape honours `change.force`.
#[must_use]
pub fn gate(change: &Change, armed_set: &ArmedSet) -> GateOutcome {
    match armed_set {
        ArmedSet::NeverArmed => GateOutcome::Ok(Vec::new()),
        ArmedSet::Faulted(GateFault::ConventionFault { detail }) => {
            GateOutcome::Refusal(GateRefusal::ConventionFault {
                detail: detail.clone(),
            })
        }
        ArmedSet::Faulted(GateFault::ArmedDrift {
            slug,
            armed_rev,
            report_rev,
        }) => GateOutcome::Refusal(GateRefusal::ArmedDrift {
            slug: slug.clone(),
            armed_rev: armed_rev.clone(),
            report_rev: report_rev.clone(),
        }),
        ArmedSet::Armed(convs) => gate_armed(change, convs),
    }
}

/// The armed-workspace decision: the U4.3 door law FIRST (INDEX-integrity floor,
/// then the binding law), then the armed conventions. The door law runs before
/// conventions because a write to the engine-managed INDEX / an armed `CHECK.md`
/// is structurally wrong regardless of what a user convention would say.
fn gate_armed(change: &Change, convs: &[ArmedConvention]) -> GateOutcome {
    let path = target_path(change);
    let is_armed_slug = |slug: &str| convs.iter().any(|c| c.slug() == slug);
    match crate::binding::classify_door_law(change.op, path, &is_armed_slug) {
        // The INDEX-integrity floor is structural — `--force` does NOT escape it
        // (security F2: deleting the marker is the silent-disarm attack).
        crate::binding::DoorLaw::IndexIntegrity { target, teaching } => {
            return GateOutcome::Refusal(GateRefusal::IndexIntegrity { target, teaching });
        }
        // A binding break is the sanctioned-bypass class: `--force` escapes it,
        // the skip becoming a journaled + rendered finding.
        crate::binding::DoorLaw::BindingBreak {
            side,
            path,
            teaching,
            legal_path,
        } => {
            if change.force {
                return GateOutcome::Ok(vec![GateFinding {
                    slug: format!("binding-break:{}", side.as_str()),
                    message: format!("FORCED past the binding law: {teaching}"),
                    passing_scenario: legal_path,
                    forced: true,
                }]);
            }
            return GateOutcome::Refusal(GateRefusal::BindingBreak {
                side,
                path,
                teaching,
                legal_path,
            });
        }
        crate::binding::DoorLaw::Clear => {}
    }
    evaluate_armed(change, convs)
}

/// The write's target file path — `change.doc.path` (the after state; stamped on
/// an absent doc for a `remove`), falling back to the before state's path.
fn target_path(change: &Change) -> &str {
    if change.doc.path.is_empty() {
        change.before.path.as_str()
    } else {
        change.doc.path.as_str()
    }
}

/// Run every in-scope armed convention over the change, stacking outcomes.
fn evaluate_armed(change: &Change, convs: &[ArmedConvention]) -> GateOutcome {
    let path = change.doc.path.as_str();
    let mut findings = Vec::new();
    let mut violations = Vec::new();
    for ac in convs {
        // Scoping: a convention only judges documents its `paths:` scope covers.
        if !ac.convention.matches_path(path) {
            continue;
        }
        let outcome = match ac.convention.check_change(change) {
            Ok(outcome) => outcome,
            // A convention that cannot COMPLETE its evaluation over the change
            // fails CLOSED — a budget/parse/runtime fault never reads as a pass.
            Err(e) => {
                return GateOutcome::Refusal(GateRefusal::ConventionFault {
                    detail: format!(
                        "armed convention `{}` cannot evaluate the change: {e}",
                        ac.slug
                    ),
                });
            }
        };
        for refusal in outcome.refusals {
            match ac.enforcement {
                Enforcement::Block => violations.push(GateViolation {
                    slug: ac.slug.clone(),
                    message: refusal.message,
                    passing_scenario: refusal.passing_scenario,
                }),
                Enforcement::Warn => findings.push(GateFinding {
                    slug: ac.slug.clone(),
                    message: refusal.message,
                    passing_scenario: refusal.passing_scenario,
                    forced: false,
                }),
                // `off` is never armed — it does not appear in a resolved set.
                Enforcement::Off => {}
            }
        }
    }

    if !violations.is_empty() {
        if change.force {
            // `--force` escapes the armed refusal (loud: the mount journals +
            // renders the skip). The escaped violations become advisory findings
            // so the render still names what was bypassed.
            let mut all = findings;
            all.extend(violations.into_iter().map(|v| GateFinding {
                slug: v.slug,
                message: format!("FORCED past armed refusal: {}", v.message),
                passing_scenario: v.passing_scenario,
                forced: true,
            }));
            return GateOutcome::Ok(all);
        }
        return GateOutcome::Refusal(GateRefusal::Blocked { violations });
    }
    GateOutcome::Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{ChangeOp, Invocation, derive_change};
    use crate::index::{Enforcement, arm, generate_index, sweep};
    use model::{Document, NodeKind};
    use std::collections::BTreeMap;

    // ── an in-memory convention source ───────────────────────────────────────

    /// A convention folder as an in-memory `rel_path → body` map.
    #[derive(Clone)]
    struct MemConv(BTreeMap<String, String>);

    impl ConventionFiles for MemConv {
        fn read(&self, rel: &str) -> std::io::Result<String> {
            self.0.get(rel).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel}"))
            })
        }
        fn exists(&self, rel: &str) -> bool {
            self.0.contains_key(rel)
        }
    }

    /// A whole `conventions/` tree: `slug → folder`.
    struct MemConventions(BTreeMap<String, MemConv>);

    impl ConventionSource for MemConventions {
        fn files_for<'a>(&'a self, slug: &str) -> Box<dyn ConventionFiles + 'a> {
            Box::new(
                self.0
                    .get(slug)
                    .cloned()
                    .unwrap_or_else(|| MemConv(BTreeMap::new())),
            )
        }
    }

    /// A CHECK.md that refuses iff `change.actor == change.doc.frontmatter.owner`
    /// (the seed shape), scoped to `tasks/**`.
    fn reviewer_check() -> String {
        "---\npaths:\n  - tasks/**\n---\n\n# reviewer-not-owner\n\n```starlark\n\
         def check_change(change):\n    owner = change.doc.frontmatter.get(\"owner\")\n    \
         actor = change.actor\n    if actor != None and owner != None and actor == owner:\n        \
         refuse(message = \"reviewer must not be the owner\", passing = \"scenarios/reviewer-close.md\")\n```\n"
            .to_string()
    }

    fn conv_folder(check_md: &str) -> MemConv {
        let mut m = BTreeMap::new();
        m.insert("CHECK.md".to_string(), check_md.to_string());
        MemConv(m)
    }

    fn conventions(pairs: &[(&str, &str)]) -> MemConventions {
        MemConventions(
            pairs
                .iter()
                .map(|(slug, check)| ((*slug).to_string(), conv_folder(check)))
                .collect(),
        )
    }

    /// An INDEX arming `slug` at `level`, pinned to the live CHECK.md rev.
    fn armed_index(slug: &str, check_md: &str, level: Enforcement) -> String {
        let files = conv_folder(check_md);
        let swept = sweep(&files, slug, CheckLimits::default()).expect("sweeps");
        let rev = swept.rev().to_string();
        let armed = arm(swept, &rev, level).expect("arms at the live rev");
        generate_index(&[armed])
    }

    fn doc_of(path: &str, md: &str) -> Document {
        let nodes = syntax::parse(md);
        let mut doc = model::build(md.to_string(), nodes);
        if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    /// A change closing `tasks/fix-parser.md` as `actor`, with a declared owner.
    fn close_change(actor: &str, owner: &str, force: bool) -> Change {
        let before = doc_of(
            "tasks/fix-parser.md",
            &format!("---\nowner: {owner}\nstatus: open\n---\n# Fix\n\nbody\n"),
        );
        let after = doc_of(
            "tasks/fix-parser.md",
            &format!("---\nowner: {owner}\nstatus: closed\n---\n# Fix\n\nbody\n"),
        );
        derive_change(
            &before,
            &after,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some(actor),
                force,
            },
            &[],
            &|_| None,
        )
    }

    // ── never-armed ──────────────────────────────────────────────────────────

    #[test]
    fn never_armed_is_a_no_op() {
        let src = conventions(&[]);
        // No marker → NeverArmed, even with an INDEX present.
        let index = armed_index("reviewer-not-owner", &reviewer_check(), Enforcement::Block);
        let set = resolve_armed_set(Some(&index), false, &src, CheckLimits::default());
        assert!(matches!(set, ArmedSet::NeverArmed));
        // Even an owner self-close (would fire the convention) lands: no-op.
        let outcome = gate(&close_change("agent:alice", "agent:alice", false), &set);
        assert_eq!(outcome, GateOutcome::Ok(Vec::new()));
    }

    // ── armed block: refuses, cites the passing scenario ─────────────────────

    #[test]
    fn armed_block_refuses_owner_self_close_naming_convention() {
        let check = reviewer_check();
        let src = conventions(&[("reviewer-not-owner", &check)]);
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Block);
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        assert!(matches!(set, ArmedSet::Armed(_)));

        // owner self-close fires the block convention → refuses.
        let outcome = gate(&close_change("agent:alice", "agent:alice", false), &set);
        let GateOutcome::Refusal(GateRefusal::Blocked { violations }) = outcome else {
            panic!("owner self-close must refuse: {outcome:?}");
        };
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].slug, "reviewer-not-owner");
        assert!(
            violations[0]
                .message
                .contains("reviewer must not be the owner")
        );
        assert_eq!(
            violations[0].passing_scenario,
            "scenarios/reviewer-close.md"
        );
    }

    #[test]
    fn armed_block_lands_reviewer_close() {
        let check = reviewer_check();
        let src = conventions(&[("reviewer-not-owner", &check)]);
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Block);
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        // A reviewer distinct from the owner → the convention passes → lands.
        let outcome = gate(&close_change("agent:bob", "agent:alice", false), &set);
        assert_eq!(outcome, GateOutcome::Ok(Vec::new()));
    }

    #[test]
    fn out_of_scope_change_is_never_judged() {
        let check = reviewer_check(); // scope tasks/**
        let src = conventions(&[("reviewer-not-owner", &check)]);
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Block);
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        // A note outside tasks/** — even an owner self-close shape lands.
        let before = doc_of("notes/plan.md", "---\nowner: agent:alice\n---\n# P\n\nx\n");
        let after = doc_of(
            "notes/plan.md",
            "---\nowner: agent:alice\nstatus: closed\n---\n# P\n\nx\n",
        );
        let change = derive_change(
            &before,
            &after,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some("agent:alice"),
                force: false,
            },
            &[],
            &|_| None,
        );
        assert_eq!(gate(&change, &set), GateOutcome::Ok(Vec::new()));
    }

    // ── warn renders, never blocks ───────────────────────────────────────────

    #[test]
    fn armed_warn_renders_finding_never_refuses() {
        let check = reviewer_check();
        let src = conventions(&[("reviewer-not-owner", &check)]);
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Warn);
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        let outcome = gate(&close_change("agent:alice", "agent:alice", false), &set);
        let GateOutcome::Ok(findings) = outcome else {
            panic!("warn never refuses: {outcome:?}");
        };
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].slug, "reviewer-not-owner");
    }

    // ── the empty-armed-set attack: gate reads the workspace INDEX ────────────

    #[test]
    fn empty_source_but_armed_index_fails_closed_not_open() {
        // An armed INDEX names a convention the source cannot provide (an empty
        // caller-supplied set cannot weaken the decision) → fail closed, never a
        // silent pass.
        let check = reviewer_check();
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Block);
        let empty = conventions(&[]); // the "empty armed_set"
        let set = resolve_armed_set(Some(&index), true, &empty, CheckLimits::default());
        assert!(
            matches!(set, ArmedSet::Faulted(GateFault::ConventionFault { .. })),
            "a named-but-absent armed convention fails closed: {set:?}"
        );
    }

    // ── fail-closed ladder ───────────────────────────────────────────────────

    #[test]
    fn missing_index_on_once_armed_fails_closed() {
        let set = resolve_armed_set(None, true, &conventions(&[]), CheckLimits::default());
        assert!(matches!(
            set,
            ArmedSet::Faulted(GateFault::ConventionFault { .. })
        ));
        assert!(matches!(
            gate(&close_change("a", "b", false), &set),
            GateOutcome::Refusal(GateRefusal::ConventionFault { .. })
        ));
    }

    #[test]
    fn corrupt_index_fails_closed_naming_it() {
        let check = reviewer_check();
        let src = conventions(&[("reviewer-not-owner", &check)]);
        // A page that is NOT a well-formed INDEX (no title) — must fail closed,
        // never read as an empty (gate-disabling) armed set.
        let corrupt = "garbage that is not an index\n- [x] tampered\n";
        let set = resolve_armed_set(Some(corrupt), true, &src, CheckLimits::default());
        let ArmedSet::Faulted(GateFault::ConventionFault { detail }) = &set else {
            panic!("corrupt INDEX must fail closed: {set:?}");
        };
        assert!(detail.contains("corrupt"), "names the corruption: {detail}");
    }

    #[test]
    fn drifted_convention_fails_closed_armed_drift() {
        let check = reviewer_check();
        // Arm at the current rev, then drift the on-disk CHECK.md.
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Block);
        let drifted = check.replace("reviewer-not-owner", "reviewer-not-owner (edited law)");
        let src = conventions(&[("reviewer-not-owner", &drifted)]);
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        let ArmedSet::Faulted(GateFault::ArmedDrift {
            slug,
            armed_rev,
            report_rev,
        }) = &set
        else {
            panic!("a drifted armed law fails closed: {set:?}");
        };
        assert_eq!(slug, "reviewer-not-owner");
        assert_ne!(armed_rev, report_rev, "report-rev ≠ armed-rev");
    }

    // ── conjunction stacking ─────────────────────────────────────────────────

    #[test]
    fn two_block_conventions_stack() {
        // Two block conventions over the same scope both fire → both violations
        // stack (conjunction).
        let a = reviewer_check();
        let b = a.replace("scenarios/reviewer-close.md", "scenarios/other.md");
        let src = conventions(&[("conv-a", &a), ("conv-b", &b)]);
        let ia = armed_index("conv-a", &a, Enforcement::Block);
        let ib = armed_index("conv-b", &b, Enforcement::Block);
        // Merge the two single-row INDEX pages into one two-row page.
        let mut rows: Vec<String> = Vec::new();
        for page in [&ia, &ib] {
            rows.extend(
                page.lines()
                    .filter(|l| l.starts_with("- ["))
                    .map(str::to_string),
            );
        }
        let index = format!(
            "# Attested conventions INDEX\n\npreamble\n\n{}\n",
            rows.join("\n")
        );
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        let GateOutcome::Refusal(GateRefusal::Blocked { violations }) =
            gate(&close_change("agent:alice", "agent:alice", false), &set)
        else {
            panic!("two block conventions must both fire");
        };
        assert_eq!(violations.len(), 2, "conjunction stacks both violations");
    }

    // ── --force escape (pure; wire plumbing deferred to U4.3) ────────────────

    #[test]
    fn force_escapes_block_refusal_as_finding() {
        let check = reviewer_check();
        let src = conventions(&[("reviewer-not-owner", &check)]);
        let index = armed_index("reviewer-not-owner", &check, Enforcement::Block);
        let set = resolve_armed_set(Some(&index), true, &src, CheckLimits::default());
        let outcome = gate(&close_change("agent:alice", "agent:alice", true), &set);
        let GateOutcome::Ok(findings) = outcome else {
            panic!("--force escapes the refusal: {outcome:?}");
        };
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("FORCED"), "the skip is named");
    }
}
