//! The run-plane entry (verdict rulings 1/8): `run(ctx)` evaluates in the SAME
//! sealed sandbox as `on_change` — same closed globals, same fuel/mem/depth
//! metering — stamps `Provenance::Run`, carries no idempotency key, and can
//! never spawn a process (starlark-invokes-bash is a permanent no).

use std::collections::BTreeMap;

use rules::{
    Effect, EffectKind, EvalError, EvalLimits, Provenance, Rule, RunCtx, eval_run,
};

fn ctx() -> RunCtx {
    RunCtx {
        page: "wiki/tasks.md".to_owned(),
        task: "fix-drift".to_owned(),
        args: vec!["a0".to_owned(), "a1".to_owned()],
        env: BTreeMap::from([("HOME_WIKI".to_owned(), "/wiki".to_owned())]),
        invocation_id: "inv-0001".to_owned(),
        root_at_eval: "b3:root".to_owned(),
    }
}

fn task(src: &str) -> Rule {
    Rule::new("fix-drift", src)
}

fn run(src: &str) -> Result<Vec<Effect>, EvalError> {
    eval_run(&task(src), &ctx(), EvalLimits::default())
}

#[test]
fn run_entry_evaluates_and_stamps_run_provenance() {
    let effects = run(
        "def run(ctx):\n    set_field(field = \"status\", value = \"done\")\n    notice(message = ctx.page)\n",
    )
    .unwrap();
    assert_eq!(effects.len(), 2);
    for (i, e) in effects.iter().enumerate() {
        assert_eq!(e.rule_id, "fix-drift", "rule_id is the task name");
        assert_eq!(e.seq, u32::try_from(i).unwrap());
        assert_eq!(e.depth, 0, "run plane is depth 0");
        assert_eq!(
            e.provenance,
            Provenance::Run {
                invocation_id: "inv-0001".to_owned(),
                root_at_eval: "b3:root".to_owned(),
            }
        );
    }
    assert_eq!(effects[0].kind, EffectKind::SetField);
    assert_eq!(effects[1].kind, EffectKind::Notice);
}

#[test]
fn run_plane_effects_carry_no_idempotency_key() {
    let effects = run("def run(ctx):\n    notice(message = \"n\")\n").unwrap();
    assert_eq!(effects[0].idempotency_key(), None);
}

#[test]
fn ctx_exposes_page_task_args_env() {
    let effects = run(
        "def run(ctx):\n    send(to = [ctx.task], message = ctx.page + \" \" + ctx.args[1] + \" \" + ctx.env[\"HOME_WIKI\"])\n",
    )
    .unwrap();
    let rules::ArgValue::Str(message) = &effects[0].args["message"] else {
        panic!("message is a scalar");
    };
    assert_eq!(message, "wiki/tasks.md a1 /wiki");
}

#[test]
fn ctx_does_not_expose_invocation_identity() {
    // The provenance facts are stamped, never injected — a task's OUTPUT must
    // not depend on its invocation identity. Reaching for them faults.
    for field in ["invocation_id", "root_at_eval"] {
        let src = format!("def run(ctx):\n    notice(message = ctx.{field})\n");
        assert!(
            matches!(
                eval_run(&task(&src), &ctx(), EvalLimits::default()),
                Err(EvalError::Runtime { .. })
            ),
            "ctx.{field} must not exist"
        );
    }
}

#[test]
fn task_id_ctx_task_divergence_is_refused() {
    // f9542789 U3-gate: error provenance (task.id) and effect provenance
    // (ctx.task) must be ONE identity — a divergent pair is refused.
    let t = Rule::new("other-name", "def run(ctx):\n    pass\n");
    let err = eval_run(&t, &ctx(), EvalLimits::default()).unwrap_err();
    let EvalError::Runtime { rule_id, reason } = err else {
        panic!("expected Runtime refusal");
    };
    assert_eq!(rule_id, "other-name");
    assert!(reason.contains("one identity"), "{reason}");
}

#[test]
fn absent_run_entry_is_typed_missing_entry() {
    assert!(matches!(
        run("x = 1\n"),
        Err(EvalError::MissingEntry {
            expected: "run",
            wrong_plane: None,
            ..
        })
    ));
}

#[test]
fn change_plane_source_addressed_as_task_is_wrong_plane() {
    let err = run("def on_change(event):\n    pass\n").unwrap_err();
    assert_eq!(
        err,
        EvalError::MissingEntry {
            rule_id: "fix-drift".to_owned(),
            expected: "run",
            wrong_plane: Some("on_change"),
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("run") && msg.contains("on_change"), "{msg}");
}

#[test]
fn run_plane_fuel_metering_terminates_a_bomb() {
    // Same budget machinery as on_change: a runaway loop is terminated, typed.
    // Tick-bound bomb (allocation-light) — the exact tick accounting is the
    // authoritative budget signal.
    let src = "def run(ctx):\n    for i in range(1000000000):\n        pass\n";
    assert!(matches!(run(src), Err(EvalError::Budget { .. })));
}

#[test]
fn run_plane_recursion_bomb_is_budget() {
    let src = "def f(n):\n    return f(n + 1)\n\ndef run(ctx):\n    f(0)\n";
    assert!(matches!(run(src), Err(EvalError::Budget { .. })));
}

#[test]
fn run_plane_source_guards_apply() {
    // The parse-DoS guards are plane-independent (shared metered path).
    let over = "x".repeat(EvalLimits::default().max_source_bytes + 1);
    assert!(matches!(
        run(&over),
        Err(EvalError::SourceTooLarge { .. })
    ));
}

#[test]
fn sandbox_exposes_no_process_or_io_names() {
    // Ruling 8: starlark-invokes-bash is a PERMANENT no. The run plane uses
    // the same closed globals as on_change — assert the exec-shaped names are
    // absent behaviorally: reaching one faults, never runs.
    for name in ["exec", "subprocess", "os", "system", "spawn", "popen", "command", "shell", "open", "getenv"] {
        let src = format!("def run(ctx):\n    {name}(\"true\")\n");
        assert!(
            matches!(
                eval_run(&task(&src), &ctx(), EvalLimits::default()),
                Err(EvalError::Runtime { .. })
            ),
            "sandbox must not bind `{name}`"
        );
    }
}

#[test]
fn run_eval_is_deterministic_across_repeats() {
    let src = "def run(ctx):\n    for a in ctx.args:\n        warn(message = a)\n    append_section(section = \"Log\", content = ctx.env[\"HOME_WIKI\"])\n";
    let a = run(src).unwrap();
    let b = run(src).unwrap();
    assert_eq!(a, b);
    let ja = serde_json::to_string(&a).unwrap();
    let jb = serde_json::to_string(&b).unwrap();
    assert_eq!(ja, jb);
    assert!(ja.contains("\"plane\":\"run\""));
}
