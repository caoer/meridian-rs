//! Purity (0003 §1: zero I/O): the sandbox exposes no filesystem or network
//! capability. The only builtins a rule reaches are the effect-descriptor
//! constructors (which read nothing and only record inert data) plus the Starlark
//! standard library (which has no ambient I/O). Any I/O-shaped name is undefined
//! and faults — it is never linked, never called.
//!
//! (The exact global-name surface is asserted as an in-crate unit test in
//! `kernel.rs`, which can reach `effect_globals()` directly.)

use rules::{EffectKind, EvalError, Rule, eval};

fn ev() -> rules::ChangeEvent {
    rules::ChangeEvent::new("f.md", "a", "b")
}

fn run(src: &str) -> Result<Vec<rules::Effect>, EvalError> {
    eval(&[Rule::new("purity", src)], &ev())
}

/// Every name a rule might reach for I/O is UNDEFINED — the rule faults rather
/// than performing the operation.
#[test]
fn io_shaped_names_are_undefined() {
    for name in [
        "open",
        "read",
        "write",
        "read_file",
        "write_file",
        "os",
        "sys",
        "socket",
        "exec",
        "system",
        "input",
        "eval",
        "compile",
        "import",
        "__import__",
        "requests",
        "urllib",
        "subprocess",
        // No stream builtins either — the standard set omits these.
        "print",
        "pprint",
        "debug",
    ] {
        let src = format!("def on_change(event):\n    {name}\n");
        // Unusable is the property: undefined (Runtime) or a reserved keyword
        // (Parse) — either way a rule cannot reach it to perform I/O; never Ok.
        assert!(
            run(&src).is_err(),
            "name `{name}` must not be usable in the sandbox"
        );
    }
}

/// `load(...)` — module loading — is disabled at the dialect: a rule cannot pull
/// in external symbols. It is a PARSE error, refused before any evaluation.
#[test]
fn load_statement_is_rejected() {
    let src = "def on_change(event):\n    pass\nload(\"other.star\", \"sym\")\n";
    assert!(
        matches!(run(src), Err(EvalError::Parse { .. })),
        "load() must be a parse error (dialect disables it)"
    );
}

/// All nine descriptor constructors ARE defined — the closed capability surface
/// is present in full. Each fires exactly one effect of its kind.
#[test]
fn all_constructors_are_defined_and_emit_their_kind() {
    let cases: &[(&str, EffectKind)] = &[
        (
            "set_field(field = \"s\", value = \"v\")",
            EffectKind::SetField,
        ),
        (
            "append_section(section = \"Log\", content = \"c\")",
            EffectKind::AppendSection,
        ),
        ("refresh_view(view = \"tasks\")", EffectKind::RefreshView),
        ("send(to = [\"a\"], message = \"m\")", EffectKind::Send),
        ("remind(message = \"m\")", EffectKind::Remind),
        ("ask(message = \"m\")", EffectKind::Ask),
        ("notice(message = \"m\")", EffectKind::Notice),
        ("warn(message = \"m\")", EffectKind::Warn),
        ("reject(message = \"m\")", EffectKind::Reject),
    ];
    for (call, kind) in cases {
        let src = format!("def on_change(event):\n    {call}\n");
        let effects = run(&src).unwrap_or_else(|e| panic!("`{call}` should emit: {e}"));
        assert_eq!(effects.len(), 1, "`{call}` emits one effect");
        assert_eq!(effects[0].kind, *kind, "`{call}` kind");
    }
}

/// The crate's own dependency graph carries no I/O runtime (tokio/mio/hyper/…).
/// A structural proof that the purity is not merely unexercised but unlinked.
#[test]
fn dependency_graph_has_no_io_runtime() {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let out = std::process::Command::new(env!("CARGO"))
        .args([
            "tree", "-p", "rules", "--edges", "normal", "--prefix", "none",
        ])
        .arg("--manifest-path")
        .arg(manifest)
        .output();
    let Ok(out) = out else {
        // No cargo in this environment — skip rather than fail spuriously.
        return;
    };
    if !out.status.success() {
        return;
    }
    let tree = String::from_utf8_lossy(&out.stdout).to_lowercase();
    for forbidden in ["tokio", "mio", "hyper", "reqwest", "async-std", "socket2"] {
        assert!(
            !tree.contains(forbidden),
            "rules must not link an I/O runtime, found `{forbidden}` in cargo tree"
        );
    }
}
