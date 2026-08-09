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
            got: 0,
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
