//! Pack loading internals for `compile` (§11.3): manifest parse, fixtures, the
//! sealed evaluation bridge, and ruleset budget-class classification.
//!
//! Everything here is `pub(crate)` — the public surface is `compile` +
//! `CompiledRuleset` in `lib.rs`. The evaluation bridge (`Evaluator`) is the
//! swappable seam P6-STARLARK/P6-EVAL land behind: they replace the stand-in
//! `BlurbEvaluator` with the real starlark-rust evaluator over `model` ASTs,
//! and neither the public `compile` signature nor `CompiledRuleset` changes.

use model::NodeRev;

use crate::{BudgetClass, CompileError, EvalBudget, Severity, Violation};

/// The §11.3 pack manifest — generic and evaluator-free. `deny_unknown_fields`
/// makes a typo (or a field from a newer `rulepack-api@N`) a loud compile
/// failure rather than a silent drop; api evolution rides the `api` pin, not
/// lenient parsing.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub id: String,
    pub api: String,
    pub budgets: EvalBudget,
    pub fixtures: Vec<String>,
    pub rules: Vec<String>,
}

/// Parse the manifest YAML. Any parse or schema-validation failure is the loud
/// `Malformed` path (taxonomy `compile_error` class).
pub(crate) fn parse_manifest(source: &str) -> Result<Manifest, CompileError> {
    serde_yaml::from_str::<Manifest>(source).map_err(|e| CompileError::Malformed {
        reason: format!("manifest parse: {e}"),
    })
}

/// Ruleset-level budget class = max class over used assertions (schema doc L188),
/// surfaced by `compile` so P6-VERDICTS can detect corpus-class-ness at load
/// (a corpus-class pack loaded sidecar-mode is later refused `daemon_only` —
/// NOT defined here). Stand-in: a conservative textual scan of rule-page sources
/// for the §4 vocabulary's file/corpus assertion names. P6-EVAL replaces this
/// with precise per-assertion analysis once real rule parsing lands; until then
/// over-classification (flag Corpus/File when unsure) is the safe direction for
/// a load gate.
pub(crate) fn classify_budget_class(rule_sources: &[String]) -> BudgetClass {
    let mut max = BudgetClass::Node;
    for src in rule_sources {
        // corpus-class (§4 #13) is the ceiling — short-circuit.
        if src.contains("link_resolves") {
            return BudgetClass::Corpus;
        }
        // file-class (§4 #11, #12).
        if src.contains("sibling_exists") || src.contains("child_exists") {
            max = BudgetClass::File;
        }
    }
    max
}

/// The demonstration a fixture asserts under its declared budgets. Authoritative
/// via frontmatter `expect:`; the `-pass`/`-fail` filename convention is
/// descriptive only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Expect {
    Pass,
    Fail,
}

impl std::fmt::Display for Expect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Expect::Pass => "pass",
            Expect::Fail => "fail",
        })
    }
}

/// Only the fixture frontmatter key the load gate reads (other keys ignored).
#[derive(serde::Deserialize)]
struct FixtureFrontmatter {
    expect: String,
}

/// One parsed fixture: its declared expectation plus the markdown body the
/// evaluator runs over.
pub(crate) struct FixtureDoc {
    pub path: String,
    pub expect: Expect,
    pub body: String,
}

/// Parse a fixture file: frontmatter declaring `expect: pass|fail`, then body.
/// A fixture with no such declaration cannot serve as a load-gate demonstration,
/// so it is a loud `FixtureFailed`.
pub(crate) fn parse_fixture(name: &str, content: &str) -> Result<FixtureDoc, CompileError> {
    let fail = |detail: String| CompileError::FixtureFailed {
        fixture: name.to_string(),
        detail,
    };
    let (frontmatter, body) = split_frontmatter(content)
        .ok_or_else(|| fail("no frontmatter declaring `expect: pass|fail`".into()))?;

    // Fixtures may carry arbitrary other frontmatter; only `expect` is read.
    let parsed: FixtureFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|e| fail(format!("frontmatter parse: {e}")))?;
    let expect = match parsed.expect.as_str() {
        "pass" => Expect::Pass,
        "fail" => Expect::Fail,
        other => return Err(fail(format!("`expect` must be pass|fail, got '{other}'"))),
    };
    Ok(FixtureDoc {
        path: name.to_string(),
        expect,
        body: body.to_string(),
    })
}

/// Split a leading `---` fenced YAML frontmatter block from the body. Returns
/// `None` when the content does not open with a frontmatter fence.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let mut lines = content.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let fm_start = first.len();
    let mut idx = fm_start;
    for line in lines {
        if line.trim_end() == "---" {
            return Some((&content[fm_start..idx], &content[idx + line.len()..]));
        }
        idx += line.len();
    }
    None
}

/// Raised when a fixture evaluation exceeds the pack's per-eval `{steps, mem}`
/// budget. At the load gate this is a fixture failure (pack refused); once
/// P6-EVAL wires real evaluation, the same exhaustion surfaces on the wire as
/// the `budget_exceeded` FINDING (never an error frame, §8).
#[derive(Debug)]
pub(crate) struct BudgetExhausted {
    pub steps: u64,
    pub mem: u64,
}

/// The sealed evaluation bridge. P6-STARLARK/P6-EVAL provide the real impl over
/// `model` ASTs; the load gate and (later) `evaluate` call through this trait,
/// so swapping the evaluator never touches the public API.
pub(crate) trait Evaluator {
    /// Evaluate the pack's rules over one fixture's body, metered under `budget`.
    /// `Ok(violations)` (empty = the fixture passes) or `Err` on budget
    /// exhaustion. Returns the §11.1 `Violation` shape the real `evaluate` will.
    fn eval_fixture(
        &self,
        rules: &[String],
        fixture: &FixtureDoc,
        budget: EvalBudget,
    ) -> Result<Vec<Violation>, BudgetExhausted>;
}

/// P6-COMPILE stand-in evaluator: the contract's own canonical `blurb-required`
/// demonstration (a heading must be followed by a non-empty, non-heading blurb
/// line), computed directly over raw fixture text. Real AST evaluation is
/// impossible now (`model::build` is a `todo!()` stub) and out of scope
/// (P6-EVAL/P6-STARLARK), so this stands in to make the load-gate mechanism real
/// and testable. It ignores rule-page contents; the real evaluator will not.
pub(crate) struct BlurbEvaluator;

impl Evaluator for BlurbEvaluator {
    fn eval_fixture(
        &self,
        _rules: &[String],
        fixture: &FixtureDoc,
        budget: EvalBudget,
    ) -> Result<Vec<Violation>, BudgetExhausted> {
        let body = &fixture.body;
        let mem_used = u64::try_from(body.len()).unwrap_or(u64::MAX);
        if mem_used > budget.mem {
            return Err(BudgetExhausted {
                steps: budget.steps,
                mem: budget.mem,
            });
        }

        // Line table with byte offsets into `body` (for violation spans).
        let mut offset = 0usize;
        let lines: Vec<(usize, &str)> = body
            .split_inclusive('\n')
            .map(|l| {
                let start = offset;
                offset += l.len();
                (start, l.trim_end_matches('\n'))
            })
            .collect();

        let mut violations = Vec::new();
        let mut steps: u64 = 0;
        for i in 0..lines.len() {
            steps += 1;
            if steps > budget.steps {
                return Err(BudgetExhausted {
                    steps: budget.steps,
                    mem: mem_used,
                });
            }
            let (start, text) = lines[i];
            let Some(_level) = heading_level(text) else {
                continue;
            };
            // A blurb = the next non-blank line, and it must not be another heading.
            let next_content = lines[i + 1..].iter().find(|(_, t)| !t.trim().is_empty());
            let has_blurb = matches!(next_content, Some((_, t)) if heading_level(t).is_none());
            if !has_blurb {
                violations.push(Violation {
                    rule: "blurb-required".into(),
                    severity: Severity::Warn,
                    path: fixture.path.clone(),
                    span: start..start + text.len(),
                    node_rev: NodeRev(String::new()),
                    hpath: Some(vec![text.trim_start_matches('#').trim().to_string()]),
                    message: "section has no blurb line".into(),
                });
            }
        }
        Ok(violations)
    }
}

/// ATX heading level (1..=6) if `line` is a heading, else `None`.
fn heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        u8::try_from(hashes).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_picks_max_class() {
        assert_eq!(classify_budget_class(&[]), BudgetClass::Node);
        assert_eq!(
            classify_budget_class(&["assert:\n  - keys_include: [a]".into()]),
            BudgetClass::Node
        );
        assert_eq!(
            classify_budget_class(&["assert:\n  - sibling_exists: {}".into()]),
            BudgetClass::File
        );
        assert_eq!(
            classify_budget_class(&[
                "assert:\n  - sibling_exists: {}".into(),
                "assert:\n  - link_resolves".into(),
            ]),
            BudgetClass::Corpus
        );
    }

    #[test]
    fn fixture_expect_parsed_from_frontmatter() {
        let fx = parse_fixture("f.md", "---\nexpect: fail\n---\n# H\n").unwrap();
        assert_eq!(fx.expect, Expect::Fail);
        assert_eq!(fx.body, "# H\n");
    }

    #[test]
    fn fixture_without_expect_declaration_is_loud() {
        assert!(matches!(
            parse_fixture("f.md", "# no frontmatter\n"),
            Err(CompileError::FixtureFailed { .. })
        ));
    }

    #[test]
    fn blurb_pass_has_no_violations() {
        let fx = parse_fixture(
            "pass.md",
            "---\nexpect: pass\n---\n# Goals\nA blurb line.\n",
        )
        .unwrap();
        let v = BlurbEvaluator
            .eval_fixture(
                &[],
                &fx,
                EvalBudget {
                    steps: 1000,
                    mem: 1 << 20,
                },
            )
            .unwrap();
        assert!(
            v.is_empty(),
            "conforming fixture must produce no violations"
        );
    }

    #[test]
    fn blurb_fail_flags_the_blurbless_heading() {
        let fx = parse_fixture(
            "fail.md",
            "---\nexpect: fail\n---\n# Goals\n## Immediately another heading\nbody\n",
        )
        .unwrap();
        let v = BlurbEvaluator
            .eval_fixture(
                &[],
                &fx,
                EvalBudget {
                    steps: 1000,
                    mem: 1 << 20,
                },
            )
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].message, "section has no blurb line");
    }

    #[test]
    fn eval_over_step_budget_is_exhaustion() {
        let fx = parse_fixture("f.md", "---\nexpect: pass\n---\n# A\nx\n# B\ny\n").unwrap();
        let out = BlurbEvaluator.eval_fixture(
            &[],
            &fx,
            EvalBudget {
                steps: 2,
                mem: 1 << 20,
            },
        );
        assert!(matches!(out, Err(BudgetExhausted { .. })));
    }

    #[test]
    fn eval_over_mem_budget_is_exhaustion() {
        let fx = parse_fixture("f.md", "---\nexpect: pass\n---\n# A\nblurb\n").unwrap();
        let out = BlurbEvaluator.eval_fixture(
            &[],
            &fx,
            EvalBudget {
                steps: 1000,
                mem: 2,
            },
        );
        assert!(matches!(out, Err(BudgetExhausted { .. })));
    }
}
