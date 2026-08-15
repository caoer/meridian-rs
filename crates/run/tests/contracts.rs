//! Contract gates: declaration parsing and the pre-eval input validation.

mod support;

use std::collections::BTreeMap;

use run::contracts::{self, Contract, ContractError, ContractViolation};
use support::{PAGE, doc};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn contract_parses_args_and_env_declarations() {
    let d = doc(PAGE);
    let c = contracts::contract_for(&d, "fix-drift").unwrap();
    assert_eq!(c.args, vec!["page".to_owned()]);
    assert_eq!(c.env, vec!["HOME_WIKI".to_owned()]);
}

#[test]
fn undeclared_task_has_the_empty_contract() {
    let d = doc(PAGE);
    let c = contracts::contract_for(&d, "check-links").unwrap();
    assert_eq!(c, Contract::default());
}

#[test]
fn valid_inputs_pass() {
    let d = doc(PAGE);
    let c = contracts::contract_for(&d, "fix-drift").unwrap();
    contracts::validate(
        "fix-drift",
        &c,
        &args(&["some/page.md"]),
        &env(&[("HOME_WIKI", "/wiki")]),
    )
    .unwrap();
}

#[test]
fn wrong_arg_count_names_the_declared_args() {
    let d = doc(PAGE);
    let c = contracts::contract_for(&d, "fix-drift").unwrap();
    let err = contracts::validate("fix-drift", &c, &[], &env(&[("HOME_WIKI", "/w")])).unwrap_err();
    assert_eq!(
        err,
        ContractViolation::ArgCount {
            task: "fix-drift".to_owned(),
            expected: vec!["page".to_owned()],
            variadic: false,
            got: 0,
        }
    );
}

/// A page declaring a fixed slot plus a tail: `task.fmt.args: title, rows...`.
const TAIL_PAGE: &str = "---\ntask.fmt: \"[[#^a-1]]\"\ntask.fmt.args: title, rows...\n---\n";

#[test]
fn tail_suffix_parses_as_a_variadic_last_slot() {
    let c = contracts::contract_for(&doc(TAIL_PAGE), "fmt").unwrap();
    assert_eq!(c.args, vec!["title".to_owned(), "rows".to_owned()]);
    assert!(c.variadic);
    assert_eq!(c.min_args(), 1);
    // Every face echoes the declaration as written.
    assert_eq!(
        c.args_declared(),
        vec!["title".to_owned(), "rows...".to_owned()]
    );
}

#[test]
fn a_tail_takes_any_count_including_none() {
    let c = contracts::contract_for(&doc(TAIL_PAGE), "fmt").unwrap();
    for supplied in [
        args(&["t"]),
        args(&["t", "r1"]),
        args(&["t", "r1", "r2", "r3"]),
    ] {
        contracts::validate("fmt", &c, &supplied, &env(&[])).unwrap();
    }
}

#[test]
fn a_tail_still_enforces_the_fixed_prefix() {
    let c = contracts::contract_for(&doc(TAIL_PAGE), "fmt").unwrap();
    let err = contracts::validate("fmt", &c, &[], &env(&[])).unwrap_err();
    assert_eq!(
        err,
        ContractViolation::ArgCount {
            task: "fmt".to_owned(),
            expected: vec!["title".to_owned(), "rows...".to_owned()],
            variadic: true,
            got: 0,
        }
    );
    assert_eq!(
        err.to_string(),
        "task 'fmt' takes at least 1 arg(s) (title, rows...), got 0"
    );
}

#[test]
fn a_tail_before_the_last_arg_is_an_authoring_fault() {
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.args: rows..., title\n---\n";
    let err = contracts::contract_for(&doc(page), "t").unwrap_err();
    assert_eq!(
        err,
        ContractError::TailNotLast {
            task: "t".to_owned(),
            name: "rows".to_owned(),
        }
    );
}

#[test]
fn env_refuses_the_tail_suffix() {
    // The name parser is shared with args, so env refuses the suffix in its
    // own words rather than accepting it silently.
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.env: HOME_WIKI...\n---\n";
    let err = contracts::contract_for(&doc(page), "t").unwrap_err();
    assert_eq!(
        err,
        ContractError::TailOnEnv {
            task: "t".to_owned(),
            name: "HOME_WIKI".to_owned(),
        }
    );
}

#[test]
fn a_bare_tail_with_no_name_is_an_authoring_fault() {
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.args: ...\n---\n";
    let err = contracts::contract_for(&doc(page), "t").unwrap_err();
    assert_eq!(
        err,
        ContractError::BadName {
            task: "t".to_owned(),
            name: "...".to_owned(),
        }
    );
}

#[test]
fn missing_declared_env_refuses() {
    let d = doc(PAGE);
    let c = contracts::contract_for(&d, "fix-drift").unwrap();
    let err = contracts::validate("fix-drift", &c, &args(&["p.md"]), &env(&[])).unwrap_err();
    assert_eq!(
        err,
        ContractViolation::MissingEnv {
            task: "fix-drift".to_owned(),
            name: "HOME_WIKI".to_owned(),
        }
    );
}

#[test]
fn undeclared_supplied_env_refuses() {
    // Deny-by-default covers inputs: an env key the task never declared is a
    // typo or a smuggle — refused either way.
    let d = doc(PAGE);
    let c = contracts::contract_for(&d, "fix-drift").unwrap();
    let err = contracts::validate(
        "fix-drift",
        &c,
        &args(&["p.md"]),
        &env(&[("HOME_WIKI", "/w"), ("SNEAKY", "x")]),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractViolation::UndeclaredEnv {
            task: "fix-drift".to_owned(),
            name: "SNEAKY".to_owned(),
        }
    );
}

#[test]
fn duplicate_declared_name_is_an_authoring_fault() {
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.env: HOME_WIKI HOME_WIKI\n---\n";
    let err = contracts::contract_for(&doc(page), "t").unwrap_err();
    assert_eq!(
        err,
        ContractError::DuplicateName {
            task: "t".to_owned(),
            name: "HOME_WIKI".to_owned(),
        }
    );
}

#[test]
fn invalid_declared_name_is_an_authoring_fault() {
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.args: 9lives\n---\n";
    let err = contracts::contract_for(&doc(page), "t").unwrap_err();
    assert_eq!(
        err,
        ContractError::BadName {
            task: "t".to_owned(),
            name: "9lives".to_owned(),
        }
    );
}
