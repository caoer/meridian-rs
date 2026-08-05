//! Fuzz rule source (fixed event): eval must terminate and return — never
//! panic, hang, or abort.
#![no_main]

use libfuzzer_sys::fuzz_target;
use effects::{eval_with_limits, ChangeEvent, EvalLimits, Rule};

fn limits() -> EvalLimits {
    EvalLimits {
        fuel: 5_000,
        mem: 4 * 1024 * 1024,
        max_call_depth: 256,
        max_depth: 8,
        max_source_bytes: 16 * 1024,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let event = ChangeEvent::new("fuzz.md", "before", "after");
    let _ = eval_with_limits(&[Rule::new("fuzz", src)], &event, limits());
});
