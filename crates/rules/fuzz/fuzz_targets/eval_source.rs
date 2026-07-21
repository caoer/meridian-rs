//! Fuzz the rule SOURCE: arbitrary bytes as Starlark, over a fixed event. The
//! invariant is liveness — eval must TERMINATE and RETURN (`Ok` or a typed
//! `EvalError`), never panic, never hang, never abort the process. libfuzzer
//! reports any panic / timeout / crash as a finding.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rules::{eval_with_limits, ChangeEvent, EvalLimits, Rule};

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
