//! Blocking gate at the armed change plane (U4.2) — the pure decision half.
//! Input is the [`ArmedLaw`] [`resolve_armed_law`] resolved at the write path.
//! A block-severity firing, drifted law, or unloadable set refuses with a
//! closed `{code, recovery}` pair. Never-armed is a no-op. Does not load from
//! disk (caller injects the law).

use crate::armed::Mode;
use crate::armed_law::{ArmedFault, ArmedLaw, ArmedRule};
use crate::change::Change;

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

/// One advisory finding a `warn`-armed rule emitted, a surviving armed-law fault,
/// OR a `--force`-escaped refusal (U4.3) — all render on the write response and
/// none refuse (§11.1 advisory shape). A `forced` finding is the loud record of a
/// skip: it renders, and the mount journals it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFinding {
    /// The rule id (or `binding-break:<side>`) that emitted it.
    pub rule: String,
    /// The teaching message.
    pub message: String,
    /// The legal path the refusal cites (the passing case).
    pub passing_scenario: String,
    /// `true` when this finding is a `--force`-escaped refusal — a skip the
    /// mount must journal (never a plain `warn` advisory).
    pub forced: bool,
}

/// One blocking violation a `block`-armed rule emitted — the change violates
/// armed law and the write refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateViolation {
    /// The rule id that refused.
    pub rule: String,
    /// The teaching message.
    pub message: String,
    /// The legal path the refusal cites (the passing case).
    pub passing_scenario: String,
}

/// Why the gate refused a write — each arm carries the facts the wire refusal
/// names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateRefusal {
    /// One or more `block`-armed rules fired over the change (conjunction
    /// stacking). Every violation names its rule and cites the passing case —
    /// the refusal teaches the legal path.
    Blocked { violations: Vec<GateViolation> },
    /// The workspace's armed law could not be honored. Carries EVERY refusing
    /// fault rather than the first, because they are one condition — the law at
    /// this path is not enforceable — and reporting one would hide the rest.
    /// The wire seam picks the envelope from the faults; the words are
    /// [`ArmedFault`]'s.
    ArmedLawFault { faults: Vec<ArmedFault> },
    /// U4.3 taxonomy row 9: a one-sided artifact↔page change stopped at the door
    /// (a hand-edited row in the artifact, or a direct edit of an armed page).
    /// Force-escapable — a `--force` write converts this to a
    /// rendered finding instead. Mints `binding_break`.
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
    /// U4.3 taxonomy row 10: deletion/rename of the armed-rules artifact or the
    /// once-armed marker, refused by the integrity floor. NOT force-escapable
    /// (security F2). Mints `index_integrity`.
    IndexIntegrity {
        /// The protected file (the artifact or the marker).
        target: String,
        /// The teaching message citing the floor.
        teaching: String,
    },
}

/// Gate one change through a resolved armed law — the DECISION half.
/// `gate(change, law) → Ok(verdicts) | Refusal(violations)` (laws.md).
///
/// - never-armed — `Ok([])`, the no-op.
/// - a REFUSING fault — `Refusal(ArmedLawFault)` (fail-closed): the law itself
///   cannot be honored at this path.
/// - otherwise — the door law first, then every in-scope armed rule runs over
///   the change and its refusals STACK (conjunction). A `block` firing refuses
///   the write; a `warn` firing renders as an advisory finding. A rule that
///   cannot EVALUATE the change fails closed — never a silent pass. Surviving
///   (non-refusing) faults ride along as advisory findings.
///
/// `--force`: a forced change escapes a `block` refusal — the skip is the
/// mount's to journal + render. It never escapes an armed-law fault: a law that
/// cannot be read is not a check to skip.
#[must_use]
pub fn gate(change: &Change, law: &ArmedLaw) -> GateOutcome {
    if law.never_armed() {
        return GateOutcome::Ok(Vec::new());
    }
    let refusing: Vec<ArmedFault> = law.refusing().cloned().collect();
    if !refusing.is_empty() {
        return GateOutcome::Refusal(GateRefusal::ArmedLawFault { faults: refusing });
    }
    gate_armed(change, law)
}

/// The armed-workspace decision: the U4.3 door law FIRST (integrity floor, then
/// the binding law), then the armed rules. The door law runs before the rules
/// because a write to the engine-managed artifact / an armed page is structurally
/// wrong regardless of what a user rule would say.
fn gate_armed(change: &Change, law: &ArmedLaw) -> GateOutcome {
    let path = target_path(change);
    let is_armed_page = |page: &str| law.armed_pages().any(|armed| armed == page);
    match crate::binding::classify_door_law(change.op, path, &is_armed_page) {
        // The integrity floor is structural — `--force` does NOT escape it
        // (security F2: deleting the marker is the silent-disarm attack).
        crate::binding::DoorLaw::IndexIntegrity { target, teaching } => {
            return GateOutcome::Refusal(GateRefusal::IndexIntegrity { target, teaching });
        }
        // A binding break is the sanctioned-bypass class: `--force` escapes it,
        // the skip becoming a rendered `forced:` finding.
        crate::binding::DoorLaw::BindingBreak {
            side,
            path,
            teaching,
            legal_path,
        } => {
            if change.force {
                return GateOutcome::Ok(vec![GateFinding {
                    rule: format!("binding-break:{}", side.as_str()),
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
    evaluate_armed(change, law)
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

/// The advisory findings the surviving (non-refusing) faults render as — a hook
/// row's fault never vetoes, and this is the channel that keeps it from being
/// silently survived instead.
fn surviving_faults(law: &ArmedLaw) -> Vec<GateFinding> {
    law.faults()
        .iter()
        .filter(|fault| !fault.refuses())
        .map(|fault| GateFinding {
            rule: fault
                .id()
                .map_or_else(|| "armed-law".to_string(), |id| id.as_str().to_string()),
            message: fault.to_string(),
            passing_scenario: crate::armed::ARMED_RULES_PATH.to_string(),
            forced: false,
        })
        .collect()
}

/// Run every in-scope armed rule over the change, stacking outcomes.
fn evaluate_armed(change: &Change, law: &ArmedLaw) -> GateOutcome {
    let path = change.doc.path.as_str();
    let mut findings = surviving_faults(law);
    let mut violations = Vec::new();
    for armed in law.rules() {
        // Scoping: a rule only judges documents its `paths:` scope covers.
        if !armed.rule().matches_path(path) {
            continue;
        }
        let outcome = match armed.rule().check_change(change) {
            Ok(outcome) => outcome,
            // A rule that cannot COMPLETE its evaluation over the change fails
            // CLOSED — a budget/parse/runtime fault never reads as a pass.
            Err(e) => {
                return GateOutcome::Refusal(GateRefusal::ArmedLawFault {
                    faults: vec![ArmedFault::Unevaluable {
                        row: armed.row().clone(),
                        detail: e.to_string(),
                    }],
                });
            }
        };
        for refusal in outcome.refusals {
            match armed.mode() {
                Mode::Block => violations.push(GateViolation {
                    rule: rule_name(armed),
                    message: refusal.message,
                    passing_scenario: refusal.passing_scenario,
                }),
                Mode::Warn => findings.push(GateFinding {
                    rule: rule_name(armed),
                    message: refusal.message,
                    passing_scenario: refusal.passing_scenario,
                    forced: false,
                }),
                // `off` never fires (it is filtered before resolution), and
                // `armed` is a hook's mode — a reaction never vetoes a write.
                Mode::Off | Mode::Armed => {}
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
                rule: v.rule,
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

/// The name a report labels an armed rule with — its id.
fn rule_name(armed: &ArmedRule) -> String {
    armed.id().as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armed::{ArmRequest, ArmRoot, Mode, PageSource, arm};
    use crate::armed_law::resolve_armed_law;
    use crate::change::{ChangeOp, Invocation, derive_change};
    use crate::check_eval::CheckLimits;
    use crate::registration::{PageRef, RuleId, RuleIndex, ScopeLayer, page_rev};
    use model::{Document, NodeKind};
    use std::collections::BTreeMap;

    // ── an in-memory page source ─────────────────────────────────────────────

    /// The workspace's pages, as a `path → bytes` map.
    struct MemPages(BTreeMap<String, String>);

    impl PageSource for MemPages {
        fn read(&self, page: &str) -> std::io::Result<String> {
            self.0.get(page).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {page}"))
            })
        }
    }

    /// A `reviewer-not-owner` CHECK page: refuses iff `change.actor ==
    /// change.doc.frontmatter.owner`, scoped to `tasks/**`. Registers by TAG —
    /// the page is the rule, and no `kind:` key restates it.
    fn reviewer_check(id: &str) -> String {
        format!(
            "---\ntags: [type/rule, rules/check]\nid: {id}\npaths:\n  - tasks/**\n---\n\n\
             # {id}\n\n```starlark\ndef check_change(change):\n    \
             owner = change.doc.frontmatter.get(\"owner\")\n    actor = change.actor\n    \
             if actor != None and owner != None and actor == owner:\n        \
             refuse(message = \"reviewer must not be the owner\", \
             passing = \"reviewer-not-owner.md#reviewer-close\")\n```\n"
        )
    }

    /// A HOOK page in the ruled shape — it declares no law, so it never refuses.
    fn hook_page(id: &str) -> String {
        format!(
            "---\ntags: [type/rule, rules/hook]\nid: {id}\nseverity: info\n\
             paths: [\"tasks/*.md\"]\ncaps:  [proto.send]\n\
             budget: {{ steps: 10000, mem: 4194304 }}\nhow:\n  route: {{ info: channel-review }}\n\
             ---\n\n```starlark\ndef on_change(event):\n    pass\n```\n"
        )
    }

    /// Arm every `(page path, bytes, id, mode)` at the workspace root, and hand
    /// back the rendered artifact plus a page source over the same bytes.
    fn armed(pages: &[(&str, String, &str, Mode)]) -> (String, MemPages) {
        let index = RuleIndex::discover(pages.iter().map(|(path, bytes, ..)| PageRef {
            layer: ScopeLayer::Workspace,
            page: path,
            bytes,
        }));
        let source = MemPages(
            pages
                .iter()
                .map(|(path, bytes, ..)| ((*path).to_string(), bytes.clone()))
                .collect(),
        );
        let artifact = arm(
            &index,
            &ArmRoot::workspace(),
            pages.iter().map(|(_, bytes, id, mode)| ArmRequest {
                id: RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: page_rev(bytes),
            }),
            &source,
            CheckLimits::default(),
        )
        .expect("the fixture arms");
        (artifact.render(), source)
    }

    /// The armed law governing a write at `tasks/fix-parser.md`.
    fn law_at(artifact: Option<&str>, ever_armed: bool, pages: &MemPages) -> ArmedLaw {
        resolve_armed_law(
            artifact,
            ever_armed,
            "tasks/fix-parser.md",
            pages,
            CheckLimits::default(),
        )
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
        // An artifact on disk but NO marker → never armed, even though the row
        // would fire.
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"),
            "reviewer.not-owner",
            Mode::Block,
        )]);
        let law = law_at(Some(&artifact), false, &pages);
        assert!(law.never_armed());
        let outcome = gate(&close_change("agent:alice", "agent:alice", false), &law);
        assert_eq!(outcome, GateOutcome::Ok(Vec::new()));
    }

    // ── armed block: refuses, names the ID, cites the passing case ───────────

    #[test]
    fn armed_block_refuses_owner_self_close_naming_the_rule_id() {
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"),
            "reviewer.not-owner",
            Mode::Block,
        )]);
        let law = law_at(Some(&artifact), true, &pages);

        let outcome = gate(&close_change("agent:alice", "agent:alice", false), &law);
        let GateOutcome::Refusal(GateRefusal::Blocked { violations }) = outcome else {
            panic!("owner self-close must refuse: {outcome:?}");
        };
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule, "reviewer.not-owner",
            "the id is the name the refusal carries, not a folder slug"
        );
        assert!(
            violations[0]
                .message
                .contains("reviewer must not be the owner")
        );
        assert_eq!(
            violations[0].passing_scenario,
            "reviewer-not-owner.md#reviewer-close"
        );
    }

    #[test]
    fn armed_block_lands_reviewer_close() {
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"),
            "reviewer.not-owner",
            Mode::Block,
        )]);
        let law = law_at(Some(&artifact), true, &pages);
        // A reviewer distinct from the owner → the rule passes → the write lands.
        let outcome = gate(&close_change("agent:bob", "agent:alice", false), &law);
        assert_eq!(outcome, GateOutcome::Ok(Vec::new()));
    }

    #[test]
    fn out_of_scope_change_is_never_judged() {
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"), // scope tasks/**
            "reviewer.not-owner",
            Mode::Block,
        )]);
        // A note outside `tasks/**` — even an owner self-close shape lands.
        let law = resolve_armed_law(
            Some(&artifact),
            true,
            "notes/plan.md",
            &pages,
            CheckLimits::default(),
        );
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
        assert_eq!(gate(&change, &law), GateOutcome::Ok(Vec::new()));
    }

    // ── warn renders, never blocks ───────────────────────────────────────────

    #[test]
    fn armed_warn_renders_finding_never_refuses() {
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"),
            "reviewer.not-owner",
            Mode::Warn,
        )]);
        let law = law_at(Some(&artifact), true, &pages);
        let outcome = gate(&close_change("agent:alice", "agent:alice", false), &law);
        let GateOutcome::Ok(findings) = outcome else {
            panic!("warn never refuses: {outcome:?}");
        };
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "reviewer.not-owner");
    }

    // ── the fail-closed ladder, through the ONE fault surface ────────────────

    #[test]
    fn a_missing_artifact_on_a_once_armed_workspace_refuses() {
        let law = law_at(None, true, &MemPages(BTreeMap::new()));
        let GateOutcome::Refusal(GateRefusal::ArmedLawFault { faults }) =
            gate(&close_change("a", "b", false), &law)
        else {
            panic!("a missing artifact on a once-armed workspace must refuse");
        };
        assert!(matches!(faults.as_slice(), [ArmedFault::Missing { .. }]));
    }

    #[test]
    fn a_corrupt_artifact_fails_closed_naming_it() {
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"),
            "reviewer.not-owner",
            Mode::Block,
        )]);
        let corrupt = artifact.replace("| id | page | rev | scope | mode |", "| id | page |");
        let law = law_at(Some(&corrupt), true, &pages);
        let GateOutcome::Refusal(GateRefusal::ArmedLawFault { faults }) =
            gate(&close_change("agent:bob", "agent:alice", false), &law)
        else {
            panic!("a corrupt artifact must fail closed");
        };
        let [fault] = faults.as_slice() else {
            panic!("one whole-artifact fault: {faults:?}");
        };
        assert!(
            fault.to_string().contains("corrupt"),
            "names the corruption: {fault}"
        );
    }

    /// A drifted armed page fails closed at the door; the fault keys on the
    /// rule id and its page — the row key.
    #[test]
    fn a_drifted_armed_page_fails_closed_naming_the_id_and_the_page() {
        let check = reviewer_check("reviewer.not-owner");
        let (artifact, mut pages) = armed(&[(
            "rules/reviewer.md",
            check.clone(),
            "reviewer.not-owner",
            Mode::Block,
        )]);
        // Edit the pinned page after arming: the row reddens.
        pages.0.insert(
            "rules/reviewer.md".to_string(),
            format!("{check}\n<!-- v2 -->\n"),
        );
        let law = law_at(Some(&artifact), true, &pages);
        let GateOutcome::Refusal(GateRefusal::ArmedLawFault { faults }) =
            gate(&close_change("agent:bob", "agent:alice", false), &law)
        else {
            panic!("a drifted armed page must fail closed");
        };
        let [ArmedFault::Red(red)] = faults.as_slice() else {
            panic!("expected one red row: {faults:?}");
        };
        assert_eq!(red.row().id().as_str(), "reviewer.not-owner");
        assert_eq!(red.row().page(), "rules/reviewer.md");
    }

    /// The armed-law fault is NOT force-escapable: a law that cannot be read is
    /// not a check to skip. Only a rule's own refusal is.
    #[test]
    fn force_never_escapes_an_armed_law_fault() {
        let law = law_at(None, true, &MemPages(BTreeMap::new()));
        assert!(matches!(
            gate(&close_change("a", "b", true), &law),
            GateOutcome::Refusal(GateRefusal::ArmedLawFault { .. })
        ));
    }

    /// A HOOK row's fault never vetoes — but it is not silently survived either:
    /// it renders as an advisory finding in the fault surface's own words.
    #[test]
    fn a_surviving_hook_fault_renders_as_a_finding_instead_of_vanishing() {
        let hook = hook_page("notify.review");
        let (artifact, mut pages) = armed(&[(
            "rules/notify.md",
            hook.clone(),
            "notify.review",
            Mode::Armed,
        )]);
        // Drift the hook page: its row reddens, and a hook fault never refuses.
        pages.0.insert(
            "rules/notify.md".to_string(),
            format!("{hook}\n<!-- v2 -->\n"),
        );
        let law = law_at(Some(&artifact), true, &pages);

        let GateOutcome::Ok(findings) =
            gate(&close_change("agent:bob", "agent:alice", false), &law)
        else {
            panic!("a red hook row never refuses a write");
        };
        assert_eq!(findings.len(), 1, "the fault still reaches the operator");
        assert_eq!(findings[0].rule, "notify.review");
        assert!(
            findings[0].message.contains("was edited after arming"),
            "in the fault surface's own words: {}",
            findings[0].message
        );
    }

    // ── conjunction stacking ─────────────────────────────────────────────────

    #[test]
    fn two_block_rules_stack() {
        let (artifact, pages) = armed(&[
            (
                "rules/a.md",
                reviewer_check("reviewer.a"),
                "reviewer.a",
                Mode::Block,
            ),
            (
                "rules/b.md",
                reviewer_check("reviewer.b"),
                "reviewer.b",
                Mode::Block,
            ),
        ]);
        let law = law_at(Some(&artifact), true, &pages);
        let GateOutcome::Refusal(GateRefusal::Blocked { violations }) =
            gate(&close_change("agent:alice", "agent:alice", false), &law)
        else {
            panic!("two block rules must both fire");
        };
        assert_eq!(violations.len(), 2, "conjunction stacks both violations");
    }

    // ── --force escape ───────────────────────────────────────────────────────

    #[test]
    fn force_escapes_block_refusal_as_finding() {
        let (artifact, pages) = armed(&[(
            "rules/reviewer.md",
            reviewer_check("reviewer.not-owner"),
            "reviewer.not-owner",
            Mode::Block,
        )]);
        let law = law_at(Some(&artifact), true, &pages);
        let outcome = gate(&close_change("agent:alice", "agent:alice", true), &law);
        let GateOutcome::Ok(findings) = outcome else {
            panic!("--force escapes the refusal: {outcome:?}");
        };
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("FORCED"), "the skip is named");
        assert!(findings[0].forced);
    }
}
