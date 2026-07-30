//! The reaction-plane payload, derived from the change plane (C1a).
//!
//! # Why this lives here and not in `effects`
//! The door already computes everything a reaction needs: [`Change`] carries the
//! before and after [`DocFacts`], so the old and new value of every changed
//! frontmatter key is a subtraction away. The reaction plane had names only
//! (`fields_changed`) and no document facts at all. This module is that
//! subtraction — one function, no new evaluation, no second diff.
//!
//! `effects` cannot do it: it must not depend on `policy` (the edge runs the other
//! way, C1), and it has no world model. So the derivation lives on the side that
//! has the facts, and `effects` owns only the shape.
//!
//! # What this deliberately does NOT carry
//! [`Change`] has an `actor`. The payload does not, and must never: *"delta, facts,
//! now are ALL the names that exist. facts has no agent-state — actor-fact
//! smuggling is a `NameError` at load."* Dropping it here is what makes that law
//! structural instead of a rule someone has to remember. `force` goes with it — an
//! invocation fact, actor-adjacent, and no predicate's business.
//!
//! `now` is **not** injected either, and that is a slice-1 decision rather than a
//! purity one: wire §9 makes `now` a wire INPUT, never something the engine
//! generates, and only the schedule kind needs it. Deferred, and tested as
//! deferred, so nobody wires a clock this slice does not need.
//!
//! # The Delta noun is untouched
//! This is the effects-plane payload, not `wire::Delta` and not
//! `model::delta::NodeDelta`. §7.4 ruled node-grain at birth for the Delta NOUN;
//! deriving old/new from the door's own [`DocFacts`] is precisely how slice 1 gets
//! key-grain facts without reopening that ruling.

use effects::{ChangeEvent, ChangeFact, ChangeFactKind, EventFacts};

use crate::change::{Change, DocFacts};

/// Derive one `on_change` payload from a landed [`Change`].
///
/// The fingerprints and `depth` are the caller's — the engine records nothing it
/// was not given (§9), so it does not invent a cursor coordinate here.
#[must_use]
pub fn derive_event(
    change: &Change,
    fingerprint_before: &str,
    fingerprint_after: &str,
    depth: u32,
) -> ChangeEvent {
    ChangeEvent {
        file: change.doc.path.clone(),
        sections_changed: change
            .sections_changed
            .iter()
            .map(|hpath| hpath.join("/"))
            .collect(),
        fields_changed: change.fields_changed.clone(),
        changes: change_facts(change),
        facts: EventFacts {
            path: change.doc.path.clone(),
            frontmatter: change.doc.frontmatter.clone(),
        },
        fingerprint_before: fingerprint_before.to_string(),
        fingerprint_after: fingerprint_after.to_string(),
        depth,
    }
}

/// The `{kind, key, old, new, hpath}` facts: one per changed frontmatter key,
/// then one per changed section.
///
/// `fields_changed` and `sections_changed` are the door's own state diffs, so this
/// derives no membership of its own — it only attaches the values that membership
/// implies. Two lists in, one list out, and the order is the door's.
fn change_facts(change: &Change) -> Vec<ChangeFact> {
    let mut out = Vec::with_capacity(change.fields_changed.len() + change.sections_changed.len());

    for key in &change.fields_changed {
        out.push(ChangeFact {
            kind: ChangeFactKind::Frontmatter,
            key: key.clone(),
            old: fm_value(&change.before, key),
            new: fm_value(&change.doc, key),
            hpath: Vec::new(),
        });
    }

    for hpath in &change.sections_changed {
        out.push(ChangeFact {
            kind: ChangeFactKind::Section,
            key: String::new(),
            old: None,
            new: None,
            hpath: hpath.clone(),
        });
    }

    out
}

/// One frontmatter value from a document's facts, or `None` when the key is absent
/// on that side. A key added by the change has no `old`; a key deleted has no
/// `new`.
fn fm_value(facts: &DocFacts, key: &str) -> Option<String> {
    facts
        .frontmatter
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{ChangeOp, Invocation, derive_change};
    use effects::{ArgValue, EffectKind, EvalLimits, Rule, eval_with_limits};
    use model::{Document, NodeKind};

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

    fn card(status: &str, extra: &str) -> String {
        format!(
            "---\ntype: task\nstatus: {status}\nreviewer: e4201e72\n{extra}---\n\n\
             # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n"
        )
    }

    /// Derive the payload for a before/after pair, with an actor set — the actor
    /// must never survive the derivation.
    fn event_for(before: &str, after: &str) -> ChangeEvent {
        let b = doc_of("tasks/widget.md", before);
        let a = doc_of("tasks/widget.md", after);
        let change = derive_change(
            &b,
            &a,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some("agent:alice"),
                force: false,
            },
            &[],
            &no_edges,
        );
        assert_eq!(
            change.actor.as_deref(),
            Some("agent:alice"),
            "the CHANGE plane carries the actor — that is what makes dropping it meaningful"
        );
        derive_event(&change, "fp-before", "fp-after", 0)
    }

    /// **THE ACCEPTANCE TEST.** The founding rule page's predicate
    /// (`foundation-panel/round-2/demo/rules/task-review-notify.md`, 2026-07-18),
    /// with `owner` renamed to `reviewer` per ZT 24-01, translated onto the
    /// ratified `on_change(event)` entry — the panel's `when(delta, facts, now)` is
    /// this same entry under the design's older name, so the signature was always
    /// going to be rewritten.
    ///
    /// `send` stands in for the panel's `intent(...)`: `intent` and `receipt_addr`
    /// are C2's constructors and are deliberately NOT pulled forward. What this
    /// card owes is that the PAYLOAD can express the predicate; C2 owes the
    /// intent shape and the receipt.
    const FOUNDING_PREDICATE: &str = "\
def on_change(event):
    for delta in event.changes:
        if delta.kind != \"frontmatter\" or delta.key != \"status\":
            continue
        if delta.new != \"review\":
            continue
        reviewer = event.facts.fm.get(\"reviewer\")
        send(to = [reviewer], message = \"task %s → review\" % event.file)
";

    fn run(event: &ChangeEvent) -> Vec<effects::Effect> {
        eval_with_limits(
            &[Rule::new("task-review-notify", FOUNDING_PREDICATE)],
            event,
            EvalLimits::default(),
        )
        .expect("the founding predicate evaluates")
    }

    /// The founding page's first fixture: `test_move_to_review_arms_one_intent`.
    /// A move to review arms exactly ONE effect, aimed at the reviewer named on the
    /// card — the target read off the document, which is the whole of gap 2.
    #[test]
    fn move_to_review_arms_one_effect_aimed_at_the_card_s_reviewer() {
        let event = event_for(&card("in-progress", ""), &card("review", ""));
        println!("POPULATION changes = {:?}", event.changes);
        println!("POPULATION facts   = {:?}", event.facts);

        let got = run(&event);
        println!("POPULATION effects = {got:?}");
        assert_eq!(got.len(), 1, "exactly one effect is armed: {got:?}");
        assert_eq!(got[0].kind, EffectKind::Send);
        assert_eq!(
            got[0].args.get("to"),
            Some(&ArgValue::List(vec!["e4201e72".to_string()])),
            "the target came off the card, not from anywhere else"
        );
    }

    /// The founding page's second fixture: `test_other_key_changes_are_silent`.
    #[test]
    fn other_key_changes_are_silent() {
        let event = event_for(
            &card("in-progress", ""),
            "---\ntype: task\nstatus: in-progress\nreviewer: 0523d4bb\n---\n\n\
             # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n",
        );
        println!("POPULATION changes = {:?}", event.changes);
        let got = run(&event);
        println!("POPULATION effects = {got:?}");
        assert!(
            got.is_empty(),
            "a reviewer change is not a status move: {got:?}"
        );
    }

    /// A MUTATION CONTROL PER CONJUNCT. The predicate fires on
    /// `kind == frontmatter` ∧ `key == status` ∧ `new == review`. Each row breaks
    /// exactly ONE conjunct and must silence it; the unbroken case fires. Without
    /// this, a predicate that ignored two of the three would pass the acceptance
    /// test above.
    #[test]
    fn each_conjunct_of_the_predicate_is_load_bearing() {
        // conjunct 3 broken: status moves, but not to `review`.
        let to_blocked = event_for(&card("in-progress", ""), &card("blocked", ""));
        println!("POPULATION new!=review changes = {:?}", to_blocked.changes);
        assert!(
            run(&to_blocked).is_empty(),
            "status → blocked is not a move to review"
        );

        // conjunct 2 broken: a frontmatter key moves to "review", but not `status`.
        let other_key = event_for(
            &card("in-progress", "stage: draft\n"),
            &card("in-progress", "stage: review\n"),
        );
        println!("POPULATION key!=status changes = {:?}", other_key.changes);
        assert!(
            run(&other_key).is_empty(),
            "another key reaching the value `review` must not fire"
        );

        // conjunct 1 broken: a SECTION changed, never a frontmatter key.
        let section_only = event_for(
            &card("in-progress", ""),
            &card("in-progress", "").replace("Ship the widget.", "Ship it faster."),
        );
        let kinds: Vec<&str> = section_only
            .changes
            .iter()
            .map(|c| c.kind.as_str())
            .collect();
        println!("POPULATION section-only kinds = {kinds:?}");
        assert!(
            section_only
                .changes
                .iter()
                .all(|c| c.kind == ChangeFactKind::Section),
            "the control must actually produce only section facts: {:?}",
            section_only.changes
        );
        assert!(
            run(&section_only).is_empty(),
            "a body edit is not a status move"
        );

        // and the unbroken case still fires — so the silences above are the
        // mutations' doing, not a predicate that never fires at all.
        let fires = event_for(&card("in-progress", ""), &card("review", ""));
        assert_eq!(run(&fires).len(), 1, "the baseline still arms one effect");
    }

    /// 🔴 **The observer exclusion, enforced structurally.** `policy::Change`
    /// carries an actor; the payload does not, so a predicate reaching for one gets
    /// a NameError-shaped fault rather than an answer. This is the law *"the engine
    /// must not observe the observer"* made unrepresentable instead of merely
    /// forbidden.
    #[test]
    fn the_payload_cannot_observe_the_observer() {
        let event = event_for(&card("in-progress", ""), &card("review", ""));

        for reach in ["event.actor", "event.force", "event.facts.actor"] {
            let src = format!("def on_change(event):\n    x = {reach}\n");
            let err =
                eval_with_limits(&[Rule::new("smuggler", src)], &event, EvalLimits::default())
                    .expect_err("an actor/session fact must not be reachable");
            println!("POPULATION {reach} -> {err}");
        }

        // The control: a field that IS on the surface resolves, so the failures
        // above are about those names and not about attribute access being broken.
        let ok = eval_with_limits(
            &[Rule::new(
                "reader",
                "def on_change(event):\n    x = event.facts.fm\n",
            )],
            &event,
            EvalLimits::default(),
        );
        assert!(ok.is_ok(), "a granted fact must still read: {ok:?}");
    }

    /// `now` is DEFERRED, not forgotten: it is a wire input the schedule kind needs
    /// and slice 1 does not. A predicate naming it faults, so the absence is
    /// visible rather than silently defaulting to a clock.
    #[test]
    fn now_is_deferred_and_its_absence_is_loud() {
        let event = event_for(&card("in-progress", ""), &card("review", ""));
        let err = eval_with_limits(
            &[Rule::new("clock", "def on_change(event):\n    x = now()\n")],
            &event,
            EvalLimits::default(),
        )
        .expect_err("`now` is not injected in slice 1");
        println!("POPULATION now() -> {err}");
    }

    /// 🔴 **S-2 does not reach this payload, and S-1 does not either** — the reason
    /// C0's ruling pointed C1a at `derive_change` rather than at Delta node entries.
    ///
    /// The gated close (status flip AND an appended `## Verdict`, one put) is the
    /// exact shape where `model::delta` emitted NOTHING and, appended alone, named
    /// only the PARENT section. The state diff names the parent, the sibling whose
    /// span moved, AND the new section by its own hpath.
    #[test]
    fn the_delta_grain_findings_do_not_reach_the_derived_payload() {
        let before = card("in-progress", "");
        let after = format!(
            "{}\n## Verdict\n\npassed - gates green\n",
            card("review", "").trim_end()
        );
        let event = event_for(&before, &after);

        println!("POPULATION fields_changed   = {:?}", event.fields_changed);
        println!("POPULATION sections_changed = {:?}", event.sections_changed);

        // S-1's shape: names AND values, no rev of any grain anywhere.
        let status: Vec<&ChangeFact> = event
            .changes
            .iter()
            .filter(|c| c.kind == ChangeFactKind::Frontmatter && c.key == "status")
            .collect();
        assert_eq!(status.len(), 1, "{:?}", event.changes);
        assert_eq!(status[0].old.as_deref(), Some("in-progress"));
        assert_eq!(status[0].new.as_deref(), Some("review"));

        // S-2's shape: the appended section is named BY ITS OWN HPATH, not merely
        // as an edit of its parent.
        let sections: Vec<&Vec<String>> = event
            .changes
            .iter()
            .filter(|c| c.kind == ChangeFactKind::Section)
            .map(|c| &c.hpath)
            .collect();
        println!("POPULATION section hpaths = {sections:?}");
        assert!(
            sections.iter().any(
                |h| h.as_slice() == ["Task: widget-rollout".to_string(), "Verdict".to_string()]
            ),
            "the appended section must be named in its own right: {sections:?}"
        );
        assert!(
            sections
                .iter()
                .any(|h| h.as_slice() == ["Task: widget-rollout".to_string()]),
            "and its parent too — the state diff is honest about implicit rev changes"
        );

        // The predicate still fires on exactly this shape: the gated close is the
        // write a status subscription exists to catch, and it now catches it.
        let got = run(&event);
        assert_eq!(got.len(), 1, "the gated close arms one effect: {got:?}");
    }

    /// Absence is carried as absence: a key added by the change has no `old`, a key
    /// removed has no `new`. `""` would make "empty" and "not there" the same fact.
    #[test]
    fn added_and_removed_keys_carry_absence_not_empty_string() {
        let added = event_for(
            &card("in-progress", ""),
            &card("in-progress", "blocked_by: x\n"),
        );
        let fact = added
            .changes
            .iter()
            .find(|c| c.key == "blocked_by")
            .expect("the added key is a change fact");
        println!("POPULATION added = {fact:?}");
        assert_eq!(fact.old, None, "added: no previous value");
        assert_eq!(fact.new.as_deref(), Some("x"));

        let removed = event_for(
            &card("in-progress", "blocked_by: x\n"),
            &card("in-progress", ""),
        );
        let fact = removed
            .changes
            .iter()
            .find(|c| c.key == "blocked_by")
            .expect("the removed key is a change fact");
        println!("POPULATION removed = {fact:?}");
        assert_eq!(fact.old.as_deref(), Some("x"));
        assert_eq!(fact.new, None, "removed: no current value");
    }
}
