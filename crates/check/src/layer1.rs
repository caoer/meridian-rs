//! Layer 1 — the workspace's armed RULES evaluated read-only: same rule, same
//! citation as the write gate.
//!
//! Each armed rule's `check_change` runs over the change through the page loader —
//! the SAME surface the door mounts. A refusal here is byte-for-byte the refusal
//! the door would mint: this layer surfaces [`policy::Refusal`]s verbatim. It
//! performs no I/O — the law is pre-resolved and the change pre-derived.
//!
//! The armed set is not this layer's to assemble: the input is
//! `&[`[`policy::ArmedRule`]`]`, whose construction is sealed to
//! [`policy::resolve_armed_law`], so this layer cannot be handed an armed set the
//! door would not have honoured.
//!
//! A fault has one name, and it is `policy`'s: a rule that will not evaluate is
//! [`policy::ArmedFault::Unevaluable`], never a fault type of this crate's — a
//! layer that carries the door's refusal byte-for-byte cannot spell the door's
//! faults differently.

use policy::armed::Mode;
use policy::{ArmedFault, ArmedRule, Change, Refusal, RuleId};

/// One armed finding: which rule fired, in the mode the ARM act attested it in, and
/// the refusal it emitted VERBATIM (message + passing scenario) — the SAME refusal
/// the door mints (U4.2), carried byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedFinding {
    /// The id of the rule that fired — the frontmatter `id:` the artifact keys its
    /// row on, not a filename and not a folder slug.
    pub id: RuleId,
    /// The attested mode. Check vocabulary by construction: only `warn` and `block`
    /// reach a finding (see [`evaluate`]).
    pub mode: Mode,
    /// The rule's refusal, verbatim.
    pub refusal: Refusal,
}

/// The armed-law verdict over one change: the refusals every in-scope armed rule
/// emitted, and the rules whose evaluation faulted.
#[derive(Debug)]
pub struct ArmedReport {
    /// Every armed finding, in armed-law order then emission order.
    pub findings: Vec<ArmedFinding>,
    /// The rules whose law could not be EVALUATED over this change, in armed-law
    /// order — [`ArmedFault::Unevaluable`], the vocabulary's own word for exactly
    /// this fact. A fault is named, never a silent pass.
    pub faults: Vec<ArmedFault>,
}

impl ArmedReport {
    /// The armed layer found a lie: a `block` finding, or a fault the ruled kind
    /// split says must not be survived. A `warn` finding annotates but does not,
    /// on its own, redden the read.
    ///
    /// Faults are asked [`ArmedFault::refuses`] rather than counted: whether an
    /// unevaluable rule may be survived is a ruled split, and a consumer
    /// re-deriving it is how two planes come to disagree about one fact.
    #[must_use]
    pub fn is_red(&self) -> bool {
        self.faults.iter().any(ArmedFault::refuses)
            || self.findings.iter().any(|f| f.mode == Mode::Block)
    }
}

/// Evaluate the armed law over one change, READ-ONLY (d2 §3 layer 1). For each
/// armed rule whose `paths:` scope matches the change's doc path, run its
/// `check_change` — the SAME surface the door mounts — and surface every refusal
/// verbatim; a rule whose eval faults is named, never a silent pass. Reads the
/// change and the pre-resolved rules only: no disk, no receipt, no cap.
///
/// `armed` is [`policy::ArmedLaw::rules`] — the law resolved at this change's own
/// path. The arm ROOT already narrowed which rows govern here; what this function
/// applies is the rule's own declared scope, which is a different question and the
/// only one left to ask.
#[must_use]
pub fn evaluate(armed: &[ArmedRule], change: &Change) -> ArmedReport {
    let mut findings = Vec::new();
    let mut faults = Vec::new();
    for rule in armed {
        if !rule.rule().matches_path(&change.doc.path) {
            continue;
        }
        let outcome = match rule.rule().check_change(change) {
            Ok(outcome) => outcome,
            // Fail closed, in the door's words: a fault never reads as a pass,
            // and the attested row says WHICH page at WHICH rev could not answer.
            Err(error) => {
                faults.push(ArmedFault::Unevaluable {
                    row: rule.row().clone(),
                    detail: error.to_string(),
                });
                continue;
            }
        };
        for refusal in outcome.refusals {
            match rule.mode() {
                // `block` is what the door would refuse on, so it reddens the
                // read; `warn` is carried and rendered and does not.
                Mode::Block | Mode::Warn => findings.push(ArmedFinding {
                    id: rule.id().clone(),
                    mode: rule.mode(),
                    refusal,
                }),
                // `off` is attested-inert and never reaches a resolved law.
                // `armed` is a HOOK's mode, and a reaction has no veto —
                // admitting it here would hand a hook enforcement power through
                // the read verb; its firing is the reaction plane's to report.
                // Unreachable today (a dual-kind page refuses to arm; a hook
                // page has no check leg), but kept exhaustive over `Mode` so a
                // relaxed upstream law cannot make this the place a hook
                // quietly acquires a veto.
                Mode::Off | Mode::Armed => {}
            }
        }
    }
    ArmedReport { findings, faults }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use model::{Document, NodeKind};
    use policy::armed::{ArmRequest, ArmRoot, PageSource, arm};
    use policy::{
        ArmedLaw, ChangeOp, CheckLimits, Invocation, PageRef, RuleId, RuleIndex, ScopeLayer,
        derive_change, page_rev, resolve_armed_law,
    };

    use super::*;

    /// The `reviewer.not-owner` CHECK page: it refuses when the actor closing a task
    /// is that task's declared owner, scoped `tasks/**`. It registers by TAG — the
    /// page IS the rule, and no `kind:` key restates it.
    const REVIEWER_PAGE: &str = r#"---
tags: [type/rule, rules/check]
id: reviewer.not-owner
paths:
  - tasks/**
---

# reviewer.not-owner

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get("owner")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(message = "reviewer must not be the owner", passing = "reviewer-close")
```
"#;
    const REVIEWER_ID: &str = "reviewer.not-owner";
    const REVIEWER_PATH: &str = "rules/reviewer.md";

    /// A HOOK page in the ruled shape — it declares no law, so it never refuses.
    const NOTIFY_PAGE: &str = r#"---
tags: [type/rule, rules/hook]
id: reviewer.notify
severity: info
paths: ["tasks/*.md"]
caps:  [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

```starlark
def on_change(event):
    pass
```
"#;
    const NOTIFY_ID: &str = "reviewer.notify";
    const NOTIFY_PATH: &str = "rules/notify.md";

    /// The workspace's pages as `path → bytes`. `policy` is I/O-free, so a unit test
    /// injects the page plane exactly where the disk edge injects its own.
    struct MemPages(BTreeMap<String, String>);

    impl PageSource for MemPages {
        fn read(&self, page: &str) -> std::io::Result<String> {
            self.0.get(page).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {page}"))
            })
        }
    }

    /// ARM every `(page path, bytes, id, mode)` at the workspace root and
    /// resolve the law governing a write at `at_path`.
    ///
    /// Goes through the real ARM act and resolver: an [`ArmedRule`] has no
    /// other constructor, so a fixture cannot hand `evaluate` a hand-built
    /// armed set.
    fn law_at(pages: &[(&str, &str, &str, Mode)], at_path: &str) -> ArmedLaw {
        let index = RuleIndex::discover(pages.iter().map(|(path, bytes, ..)| PageRef {
            layer: ScopeLayer::Workspace,
            page: path,
            bytes,
        }));
        let artifact = arm(
            &index,
            &ArmRoot::workspace(),
            pages.iter().map(|(_, bytes, id, mode)| ArmRequest {
                id: RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: page_rev(bytes),
            }),
        )
        .expect("the fixture arms");
        let source = MemPages(
            pages
                .iter()
                .map(|(path, bytes, ..)| ((*path).to_string(), (*bytes).to_string()))
                .collect(),
        );
        resolve_armed_law(
            Some(&artifact.render()),
            true,
            at_path,
            &source,
            CheckLimits::default(),
        )
    }

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

    /// A close-the-task change under `actor`, over a task in the rule's `tasks/**`
    /// scope.
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

    /// Layer 1 surfaces the armed rule's refusal VERBATIM — byte-for-byte the
    /// refusals `Rule::check_change` returns directly (the door's surface).
    #[test]
    fn armed_layer_matches_the_direct_evaluator_byte_for_byte() {
        let change = close_change("agent:alice"); // owner self-close: fires
        let law = law_at(
            &[(REVIEWER_PATH, REVIEWER_PAGE, REVIEWER_ID, Mode::Block)],
            "tasks/t.md",
        );
        let [armed] = law.rules() else {
            panic!("one rule governs tasks/t.md: {:?}", law.faults());
        };

        // Direct evaluator output (what the door mounts).
        let direct = armed.rule().check_change(&change).expect("clean eval");
        assert!(direct.fired(), "owner self-close fires the armed rule");

        // Through the armed layer.
        let report = evaluate(law.rules(), &change);
        assert!(report.faults.is_empty(), "no eval fault");
        let surfaced: Vec<Refusal> = report.findings.iter().map(|f| f.refusal.clone()).collect();

        assert_eq!(
            surfaced, direct.refusals,
            "the armed layer surfaces the direct refusals byte-for-byte"
        );
        assert_eq!(
            report.findings[0].id.as_str(),
            REVIEWER_ID,
            "the ID is the name a finding carries, not a folder slug"
        );
        assert_eq!(report.findings[0].mode, Mode::Block);
        assert!(report.is_red(), "a block finding reddens the read");
    }

    /// A reviewer distinct from the owner passes — no finding, matching the direct
    /// evaluator (which returns no refusal).
    #[test]
    fn armed_layer_passes_a_reviewer_close_like_the_direct_evaluator() {
        let change = close_change("agent:bob");
        let law = law_at(
            &[(REVIEWER_PATH, REVIEWER_PAGE, REVIEWER_ID, Mode::Block)],
            "tasks/t.md",
        );
        let [armed] = law.rules() else {
            panic!("one rule governs tasks/t.md: {:?}", law.faults());
        };
        let direct = armed.rule().check_change(&change).expect("clean eval");
        assert!(!direct.fired(), "reviewer != owner passes directly");

        let report = evaluate(law.rules(), &change);
        assert!(report.findings.is_empty(), "no armed finding");
        assert!(!report.is_red());
    }

    /// A change outside the rule's `paths:` scope is never evaluated. The row
    /// still GOVERNS the path (armed at the workspace root); what excludes it
    /// is its own declared scope.
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
        let law = law_at(
            &[(REVIEWER_PATH, REVIEWER_PAGE, REVIEWER_ID, Mode::Block)],
            "notes/n.md",
        );
        assert_eq!(
            law.rules().len(),
            1,
            "the row is armed at the workspace root, so it governs this path"
        );

        let report = evaluate(law.rules(), &change);
        assert!(
            report.findings.is_empty() && report.faults.is_empty(),
            "notes/n.md is outside tasks/** — the rule never runs"
        );
    }

    /// A `warn`-armed rule ANNOTATES: its refusal is carried verbatim and the
    /// read stays green.
    #[test]
    fn a_warn_armed_rule_annotates_without_reddening_the_read() {
        let change = close_change("agent:alice"); // owner self-close: fires
        let law = law_at(
            &[(REVIEWER_PATH, REVIEWER_PAGE, REVIEWER_ID, Mode::Warn)],
            "tasks/t.md",
        );

        let report = evaluate(law.rules(), &change);
        assert_eq!(report.findings.len(), 1, "the warn rule fired");
        assert_eq!(report.findings[0].mode, Mode::Warn);
        assert!(
            report.findings[0]
                .refusal
                .message
                .contains("reviewer must not be the owner"),
            "and its words are the rule's own: {:?}",
            report.findings[0].refusal
        );
        assert!(!report.is_red(), "a warn annotates; it does not redden");
    }

    /// An armed HOOK contributes nothing to a check read. The row is armed, in
    /// scope, and present in the resolved law — and the report is still green,
    /// because a reaction has no veto and this report is what reddens a read.
    #[test]
    fn an_armed_hook_never_reddens_a_layer_one_read() {
        let change = close_change("agent:alice"); // the owner self-close, which fires a CHECK
        let law = law_at(
            &[(NOTIFY_PATH, NOTIFY_PAGE, NOTIFY_ID, Mode::Armed)],
            "tasks/t.md",
        );
        let [armed] = law.rules() else {
            panic!(
                "the hook is armed and governs tasks/t.md: {:?}",
                law.faults()
            );
        };
        assert_eq!(armed.mode(), Mode::Armed, "the hook's binary activation");

        let report = evaluate(law.rules(), &change);
        assert!(
            report.findings.is_empty() && report.faults.is_empty(),
            "a hook's firing is the reaction plane's to report, never this one's"
        );
        assert!(!report.is_red());
    }
}
