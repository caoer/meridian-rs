//! Robustness fuzz layer (0003 § Testing methodology): the stable-runnable
//! counterpart to the cargo-fuzz targets under `fuzz/`. proptest throws arbitrary
//! and semi-structured Starlark source plus arbitrary events at the kernel; every
//! case must TERMINATE and RETURN (`Ok` or a typed `EvalError`) — never panic,
//! never hang, never crash. If any case looped, the suite would hang; if any
//! panicked, proptest would fail — so a green run IS the liveness proof.

use effects::{ChangeEvent, EvalLimits, EventFacts, Rule, eval_with_limits};
use proptest::prelude::*;

/// Tight limits keep each fuzz case fast (a valid infinite loop still terminates
/// at the fuel bound, just sooner).
fn fuzz_limits() -> EvalLimits {
    EvalLimits {
        fuel: 5_000,
        mem: 4 * 1024 * 1024,
        max_call_depth: 256,
        max_depth: 8,
        max_source_bytes: 16 * 1024,
    }
}

fn any_event() -> impl Strategy<Value = ChangeEvent> {
    (
        ".*",
        prop::collection::vec(".*", 0..4),
        prop::collection::vec(".*", 0..4),
        ".*",
        ".*",
        0u32..12,
    )
        .prop_map(
            |(file, sections, fields, before, after, depth)| ChangeEvent {
                file,
                sections_changed: sections,
                fields_changed: fields,
                changes: vec![],
                facts: EventFacts::default(),
                fingerprint_before: before,
                fingerprint_after: after,
                depth,
            },
        )
}

/// A vocabulary of Starlark-ish fragments, so a meaningful fraction of generated
/// bodies actually parse and exercise the constructors, not just the parser.
fn body_line() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "notice(message = \"x\")".to_string(),
        "set_field(field = \"a\", value = \"b\")".to_string(),
        "send(to = [event.file], message = event.fingerprint_after)".to_string(),
        "append_section(section = \"Log\", content = \"c\")".to_string(),
        "refresh_view(view = \"tasks\")".to_string(),
        "for s in event.sections_changed:".to_string(),
        "if \"status\" in event.fields_changed:".to_string(),
        "x = event.depth + 1".to_string(),
        "_ = event.file".to_string(),
        "pass".to_string(),
        "return".to_string(),
        "1 / 0".to_string(),
        "event.sections_changed[3]".to_string(),
        "open(\"/etc/passwd\")".to_string(),
        "((((".to_string(),
        "not not not".to_string(),
    ])
}

fn structured_source() -> impl Strategy<Value = String> {
    prop::collection::vec(body_line(), 0..8).prop_map(|lines| {
        let mut src = String::from("def on_change(event):\n");
        for line in lines {
            src.push_str("    ");
            src.push_str(&line);
            src.push('\n');
        }
        src
    })
}

/// Fuzz depth: 400 by default (a strong local run), overridable via
/// `PROPTEST_CASES` (proptest's own env knob) so mutation/CI runs can dial it down.
fn fuzz_cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: fuzz_cases(), ..ProptestConfig::default() })]

    /// Arbitrary UTF-8 source: whatever it is, eval returns, never panics.
    #[test]
    fn arbitrary_source_terminates(src in ".{0,2000}", event in any_event()) {
        let _ = eval_with_limits(&[Rule::new("fuzz", src)], &event, fuzz_limits());
    }

    /// Semi-structured source (indentation-valid bodies over a real vocabulary):
    /// exercises the constructors + control flow; still always returns.
    #[test]
    fn structured_source_terminates(src in structured_source(), event in any_event()) {
        let _ = eval_with_limits(&[Rule::new("fuzz", src)], &event, fuzz_limits());
    }

    /// A batch of several fuzzed rules over one event terminates too.
    #[test]
    fn multi_rule_batch_terminates(
        srcs in prop::collection::vec(structured_source(), 0..5),
        event in any_event(),
    ) {
        let rules: Vec<Rule> = srcs
            .into_iter()
            .enumerate()
            .map(|(i, s)| Rule::new(format!("fuzz-{i}"), s))
            .collect();
        let _ = eval_with_limits(&rules, &event, fuzz_limits());
    }
}
