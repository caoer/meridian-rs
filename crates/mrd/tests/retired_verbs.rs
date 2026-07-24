//! U12 retirement gates — the superseded pin/attest impl is DELETED, not
//! shimmed (plan D-Remove; the migrate-kit self-retirement discipline). The
//! verbs no longer parse; the CLI refuses them loudly (exit 2), and the USAGE
//! text no longer teaches them. The replacement surface is the stage-2 pin
//! behavior over `crates/lock` + `model::fingerprint` — never these verbs.

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

/// `mrd pin` is retired the same release — no alias, no shim.
#[test]
fn mrd_pin_no_longer_parses() {
    let out = mrd(&["pin", "some-page.md"]);
    assert_eq!(code(&out), 2, "the retired verb is a tool failure");
    assert!(
        stderr(&out).contains("unknown subcommand: pin"),
        "the CLI refuses the retired verb: {}",
        stderr(&out)
    );
}

/// `mrd attest` is retired the same release — no alias, no shim.
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

/// The USAGE text teaches neither retired verb (it IS printed on the refusal).
#[test]
fn usage_no_longer_lists_the_retired_verbs() {
    let out = mrd(&["pin", "x"]);
    let usage = stderr(&out);
    assert!(
        !usage.contains("mrd pin <PAGE>") && !usage.contains("mrd attest <PAGE>"),
        "USAGE still teaches a retired verb:\n{usage}"
    );
}
