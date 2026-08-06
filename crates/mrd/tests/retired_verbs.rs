//! Retirement gates — superseded verbs are deleted, not shimmed, and each refusal names its
//! successor.

use std::process::{Command, Output};

fn mrd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .output()
        .expect("run mrd")
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// `mrd pin` is the stage-2 verb, and it is not a shim for the retired one: the old single-page
/// invocation (`mrd pin <PAGE>`, which resolved that pages `^inputs`) is refused exit 2,
/// teaching the lock grammar instead of quietly doing something else with the same arguments.
#[test]
fn mrd_pin_is_the_stage_2_verb_not_a_shim_for_the_retired_one() {
    let out = mrd(&["pin", "some-page.md"]);
    assert_eq!(code(&out), 2, "the retired invocation is a tool failure");
    let err = stderr(&out);
    assert!(
        err.contains("pin needs PAGE and TARGET#SELECTOR"),
        "the refusal teaches the stage-2 grammar: {err}"
    );
    assert!(
        !err.contains("unknown subcommand"),
        "the verb itself is live: {err}"
    );
}

/// `mrd attest` is retired with no alias and no shim. (`attest` = pin + check + receipt; the
/// composite verb returns only once the pin, the check, and the receipt each exist on the new
/// surface.)
#[test]
fn mrd_attest_no_longer_parses() {
    let out = mrd(&["attest", "some-page.md", "--dry"]);
    assert_eq!(code(&out), 2, "the retired verb is a tool failure");
    assert!(
        stderr(&out).contains("unknown subcommand: attest"),
        "the CLI refuses the retired verb: {}",
        stderr(&out)
    );
}

/// USAGE teaches the retired verbs successors, never the retired verbs: no `mrd attest` at all,
/// and the `pin` line is the stage-2 grammar (two operands, `TARGETSELECTOR`) rather than the
/// retired one-page form.
#[test]
fn usage_teaches_the_new_pin_grammar_and_no_retired_verb() {
    let out = mrd(&["attest", "x"]);
    let usage = stderr(&out);
    assert!(
        !usage.contains("mrd attest <PAGE>"),
        "USAGE still teaches a retired verb:\n{usage}"
    );
    assert!(
        !usage.contains("mrd pin <PAGE>\n") && !usage.contains("mrd pin <PAGE> ["),
        "USAGE still teaches the RETIRED one-page pin form:\n{usage}"
    );
    assert!(
        usage.contains("mrd pin <PAGE> <TARGET>#<SELECTOR>"),
        "USAGE teaches the stage-2 pin grammar:\n{usage}"
    );
}

/// `mrd hook` is retired with no alias and no shim; `mrd skill hook` emits the document that
/// says what to place.
#[test]
fn mrd_hook_no_longer_parses_and_usage_teaches_the_emitter() {
    for args in [
        vec!["hook", "install"],
        vec!["hook", "uninstall"],
        vec!["hook", "status"],
        vec!["hook"],
    ] {
        let out = mrd(&args);
        assert_eq!(code(&out), 2, "{args:?} is a retired verb");
        let err = stderr(&out);
        assert!(
            err.contains("unknown subcommand: hook"),
            "the CLI refuses the retired verb rather than shimming it: {err}"
        );
        assert!(
            err.contains("mrd skill hook"),
            "and USAGE — printed on the same refusal — names the successor: {err}"
        );
    }
}

/// USAGE teaches the emitter's contract and no retired hook grammar: no
/// `install`, no `uninstall`, no `status` hanging off `hook`.
#[test]
fn usage_teaches_the_emitter_and_no_retired_hook_grammar() {
    let out = mrd(&["attest", "x"]);
    let usage = stderr(&out);
    assert!(
        !usage.contains("mrd hook <install"),
        "USAGE still teaches the retired hook plane:\n{usage}"
    );
    assert!(
        usage.contains("mrd skill hook"),
        "USAGE teaches the emitter:\n{usage}"
    );
    assert!(
        usage.contains("--commit-gate"),
        "and names the question the emitted body asks — the one thing an operator \
         reading only USAGE most needs to know changed:\n{usage}"
    );
}

/// `mrd journal` is retired with no alias and no shim. The verb existed to reset a ledger the
/// engine kept of its own writes; that ledger is gone, so there is nothing to reset and no
/// successor verb.
#[test]
fn mrd_journal_no_longer_parses_and_usage_teaches_where_history_lives() {
    for args in [
        vec!["journal", "genesis", "--ruling", "some-ruling"],
        vec!["journal", "genesis"],
        vec!["journal"],
    ] {
        let out = mrd(&args);
        assert_eq!(code(&out), 2, "{args:?} is a retired verb");
        let err = stderr(&out);
        assert!(
            err.contains("unknown subcommand: journal"),
            "the CLI refuses the retired verb rather than shimming it: {err}"
        );
        assert!(
            err.contains("mrd test --history"),
            "and USAGE — printed on the same refusal — names the surface that reads \
             history now: {err}"
        );
    }
}

/// USAGE teaches no retired JOURNAL grammar, and the history tiers line says where history
/// actually lives. A reader who only reads USAGE must not be able to conclude that the engine
/// still keeps a ledger.
#[test]
fn usage_teaches_git_history_and_no_retired_journal_grammar() {
    let out = mrd(&["attest", "x"]);
    let usage = stderr(&out);
    assert!(
        !usage.contains("mrd journal"),
        "USAGE still teaches the retired journal plane:\n{usage}"
    );
    assert!(
        !usage.contains("^r-NNNNNN"),
        "USAGE still teaches the retired journal's row anchors:\n{usage}"
    );
    assert!(
        usage.contains("History is git"),
        "USAGE states where history lives now:\n{usage}"
    );
    assert!(
        usage.contains("<commit>:<path>"),
        "and names the item id a golden list declares against:\n{usage}"
    );
}

/// USAGE teaches no retired marker either: `.meridian.toml` is a file the engine reads
/// nowhere. It must name what init actually writes — the roots own `MERIDIAN.md`
/// self-declaration.
#[test]
fn usage_teaches_the_root_declaration_and_no_retired_marker() {
    let out = mrd(&["attest", "x"]);
    let usage = stderr(&out);
    assert!(
        !usage.contains(".meridian.toml") && !usage.contains(".meridian.yaml"),
        "USAGE still advertises a retired marker file:\n{usage}"
    );
    assert!(
        usage.contains("MERIDIAN.md") && usage.contains("type: meridian-root"),
        "USAGE teaches what `mrd init` writes now:\n{usage}"
    );
    assert!(
        usage.contains("--name NAME"),
        "USAGE teaches the name escape:\n{usage}"
    );
}
