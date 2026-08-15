//! The `rules` op (`docs/wire-contract.md` § A.12): effective rules at a
//! path over the wire — per rule id, the page that governs, its scope, the
//! pages it shadows, and the ARM artifact's cell. The diagnostic for a write
//! this wire refused on an armed rule.
//!
//! Every judgement is `policy`'s — `RuleIndex` discovers, `narrowed_to`
//! narrows, `resolve` decides, `ArmedArtifact::verify_at` reddens. This
//! module is `mrd rules`' "one resolver, two consumers" charter held to a
//! third consumer: it contains no override law, compares no scopes, and does
//! no depth arithmetic — it feeds pages and projects the judgements onto the
//! wire shape.
//!
//! Snapshot posture (the crossing's trace, session result
//! `rules-snapshot-safety-trace.md`): every corpus judgement — discovery,
//! the armed artifact, the rev check, the orphan set — reads the warm
//! engine's one borrow at one fingerprint, which the answer cites. The user
//! rung and the §12.1 census are per-request reads by nature: a different
//! tree, and out-of-domain disk the projection cannot hold, definitionally —
//! the CLI verb's own feed split.

use std::collections::BTreeMap;
use std::path::Path;

use policy::armed::{ArmedArtifact, ArmedRow, PageSource, Redness};
use policy::{EffectiveSet, PageRef, RuleIndex, ScopeLayer};
use wire::{
    ResponseBody, RuleArmedCell, RuleChainEntry, RuleOrphan, RuleRow, RulesArmedSet,
    RulesNotOffered, RulesUserScope,
};

use crate::WorkspaceEngine;

/// Serve one `rules` call from the warm engine's borrowed state.
///
/// Infallible past the dispatch guards: findings ride the body (§ A.12 — a
/// diagnostic that refused to render its own findings would be useless
/// exactly when it is needed), and a never-landed `path` is served, because
/// a refused CREATE names a page that never landed.
pub(crate) fn serve(
    engine: &WorkspaceEngine,
    ws: &Path,
    path: Option<&wire::Path>,
) -> ResponseBody {
    let at = path.map_or("", |p| p.0.as_str());

    // The user rung, per request — the config plane's answer, never a guess.
    // Non-UTF-8 user pages are named holes, exactly like domain members.
    let mut user_pages: Vec<(String, String)> = Vec::new();
    let mut unreadable: Vec<String> = engine.unserved.keys().cloned().collect();
    let user_scope = load_user_rung(&mut user_pages, &mut unreadable);

    // Discovery over both rungs: the workspace feed is the engine's borrow
    // (raw bytes at one fingerprint), the user feed the read above.
    let index = RuleIndex::discover(
        engine
            .docs
            .iter()
            .map(|(page, doc)| PageRef {
                layer: ScopeLayer::Workspace,
                page,
                bytes: &doc.raw,
            })
            .chain(user_pages.iter().map(|(page, bytes)| PageRef {
                layer: ScopeLayer::User,
                page,
                bytes,
            })),
    );
    let narrowed = index.narrowed_to(at);
    let effective = narrowed.resolve();

    let (armed_set, artifact) = load_armed(engine);
    let rules = rows(&effective, artifact.as_ref(), at, &EnginePages(engine));
    let armed_orphans = orphans(artifact.as_ref(), at, &effective, ws);
    let (workspace_declined, workspace_undecidable) = declined_workspace(ws, engine);
    let (user_declined, user_undecidable) = declined_user();
    let mut undecidable = workspace_undecidable;
    undecidable.extend(user_undecidable);
    undecidable.sort();
    undecidable.dedup();

    ResponseBody::Rules {
        at: at.to_owned(),
        fingerprint: engine.at_fingerprint.0.clone(),
        user_scope,
        armed_set,
        rules,
        refused: narrowed.refused().iter().map(ToString::to_string).collect(),
        unreadable,
        armed_orphans,
        not_offered: RulesNotOffered {
            workspace: workspace_declined,
            user: user_declined,
            undecidable,
        },
    }
}

/// The user rung's pages plus where the rung came from. The enumeration law
/// lives in [`fs::user_rule_pages`], shared with the CLI verb and the
/// discovery feed — never a second enumeration that could disagree.
fn load_user_rung(
    pages: &mut Vec<(String, String)>,
    unreadable: &mut Vec<String>,
) -> RulesUserScope {
    let absent = |reason: String| RulesUserScope {
        scope: None,
        anchor: None,
        reason: Some(reason),
    };
    let anchor = match config::resolve_path(&config::Env::from_process()) {
        Ok(anchor) => anchor,
        Err(e) => return absent(format!("the config plane refused: {e}")),
    };
    if !anchor.is_file() {
        return absent(format!("no anchor at {}", anchor.display()));
    }
    let scope = anchor
        .parent()
        .map_or_else(|| anchor.display().to_string(), |p| p.display().to_string());
    match fs::user_rule_pages(&anchor) {
        Ok(found) => {
            for (page, bytes) in found {
                match String::from_utf8(bytes) {
                    Ok(text) => pages.push((page, text)),
                    Err(_) => unreadable.push(page),
                }
            }
            RulesUserScope {
                scope: Some(scope),
                anchor: Some(anchor.display().to_string()),
                reason: None,
            }
        }
        Err(e) => absent(format!(
            "{}/{} is unreadable: {e}",
            scope,
            fs::USER_RULES_DIR
        )),
    }
}

/// The warm engine as a [`PageSource`], so the armed set's rev check runs
/// against the same bytes discovery read — one borrow, one answer.
struct EnginePages<'a>(&'a WorkspaceEngine);

impl PageSource for EnginePages<'_> {
    fn read(&self, page: &str) -> std::io::Result<String> {
        self.0.docs.get(page).map(|d| d.raw.clone()).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("{page} is not here"))
        })
    }
}

/// Read the attested armed set out of the engine's own borrow.
fn load_armed(engine: &WorkspaceEngine) -> (RulesArmedSet, Option<ArmedArtifact>) {
    let path = policy::armed::ARMED_RULES_PATH.to_owned();
    let Some(page) = engine.docs.get(&path) else {
        return (
            RulesArmedSet {
                path,
                state: "absent".to_owned(),
                rows: None,
                detail: None,
            },
            None,
        );
    };
    match policy::armed::parse_artifact(&page.raw) {
        Ok(artifact) => {
            let rows = u64::try_from(artifact.rows().len()).unwrap_or(u64::MAX);
            (
                RulesArmedSet {
                    path,
                    state: "present".to_owned(),
                    rows: Some(rows),
                    detail: None,
                },
                Some(artifact),
            )
        }
        Err(corrupt) => (
            RulesArmedSet {
                path,
                state: "unreadable".to_owned(),
                rows: None,
                detail: Some(corrupt.to_string()),
            },
            None,
        ),
    }
}

/// One row per resolved id and one per collision, id-ascending within each,
/// the resolved set first — the CLI verb's own order.
/// Each armed row's redness at `at`, keyed by (id, arm root) — the
/// artifact's own fail-closed rev check, never a second hash law here.
fn redness_at(
    artifact: Option<&ArmedArtifact>,
    at: &str,
    pages: &dyn PageSource,
) -> BTreeMap<(String, String), &'static str> {
    let mut reddened = BTreeMap::new();
    if let Some(artifact) = artifact {
        let verdict = artifact.verify_at(at, pages);
        for red in verdict.red() {
            let why = match red.why() {
                Redness::Drifted { .. } => "drifted",
                Redness::Missing { .. } => "missing",
                Redness::ModeOutsideKind { .. } => "kind-mismatch",
            };
            reddened.insert(
                (
                    red.row().id().as_str().to_owned(),
                    red.row().scope().as_str().to_owned(),
                ),
                why,
            );
        }
    }
    reddened
}

fn rows(
    effective: &EffectiveSet,
    artifact: Option<&ArmedArtifact>,
    at: &str,
    pages: &dyn PageSource,
) -> Vec<RuleRow> {
    // The selection law, from the artifact: per id, the deepest armed row
    // whose arm root contains this path — keyed by (id, arm root).
    let selected: Vec<&ArmedRow> = artifact.map(|a| a.select_at(at)).unwrap_or_default();
    let reddened = redness_at(artifact, at, pages);
    let cell = |id: &str, winner: &policy::Registration| -> Option<RuleArmedCell> {
        let row = selected.iter().find(|row| row.id().as_str() == id)?;
        let mode = row.mode().as_str().to_owned();
        let pinned_page = (row.page() != winner.page()).then(|| row.page().to_owned());
        let redness = reddened
            .get(&(
                row.id().as_str().to_owned(),
                row.scope().as_str().to_owned(),
            ))
            .map(|why| (*why).to_owned());
        let mut rendered = mode.clone();
        if let Some(why) = &redness {
            rendered.push('(');
            rendered.push_str(why);
            rendered.push(')');
        }
        if let Some(page) = &pinned_page {
            rendered.push('@');
            rendered.push_str(page);
        }
        Some(RuleArmedCell {
            mode,
            pinned_page,
            redness,
            rendered,
        })
    };

    let entry = |role: &str, registration: &policy::Registration| -> RuleChainEntry {
        let scope = registration.scope();
        RuleChainEntry {
            role: role.to_owned(),
            page: registration.page().to_owned(),
            rev: registration.rev().to_owned(),
            layer: scope.layer().as_str().to_owned(),
            depth: u64::try_from(scope.depth()).unwrap_or(u64::MAX),
            kinds: registration
                .kinds()
                .iter()
                .map(|k| k.as_str().to_owned())
                .collect(),
        }
    };

    let mut rows: Vec<RuleRow> = effective
        .resolved()
        .iter()
        .map(|(id, resolution)| RuleRow {
            id: id.as_str().to_owned(),
            state: "resolved".to_owned(),
            collision_scope: None,
            armed: cell(id.as_str(), resolution.winner()),
            chain: resolution
                .chain()
                .enumerate()
                .map(|(position, registration)| {
                    entry(
                        if position == 0 { "winner" } else { "shadowed" },
                        registration,
                    )
                })
                .collect(),
        })
        .collect();

    for collision in effective.collisions() {
        rows.push(RuleRow {
            id: collision.id().as_str().to_owned(),
            state: "collision".to_owned(),
            collision_scope: Some(collision.scope().to_string()),
            // A collided id resolves to nothing, so there is no winner for
            // an armed row to be about.
            armed: None,
            chain: collision
                .tied()
                .iter()
                .map(|tied| entry("tied", tied))
                .chain(
                    collision
                        .shadowed()
                        .iter()
                        .map(|page| entry("shadowed", page)),
                )
                .collect(),
        });
    }
    rows
}

/// The armed rows selected at `at` whose id discovery no longer resolves —
/// keyed on the id's absence from the resolved set, never a path predicate.
fn orphans(
    artifact: Option<&ArmedArtifact>,
    at: &str,
    effective: &EffectiveSet,
    ws: &Path,
) -> Vec<RuleOrphan> {
    let Some(artifact) = artifact else {
        return Vec::new();
    };
    artifact
        .select_at(at)
        .into_iter()
        .filter(|row| effective.resolved().get(row.id().as_str()).is_none())
        .map(|row| RuleOrphan {
            id: row.id().as_str().to_owned(),
            page: row.page().to_owned(),
            scope: row.scope().as_str().to_owned(),
            mode: row.mode().as_str().to_owned(),
            cause: orphan_cause(ws, row.page()),
        })
        .collect()
}

/// Why an armed row's pinned page is absent from the corpus — decided by
/// looking (one stat), never by inheriting `verify_at`'s `missing`: the page
/// can sit on disk byte-for-byte unchanged with only the declared domain
/// moved, and sending a reader to look for a deleted file costs them the
/// search before they find the ignore rule.
fn orphan_cause(ws: &Path, page: &str) -> String {
    if Path::new(page)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        && ws.join(page).is_file()
    {
        "on disk, outside the hash domain — an ignore rule or a dot segment excludes it".to_owned()
    } else {
        "not on disk".to_owned()
    }
}

/// RULE pages the workspace hash domain does not carry — the projection's
/// own population (`fs::walk` minus the engine's served + unserved members),
/// then narrowed by ASKING THE REGISTRAR, never a path predicate. The
/// walk-op census names every excluded file; a rules answer is about rule
/// pages, so a sentence it prints must be about rule pages.
fn declined_workspace(ws: &Path, engine: &WorkspaceEngine) -> (Vec<String>, Vec<String>) {
    let root = fs::WorkspaceRoot(ws.to_path_buf());
    let Ok(all) = fs::walk(&root) else {
        return (Vec::new(), Vec::new());
    };
    let outside: Vec<(String, String)> = all
        .iter()
        .filter_map(|rel| rel.to_str().map(str::to_owned))
        .filter(|rel| !engine.docs.contains_key(rel) && !engine.unserved.contains_key(rel))
        .filter_map(|rel| {
            // An unreadable or non-UTF-8 excluded file cannot be shown to
            // carry a rule tag, so it is not claimed as one.
            std::fs::read_to_string(ws.join(&rel))
                .ok()
                .map(|bytes| (rel, bytes))
        })
        .collect();
    rule_pages_among(&outside)
}

/// RULE pages under the user `rules/` tree that a dot segment declined, from
/// the rung's own traversal ([`fs::user_rule_pages_declined`]) — never a
/// second enumeration that could disagree with the one that declined them.
fn declined_user() -> (Vec<String>, Vec<String>) {
    let Ok(anchor) = config::resolve_path(&config::Env::from_process()) else {
        return (Vec::new(), Vec::new());
    };
    let Some(user_scope) = anchor.parent().map(Path::to_path_buf) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(declined) = fs::user_rule_pages_declined(&anchor) else {
        return (Vec::new(), Vec::new());
    };
    let candidates: Vec<(String, String)> = declined
        .into_iter()
        .filter_map(|rel| {
            std::fs::read_to_string(user_scope.join(&rel))
                .ok()
                .map(|bytes| (rel, bytes))
        })
        .collect();
    rule_pages_among(&candidates)
}

/// Split `candidates` into pages that OFFER THEMSELVES to rule registration
/// (registered or refused — both are rule pages whose law is missing from
/// this answer) and pages whose rule-ness CANNOT BE ANSWERED
/// (`RegisterFault::FrontmatterUnparsed` — neither claimed nor cleared).
/// The registrar decides, never a predicate here.
fn rule_pages_among(candidates: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let index = RuleIndex::discover(candidates.iter().map(|(page, bytes)| PageRef {
        layer: ScopeLayer::Workspace,
        page,
        bytes,
    }));
    let mut offered: Vec<String> = index
        .registered()
        .iter()
        .map(|r| r.page().to_owned())
        .collect();
    let mut undecidable = Vec::new();
    for refusal in index.refused() {
        if matches!(
            refusal.fault(),
            policy::RegisterFault::FrontmatterUnparsed { .. }
        ) {
            undecidable.push(refusal.page().to_owned());
        } else {
            offered.push(refusal.page().to_owned());
        }
    }
    offered.sort();
    offered.dedup();
    undecidable.sort();
    undecidable.dedup();
    (offered, undecidable)
}
