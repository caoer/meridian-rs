//! `mrd repair` — lost-pin repair by git-history walk.
//!
//! ```text
//! mrd repair [PAGE] [--dry] [--json]
//! ```
//!
//! # What "lost" means
//! A `meridian-lock` pin carries two planes that fail independently:
//!
//! - the **claim** plane — the `fp1.…` fingerprint, verified against the live target by
//!   [`model::selector::classify_pin`];
//! - the **retrieval** plane — the `hash`, a git blob sha, asked of the object store.
//!
//! A pin is lost when both are dark: the live target no longer verifies the fingerprint and git
//! does not hold the recorded blob. A red pin whose blob is still in the store is ordinary drift
//! with its evidence intact; it is not lost and this verb does not touch it.
//!
//! # The repair
//! One `git log` (`git::Repo::path_history`) over every lost target, then one
//! `git cat-file --batch` (`git::Repo::blobs_with_oids_at`) for every version those commits
//! recorded — never a spawn per pin or per commit. Each historical version is rebuilt into a
//! `Document` and put to the same [`model::selector::classify_pin`] the walk, `status` and
//! `check` colour with; a green answer means that commit's bytes are the pinned content.
//!
//! # The forgery invariant
//! Repair rewrites the pin's `hash` and nothing else — `object`, `selector` and `fingerprint`
//! are never touched. The verb restores the retrieval plane and leaves the claim exactly as the
//! ledger recorded it, so a target that genuinely drifted is still red after a successful repair.
//!
//! No version anywhere in that path's history carries the pinned content ⇒ a TRUE LOSS:
//! reported, never auto-fixed, exit 1.
//!
//! # `--dry`
//! Everything except the disk write: the whole walk runs, every recovery is computed and
//! reported, and the lock write alone is skipped (it rides `LockWriteArgs::dry`, so the door's
//! guards still run). Not a diff face.
//!
//! # Scope: the ambient root only
//! A pin naming another root names another object store and another history, and this verb holds
//! one repository handle. Cross-root pins are skipped and their population is stated, as
//! `check`'s pin plane states `out_of_jurisdiction`.
//!
//! Exit triad: 0 nothing lost, or every lost pin repaired (or rehearsed under `--dry`) / 1 at
//! least one TRUE LOSS / 2 bad invocation or a tool failure.

use std::collections::{BTreeMap, BTreeSet};

use model::Document;
use model::selector::Color;
use serde_json::json;
use wire::{NodeRev, Path as WirePath};
use wire_serve::write::{LockWriteArgs, lock_write};

use crate::{EXIT_FINDINGS, Fail, Format, current_dir};

/// Run `mrd repair [PAGE] [--dry] [--json]`. Errors [`Fail`] — exit 2 on a bad invocation or a
/// tool failure (the workspace cannot be resolved, the corpus cannot be read, git cannot be
/// asked, the lock door refused); exit 1 when any pin is a TRUE LOSS.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Repair::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical.clone());

    let (files, _fold) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let (_index, docs, _unserved) = fs::build_corpus(files);

    let mut survey = survey(&docs, parsed.page.as_deref())?;
    let repo = git::Repo::at(root.0.clone());
    let lost = lost_pins(&repo, &docs, std::mem::take(&mut survey.candidates))?;

    progress(&format!(
        "scanned {} pin(s) in {} page(s) — {} lost, {} outside this root",
        survey.scanned,
        survey.pages,
        lost.len(),
        survey.outside.len()
    ));

    let prefix = repo_prefix(&repo, &root)?;
    let outcomes = recover(&repo, &lost, &prefix)?;
    let applied = apply(&root, &docs, &outcomes, parsed.dry)?;

    let true_loss = outcomes.iter().filter(|o| o.recovered.is_none()).count();
    emit(
        parsed.format,
        &canonical,
        &survey,
        &outcomes,
        applied,
        parsed.dry,
    );

    if true_loss > 0 {
        return Err(Fail::with_code(
            EXIT_FINDINGS,
            format!(
                "{true_loss} pin(s) are a TRUE LOSS: no version in this repository's history \
                 carries the content they claim, so there is nothing to repair them WITH. \
                 Nothing was invented and nothing was written for them."
            ),
        ));
    }
    Ok(())
}

// the survey — which pins are even askable

/// One pin as this verb reads it: where it is declared, what it claims, and the file its blob is
/// the blob of.
#[derive(Debug, Clone)]
struct PinSite {
    /// The page whose `meridian-lock` block declares the row.
    src_path: String,
    /// The row, verbatim — the only thing repair ever rewrites is its `hash`.
    entry: lock::PinEntry,
    /// The target's workspace-relative path with `.md` back on: `object` is the link spelling,
    /// which drops it, and `check::layer0::ask_store` makes the same re-attachment.
    target: String,
}

/// What the corpus offered, including the populations that were not measured.
struct Survey {
    candidates: Vec<PinSite>,
    /// Pins whose object names another root — stated, never silently dropped.
    outside: Vec<String>,
    /// Pins whose `hash` is not an object id, so git cannot be asked at all.
    unaskable: Vec<String>,
    scanned: usize,
    pages: usize,
}

/// Read every `meridian-lock` pin in the corpus (or in one page), splitting the askable from the
/// two populations that are not.
fn survey(docs: &BTreeMap<String, Document>, page: Option<&str>) -> Result<Survey, Fail> {
    let mut out = Survey {
        candidates: Vec::new(),
        outside: Vec::new(),
        unaskable: Vec::new(),
        scanned: 0,
        pages: 0,
    };
    if let Some(page) = page
        && !docs.contains_key(page)
    {
        return Err(Fail::tool(format!(
            "no page `{page}` in this workspace's corpus, so there is no lock block to repair. \
             Give a workspace-relative path, or omit it to scan the whole corpus."
        )));
    }
    for (path, doc) in docs {
        if page.is_some_and(|only| only != path) {
            continue;
        }
        let Ok(Some(found)) = lock::find(doc) else {
            // A refused lock is `check`'s finding to report, not this verb's: repair reads the
            // locks it can parse and stays silent about the rest.
            continue;
        };
        out.pages += 1;
        for entry in found.lock.pins {
            out.scanned += 1;
            // Jurisdiction is decided on structure first (the ordering `check::layer0::objects_in`
            // holds): a behavioural skip would let a broken ambient store hide inside the exemption.
            match addr::Addr::parse(&entry.object) {
                Ok(addr) => {
                    if let Some(name) = addr.root() {
                        out.outside
                            .push(format!("`{path}` pin `{}` (root `{name}`)", entry.object));
                        continue;
                    }
                }
                Err(e) => {
                    out.unaskable.push(format!(
                        "`{path}` pin `{}` is not an address, so WHICH git holds it is unknown \
                         ({e})",
                        entry.object
                    ));
                    continue;
                }
            }
            if !git::is_oid(&entry.hash) {
                out.unaskable.push(format!(
                    "`{path}` pin `{}` has a hash that is not an object id, so git cannot be \
                     asked whether it is still held",
                    entry.object
                ));
                continue;
            }
            let target = format!("{}.md", entry.object);
            out.candidates.push(PinSite {
                src_path: path.clone(),
                entry,
                target,
            });
        }
    }
    Ok(out)
}

/// The pins that are lost — both planes dark. One batched `cat-file --batch-check` answers the
/// retrieval plane for every candidate at once (`git::Repo::object_info`), and the claim plane is
/// [`model::selector::classify_pin`] against the live target. A pin that fails only one of the
/// two is not this verb's business.
fn lost_pins(
    repo: &git::Repo,
    docs: &BTreeMap<String, Document>,
    candidates: Vec<PinSite>,
) -> Result<Vec<PinSite>, Fail> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let oids: Vec<&str> = candidates.iter().map(|c| c.entry.hash.as_str()).collect();
    let held = repo.object_info(&oids).map_err(|e| {
        Fail::tool(format!(
            "the object store could not be asked which pinned blobs it still holds ({e}), and a \
             pin cannot be called lost on an unread store. Nothing was written."
        ))
    })?;
    let mut lost = Vec::new();
    for (site, present) in candidates.into_iter().zip(held) {
        if present.is_some() {
            // Drift with its blob intact, whatever the claim plane says — not a loss.
            continue;
        }
        if matches!(live_color(docs, &site), Color::Green) {
            // The live target still verifies the claim, even though the recorded blob is gone.
            continue;
        }
        lost.push(site);
    }
    Ok(lost)
}

/// The pin's colour against the live target, through [`model::selector::classify_pin`] itself.
/// The lock-row-to-selector projection is likewise the one `view::walk` colours through.
fn live_color(docs: &BTreeMap<String, Document>, site: &PinSite) -> Color {
    let selector = view::walk::model_selector(&site.entry.object, &site.entry.selector);
    model::selector::classify_pin(&selector, &site.entry.fingerprint, docs.get(&site.target))
}

// the walk — one `git log`, one `cat-file --batch`

/// What the history walk concluded about one lost pin.
struct Outcome {
    site: PinSite,
    /// The commit and blob oid whose bytes are the pinned content — `None` is a TRUE LOSS.
    recovered: Option<Recovered>,
    /// How many historical versions of the target were read for this pin.
    versions: usize,
}

#[derive(Debug, Clone)]
struct Recovered {
    commit: String,
    oid: String,
}

/// The workspace's path prefix inside its repository (`""` when the workspace is the top level).
/// `git log --name-status` prints paths from the repository root while the corpus speaks
/// workspace-relative ones; without the prefix, a vault in a subdirectory would match nothing and
/// every pin in it would read as a fabricated TRUE LOSS.
fn repo_prefix(repo: &git::Repo, root: &fs::WorkspaceRoot) -> Result<String, Fail> {
    let top = repo.top_level().map_err(|e| {
        Fail::tool(format!(
            "this workspace's repository could not be located ({e}), and history is git — there \
             is nothing to walk. Nothing was written."
        ))
    })?;
    let Ok(rel) = root.0.strip_prefix(&top) else {
        return Ok(String::new());
    };
    let rel = rel.to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{rel}/"))
    }
}

/// Walk the recorded history of every lost pin's target and decide each one: one `git log` and
/// one `cat-file --batch` for the whole run. The latest matching version wins — history is read
/// oldest-first, so the last green answer is the most recent commit that carried the pinned
/// content.
fn recover(repo: &git::Repo, lost: &[PinSite], prefix: &str) -> Result<Vec<Outcome>, Fail> {
    if lost.is_empty() {
        return Ok(Vec::new());
    }
    let paths: BTreeSet<String> = lost
        .iter()
        .map(|site| format!("{prefix}{}", site.target))
        .collect();
    let pathspec: Vec<&str> = paths.iter().map(String::as_str).collect();
    let changes = repo.path_history(&pathspec).map_err(|e| {
        Fail::tool(format!(
            "the recorded history could not be read ({e}), so no pin can be called a TRUE LOSS \
             on it — that verdict needs a history that was actually walked. Nothing was written."
        ))
    })?;
    // A removal records no bytes at that path, so there is nothing to read for it.
    let specs: Vec<String> = changes
        .iter()
        .filter(|change| change.status != git::ChangeStatus::Deleted)
        .map(|change| format!("{}:{}", change.commit, change.path))
        .collect();
    progress(&format!(
        "walking {} path(s) — {} recorded version(s) to read",
        paths.len(),
        specs.len()
    ));
    let refs: Vec<&str> = specs.iter().map(String::as_str).collect();
    let blobs = repo.blobs_with_oids_at(&refs).map_err(|e| {
        Fail::tool(format!(
            "the recorded bytes could not be read ({e}); the walk found the versions and could \
             not open them, which is not the same as finding nothing. Nothing was written."
        ))
    })?;

    // Index the read versions by repository path, in walk order (oldest first).
    let mut by_path: BTreeMap<&str, Vec<(&str, &git::BlobAt)>> = BTreeMap::new();
    let mut at = 0usize;
    for change in &changes {
        if change.status == git::ChangeStatus::Deleted {
            continue;
        }
        if let Some(Some(blob)) = blobs.get(at) {
            by_path
                .entry(change.path.as_str())
                .or_default()
                .push((change.commit.as_str(), blob));
        }
        at += 1;
    }

    let mut out = Vec::new();
    for (index, site) in lost.iter().enumerate() {
        let key = format!("{prefix}{}", site.target);
        let versions = by_path.get(key.as_str()).map_or(&[][..], Vec::as_slice);
        let selector = view::walk::model_selector(&site.entry.object, &site.entry.selector);
        let mut recovered = None;
        for (commit, blob) in versions {
            let Ok(text) = std::str::from_utf8(&blob.bytes) else {
                // A non-UTF-8 version is not a markdown document; it cannot carry the claim and
                // is not evidence of its absence either.
                continue;
            };
            // A plain `Document`, never a `CandidateDocument`: these bytes are being read, not
            // landed, and a candidate here would enter the byte-landing door census.
            let doc = model::build(text.to_owned(), syntax::parse(text));
            if matches!(
                model::selector::classify_pin(&selector, &site.entry.fingerprint, Some(&doc)),
                Color::Green
            ) {
                recovered = Some(Recovered {
                    commit: (*commit).to_owned(),
                    oid: blob.oid.clone(),
                });
            }
        }
        progress(&format!(
            "[{}/{}] `{}` pin `{}` — {}",
            index + 1,
            lost.len(),
            site.src_path,
            site.entry.object,
            match &recovered {
                Some(found) => format!("recovered at {}", short(&found.commit)),
                None => "TRUE LOSS".to_owned(),
            }
        ));
        out.push(Outcome {
            site: site.clone(),
            recovered,
            versions: versions.len(),
        });
    }
    Ok(out)
}

// the write — through the existing lock door

/// Land every recovery, one guarded lock write per declaring page, through
/// `wire_serve::write::lock_write` — inheriting its flock, world guard, write-what-you-read CAS
/// and artifact guard whole.
fn apply(
    root: &fs::WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    outcomes: &[Outcome],
    dry: bool,
) -> Result<usize, Fail> {
    let mut by_page: BTreeMap<&str, Vec<&Outcome>> = BTreeMap::new();
    for outcome in outcomes {
        if outcome.recovered.is_some() {
            by_page
                .entry(outcome.site.src_path.as_str())
                .or_default()
                .push(outcome);
        }
    }
    let mut written = 0usize;
    for (page, repairs) in by_page {
        let Some(doc) = docs.get(page) else {
            return Err(Fail::tool(format!(
                "`{page}` left the corpus between the scan and the write. Nothing was written."
            )));
        };
        let found = lock::find(doc)
            .map_err(|e| Fail::tool(format!("`{page}`'s lock block could not be read ({e})")))?
            .ok_or_else(|| {
                Fail::tool(format!(
                    "`{page}` no longer carries a lock block. Nothing was written."
                ))
            })?;
        let mut updated = found.lock;
        for outcome in &repairs {
            let recovered = outcome.recovered.as_ref().expect("filtered above");
            // The one mutation this verb performs: the retrieval plane's hash. `object`,
            // `selector` and `fingerprint` ride through untouched.
            let mut entry = outcome.site.entry.clone();
            entry.hash.clone_from(&recovered.oid);
            updated.upsert_pin(entry);
        }
        let args = LockWriteArgs {
            id: None,
            path: WirePath(page.to_owned()),
            lock: updated,
            actor: None,
            now: None,
            if_root: None,
            if_file_rev: NodeRev(doc.root.node_rev.0.clone()),
            dry,
        };
        lock_write(root, None, &args).map_err(|e| {
            // The door's own words, verbatim — the CLI re-spells no refusal.
            let code = serde_json::to_value(e.code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "error".to_owned());
            Fail::tool(format!(
                "the lock door refused the repair of `{page}`: {code}{}{}. Nothing was written \
                 for that page.",
                e.message
                    .as_ref()
                    .map_or_else(String::new, |message| format!(": {message}")),
                e.cause
                    .as_ref()
                    .map_or_else(String::new, |cause| format!(" ({cause})"))
            ))
        })?;
        written += repairs.len();
    }
    Ok(written)
}

// the faces

/// The progress plane: counted lines on stderr, so `--json` on stdout stays machine-clean.
fn progress(line: &str) {
    eprintln!("repair: {line}");
}

fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}

fn emit(
    format: Format,
    workspace: &std::path::Path,
    survey: &Survey,
    outcomes: &[Outcome],
    applied: usize,
    dry: bool,
) {
    let repaired = outcomes.iter().filter(|o| o.recovered.is_some()).count();
    let true_loss = outcomes.len() - repaired;
    match format {
        Format::Json => {
            let rows: Vec<_> = outcomes
                .iter()
                .map(|outcome| {
                    json!({
                        "page": outcome.site.src_path,
                        "object": outcome.site.entry.object,
                        "fingerprint": outcome.site.entry.fingerprint,
                        "hash_before": outcome.site.entry.hash,
                        "versions_read": outcome.versions,
                        "verdict": if outcome.recovered.is_some() { "repaired" } else { "true-loss" },
                        "commit": outcome.recovered.as_ref().map(|r| r.commit.clone()),
                        "hash_after": outcome.recovered.as_ref().map(|r| r.oid.clone()),
                    })
                })
                .collect();
            let value = json!({
                "workspace": workspace.display().to_string(),
                "dry": dry,
                "scanned": survey.scanned,
                "pages": survey.pages,
                "lost": outcomes.len(),
                "repaired": repaired,
                "true_loss": true_loss,
                "written": applied,
                "out_of_jurisdiction": survey.outside,
                "unaskable": survey.unaskable,
                "pins": rows,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!("workspace: {}", workspace.display());
            println!(
                "pins scanned: {}   pages: {}   lost: {}   repaired: {}   true loss: {}{}",
                survey.scanned,
                survey.pages,
                outcomes.len(),
                repaired,
                true_loss,
                if dry { "   (dry: nothing written)" } else { "" }
            );
            for outcome in outcomes {
                match &outcome.recovered {
                    Some(found) => println!(
                        "  repaired  {} · {} → hash {} (at {}, {} version(s) read)",
                        outcome.site.src_path,
                        outcome.site.entry.object,
                        short(&found.oid),
                        short(&found.commit),
                        outcome.versions
                    ),
                    None => println!(
                        "  TRUE LOSS {} · {} — {} recorded version(s) read, none carries the \
                         pinned content",
                        outcome.site.src_path, outcome.site.entry.object, outcome.versions
                    ),
                }
            }
            if !survey.outside.is_empty() {
                println!("outside this root (not measured): {}", survey.outside.len());
            }
            if !survey.unaskable.is_empty() {
                for line in &survey.unaskable {
                    println!("  unaskable {line}");
                }
            }
        }
    }
}

// invocation

#[derive(Debug)]
struct Repair {
    /// Scan one declaring page instead of the whole corpus.
    page: Option<String>,
    dry: bool,
    format: Format,
}

impl Repair {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut page = None;
        let mut dry = false;
        let mut format = Format::Human;
        for arg in args {
            match arg.as_str() {
                "--dry" => dry = true,
                "--json" => format = Format::Json,
                other if other.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown option: {other}")));
                }
                other => {
                    if page.replace(other.to_owned()).is_some() {
                        return Err(Fail::tool(
                            "repair takes at most one PAGE — the page whose lock block declares \
                             the pins to repair; omit it to scan the whole corpus"
                                .to_owned(),
                        ));
                    }
                }
            }
        }
        Ok(Self { page, dry, format })
    }
}
