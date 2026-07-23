//! Layer 1 — armed conventions evaluated read-only (d2 §3 check: "Layer 1: armed
//! rules packs evaluated read-only — same rule, same citation as the write gate").
//!
//! Each armed convention's `check_change` runs over the change through the U1.3
//! loader — the SAME surface the door mounts (U4.2). A refusal here is byte-for-
//! byte the refusal the door would mint: this layer surfaces
//! [`policy::Refusal`]s verbatim, it does not re-interpret them. It performs no
//! I/O — the conventions are pre-loaded (read-only) and the change is pre-derived,
//! so a layer-1 run mutates nothing.

use policy::{Change, CheckError, Convention, Enforcement, Refusal};

/// An armed convention ready to evaluate: the convention loaded read-only through
/// the U1.3 loader, plus its INDEX arming (the [`Enforcement`] severity). A member
/// of the armed set is armed by construction — `warn` or `block`, never `off`.
#[derive(Debug)]
pub struct ArmedConvention {
    /// The convention's subject slug (its folder name / INDEX row).
    pub slug: String,
    /// The armed severity — `warn` (annotate) or `block` (the door would refuse).
    pub enforcement: Enforcement,
    /// The loaded convention (folder grammar + capability ceiling + load gate all
    /// passed). Read-only: `check_change` reads the change and emits refusals.
    pub convention: Convention,
}

/// One armed finding: which convention fired, at what severity, and the refusal it
/// emitted VERBATIM (message + passing scenario) — the SAME refusal the door mints
/// (U4.2), carried byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedFinding {
    /// The convention that fired.
    pub slug: String,
    /// Its armed severity.
    pub enforcement: Enforcement,
    /// The convention's refusal, verbatim.
    pub refusal: Refusal,
}

/// A convention whose `check_change` FAULTED (budget / parse / runtime) — named,
/// never silently passed. A fault fails closed: the caller treats a faulting armed
/// convention as unsatisfied (the write gate's `convention-fault`, U4.1 row 6).
#[derive(Debug)]
pub struct ArmedFault {
    /// The convention that faulted.
    pub slug: String,
    /// The typed evaluation fault.
    pub error: CheckError,
}

/// The armed-set verdict over one change: the refusals every in-scope armed
/// convention emitted, and the conventions whose eval faulted.
#[derive(Debug)]
pub struct ArmedReport {
    /// Every armed finding, in armed-set order then emission order.
    pub findings: Vec<ArmedFinding>,
    /// Conventions whose eval faulted (fail-closed), in armed-set order.
    pub faults: Vec<ArmedFault>,
}

impl ArmedReport {
    /// The armed layer found a lie: a `block` finding, or a fault (fail-closed). A
    /// `warn` finding annotates but does not, on its own, redden the read.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.faults.is_empty()
            || self
                .findings
                .iter()
                .any(|f| f.enforcement == Enforcement::Block)
    }
}

/// Evaluate the armed set over one change, READ-ONLY (d2 §3 layer 1). For each
/// armed convention whose scope matches the change's doc path, run its
/// `check_change` — the SAME surface the door mounts — and surface every refusal
/// verbatim; a convention whose eval faults is named, never a silent pass. Reads
/// the change and the pre-loaded conventions only: no disk, no receipt, no cap.
#[must_use]
pub fn evaluate(armed: &[ArmedConvention], change: &Change) -> ArmedReport {
    let mut findings = Vec::new();
    let mut faults = Vec::new();
    for conv in armed {
        if !conv.convention.matches_path(&change.doc.path) {
            continue;
        }
        match conv.convention.check_change(change) {
            Ok(outcome) => {
                for refusal in outcome.refusals {
                    findings.push(ArmedFinding {
                        slug: conv.slug.clone(),
                        enforcement: conv.enforcement,
                        refusal,
                    });
                }
            }
            Err(error) => faults.push(ArmedFault {
                slug: conv.slug.clone(),
                error,
            }),
        }
    }
    ArmedReport { findings, faults }
}

#[cfg(test)]
mod tests {
    use model::{Document, NodeKind};
    use policy::{ChangeOp, CheckLimits, Invocation, derive_change, load_seed_convention};

    use super::*;

    /// Build a real `model::Document` from markdown, stamping its path (what the
    /// disk edge does; `model::build` leaves the path empty).
    fn doc_of(path: &str, md: &str) -> Document {
        let nodes = syntax::parse(md);
        let mut doc = model::build(md.to_string(), nodes);
        if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    fn no_edges(_: &str) -> Option<(String, Document)> {
        None
    }

    /// A close-the-task change under `actor`, over a task in the seed convention's
    /// `tasks/**` scope.
    fn close_change(actor: &str) -> Change {
        let before = doc_of(
            "tasks/t.md",
            "---\nowner: agent:alice\nstatus: open\n---\n# T\n\nx\n",
        );
        let after = doc_of(
            "tasks/t.md",
            "---\nowner: agent:alice\nstatus: closed\n---\n# T\n\nx\n",
        );
        derive_change(
            &before,
            &after,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some(actor),
                force: false,
            },
            &[],
            &no_edges,
        )
    }

    /// Arm the seed convention at `block` for the tests (the seed says "do not arm"
    /// in production; a test arms it to exercise the armed layer against a real
    /// `check_change`).
    fn armed_seed() -> ArmedConvention {
        ArmedConvention {
            slug: "reviewer-not-owner".to_string(),
            enforcement: Enforcement::Block,
            convention: load_seed_convention(CheckLimits::default()).expect("seed loads"),
        }
    }

    /// Layer 1 surfaces the seed convention's refusal VERBATIM — byte-for-byte the
    /// refusals `Convention::check_change` returns directly (the door's surface).
    #[test]
    fn armed_layer_matches_the_direct_evaluator_byte_for_byte() {
        let change = close_change("agent:alice"); // owner self-close: fires
        let seed = armed_seed();

        // Direct evaluator output (what the door mounts).
        let direct = seed.convention.check_change(&change).expect("clean eval");
        assert!(direct.fired(), "owner self-close fires the seed convention");

        // Through the armed layer.
        let report = evaluate(std::slice::from_ref(&seed), &change);
        assert!(report.faults.is_empty(), "no eval fault");
        let surfaced: Vec<Refusal> = report.findings.iter().map(|f| f.refusal.clone()).collect();

        assert_eq!(
            surfaced, direct.refusals,
            "the armed layer surfaces the direct refusals byte-for-byte"
        );
        assert_eq!(report.findings[0].slug, "reviewer-not-owner");
        assert_eq!(report.findings[0].enforcement, Enforcement::Block);
        assert!(report.is_red(), "a block finding reddens the read");
    }

    /// A reviewer distinct from the owner passes — no finding, matching the direct
    /// evaluator (which returns no refusal).
    #[test]
    fn armed_layer_passes_a_reviewer_close_like_the_direct_evaluator() {
        let change = close_change("agent:bob");
        let seed = armed_seed();
        let direct = seed.convention.check_change(&change).expect("clean eval");
        assert!(!direct.fired(), "reviewer != owner passes directly");

        let report = evaluate(std::slice::from_ref(&seed), &change);
        assert!(report.findings.is_empty(), "no armed finding");
        assert!(!report.is_red());
    }

    /// A change outside the convention's `paths:` scope is never evaluated — the
    /// convention is not that document's concern.
    #[test]
    fn out_of_scope_change_is_not_evaluated() {
        let before = doc_of("notes/n.md", "---\nowner: agent:alice\n---\n# N\n\nx\n");
        let after = doc_of("notes/n.md", "---\nowner: agent:alice\n---\n# N\n\ny\n");
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
            &no_edges,
        );
        let report = evaluate(std::slice::from_ref(&armed_seed()), &change);
        assert!(
            report.findings.is_empty() && report.faults.is_empty(),
            "notes/n.md is outside tasks/** — the convention never runs"
        );
    }
}
