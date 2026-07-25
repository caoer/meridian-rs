//! U12 retirement gates — the superseded pin/attest impl is DELETED, not
//! shimmed (plan D-Remove; the migrate-kit self-retirement discipline).
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
