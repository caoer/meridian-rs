//! The in-process move door (`docs/move.md` §8–§9): the one composed
//! multi-file write — plan under the write flock, one atomic replace per
//! referring page, the rename last — beside `create` and `remove`, which the
//! preset lane already calls in-process. Never a wire op
//! (`docs/wire-contract.md` §16).
//!
//! The order is the crash law (`move.md` §9): every referring page is
//! rewritten first, the rename of OLD is the last act, so a crash anywhere
//! leaves a tree the same command converges from.

use std::path::Path as FsPath;

use query::relocate::{self as planner, LinkCensus, MovePlan, MoveSpec};
use wire::{ErrorBody, ErrorCode, Path};

use crate::write::{
    acquire_write_lock, io_refusal, io_to_wire, path_confined, referential_files,
    stored_form_guard_lazy,
};

/// The caller's request, already through the CLI's own admission (`[root:]path`
/// peeled, the §1 path law at exit 2).
#[derive(Debug, Clone)]
pub struct RelocateArgs {
    /// OLD — a file or directory of the root.
    pub old: Path,
    /// NEW — a full path, or the into-form: a trailing `/` lands OLD under it.
    pub new: Path,
    /// Prefixes never written (`--immutable`).
    pub immutable: Vec<String>,
    /// The names this root answers to (declared name, bound alias).
    pub root_names: Vec<String>,
    pub dry: bool,
}

/// The plan, and — after a real run — what the disk reads back.
#[derive(Debug, Clone)]
pub struct RelocateOutcome {
    pub old: String,
    pub new: String,
    pub is_dir: bool,
    pub plan: MovePlan,
    /// Files under OLD that move with it but are not corpus members
    /// (non-markdown, or outside the hash domain): moved, never rewritten toward.
    pub moved_outside_domain: usize,
    /// The census re-read from disk after the write; `None` on a dry run or a
    /// plan the ambiguity class refused.
    pub read_back: Option<LinkCensus>,
    pub applied: bool,
    pub dry: bool,
}

/// Move OLD to NEW and rewrite every reference the plan names.
///
/// # Errors
/// `bad_path` (an unconfined spelling, NEW occupied or inside OLD, OLD and NEW
/// equal, OLD or NEW under an immutable prefix), `file_not_found` (OLD absent),
/// `workspace_busy` (another writer holds the flock), `bad_request` (a rewrite
/// would land an agent-plane address), or an I/O failure. A plan the ambiguity
/// class refuses is returned as an outcome with `applied: false` and the pairs
/// in `plan.ambiguous` — the plan IS the refusal's payload; the door writes
/// nothing for it.
#[allow(clippy::too_many_lines)]
pub fn relocate(
    root: &fs::WorkspaceRoot,
    args: &RelocateArgs,
) -> Result<RelocateOutcome, Box<ErrorBody>> {
    let old = args.old.0.trim_end_matches('/').to_owned();
    let into = args.new.0.ends_with('/');
    let new_spelled = args.new.0.trim_end_matches('/').to_owned();
    path_confined(root, &Path(old.clone()))?;
    path_confined(root, &Path(new_spelled.clone()))?;

    let old_abs = root.0.join(&old);
    let meta = match std::fs::symlink_metadata(&old_abs) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(file_not_found(&old));
        }
        Err(e) => return Err(io_to_wire(&e)),
    };
    let is_dir = meta.is_dir();

    let new = if into || root.0.join(&new_spelled).is_dir() {
        let base = old.rsplit('/').next().unwrap_or(&old);
        format!("{new_spelled}/{base}")
    } else {
        new_spelled
    };
    if new == old {
        return Err(bad_path(
            &new,
            format!("{old} and {new} are the same path. Nothing was moved."),
        ));
    }
    if is_dir && planner::under_prefix(&old, &new) {
        return Err(bad_path(
            &new,
            format!(
                "{new} lies inside {old} — a directory cannot move into itself. Nothing was moved."
            ),
        ));
    }
    let new_abs = root.0.join(&new);
    if std::fs::symlink_metadata(&new_abs).is_ok() {
        return Err(bad_path(
            &new,
            format!(
                "{new} is occupied — the destination of a move must not exist. Nothing was moved."
            ),
        ));
    }
    for prefix in &args.immutable {
        for (which, path) in [("OLD", &old), ("NEW", &new)] {
            if planner::under_prefix(prefix, path) {
                return Err(bad_path(
                    path,
                    format!(
                        "{which} {path} lies under the immutable prefix {prefix} — a move into, out \
                         of, or across a frozen tree is a write to it. Nothing was moved."
                    ),
                ));
            }
        }
    }

    // The flock first, then the read: the plan is computed against the bytes
    // it will rewrite, and no cooperating writer interleaves (G2).
    let flock = if args.dry {
        None
    } else {
        Some(acquire_write_lock(root)?)
    };

    let (index, docs, _unserved) = fs::build_corpus(referential_files(root)?);
    let canvases = canvas_carriers(root)?;
    let spec = MoveSpec {
        old: old.clone(),
        new: new.clone(),
        is_dir,
        immutable: args
            .immutable
            .iter()
            .map(|p| p.trim_end_matches('/').to_owned())
            .collect(),
        root_names: args.root_names.clone(),
    };
    let plan = planner::plan(&index, &docs, &canvases, &spec);
    let on_disk = if is_dir {
        count_files(&old_abs).map_err(|e| io_to_wire(&e))?
    } else {
        1
    };
    let moved_outside_domain = on_disk.saturating_sub(plan.renames.len());

    if args.dry || !plan.ambiguous.is_empty() {
        return Ok(RelocateOutcome {
            old,
            new,
            is_dir,
            plan,
            moved_outside_domain,
            read_back: None,
            applied: false,
            dry: args.dry,
        });
    }

    // Every candidate is composed and guarded before the first byte lands, so
    // a refusal here leaves the tree untouched.
    let mut staged = Vec::with_capacity(plan.rewrites.len());
    for file in &plan.rewrites {
        let Some(doc) = docs.get(&file.path) else {
            return Err(io_refusal(format!(
                "{} left the corpus while the plan was being composed",
                file.path
            )));
        };
        let candidate = model::candidate_of_body(&file.path, file.apply(&doc.raw));
        stored_form_guard_lazy(Some(doc), &candidate, &Path(file.path.clone()))?;
        staged.push((file.path.clone(), candidate));
    }
    // A canvas carries no document to seal: its bytes are composed against the
    // same pre-image the plan's spans index, and land raw (`move.md` §4 class 5).
    let mut staged_canvases = Vec::with_capacity(plan.canvases.len());
    for file in &plan.canvases {
        let raw = canvases
            .get(&file.path)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| {
                io_refusal(format!(
                    "{} stopped reading as a canvas while the plan was being composed",
                    file.path
                ))
            })?;
        staged_canvases.push((file.path.clone(), file.apply(raw)));
    }
    for (path, candidate) in &staged {
        fs::replace_file(root, FsPath::new(path), candidate).map_err(|e| io_to_wire(&e))?;
    }
    for (path, bytes) in &staged_canvases {
        fs::replace_bytes(root, FsPath::new(path), bytes.as_bytes()).map_err(|e| io_to_wire(&e))?;
    }

    // The rename is the last act (`move.md` §9).
    if let Some(parent) = new_abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_to_wire(&e))?;
    }
    std::fs::rename(&old_abs, &new_abs).map_err(|e| io_to_wire(&e))?;
    for dir in [old_abs.parent(), new_abs.parent()].into_iter().flatten() {
        sync_dir(dir).map_err(|e| io_to_wire(&e))?;
    }

    // The receipt's `after` is read back from disk, never copied from the plan.
    let (index_after, docs_after, _) = fs::build_corpus(referential_files(root)?);
    let canvases_after = canvas_carriers(root)?;
    let read_back = planner::link_census(&index_after, &docs_after, &canvases_after);
    drop(flock);

    Ok(RelocateOutcome {
        old,
        new,
        is_dir,
        plan,
        moved_outside_domain,
        read_back: Some(read_back),
        applied: true,
        dry: false,
    })
}

/// The `ambiguous_ref` refusal a plan with §5 pairs carries: every pair named,
/// nothing written.
#[must_use]
pub fn ambiguity_refusal(plan: &MovePlan) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
    let pairs: Vec<String> = plan
        .ambiguous
        .iter()
        .map(|a| {
            format!(
                "{}: [[{}]] would resolve between {}",
                a.source,
                a.linkpath,
                a.candidates.join(" and ")
            )
        })
        .collect();
    e.message = Some(format!(
        "refused: the move would leave {} bare link(s) resolving between two files of one name — \
         {}. Nothing was moved. Rename the destination, qualify those links, or move the twin \
         first.",
        pairs.len(),
        pairs.join("; ")
    ));
    Box::new(e)
}

/// The workspace's `.canvas` carriers (`move.md` §4 class 5), keyed by path.
/// A carrier the walk found but could not read fails the whole move: the door
/// is about to rewrite references, and it promises nothing it did not read.
fn canvas_carriers(root: &fs::WorkspaceRoot) -> Result<planner::Canvases, Box<ErrorBody>> {
    Ok(fs::canvas::canvas_files(root)
        .map_err(|e| io_refusal(e.to_string()))?
        .into_iter()
        .collect())
}

fn bad_path(path: &str, message: String) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::BadPath);
    e.path = Some(Path(path.to_owned()));
    e.message = Some(message);
    Box::new(e)
}

fn file_not_found(path: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::FileNotFound);
    e.path = Some(Path(path.to_owned()));
    e.message = Some(format!(
        "{path} does not exist in this workspace. Nothing was moved."
    ));
    Box::new(e)
}

/// Every file under `dir`, recursively — symlinks counted as files.
fn count_files(dir: &FsPath) -> std::io::Result<usize> {
    let mut count = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            count += count_files(&entry.path())?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}

fn sync_dir(dir: &FsPath) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}
