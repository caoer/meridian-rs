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
//! # Two rungs, and the target assessment is a DOOR
//! The PIN SWEEP is an enumerator — a population of pins under a root. The per-pin TARGET
//! ASSESSMENT is a DOOR, because the pin row names ONE target, so it owes the read every
//! named-path door owes: the target is READ FROM DISK whether or not the hash domain holds it
//! (§12.1 — corpus residency is never a read admission test). An intact target means the pin is
//! NOT LOST, so the row stays byte-identical as the ordinary consequence of having read it —
//! not as a carve-out for excluded paths.
//!
//! # The floor
//! Under every remedy shape: a repair never writes `hash` to a blob absent from disk while
//! `fingerprint` describes one that is present. [`floor`] asserts that against the disk alone,
//! before any page is written.
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
    let (_index, mut docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);
    if let Some(page) = parsed.page.as_deref() {
        crate::walk_cmd::admit_named_page(&root, &mut docs, page);
    }
    let docs = docs;

    let mut survey = survey(&docs, parsed.page.as_deref())?;
    let repo = git::Repo::at(root.0.clone());
    let lost = lost_pins(&repo, &root, &docs, std::mem::take(&mut survey.candidates))?;

    progress(&format!(
        "scanned {} pin(s) in {} page(s) — {} lost, {} in another root's object store",
        survey.scanned,
        survey.pages,
        lost.len(),
        survey.outside.len()
    ));

    let prefix = repo_prefix(&repo, &root)?;
    let outcomes = recover(&repo, &lost, &prefix)?;
    let applied = apply(
        &root,
        &docs,
        &outcomes,
        parsed.dry,
        parsed.format,
        &canonical,
    )?;

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
    root: &fs::WorkspaceRoot,
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
        if matches!(live_color(root, docs, &site), Color::Green) {
            // The live target still verifies the claim, even though the recorded blob is gone.
            continue;
        }
        lost.push(site);
    }
    Ok(lost)
}

/// The pin's colour against the live target, through [`model::selector::classify_pin`] itself.
/// The lock-row-to-selector projection is likewise the one `view::walk` colours through.
///
/// The pin row names ONE target, so this assessment is a DOOR and owes the read every named-path
/// door owes: the corpus snapshot is the hash domain, and **corpus residency is never a read
/// admission test** (§12.1, `wire-contract.md:465`, `:813`). A target the domain excludes is
/// therefore READ FROM DISK here rather than answered as absent — otherwise an intact pin reads
/// as lost purely because its target is out of domain, and `repair` moves its `hash` off live
/// content. `pin` already obeyed this law when the caller named such a target.
fn live_color(
    root: &fs::WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    site: &PinSite,
) -> Color {
    let selector = view::walk::model_selector(&site.entry.object, &site.entry.selector);
    if let Some(doc) = docs.get(&site.target) {
        model::selector::classify_pin(&selector, &site.entry.fingerprint, Some(doc))
    } else {
        // A miss here is "not in the hash domain" OR "no such file"; the door read tells them
        // apart. A read failure leaves the answer exactly what it was before this clause —
        // the pin is assessed against no document, never against a fabricated one.
        let loaded = fs::load(root, std::path::Path::new(&site.target)).ok();
        model::selector::classify_pin(&selector, &site.entry.fingerprint, loaded.as_ref())
    }
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
            "[{}/{}] `{}` pin `{}` target `{}` — {}",
            index + 1,
            lost.len(),
            site.src_path,
            site.entry.object,
            site.target,
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

/// THE FLOOR — the one thing a repair may never do, asserted on its own arm and independent of
/// every domain question above it: **a repair may never write `hash:` to a blob absent from disk
/// while `fingerprint:` describes one that is present.** The two fields of one pin row may not
/// disagree about which version they attest.
///
/// This asks the DISK and nothing else — not the corpus, not the hash domain, not how the pin was
/// classified — so it still holds if the assessment above it is later reshaped. A target that
/// verifies the fingerprint right now IS the version the claim plane describes, and repointing the
/// retrieval plane at some other blob would move the attestation off live content.
///
/// It runs BEFORE any page is written, so a violation writes nothing at all rather than leaving a
/// half-repaired corpus.
fn floor(root: &fs::WorkspaceRoot, outcomes: &[Outcome]) -> Result<(), Fail> {
    for outcome in outcomes {
        let Some(recovered) = outcome.recovered.as_ref() else {
            continue;
        };
        if recovered.oid == outcome.site.entry.hash {
            continue;
        }
        let selector =
            view::walk::model_selector(&outcome.site.entry.object, &outcome.site.entry.selector);
        let Ok(live) = fs::load(root, std::path::Path::new(&outcome.site.target)) else {
            continue;
        };
        if !matches!(
            model::selector::classify_pin(&selector, &outcome.site.entry.fingerprint, Some(&live)),
            Color::Green
        ) {
            continue;
        }
        return Err(Fail::tool(format!(
            "refusing to repair `{}` pin `{}`: its target `{}` is PRESENT on disk and still \
             carries the content its fingerprint describes, so this pin is not lost and its \
             `hash` would be moved to `{}` — a blob that is not that live version. The two \
             fields of one pin row may not disagree about which version they attest. Nothing \
             was written for any page.",
            outcome.site.src_path, outcome.site.entry.object, outcome.site.target, recovered.oid
        )));
    }
    Ok(())
}

/// Land every recovery, one guarded lock write per declaring page, through
/// `wire_serve::write::lock_write` — inheriting its flock, world guard, write-what-you-read CAS
/// and artifact guard whole.
fn apply(
    root: &fs::WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    outcomes: &[Outcome],
    dry: bool,
    format: Format,
    workspace: &std::path::Path,
) -> Result<usize, Fail> {
    floor(root, outcomes)?;
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
            // The `--json` face owes a frame on every leg that can refuse; the exit triad stays
            // this verb's — a refused door is a TOOL failure, never the findings leg.
            crate::engine::json_error_frame(format, workspace, &e);
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
                        "target": outcome.site.target,
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
                    // Every action names WHICH PIN and WHICH TARGET. This verb acts on many pins
                    // in one invocation, so an aggregate count is not correlatable by a caller
                    // holding its own request — the subject rides each line.
                    Some(found) => println!(
                        "  repaired  {} · pin {} · target {} — hash {} → {} (at {}, {} \
                         version(s) read)",
                        outcome.site.src_path,
                        outcome.site.entry.object,
                        outcome.site.target,
                        short(&outcome.site.entry.hash),
                        short(&found.oid),
                        short(&found.commit),
                        outcome.versions
                    ),
                    None => println!(
                        "  TRUE LOSS {} · pin {} · target {} — {} recorded version(s) read, none \
                         carries the pinned content",
                        outcome.site.src_path,
                        outcome.site.entry.object,
                        outcome.site.target,
                        outcome.versions
                    ),
                }
            }
            if !survey.outside.is_empty() {
                println!(
                    "in another root's object store (not measured here): {}",
                    survey.outside.len()
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace holding one dot-segment target, plus the pin row that names it
    /// with a fingerprint minted over the section as it stands on disk.
    fn dot_segment_pin(body: &str) -> (tempfile::TempDir, fs::WorkspaceRoot, PinSite) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(tmp.path().to_path_buf());
        std::fs::create_dir_all(tmp.path().join(".github")).expect("mkdir");
        std::fs::write(tmp.path().join(".github/dotspec.md"), body).expect("write");

        let doc = fs::load(&root, std::path::Path::new(".github/dotspec.md")).expect("load");
        let selector = model::selector::Selector::parse(".github/dotspec#Rule");
        let (_, resolved) =
            model::selector::resolve_selector(&selector, Some(&doc)).expect("the section resolves");
        let fingerprint = model::fingerprint::fingerprint_span(&doc, &resolved.span, &[])
            .expect("the section is non-empty")
            .into_string();

        let site = PinSite {
            src_path: "holder.md".to_owned(),
            entry: lock::PinEntry {
                object: ".github/dotspec".to_owned(),
                // A blob the store does not hold — the retrieval plane is dark, which
                // is what put this pin in front of the walk at all.
                hash: "1111111111111111111111111111111111111111".to_owned(),
                selector: lock::Selector::Path(vec!["Rule".to_owned()]),
                fingerprint,
                extra: BTreeMap::new(),
            },
            target: ".github/dotspec.md".to_owned(),
        };
        (tmp, root, site)
    }

    fn outcome(site: PinSite, oid: &str) -> Outcome {
        Outcome {
            site,
            recovered: Some(Recovered {
                commit: "2222222222222222222222222222222222222222".to_owned(),
                oid: oid.to_owned(),
            }),
            versions: 1,
        }
    }

    /// ⛔ THE FLOOR, asserted on its own arm and reached DIRECTLY — not through
    /// the assessment above it, so it still binds if that assessment is later
    /// reshaped. The target is PRESENT on disk and still carries the content its
    /// fingerprint describes, so moving `hash` to any other blob would leave the
    /// two fields of one pin row attesting different versions.
    #[test]
    fn the_floor_refuses_to_move_a_hash_off_content_that_is_present() {
        let (_tmp, root, site) = dot_segment_pin("# Rule\n\nthe value is SEVEN\n");
        let outcomes = vec![outcome(site, "3333333333333333333333333333333333333333")];
        let refused = floor(&root, &outcomes).expect_err("the floor refuses");
        let said = format!("{refused:?}");
        assert!(
            said.contains(".github/dotspec.md"),
            "and it names the target it read: {said}"
        );
    }

    /// The control that makes the gate above discriminate: when the live target
    /// does NOT carry the pinned content, the fingerprint describes nothing that
    /// is present, the floor has no opinion, and an ordinary repair proceeds.
    #[test]
    fn the_floor_is_silent_when_the_live_target_no_longer_carries_the_claim() {
        let (_tmp, root, mut site) = dot_segment_pin("# Rule\n\nthe value is SEVEN\n");
        std::fs::write(
            root.0.join(".github/dotspec.md"),
            "# Rule\n\nthe value DRIFTED away from the claim\n",
        )
        .expect("drift the target");
        site.entry.hash = "1111111111111111111111111111111111111111".to_owned();
        let outcomes = vec![outcome(site, "3333333333333333333333333333333333333333")];
        floor(&root, &outcomes).expect("a genuinely lost pin is not blocked by the floor");
    }
}
