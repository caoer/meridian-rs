//! `mrd check`'s armed plane (`docs/armed-plane.md` § Part A3): the armed rows
//! verified by the write door's own law, and every armed `rules/query` run over
//! the check's snapshot.
//!
//! The rows are verified through [`policy::armed::ArmedArtifact::verify`] —
//! the `verify_rows` law the door applies — so `off` rows are inert here as
//! there. Each firing row whose page carries a query leg runs its SQL against
//! ONE `:memory:` projection, the build `mrd sql`'s memory lane makes
//! ([`crate::sql::memory_projection`]), so a query answers the same rows on
//! both verbs. A result row is the rule's only where that armed row governs
//! its path ([`policy::armed::ArmedArtifact::select_at`]).

use policy::armed::{ArmedArtifact, ArmedRow, Mode, PageSource, parse_artifact};
use policy::{ArmedFault, PageRef, ScopeLayer};
use serde_json::{Value, json};

/// What the armed plane found, before any path narrowing.
pub(crate) struct ArmedPlane {
    /// Never armed: no marker, so nothing here is law.
    never_armed: bool,
    /// The artifact rows, when the artifact parsed.
    rows: usize,
    /// Whole-artifact faults (missing, corrupt, zero rows) and red firing
    /// rows — every one a rule-level finding.
    faults: Vec<String>,
    /// One entry per firing query row.
    rules: Vec<QueryRule>,
}

/// One armed query's answer.
struct QueryRule {
    id: String,
    page: String,
    mode: Mode,
    /// The rows this armed row governs, control excluded.
    rows: Vec<Finding>,
    /// The declared control and whether its row came back.
    control: Option<(String, bool)>,
    /// Why the query has no answer, when it has none.
    error: Option<String>,
}

/// One row a query returned.
struct Finding {
    path: String,
    detail: String,
}

/// Whether `path` lies under one of `paths` — all of them when none is given.
/// A path names itself and everything beneath it; the workspace root (`""`)
/// covers everything.
pub(crate) fn under(paths: &[String], path: &str) -> bool {
    paths.is_empty()
        || paths.iter().any(|p| {
            p.is_empty() || path == p || path.starts_with(&format!("{}/", p.trim_end_matches('/')))
        })
}

/// Assess the armed plane of `root` over `docs`, the check's snapshot.
pub(crate) fn assess(
    root: &fs::WorkspaceRoot,
    docs: &model::Docs,
    domain: &fs::domain::Domain,
    fold: &str,
) -> ArmedPlane {
    let mut plane = ArmedPlane {
        never_armed: false,
        rows: 0,
        faults: Vec::new(),
        rules: Vec::new(),
    };
    if !wire_serve::armed_disk::once_armed(root) {
        plane.never_armed = true;
        return plane;
    }
    let path = || policy::armed::ARMED_RULES_PATH.to_owned();
    let Some(page) = wire_serve::armed_disk::read_artifact(root) else {
        plane
            .faults
            .push(ArmedFault::Missing { path: path() }.to_string());
        return plane;
    };
    let artifact = match parse_artifact(&page) {
        Ok(artifact) => artifact,
        Err(corrupt) => {
            let fault = ArmedFault::Corrupt {
                path: path(),
                detail: corrupt.detail,
            };
            plane.faults.push(fault.to_string());
            return plane;
        }
    };
    plane.rows = artifact.rows().len();
    if artifact.rows().is_empty() {
        plane
            .faults
            .push(ArmedFault::Disarmed { path: path() }.to_string());
        return plane;
    }

    let pages = wire_serve::armed_disk::DiskPages::new(root);
    let verdict = artifact.verify(&pages);
    plane
        .faults
        .extend(verdict.red().iter().map(ToString::to_string));

    let queries: Vec<(&ArmedRow, Result<policy::QueryLeg, String>)> = verdict
        .firing()
        .iter()
        .filter_map(|row| query_leg(row, &pages).map(|leg| (row, leg)))
        .collect();
    if queries.is_empty() {
        return plane;
    }

    let projection = project(root, docs, domain, fold);
    for (row, leg) in queries {
        let mut rule = QueryRule {
            id: row.id().to_string(),
            page: row.page().to_owned(),
            mode: row.mode(),
            rows: Vec::new(),
            control: None,
            error: None,
        };
        match (&projection, leg) {
            (_, Err(error)) => rule.error = Some(error),
            (Err(error), _) => rule.error = Some(error.clone()),
            (Ok(conn), Ok(leg)) => {
                rule.control = leg.control().map(|c| (c.to_owned(), false));
                match run(conn, leg.sql()) {
                    Err(error) => rule.error = Some(error),
                    Ok(found) => {
                        for finding in found {
                            if let Some((control, fired)) = &mut rule.control
                                && *control == finding.path
                            {
                                *fired = true;
                            } else if governs(&artifact, row, &finding.path) {
                                rule.rows.push(finding);
                            }
                        }
                    }
                }
            }
        }
        plane.rules.push(rule);
    }
    plane
}

/// The query leg of a firing row's page, or `None` when the page has none. The
/// row passed verification, so these are the attested bytes and they register;
/// a page that no longer loads answers the loader's refusal as the error.
fn query_leg(row: &ArmedRow, pages: &dyn PageSource) -> Option<Result<policy::QueryLeg, String>> {
    let bytes = match pages.read(row.page()) {
        Ok(bytes) => bytes,
        Err(e) => return Some(Err(format!("`{}` cannot be read: {e}", row.page()))),
    };
    let registration = policy::register_page(PageRef {
        layer: ScopeLayer::Workspace,
        page: row.page(),
        bytes: &bytes,
    })
    .ok()
    .flatten()?;
    if !registration.kinds().contains(&policy::RuleKind::Query) {
        return None;
    }
    Some(
        policy::load_rule(&registration, &bytes, policy::CheckLimits::default())
            .map_err(|e| e.to_string())
            .and_then(|rule| {
                rule.query()
                    .cloned()
                    .ok_or_else(|| "the page loaded with no query leg".to_owned())
            }),
    )
}

/// The one projection every query of this check runs against.
fn project(
    root: &fs::WorkspaceRoot,
    docs: &model::Docs,
    domain: &fs::domain::Domain,
    fold: &str,
) -> Result<duckdb::Connection, String> {
    let mounts =
        crate::walk_cmd::load_mounts_for(&crate::walk_cmd::link_addressed_roots(docs, None));
    // A query that reads the base plane must not read an empty one because the
    // walk failed: fail closed, every query of this check reports it.
    let base = fs::base::base_snapshot_under(root, domain)
        .map_err(|e| format!("cannot walk the base plane: {e}"))?;
    crate::sql::memory_projection(root, docs, domain, &mounts, fold, Some(&base))
}

/// Run one query and read its `path` / `detail` columns by name.
fn run(conn: &duckdb::Connection, sql: &str) -> Result<Vec<Finding>, String> {
    let (columns, rows) = view::store::run_query(conn, sql)?;
    let index = |name: &str| columns.iter().position(|c| c.name == name);
    let Some(path_at) = index("path") else {
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        return Err(format!(
            "the query returns no `path` column (columns: {}) — a query rule names the page \
             breaking it in a column called `path`",
            names.join(", ")
        ));
    };
    let detail_at = index("detail");
    rows.into_iter()
        .map(|row| {
            let path = match row.get(path_at) {
                Some(Value::String(path)) => path.clone(),
                Some(Value::Null) | None => {
                    return Err("the query returned a row whose `path` is null".to_owned());
                }
                Some(other) => other.to_string(),
            };
            let detail = match detail_at.and_then(|i| row.get(i)) {
                Some(Value::String(detail)) => detail.clone(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            Ok(Finding { path, detail })
        })
        .collect()
}

/// Whether `row` is the armed row that governs `path` — the selection law,
/// never re-derived here.
fn governs(artifact: &ArmedArtifact, row: &ArmedRow, path: &str) -> bool {
    artifact.select_at(path).contains(&row)
}

impl ArmedPlane {
    /// The findings that fail the check, narrowed to `paths` where a finding
    /// has a path: rule-level findings count whatever the paths, because a
    /// rule that cannot run has no answer for any path.
    pub(crate) fn findings(&self, paths: &[String]) -> Vec<String> {
        let mut findings: Vec<String> = self.faults.iter().map(|f| one_line(f)).collect();
        for rule in &self.rules {
            if let Some(error) = &rule.error {
                findings.push(one_line(&format!(
                    "query `{}` ({}) could not run: {error}",
                    rule.id, rule.page
                )));
                continue;
            }
            if let Some((control, false)) = &rule.control {
                findings.push(format!(
                    "query `{}` did not report its control `{control}` — a rule whose planted \
                     breach goes unseen cannot be trusted to see a real one",
                    rule.id
                ));
            }
            let counted = rule.rows.iter().filter(|r| under(paths, &r.path)).count();
            if rule.mode == Mode::Block && counted > 0 {
                findings.push(format!("query `{}` (block): {counted} row(s)", rule.id));
            }
        }
        findings
    }

    /// The human lines: the artifact state, its faults, then each query and
    /// the rows under `paths`.
    pub(crate) fn render(&self, paths: &[String]) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.never_armed {
            let _ = writeln!(out, "  armed: never armed — no rule is law here");
            return out;
        }
        let _ = writeln!(
            out,
            "  armed: {} row(s) in {}",
            self.rows,
            policy::armed::ARMED_RULES_PATH
        );
        for fault in &self.faults {
            let _ = writeln!(out, "  armed: RED — {}", one_line(fault));
        }
        for rule in &self.rules {
            let shown: Vec<&Finding> = rule.rows.iter().filter(|r| under(paths, &r.path)).collect();
            let control = match &rule.control {
                None => String::new(),
                Some((path, true)) => format!(", control {path} fired"),
                Some((path, false)) => format!(", control {path} MISSING"),
            };
            match &rule.error {
                Some(error) => {
                    let _ = writeln!(
                        out,
                        "  query {} ({}): ERROR — {}",
                        rule.id,
                        rule.mode,
                        one_line(error)
                    );
                }
                None => {
                    let _ = writeln!(
                        out,
                        "  query {} ({}): {} row(s){control}",
                        rule.id,
                        rule.mode,
                        shown.len()
                    );
                }
            }
            for row in shown {
                if row.detail.is_empty() {
                    let _ = writeln!(out, "    {}", row.path);
                } else {
                    let _ = writeln!(out, "    {} — {}", row.path, one_line(&row.detail));
                }
            }
        }
        out
    }

    /// The `--json` members: `rules` and `armed_rows`.
    pub(crate) fn json(&self, paths: &[String]) -> (Value, Value) {
        let rules = self
            .rules
            .iter()
            .map(|rule| {
                json!({
                    "id": rule.id,
                    "page": rule.page,
                    "mode": rule.mode.as_str(),
                    "rows": rule
                        .rows
                        .iter()
                        .filter(|r| under(paths, &r.path))
                        .map(|r| json!({ "path": r.path, "detail": r.detail }))
                        .collect::<Vec<_>>(),
                    "control": rule
                        .control
                        .as_ref()
                        .map(|(path, fired)| json!({ "path": path, "fired": fired })),
                    "error": rule.error,
                })
            })
            .collect::<Vec<_>>();
        let armed = json!({
            "state": if self.never_armed { "never-armed" } else { "armed" },
            "rows": self.rows,
            "red": self.faults,
        });
        (Value::Array(rules), armed)
    }
}

/// A finding on one line: the refusal list numbers one finding per line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
