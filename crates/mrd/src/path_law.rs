//! The §1 path-law admission and its teachings, shared by the whole door
//! family.
//!
//! The rule binds a DOOR FAMILY, and the family is every door the caller names
//! a path at (`docs/wire-contract.md` §12.1, the door-family clause) — so the
//! admission, the `bad_path` teaching, and the fitted cwd respelling live HERE,
//! once, and each door states only its own name and consequence. Before this
//! module the law was enforced at `read` and nowhere else: handed the same
//! absolute path, `walk` walked it, `links` served it, `repair` accepted it,
//! `realise` opened it, and `run` executed it — writing the receipt of a page
//! OUTSIDE the workspace into the workspace's own `receipts/run.md` (measured
//! 2026-08-15 at `073d184f1`, re-measured at `cac5bc8b`).
//!
//! The respell computation stays ONE implementation for every refusal that
//! carries one: [`wire_serve::write::relative_respelling`], the same function
//! the write door and the read door already teach with.

use std::fmt::Write as _;
use std::path::Path;

use wire::ErrorBody;

use crate::Fail;

/// The §1 path-law admission the warm decoder applies (`req_path`), asked of
/// the ONE confinement implementation ([`addr::confined`]) so the CLI doors
/// and the wire cannot drift: workspace-relative, `/`-separated, never
/// absolute, no `.`/`..`/empty segment, no root separator in the head. This
/// was a hand-copy once, and it drifted on exactly the head-colon arm — the
/// wire refused `root:page` while the CLI doors misread it as a literal
/// filename (measured 2026-08-18 at `72099f257`).
pub(crate) fn violates_path_law(path: &str) -> bool {
    !addr::confined(path)
}

/// The family `bad_path` teaching: the §1 rule with the door's own name and
/// consequence, then — when the spelling lies inside this workspace — the
/// respell the write door already computes, then the enumeration pointer for
/// the whole-corpus gesture (laws.md § the face-honesty law, clause 3). Every
/// other bad path gains no pointer — a wrong pointer is worse than none.
pub(crate) fn bad_path_message(
    workspace: &Path,
    path: &str,
    door: &str,
    consequence: &str,
) -> String {
    let mut m = format!(
        "{path} is not a workspace-relative path — the {door} door admits only \
         workspace-relative spellings (§1 path law: no absolute path, no `.`/`..`/empty \
         segment, no `root:` prefix in the head). {consequence}"
    );
    if let Some(respell) = workspace_respell(workspace, path) {
        m.push_str(&respell);
    }
    if crate::names_the_whole_corpus(path) {
        let _ = write!(
            m,
            " To list the corpus instead of one page, `mrd links --json` enumerates every file."
        );
    }
    m
}

/// The workspace-respell suffix every family refusal shares: when the
/// offending spelling is an absolute path lying inside THIS workspace, the
/// relative respelling the one shipped computation produces
/// ([`wire_serve::write::relative_respelling`]) — earned, never guessed.
/// `None` for every other spelling.
fn workspace_respell(workspace: &Path, path: &str) -> Option<String> {
    let canonical = workspace::canonicalize(workspace).ok()?;
    let root = fs::WorkspaceRoot(canonical);
    let rel = wire_serve::write::relative_respelling(&root, path)?;
    Some(format!(
        " This path lies inside this workspace — respell it as `{rel}`."
    ))
}

/// The `--scope` fitting of the family `bad_path` teaching (dogfood
/// 88877785): the flag is named in the message because a put carries two
/// caller-spelled paths and the empty spelling echoes nothing at all, so the
/// bare wire echo cannot say what offended. Both legs state the §1 rule, the
/// §5.4 premise role, the nothing-happened clause, and the recoveries that
/// are always true: a scoped token binds the exact spelling the §4.7 mint
/// echoed, and dropping `--scope` re-arms the §5.1 world-grain premise. The
/// empty leg additionally names the measured cause — an unquoted shell
/// variable that expanded to nothing.
pub(crate) fn scope_bad_path_message(workspace: &Path, scope: &str) -> String {
    let mut m = if scope.is_empty() {
        "--scope is empty — it names no node, and an empty value is usually an \
         unquoted shell variable that expanded to nothing. A scope is the \
         workspace-relative path of the node the --if-fingerprint premise binds \
         (§1 path law: no absolute path, no `.`/`..`/empty segment; wire-contract \
         §5.4)."
            .to_owned()
    } else {
        format!(
            "--scope {scope} is not a workspace-relative path — the put door admits \
             only workspace-relative spellings (§1 path law: no absolute path, no \
             `.`/`..`/empty segment), and the scope names the node the \
             --if-fingerprint premise binds (wire-contract §5.4)."
        )
    };
    m.push_str(" Nothing was sent and nothing was written.");
    if let Some(respell) = workspace_respell(workspace, scope) {
        m.push_str(&respell);
    }
    m.push_str(
        " A scoped token binds the exact spelling the §4.7 mint echoed — pass the \
         `mint.scope` your `mrd read --json` answered beside the token. Without \
         --scope, `--if-fingerprint` takes the §5.1 world fingerprint instead.",
    );
    m
}

/// The door-level admission for the in-process doors (`walk`, `repair`,
/// `realise`, `run`): a violating spelling refuses exit 2 — a bad invocation,
/// before any corpus is read or any task runs — with the family teaching.
/// The envelope doors (`read`, `links`) run the same admission but refuse
/// through their `ErrorBody` seam ([`teach_bad_path`]) so their `--json` faces
/// keep the `{workspace, error}` frame.
pub(crate) fn admit(
    workspace: &Path,
    path: &str,
    door: &str,
    consequence: &str,
) -> Result<(), Fail> {
    if violates_path_law(path) {
        return Err(Fail::tool(bad_path_message(
            workspace,
            path,
            door,
            consequence,
        )));
    }
    Ok(())
}

/// The `bad_path` teaching on an `ErrorBody` (dogfood NEW-A): the warm
/// decoder's refusal echoes only the offending path, so the face composes the
/// family message. No-op on any other code or an already-taught refusal.
pub(crate) fn teach_bad_path(
    workspace: &Path,
    error: &mut ErrorBody,
    door: &str,
    consequence: &str,
) {
    if error.code != wire::ErrorCode::BadPath || error.message.is_some() {
        return;
    }
    let Some(path) = error.path.clone() else {
        return;
    };
    error.message = Some(bad_path_message(workspace, &path.0, door, consequence));
}

/// The fitted half of the miss teaching, one sentence with identical bytes at
/// every door: when the caller's cwd lies inside the workspace and the ref
/// names a file that EXISTS relative to that cwd, the workspace-relative
/// spelling — computed, copy-pasteable, earned and never guessed. `None` when
/// no candidate exists, so no door ever suggests a spelling that is not there.
pub(crate) fn cwd_respell_suffix(workspace: &Path, cwd: &Path, reference: &str) -> Option<String> {
    if reference.starts_with('/') {
        return None;
    }
    let candidate = cwd.join(reference);
    if !candidate.is_file() {
        return None;
    }
    let canonical = workspace::canonicalize(workspace).ok()?;
    let root = fs::WorkspaceRoot(canonical);
    let abs = candidate.to_str()?;
    let rel = wire_serve::write::relative_respelling(&root, abs)?;
    if rel == reference {
        return None;
    }
    Some(format!(
        " Did you mean `{rel}`? — that file exists relative to your cwd, and refs are spelled \
         from the workspace root, never from where you stand."
    ))
}

/// [`cwd_respell_suffix`] on an `ErrorBody`: the `file_not_found` companion to
/// [`teach_bad_path`], appended so admission is unchanged, the ref grammar is
/// unchanged, and nothing downstream sees a rewritten path.
pub(crate) fn teach_cwd_respelling(workspace: &Path, cwd: &Path, error: &mut ErrorBody) {
    if error.code != wire::ErrorCode::FileNotFound {
        return;
    }
    let Some(path) = error.path.clone() else {
        return;
    };
    let Some(suffix) = cwd_respell_suffix(workspace, cwd, &path.0) else {
        return;
    };
    let m = error.message.get_or_insert_with(String::new);
    m.push_str(&suffix);
}
