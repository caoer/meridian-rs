//! **SELF-RETIRING (U3.2 migration kit, d1 § Migration plan).** The ONE
//! quarantined place the dead `draws-from` key is spelled in engine Rust.
//!
//! `migrate_inputs` renames the `draws-from:` frontmatter key to `inputs:`
//! across the corpus, value bytes preserved, splicing each page through the
//! strict writer under CAS ([`wire_serve::write::migrate_key`]). Every rename
//! mints ONE `^r-NNNNNN` wire audit row and ZERO attestation receipts (`attest`
//! is their sole minter — strict-writer law, d1 § Actor on migration writes).
//!
//! Three disciplines the migration plan (d1) names:
//! - **dry-run first** — `dry: true` reports every rename and writes nothing.
//! - **idempotent** — a page already carrying `inputs:` (and no `draws-from:`)
//!   is not a rename target, so a re-run over a fully-migrated corpus mints
//!   nothing (0 renames, 0 rows).
//! - **resumable** — each page renames independently under its own
//!   write-what-you-read CAS, so a mid-corpus failure resumes by re-running:
//!   already-renamed pages are skipped, only the `draws-from:` remainder writes.
//!
//! A page carrying BOTH keys is a synonym-window defect the migration refuses to
//! auto-resolve — it is reported as a [`PageOutcome::Conflict`] and left for a
//! human, never silently merged.
//!
//! **Retirement:** this crate, [`wire_serve::write::migrate_key`], and the `mrd
//! migrate` subcommand die at the pin-leg cutover (deletion is in this leg's
//! definition of done; the delete rides the U3.3 storm that consumes this
//! world). After that, `grep -r draws-from` over engine Rust is empty.

use model::{Document, Ref, resolve};
use serde::Serialize;

/// The dead key this kit renames — the ONE spelling of it in engine Rust.
const DRAWS_FROM: &str = "draws-from";
/// The live key it becomes (the engine speaks only this on every other plane).
const INPUTS: &str = "inputs";

/// Options for a migration run.
#[derive(Debug, Clone, Default)]
pub struct MigrateOptions {
    /// Report every rename and write nothing (dry-run-first, d1 stage 2).
    pub dry: bool,
    /// The recorded actor (§9: recorded exactly as given, never invented).
    pub actor: Option<String>,
    /// The recorded timestamp (§9: recorded exactly as given, never invented).
    pub now: Option<String>,
}

/// A tool failure — the migration could not run to a verdict (exit 2). Distinct
/// from a per-page conflict, which IS a reported outcome (the run completed).
#[derive(Debug)]
pub enum MigrateError {
    /// The workspace could not be read or parsed.
    Corpus(String),
    /// A page declared `draws-from:` but its grain could not be planned (a
    /// malformed frontmatter key line with no colon — never seen from the parser).
    Plan(String),
    /// The CAS-guarded write refused or failed for `path` (the page drifted, or I/O).
    Write { path: String, detail: String },
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::Corpus(m) => write!(f, "cannot read the workspace: {m}"),
            MigrateError::Plan(p) => {
                write!(f, "cannot plan the rename for {p} (malformed key line)")
            }
            MigrateError::Write { path, detail } => {
                write!(f, "migrate write failed for {path}: {detail}")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

/// One page's migration outcome. Only pages the migration ACTS on or refuses are
/// reported; a page with no `draws-from:` key is out of scope and omitted.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PageOutcome {
    /// The `draws-from:` key was renamed to `inputs:`. `receipt` is the minted
    /// `^r-NNNNNN` journal anchor, or `None` on a dry run.
    Renamed {
        path: String,
        receipt: Option<String>,
    },
    /// The page carries BOTH keys — a synonym-window defect. Refused (nothing
    /// written); a human resolves which value wins.
    Conflict { path: String },
}

/// The report of a migration run — the acted-on pages plus the run's mode.
#[derive(Debug, Serialize)]
pub struct MigrateReport {
    /// Per-page outcomes, in corpus (path-sorted) order.
    pub pages: Vec<PageOutcome>,
    /// This was a dry run (nothing written).
    pub dry: bool,
}

impl MigrateReport {
    /// How many pages were renamed (or, on a dry run, would be).
    #[must_use]
    pub fn renamed(&self) -> usize {
        self.pages
            .iter()
            .filter(|p| matches!(p, PageOutcome::Renamed { .. }))
            .count()
    }

    /// How many pages carried both keys and were refused.
    #[must_use]
    pub fn conflicts(&self) -> usize {
        self.pages
            .iter()
            .filter(|p| matches!(p, PageOutcome::Conflict { .. }))
            .count()
    }

    /// The minted `^r-NNNNNN` journal anchors, in order (empty on a dry run).
    #[must_use]
    pub fn receipts(&self) -> Vec<&str> {
        self.pages
            .iter()
            .filter_map(|p| match p {
                PageOutcome::Renamed {
                    receipt: Some(r), ..
                } => Some(r.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// **Run the `draws-from` → `inputs` migration** (d1 § Migration plan): scan the
/// corpus, rename every `draws-from:` frontmatter key to `inputs:` (value bytes
/// preserved) through the strict writer under CAS, and report each acted-on
/// page. Idempotent, dry-run-first, resumable (see the module docs).
///
/// # Errors
/// [`MigrateError`] on a tool failure — an unreadable workspace, an unplannable
/// key line, or a refused/failed CAS write. A page carrying both keys is NOT an
/// error: it is a reported [`PageOutcome::Conflict`].
pub fn migrate_inputs(
    root: &fs::WorkspaceRoot,
    opts: &MigrateOptions,
) -> Result<MigrateReport, MigrateError> {
    let (files, _) = fs::domain_snapshot(root).map_err(|e| MigrateError::Corpus(e.to_string()))?;
    let (_index, docs) =
        fs::build_corpus(files).map_err(|e| MigrateError::Corpus(e.to_string()))?;

    let mut pages = Vec::new();
    // BTreeMap iteration is path-sorted → a deterministic, resumable order.
    for (path, doc) in &docs {
        let has_draws = resolve(doc, &Ref::FmKey(DRAWS_FROM.to_string())).is_ok();
        let has_inputs = resolve(doc, &Ref::FmKey(INPUTS.to_string())).is_ok();
        match (has_draws, has_inputs) {
            // Both keys: a synonym-window defect — refuse, report, never merge.
            (true, true) => pages.push(PageOutcome::Conflict { path: path.clone() }),
            // The rename target.
            (true, false) => {
                let plan = plan_rename(doc).ok_or_else(|| MigrateError::Plan(path.clone()))?;
                let args = wire_serve::write::MigrateKeyArgs {
                    id: None,
                    path: wire::Path(path.clone()),
                    actor: opts.actor.clone(),
                    now: opts.now.clone(),
                    if_root: None,
                    if_file_rev: wire::NodeRev(doc.root.node_rev.0.clone()),
                    span: wire::Span(plan.span.start as u64, plan.span.end as u64),
                    new_text: plan.new_grain,
                    from_key: DRAWS_FROM.to_string(),
                    to_key: INPUTS.to_string(),
                    before_rev: wire::NodeRev(plan.before_rev),
                    after_rev: wire::NodeRev(plan.after_rev),
                    dry: opts.dry,
                };
                let outcome = wire_serve::write::migrate_key(root, 0, &args).map_err(|e| {
                    MigrateError::Write {
                        path: path.clone(),
                        detail: wire_err(&e),
                    }
                })?;
                pages.push(PageOutcome::Renamed {
                    path: path.clone(),
                    receipt: outcome.journal_anchor,
                });
            }
            // No dead key → already-migrated or never-declared: out of scope.
            (false, _) => {}
        }
    }
    Ok(MigrateReport {
        pages,
        dry: opts.dry,
    })
}

/// The computed rename for one page: the `draws-from:` grain span, its renamed
/// bytes, and the grain's node-rev transition (for the journal row).
struct RenamePlan {
    span: std::ops::Range<usize>,
    new_grain: String,
    before_rev: String,
    after_rev: String,
}

/// Plan the `draws-from` → `inputs` rename over the PRE-write document. The
/// `draws-from:` `fm_key` grain (key line + any block-sequence value, U2.11 grain)
/// is resolved; only the KEY TOKEN on the first line is renamed, so the colon,
/// spacing, and the whole value (inline flow or indented block items) survive
/// byte-for-byte. `None` when the key line carries no colon (unreachable from
/// the parser — a top-level fm key always has one).
fn plan_rename(doc: &Document) -> Option<RenamePlan> {
    let target = resolve(doc, &Ref::FmKey(DRAWS_FROM.to_string())).ok()?;
    let span = target.span.clone();
    let grain = doc.raw.get(span.clone())?;
    // The grain opens with the key line `<key>:<value…>`; the first colon is the
    // key's colon (the value follows it). Rename only the key portion.
    let colon = grain.find(':')?;
    let key_part = &grain[..colon];
    let new_key_part = key_part.replacen(DRAWS_FROM, INPUTS, 1);
    let new_grain = format!("{new_key_part}{}", &grain[colon..]);
    Some(RenamePlan {
        before_rev: node_rev(grain.as_bytes()),
        after_rev: node_rev(new_grain.as_bytes()),
        new_grain,
        span,
    })
}

/// The node-rev of a byte slice — `blake3-256[:16]`, contract §1 (the same rev
/// law `model` and `pin` compute, the family the engine already uses).
fn node_rev(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().as_str()[..16].to_string()
}

/// A one-line diagnostic for a wire error body (the write-refusal render),
/// mirroring the pin crate's renderer.
fn wire_err(e: &wire::ErrorBody) -> String {
    let code = serde_json::to_value(e.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "error".to_owned());
    match &e.message {
        Some(m) => format!("{code}: {m}"),
        None => code,
    }
}
