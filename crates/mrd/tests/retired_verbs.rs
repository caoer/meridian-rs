//! U12 retirement gates — the superseded pin/attest impl is DELETED, not
//! shimmed (plan D-Remove).
//!
//! # Stage-2 S7 amends the `pin` half of this gate — deliberately
//! M1 deleted the OLD `pin`/`attest` implementation (`^inputs` + `pin_lock`) and
//! pinned that deletion here, naming its own successor: "the replacement surface
//! is the stage-2 pin behavior over `crates/lock` + `model::fingerprint`". Stage
//! 2 §4 S7 builds exactly that verb, and exit criterion 2 is "`mrd pin` mints a
//! real meridian-lock pin" — so "`pin` must not parse" has served its purpose and
//! is now superseded BY THE PLAN, not worked around.
//!
//! What still holds, and is still asserted: `attest` does not parse at all, the
//! new `pin` is NOT a shim for the retired one (it refuses the old invocation
//! shape and speaks the lock grammar instead), and USAGE teaches no retired verb.

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

/// `mrd pin` is the STAGE-2 verb, and it is not a shim for the retired one: the
/// old single-page invocation (`mrd pin <PAGE>`, which resolved that page's
/// `^inputs`) is refused exit 2, teaching the lock grammar instead of quietly
/// doing something else with the same arguments.
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

/// `mrd attest` is retired with no alias and no shim. (`attest` = pin + check +
/// receipt; the composite verb returns only once the pin, the check, and the
/// receipt each exist on the new surface.)
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

/// USAGE teaches the retired verbs' successors, never the retired verbs: no
/// `mrd attest` at all, and the `pin` line is the stage-2 grammar (two operands,
/// `TARGET#SELECTOR`) rather than the retired one-page form.
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

/// **`mrd hook` is retired with no alias and no shim** (hook-rework ruling,
/// 2026-07-29). The install / uninstall / status plane wrote into `$GIT_DIR` and
/// carried an uninstaller, a lock, a downgrade guard and an ownership check to do
/// it safely; `mrd skill hook` emits the document that says what to place instead,
/// and the reader places it.
///
/// The refusal is the CLI's own — the verb name resolves to nothing, so USAGE
/// prints and the successor is one line away. A shim that quietly emitted the
/// document for `mrd hook install` would leave an operator believing a file had
/// been written.
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

/// **`mrd journal` is retired with no alias and no shim** (ZT 2026-08-03:
/// *"Engine does not have memory. It should not have. History is pin to git when
/// we lock. Anything between locks is not history."*). The verb existed to reset
/// a ledger the engine kept of its own writes; that ledger is gone, so there is
/// nothing to reset and no successor verb — the successor is git.
///
/// A shim would be worse here than anywhere else in this file: `journal genesis`
/// MOVED rows and truncated a file, so an operator who typed it and saw success
/// would believe an archive exists.
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

/// USAGE teaches no retired JOURNAL grammar, and the history tier's line says
/// where history actually lives. A reader who only reads USAGE must not be able
/// to conclude that the engine still keeps a ledger.
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

/// USAGE teaches no retired MARKER either (marker-retirement ruling,
/// 2026-07-26). `mrd init`'s line sold `.meridian.toml` — a file the engine now
/// reads nowhere — so the help text was advertising a contract that had died.
/// It must name what init actually writes: the root's own `MERIDIAN.md`
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
