//! Hostility (0003 § heavily tested): malformed Starlark, wrong-typed arguments,
//! and descriptor-forgery attempts all resolve to TYPED errors — never a panic,
//! never a silent wrong effect.

use effects::{EvalError, Rule, eval};

fn ev() -> effects::ChangeEvent {
    effects::ChangeEvent::new("f.md", "a", "b")
}

fn run(src: &str) -> Result<Vec<effects::Effect>, EvalError> {
    eval(&[Rule::new("hostile", src)], &ev())
}

#[test]
fn malformed_syntax_is_parse_error() {
    assert!(matches!(
        run("def on_change(event:\n    pass\n"),
        Err(EvalError::Parse { rule_id, .. }) if rule_id == "hostile"
    ));
}

#[test]
fn missing_on_change_hook_is_typed_missing_entry() {
    // Parses fine, defines no `on_change` — a rule defect, surfaced loud.
    assert!(matches!(
        run("x = 1\n"),
        Err(EvalError::MissingEntry {
            expected: "on_change",
            wrong_plane: None,
            ..
        })
    ));
}

#[test]
fn run_plane_source_fed_a_change_event_is_wrong_plane() {
    // A `def run(ctx)` task addressed as a change-plane rule: the typed error
    // names BOTH what was required and what the source defines.
    assert!(matches!(
        run("def run(ctx):\n    pass\n"),
        Err(EvalError::MissingEntry {
            expected: "on_change",
            wrong_plane: Some("run"),
            ..
        })
    ));
}

#[test]
fn wrong_on_change_arity_is_runtime_error() {
    // `on_change` taking no params cannot receive the injected event.
    assert!(matches!(
        run("def on_change():\n    pass\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn calling_undefined_builtin_is_runtime_error() {
    // `open` is not in the sandbox — a rule reaching for I/O faults, never runs it.
    assert!(matches!(
        run("def on_change(event):\n    open(\"/etc/passwd\")\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn wrong_typed_argument_is_runtime_error() {
    // `set_field(field=...)` wants a string; an int is a type fault at the unpack.
    assert!(matches!(
        run("def on_change(event):\n    set_field(field = 7, value = \"x\")\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn missing_required_argument_is_runtime_error() {
    assert!(matches!(
        run("def on_change(event):\n    set_field(field = \"status\")\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn positional_argument_is_rejected() {
    // Every constructor arg is named-only; a positional call is a fault.
    assert!(matches!(
        run("def on_change(event):\n    notice(\"positional\")\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn send_to_wrong_element_type_is_runtime_error() {
    // `to` is a list of strings; a list of ints fails the unpack.
    assert!(matches!(
        run("def on_change(event):\n    send(to = [1, 2], message = \"x\")\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn raised_error_is_runtime_error() {
    assert!(matches!(
        run("def on_change(event):\n    fail(\"boom\")\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn index_out_of_range_is_runtime_error() {
    assert!(matches!(
        run("def on_change(event):\n    _ = event.sections_changed[999]\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn unknown_event_field_is_runtime_error() {
    assert!(matches!(
        run("def on_change(event):\n    _ = event.no_such_field\n"),
        Err(EvalError::Runtime { .. })
    ));
}

#[test]
fn provenance_cannot_be_forged() {
    // A rule cannot fabricate a foreign rule_id: the constructors take no rule_id
    // argument; the kernel stamps it. Whatever the rule does, the effect carries
    // the caller-assigned id, never an id the rule chose.
    let src = "def on_change(event):\n    notice(message = \"trying to look like someone else\")\n";
    let effects = eval(&[Rule::new("real-id", src)], &ev()).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].rule_id, "real-id");
}

#[test]
fn first_failing_rule_fails_the_batch_loudly() {
    // A good rule then a malformed one: the batch surfaces the malformed rule's
    // typed error naming it (callers validate() at load to separate authoring
    // faults from per-event faults).
    let good = Rule::new(
        "good",
        "def on_change(event):\n    notice(message = \"ok\")\n",
    );
    let bad = Rule::new("bad", "def on_change(event:\n");
    let err = eval(&[good, bad], &ev()).unwrap_err();
    assert!(matches!(err, EvalError::Parse { rule_id, .. } if rule_id == "bad"));
}
