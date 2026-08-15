//! Arg/env contracts — validated PRE-eval, so a task never starts against
//! inputs it did not declare. The declaration lives beside the binding:
//! `task.<name>.args` names the positional args (exact count, in order);
//! `task.<name>.env` names the env keys the caller must supply via `--env`.
//! Absent keys declare an empty contract. Undeclared supplied env refuses —
//! deny-by-default covers inputs too, and it catches `--env` typos loudly.
//!
//! The LAST args name may carry a `...` tail suffix (`title, rows...`): the
//! names before it stay fixed slots, and the tail takes every remaining arg,
//! zero included. Both dispatchers already consume a positional list (bash
//! argv, starlark `ctx.args`), so a tail needs no supply-side change. `.env`
//! has nothing to count and refuses the suffix.

use std::collections::BTreeMap;

use model::Document;

use crate::address::frontmatter;

/// A task's declared input contract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contract {
    /// Declared positional arg names, in order, tail suffix stripped. The CLI
    /// supplies exactly this many (after `--`) unless [`Contract::variadic`].
    pub args: Vec<String>,
    /// The last args name carried the `...` tail: every name before it is a
    /// fixed slot, and the tail absorbs the remaining args (zero allowed).
    pub variadic: bool,
    /// Declared required env keys, each supplied via `--env KEY=VALUE`.
    pub env: Vec<String>,
}

impl Contract {
    /// How many args the caller must supply at minimum: all declared slots, or
    /// the fixed prefix when a tail is declared.
    #[must_use]
    pub fn min_args(&self) -> usize {
        if self.variadic {
            self.args.len().saturating_sub(1)
        } else {
            self.args.len()
        }
    }

    /// The declaration as written — names in order, the tail carrying its
    /// `...`. This is what every face echoes back, so the caller reads the
    /// contract in the same spelling they declared it.
    #[must_use]
    pub fn args_declared(&self) -> Vec<String> {
        let last = self.args.len().saturating_sub(1);
        self.args
            .iter()
            .enumerate()
            .map(|(i, name)| {
                if self.variadic && i == last {
                    format!("{name}...")
                } else {
                    name.clone()
                }
            })
            .collect()
    }
}

/// An authoring fault in a contract declaration (found at load).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// A declared name repeats within one list.
    DuplicateName { task: String, name: String },
    /// A declared name is not an identifier (`[A-Za-z_][A-Za-z0-9_-]*`).
    BadName { task: String, name: String },
    /// An args name before the last one carries the `...` tail suffix.
    TailNotLast { task: String, name: String },
    /// An env key carries the `...` tail suffix.
    TailOnEnv { task: String, name: String },
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
            ContractError::TailNotLast { task, name } => write!(
                f,
                "task '{task}' declares '{name}...' before the last arg: a tail \
                 takes every remaining arg, so nothing after it could be filled \
                 — move '{name}...' last, or drop the '...'"
            ),
            ContractError::TailOnEnv { task, name } => write!(
                f,
                "task '{task}' declares env '{name}...': env is supplied by name, \
                 one value per key, so a tail has nothing to absorb — declare \
                 '{name}', and take the variable part as an arg tail if you need one"
            ),
        }
    }
}

impl std::error::Error for ContractError {}

/// A supplied-input violation of a declared contract (refused pre-eval, with
/// the contract named so the caller can correct the invocation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractViolation {
    /// Wrong positional arg count. `expected` is the declaration as written
    /// (the tail name carries its `...`); with a tail the count is a floor.
    ArgCount {
        task: String,
        expected: Vec<String>,
        variadic: bool,
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
                variadic,
                got,
            } => write!(
                f,
                "task '{task}' takes {}{} arg(s) ({}), got {got}",
                if *variadic { "at least " } else { "" },
                if *variadic {
                    expected.len().saturating_sub(1)
                } else {
                    expected.len()
                },
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
///
/// `tail` says whether a `...` suffix is declarable here: args accept it on the
/// last name only, env refuses it outright. Returns the names with the suffix
/// stripped, plus whether the last one carried it.
fn parse_names(task: &str, raw: &str, tail: bool) -> Result<(Vec<String>, bool), ContractError> {
    let mut out: Vec<String> = Vec::new();
    let mut variadic = false;
    for raw_name in raw
        .trim()
        .trim_matches(['"', '\''])
        .split([',', ' ', '\t'])
        .filter(|s| !s.is_empty())
    {
        // A tail already closed the list: nothing may follow it, and env
        // never opens one.
        if variadic {
            return Err(ContractError::TailNotLast {
                task: task.to_owned(),
                name: out.last().cloned().unwrap_or_default(),
            });
        }
        let name = match raw_name.strip_suffix("...") {
            Some(stem) if tail => {
                variadic = true;
                stem
            }
            Some(stem) => {
                return Err(ContractError::TailOnEnv {
                    task: task.to_owned(),
                    name: stem.to_owned(),
                });
            }
            None => raw_name,
        };
        if !is_identifier(name) {
            return Err(ContractError::BadName {
                task: task.to_owned(),
                name: raw_name.to_owned(),
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
    Ok((out, variadic))
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
                let (names, variadic) = parse_names(task, value, true)?;
                contract.args = names;
                contract.variadic = variadic;
            } else if rest == format!("{task}.env") {
                let (names, _) = parse_names(task, value, false)?;
                contract.env = names;
            }
        }
    }
    Ok(contract)
}

/// Validate supplied inputs against the declared contract — the pre-eval gate.
/// The arg count is exact, or a floor at the fixed prefix when the declaration
/// ends in a tail.
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
    let short = if contract.variadic {
        args.len() < contract.min_args()
    } else {
        args.len() != contract.args.len()
    };
    if short {
        return Err(ContractViolation::ArgCount {
            task: task.to_owned(),
            expected: contract.args_declared(),
            variadic: contract.variadic,
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
