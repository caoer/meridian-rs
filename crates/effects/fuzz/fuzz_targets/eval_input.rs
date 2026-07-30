//! Fuzz BOTH the rule source and the change event together (via `arbitrary`).
//! Same liveness invariant as `eval_source`: always terminates and returns,
//! never panics / hangs / crashes.
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use effects::{eval_with_limits, ChangeEvent, EventFacts, EvalLimits, Rule};

#[derive(Arbitrary, Debug)]
struct Input {
    source: String,
    file: String,
    sections_changed: Vec<String>,
    fields_changed: Vec<String>,
    fingerprint_before: String,
    fingerprint_after: String,
    depth: u32,
}

fn limits() -> EvalLimits {
    EvalLimits {
        fuel: 5_000,
        mem: 4 * 1024 * 1024,
        max_call_depth: 256,
        max_depth: 8,
        max_source_bytes: 16 * 1024,
    }
}

fuzz_target!(|input: Input| {
    let event = ChangeEvent {
        file: input.file,
        sections_changed: input.sections_changed,
        fields_changed: input.fields_changed,
        changes: vec![],
        facts: EventFacts::default(),
        fingerprint_before: input.fingerprint_before,
        fingerprint_after: input.fingerprint_after,
        depth: input.depth,
    };
    let _ = eval_with_limits(&[Rule::new("fuzz", input.source)], &event, limits());
});
