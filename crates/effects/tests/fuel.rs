//! Fuel/bomb: runaway loop, recursion, huge alloc → budget error; never hang or
//! crash.

use effects::{EvalError, EvalLimits, Rule, eval_with_limits};

fn tight() -> EvalLimits {
    EvalLimits {
        fuel: 100_000,
        mem: 8 * 1024 * 1024,
        max_call_depth: 1000,
        max_depth: 8,
        max_source_bytes: 64 * 1024,
    }
}

fn ev() -> effects::ChangeEvent {
    effects::ChangeEvent::new("f.md", "a", "b")
}

fn run(src: &str, limits: EvalLimits) -> Result<Vec<effects::Effect>, EvalError> {
    eval_with_limits(&[Rule::new("bomb", src)], &ev(), limits)
}

#[test]
fn runaway_while_loop_terminates_as_budget() {
    let src =
        "def on_change(event):\n    x = 0\n    for _ in range(100000000):\n        x = x + 1\n";
    assert!(matches!(run(src, tight()), Err(EvalError::Budget { .. })));
}

#[test]
fn unbounded_recursion_terminates_as_budget() {
    // The tick guard stops runtime recursion at a call boundary before the
    // native stack overflows.
    let src = "def rec(n):\n    return rec(n + 1)\ndef on_change(event):\n    rec(0)\n";
    assert!(matches!(run(src, tight()), Err(EvalError::Budget { .. })));
}

#[test]
fn huge_single_allocation_is_metered_not_admitted() {
    // A non-looping allocation defeats the coarse runaway guard (no backedge),
    // but the exact post-eval mem accounting still refuses it as budget. 8 MiB of
    // string over a 1 MiB budget — well past the bound, quick to allocate.
    let src = "def on_change(event):\n    s = \"x\" * 8000000\n    _ = len(s)\n";
    let limits = EvalLimits {
        mem: 1024 * 1024,
        ..tight()
    };
    assert!(matches!(run(src, limits), Err(EvalError::Budget { .. })));
}

#[test]
fn list_blowup_terminates_as_budget() {
    let src = "def on_change(event):\n    xs = list(range(2000000))\n    _ = len(xs)\n";
    let limits = EvalLimits {
        mem: 1024 * 1024,
        ..tight()
    };
    assert!(matches!(run(src, limits), Err(EvalError::Budget { .. })));
}

#[test]
fn tiny_mem_budget_refuses_even_a_noop() {
    // A sub-chunk mem budget always exhausts (the arena's minimum chunk exceeds
    // it) — the conservative direction for a resource gate.
    let src = "def on_change(event):\n    pass\n";
    let limits = EvalLimits { mem: 2, ..tight() };
    assert!(matches!(run(src, limits), Err(EvalError::Budget { .. })));
}

#[test]
fn one_tick_budget_exhausts_a_real_rule() {
    let src = "def on_change(event):\n    for _ in range(10):\n        notice(message = \"x\")\n";
    let limits = EvalLimits { fuel: 1, ..tight() };
    assert!(matches!(run(src, limits), Err(EvalError::Budget { .. })));
}

#[test]
fn deeply_nested_unary_is_rejected_not_crashed() {
    // starlark-rust parser stack-overflow bug (issue #66): a long unary chain
    // would recurse the parser and abort the process, uncatchable by
    // catch_unwind. The pre-parse nesting guard rejects it before the parser.
    let mut src = String::from("def on_change(event):\n    x = ");
    src.push_str(&"-".repeat(50_000));
    src.push_str("1\n");
    assert!(matches!(run(&src, tight()), Err(EvalError::Parse { .. })));
}

#[test]
fn deeply_nested_not_chain_is_rejected() {
    // The literal issue-#66 shape: `not not not …`.
    let mut src = String::from("def on_change(event):\n    x = ");
    for _ in 0..5_000 {
        src.push_str("not ");
    }
    src.push_str("True\n");
    assert!(matches!(run(&src, tight()), Err(EvalError::Parse { .. })));
}

#[test]
fn nested_brackets_are_rejected_not_crashed() {
    // The other parser-recursion vector: nested list literals.
    let depth = 5_000;
    let mut src = String::from("def on_change(event):\n    x = ");
    src.push_str(&"[".repeat(depth));
    src.push_str(&"]".repeat(depth));
    src.push('\n');
    assert!(matches!(run(&src, tight()), Err(EvalError::Parse { .. })));
}

#[test]
fn shallow_but_bracket_heavy_source_is_admitted() {
    // Many brackets but shallow nesting must not be rejected — the guard
    // bounds depth, not count.
    use std::fmt::Write as _;
    let mut body = String::from("def on_change(event):\n");
    for i in 0..400 {
        let _ = writeln!(body, "    _{i} = (1, 2)");
    }
    // Parses and runs cleanly (no effects); the guard does not false-trip.
    assert!(run(&body, tight()).is_ok());
}

#[test]
fn source_over_cap_is_refused_before_parsing() {
    let big = "#".repeat(2000);
    let src = format!("def on_change(event):\n    pass\n{big}");
    let limits = EvalLimits {
        max_source_bytes: 100,
        ..tight()
    };
    assert!(matches!(
        run(&src, limits),
        Err(EvalError::SourceTooLarge { .. })
    ));
}

#[test]
fn source_exactly_at_cap_is_admitted() {
    // Boundary: len == max_source_bytes is allowed (the cap is `>`, not `>=`).
    let src = "def on_change(event):\n    pass\n";
    let limits = EvalLimits {
        max_source_bytes: src.len(),
        ..tight()
    };
    assert!(
        run(src, limits).is_ok(),
        "a rule exactly at the cap must be admitted"
    );
    // One byte over the same cap is refused.
    let over = format!("{src} ");
    assert!(matches!(
        run(&over, limits),
        Err(EvalError::SourceTooLarge { .. })
    ));
}
