//! Arg/env contracts — validated PRE-eval, so a task never starts against
//! inputs it did not declare. The declaration lives beside the binding:
//! `task.<name>.args` names the positional args (exact count, in order);
//! `task.<name>.env` names the env keys the caller must supply via `--env`.
//! Absent keys declare an empty contract. Undeclared supplied env refuses —
//! deny-by-default covers inputs too, and it catches `--env` typos loudly.

use std::collections::BTreeMap;

use model::Document;

use crate::address::frontmatter;

/// A task's declared input contract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contract {
    /// Declared positional arg names, in order. The CLI supplies exactly this
    /// many (after `--`).
    pub args: Vec<String>,
    /// Declared required env keys, each supplied via `--env KEY=VALUE`.
    pub env: Vec<String>,
}

/// An authoring fault in a contract declaration (found at load).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// A declared name repeats within one list.
    DuplicateName { task: String, name: String },
    /// A declared name is not an identifier (`[A-Za-z_][A-Za-z0-9_-]*`).
    BadName { task: String, name: String },
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractError::DuplicateName { task, name } => {
                write!(f, "task '{task}' declares '{name}' twice")
            }
            ContractError::BadName { task, name } => {
                write!(f, "task '{task}' declares invalid name '{name}'")
            }
        }
    }
}

impl std::error::Error for ContractError {}

/// A supplied-input violation of a declared contract (refused pre-eval, with
/// the contract named so the caller can correct the invocation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractViolation {
    /// Wrong positional arg count.
    ArgCount {
        task: String,
        expected: Vec<String>,
        got: usize,
    },
    /// A declared env key was not supplied.
    MissingEnv { task: String, name: String },
    /// A supplied env key was never declared.
    UndeclaredEnv { task: String, name: String },
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractViolation::ArgCount {
                task,
                expected,
                got,
            } => write!(
                f,
                "task '{task}' takes {} arg(s) ({}), got {got}",
                expected.len(),
                if expected.is_empty() {
                    "none".to_owned()
                } else {
                    expected.join(", ")
                }
            ),
            ContractViolation::MissingEnv { task, name } => {
                write!(f, "task '{task}' requires --env {name}=…")
            }
            ContractViolation::UndeclaredEnv { task, name } => {
                write!(f, "task '{task}' does not declare env '{name}'")
            }
        }
    }
}

impl std::error::Error for ContractViolation {}

/// Is `name` a declarable identifier: `[A-Za-z_]` then `[A-Za-z0-9_-]*`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse one comma/whitespace-separated name list, validating each name.
fn parse_names(task: &str, raw: &str) -> Result<Vec<String>, ContractError> {
    let mut out: Vec<String> = Vec::new();
    for name in raw
        .trim()
        .trim_matches(['"', '\''])
        .split([',', ' ', '\t'])
        .filter(|s| !s.is_empty())
    {
        if !is_identifier(name) {
            return Err(ContractError::BadName {
                task: task.to_owned(),
                name: name.to_owned(),
            });
        }
        if out.iter().any(|n| n == name) {
            return Err(ContractError::DuplicateName {
                task: task.to_owned(),
                name: name.to_owned(),
            });
        }
        out.push(name.to_owned());
    }
    Ok(out)
}

/// The declared contract for `task` on this page (`task.<name>.args` /
/// `task.<name>.env` frontmatter keys; absent keys = empty contract).
///
/// # Errors
/// [`ContractError`] on a malformed declaration.
pub fn contract_for(doc: &Document, task: &str) -> Result<Contract, ContractError> {
    let mut contract = Contract::default();
    let Some(map) = frontmatter(doc) else {
        return Ok(contract);
    };
    for (key, value) in &map.0 {
        if let Some(rest) = key.strip_prefix("task.") {
            if rest == format!("{task}.args") {
                contract.args = parse_names(task, value)?;
            } else if rest == format!("{task}.env") {
                contract.env = parse_names(task, value)?;
            }
        }
    }
    Ok(contract)
}

/// Validate supplied inputs against the declared contract — the pre-eval gate.
///
/// # Errors
/// The first [`ContractViolation`], deterministically: arg count, then missing
/// env in declaration order, then undeclared env in supplied order.
pub fn validate(
    task: &str,
    contract: &Contract,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<(), ContractViolation> {
    if args.len() != contract.args.len() {
        return Err(ContractViolation::ArgCount {
            task: task.to_owned(),
            expected: contract.args.clone(),
            got: args.len(),
        });
    }
    for name in &contract.env {
        if !env.contains_key(name) {
            return Err(ContractViolation::MissingEnv {
                task: task.to_owned(),
                name: name.clone(),
            });
        }
    }
    for name in env.keys() {
        if !contract.env.iter().any(|n| n == name) {
            return Err(ContractViolation::UndeclaredEnv {
                task: task.to_owned(),
                name: name.clone(),
            });
        }
    }
    Ok(())
}
