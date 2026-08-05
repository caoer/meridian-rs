//! Git plumbing: the one organ that shells out to `git` for content-addressing.
//!
//! # Charter
//! **Owns:** the operational blob-sha path (`git hash-object`, and the eager `-w` write),
//! the object-reachability probe (`git rev-list --objects --all` computed ONCE into a
//! [`ReachableSet`]), and object presence/size lookups (one batched `git cat-file
//! --batch-check`), and **the write history** — the first-parent `git log --name-status`
//! walk ([`Repo::path_history`]) plus the batched blob read that recovers each side's
//! bytes ([`Repo::blobs_at`], one `git cat-file --batch`). History is git's by law (.
//! Every call runs against a [`Repo`] handle — a path plus the git program to run — so a
//! later per-root world constructs one handle per root and nothing here changes.
//!
//! **Never does:** compute an object id itself. **git owns content-addressing** (the
//! ratified law): this crate asks git and reports what git said. When git is absent, or
//! the handle's root is not a repository, every call returns a typed [`GitFail`] — a
//! fabricated or guessed oid is a correctness breach, not a fallback (plan §5, S5 rescue
//! row).
//!
//! **Never does (2):** name a workspace, resolve a root, or hold a global. The caller
//! supplies the root; there is no ambient "the repo" here.
//!
//! **Dependencies:** `std` only. The whole shell-out surface is one auditable leaf, so
//! `git`-invocation churn is a one-crate event (the corollary the `syntax`/pulldown-cmark
//! edge already sets), and every consumer — `mrd`, `wire-serve`, `view` — can take the
//! edge without dragging `model`/`wire` in.
//!
//! # This is NOT the fingerprint hasher
//! `model::fingerprint` owns the engine's content identity (`fp1.span2.b3.…` over
//! normalized span bytes) and `model::fingerprint::verify_object` is pure equality
//! against a git oid the engine did not compute. THIS crate is the operational path that
//! produces that oid for the `objects:` plane. The two never merge: one is the engine's
//! hash, the other is git's.
//!
//! # Filters are applied, deliberately
//! [`Repo::blob_oid`] hashes a work-tree path through git's default path-based rules, so
//! `.gitattributes` clean filters and eol conversion apply — the oid it returns is **the
//! blob git would store**, identical to `git rev-parse HEAD:<path>` after a commit of the
//! same bytes. `--no-filters` would return a different id for a filtered path, and that
//! id would never appear in the object database, so every reachability answer over it
//! would be wrong. The gate proving this is
//! `tests/plumbing.rs::blob_oid_matches_the_committed_blob_under_a_clean_filter`.
//!
//! # The three-state anchoring check
//! This crate gathers the facts; `receipt::anchor::ObjectAnchor` classifies them (the
//! same fact/classify split the origin-anchor axis already uses). One check, one
//! `rev-list`:
//!
//! ```no_run
//! # use std::path::Path;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let repo = git::Repo::at("/path/to/root");
//! let reachable = repo.reachable_objects()?; // ONCE per check, never per blob
//! let oid = repo.blob_oid(Path::new("notes/plan.md"))?;
//! let present = repo.object_exists(&oid)?;
//! // receipt::anchor::ObjectAnchor::classify(&ObjectAnchorFacts {
//! //     object_present: present,
//! //     reachable_from_commit: reachable.contains(&oid),
//! // })
//! # let _ = (present, reachable.contains(&oid));
//! # Ok(())
//! # }
//! ```
//!
//!
//!
//!

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The default git program — resolved through `PATH` like any other caller.
const GIT: &str = "git";

/// The stable fragment of git's not-a-repository fatal. Every invocation runs
/// with `LC_ALL=C` so this match is against git's C-locale wording, never a
/// translated one.
const NOT_A_REPO: &str = "not a git repository";

/// One path in an interval that is not the worktree's, as
/// [`Repo::staged_divergence`] reports it: the path, and the content a commit will
/// record for it — `Some(bytes)`, or `None` when the commit REMOVES the path.
///
/// The shape, not a wrapper, so `fs::overlay_snapshot` consumes it without this
/// std-only leaf appearing in that crate's dependency graph. One vocabulary across
/// the seam: `None` means REMOVED on both sides of it.
pub type StagedPath = (String, Option<Vec<u8>>);

/// A handle on ONE git repository: the root to run `git -C` in, plus the git
/// program to run.
///
/// **The repo is a handle, never a singleton (seam rule D12).** Nothing in this
/// crate discovers "the" repository or caches one globally; a caller holding
/// two roots constructs two handles and the code above is unchanged. A later
/// per-root git repo plugs in by constructing `Repo::at(root_for(root_name))`
/// at the call site.
///
/// Construction is infallible and does no I/O — a non-repo root is not an error
/// until a call is made, and then it is the typed [`GitFail::NotARepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
    program: OsString,
}

impl Repo {
    /// A handle on the repository rooted at `root`, run through the `git` on
    /// `PATH`.
    pub fn at(root: impl Into<PathBuf>) -> Repo {
        Repo {
            root: root.into(),
            program: OsString::from(GIT),
        }
    }

    /// A handle that runs `program` instead of the `git` on `PATH` — for a
    /// pinned git binary, and the seam the honest-degradation gate uses to
    /// prove an absent git returns [`GitFail::Spawn`] rather than a sha.
    pub fn at_with_program(root: impl Into<PathBuf>, program: impl Into<OsString>) -> Repo {
        Repo {
            root: root.into(),
            program: program.into(),
        }
    }

    /// The root this handle runs `git -C` in.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The object id of `path`'s content as git would store it — read-only, the
    /// object database is not touched.
    ///
    /// `path` may be absolute inside [`Repo::root`] or already repo-relative.
    /// Filters apply (see the crate docs): the returned oid is the blob a
    /// commit of these bytes would reference.
    ///
    /// # Errors
    /// [`GitFail::Spawn`] when git cannot be run, [`GitFail::NotARepo`] when the
    /// root is not a repository, [`GitFail::Refused`] when git rejects the path
    /// (missing file, outside the work tree).
    pub fn blob_oid(&self, path: &Path) -> Result<String, GitFail> {
        self.hash_object(path, false)
    }

    /// The object id of `path`'s content, **written into the object database**
    /// (`git hash-object -w`) — the vibe eager write: the blob exists before any
    /// commit references it, so a pin can be verified against it immediately.
    ///
    /// # Named residual G1 — the pending-anchor TTL is `gc.pruneExpire`
    /// A blob written this way is unreachable from every ref until the file is
    /// committed. Git prunes unreachable objects once they age past the
    /// repository's `gc.pruneExpire` (default `2.weeks.ago`), so the
    /// pending-anchor state has a **local-config TTL that this engine documents
    /// and does not prevent** (plan §5, residual G1). Committing the file is the
    /// only durable anchor; until then the gauge counts vibe debt.
    ///
    /// # Errors
    /// As [`Repo::blob_oid`].
    pub fn write_blob(&self, path: &Path) -> Result<String, GitFail> {
        self.hash_object(path, true)
    }

    /// The object id of `bytes` **as if they were the file at `path`**
    /// (`hash-object --path <path> --stdin`) — the oid of content that is not on
    /// disk yet. `write` additionally stores the blob (`-w`), with the same
    /// residual G1 as [`Repo::write_blob`].
    ///
    /// A caller that must record the oid of bytes it has decided to write, but
    /// has not written, needs this: hashing the file first would content-address
    /// the state being replaced. `--path` is what keeps the answer equal to
    /// [`Repo::blob_oid`] on those same bytes once they land — git applies that
    /// path's own `.gitattributes` filters, so a filtered page cannot get one oid
    /// from disk and another from memory.
    ///
    /// # Errors
    /// As [`Repo::blob_oid`].
    pub fn blob_oid_of_bytes(
        &self,
        path: &Path,
        bytes: &[u8],
        write: bool,
    ) -> Result<String, GitFail> {
        // Same guard, same reason as `hash_object`: outside a repository git
        // answers with a number about nothing.
        self.require_repo()?;

        let mut cmd = self.command();
        cmd.arg("hash-object");
        if write {
            cmd.arg("-w");
        }
        cmd.arg("--path").arg(self.relative(path)).arg("--stdin");
        let what = if write {
            "hash-object -w --stdin"
        } else {
            "hash-object --stdin"
        };
        let stdout = self.run_with_stdin(cmd, what, bytes.to_vec())?;
        let text = std::str::from_utf8(&stdout).unwrap_or_default().trim();
        if !is_oid(text) {
            return Err(GitFail::Unexpected {
                what: what.to_owned(),
                detail: format!("output is not an object id: {text:?}"),
            });
        }
        Ok(text.to_ascii_lowercase())
    }

    /// Every object reachable from every ref, computed in ONE
    /// `git rev-list --objects --all` — the anchoring check's reachable set.
    ///
    /// Call this **once per check** and answer membership from the returned set
    /// ([`ReachableSet::contains`] is O(1)). Asking git per blob is the
    /// performance trap this call exists to close. An empty repository (no
    /// commits) yields an empty set, not a failure.
    ///
    /// # Errors
    /// [`GitFail::Spawn`], [`GitFail::NotARepo`], or [`GitFail::Refused`] per
    /// [`Repo::blob_oid`]; [`GitFail::Unexpected`] when git's output is not the
    /// documented `<oid> [path]` shape.
    pub fn reachable_objects(&self) -> Result<ReachableSet, GitFail> {
        let mut cmd = self.command();
        cmd.args(["rev-list", "--objects", "--all"]);
        let stdout = self.run(cmd, "rev-list --objects --all")?;

        // Lines are `<oid>` (commits) or `<oid> <path>` (trees/blobs), and the
        // path is raw bytes — never assume the whole stream is UTF-8.
        let mut oids = HashSet::new();
        for line in stdout.split(|b| *b == b'\n') {
            let token = match line.iter().position(|b| *b == b' ') {
                Some(sp) => &line[..sp],
                None => line,
            };
            if token.is_empty() {
                continue;
            }
            let oid = std::str::from_utf8(token)
                .ok()
                .filter(|t| is_oid(t))
                .ok_or_else(|| GitFail::Unexpected {
                    what: "rev-list --objects --all".to_owned(),
                    detail: format!(
                        "line does not start with an object id: {:?}",
                        token.escape_ascii().to_string()
                    ),
                })?;
            oids.insert(oid.to_ascii_lowercase());
        }
        Ok(ReachableSet { oids })
    }

    /// Type and size for each of `oids`, in input order — `None` where the
    /// object is absent from the database. ONE `git cat-file --batch-check`
    /// process answers the whole slice, so a gauge over N pinned blobs spawns
    /// git once, not N times.
    ///
    /// # Errors
    /// [`GitFail::BadOid`] (before anything is spawned) when an entry is not a
    /// hex object id — a ref name like `HEAD` would otherwise resolve and answer
    /// a question nobody asked. Otherwise as [`Repo::reachable_objects`].
    pub fn object_info(&self, oids: &[&str]) -> Result<Vec<Option<ObjectInfo>>, GitFail> {
        for oid in oids {
            if !is_oid(oid) {
                return Err(GitFail::BadOid {
                    oid: (*oid).to_owned(),
                });
            }
        }
        if oids.is_empty() {
            return Ok(Vec::new());
        }

        let mut query = String::new();
        for oid in oids {
            query.push_str(oid);
            query.push('\n');
        }

        let mut cmd = self.command();
        cmd.args(["cat-file", "--batch-check"]);
        let stdout = self.run_with_stdin(cmd, "cat-file --batch-check", query.into_bytes())?;

        let text = std::str::from_utf8(&stdout).map_err(|_| GitFail::Unexpected {
            what: "cat-file --batch-check".to_owned(),
            detail: "output is not UTF-8".to_owned(),
        })?;
        let mut info = Vec::with_capacity(oids.len());
        for line in text.lines() {
            info.push(parse_batch_line(line)?);
        }
        if info.len() != oids.len() {
            return Err(GitFail::Unexpected {
                what: "cat-file --batch-check".to_owned(),
                detail: format!(
                    "asked about {} objects, got {} answers",
                    oids.len(),
                    info.len()
                ),
            });
        }
        Ok(info)
    }

    /// Whether `oid` is present in the object database — the fact that splits
    /// pending-anchor (present, unreachable) from never-anchored (absent).
    ///
    /// For more than one oid call [`Repo::object_info`] with the whole slice;
    /// this is the single-object convenience over it.
    ///
    /// # Errors
    /// As [`Repo::object_info`].
    pub fn object_exists(&self, oid: &str) -> Result<bool, GitFail> {
        Ok(self
            .object_info(&[oid])?
            .first()
            .is_some_and(Option::is_some))
    }

    // -----------------------------------------------------------------------
    // the write HISTORY — what git recorded, path by path
    // -----------------------------------------------------------------------

    /// **Every recorded write, oldest first** — one [`PathChange`] per (commit,
    /// path) pair on the first-parent walk.
    ///
    /// This is the engine's ONLY history. The engine keeps no memory of its own
    /// (ZT 2026-08-03: *"Engine does not have memory. It should not have. History
    /// is pin to git when we lock. Anything between locks is not history."*), so
    /// archaeology is a git question and this is where it is asked.
    ///
    /// `pathspec` narrows the walk the way `git log -- <path>…` does; an empty
    /// slice walks the whole tree.
    ///
    /// **ONE process for the whole walk.** `--name-status -z` is not optional:
    /// without `-z` git quotes unusual paths and a quoted path matches nothing on
    /// disk (the `nul_paths` lesson, held here too).
    ///
    /// A rename or copy reports the DESTINATION path — that is the path whose
    /// bytes the commit recorded, and the only one a reader can ask about at that
    /// commit.
    ///
    /// # Errors
    /// As [`Repo::blob_oid`]; a repository with no commits yet answers empty
    /// rather than failing, because "nothing recorded" is a history, not a fault.
    pub fn path_history(&self, pathspec: &[&str]) -> Result<Vec<PathChange>, GitFail> {
        self.require_repo()?;
        if !self.head_exists()? {
            return Ok(Vec::new());
        }

        let mut cmd = self.command();
        cmd.args([
            "log",
            "--reverse",
            "--first-parent",
            "--name-status",
            "-z",
            "--format=%x01%H%x02%an%x02%aI",
        ]);
        if !pathspec.is_empty() {
            cmd.arg("--");
            cmd.args(pathspec);
        }
        let stdout = self.run(cmd, "log --name-status")?;
        let text = String::from_utf8_lossy(&stdout);

        let mut out = Vec::new();
        let mut head: Option<(String, String, String)> = None;
        // `--format` terminates with a newline that `-z` does not swallow, so a
        // status token arrives as "\nM" and a header as "\n\u{1}<sha>…". Trimming
        // it is what makes the token classifiable at all — without this every
        // status reads as "not A, not D" and the whole walk renders as splices.
        let mut fields = text
            .split('\0')
            .map(|f| f.trim_matches('\n'))
            .filter(|f| !f.is_empty());
        while let Some(field) = fields.next() {
            if let Some(rest) = field.strip_prefix('\u{1}') {
                let mut parts = rest.splitn(3, '\u{2}');
                let commit = parts.next().unwrap_or_default().to_owned();
                let author = parts.next().unwrap_or_default().to_owned();
                let now = parts.next().unwrap_or_default().to_owned();
                head = Some((commit, author, now));
                continue;
            }
            let Some((commit, author, now)) = head.clone() else {
                // A status token before any commit header cannot be attributed,
                // and an unattributed write is not a fact — skip it rather than
                // inventing the commit it belongs to.
                continue;
            };
            let status = ChangeStatus::of(field);
            // A rename or copy carries the source path first; the destination is
            // the path this commit recorded.
            let path = if field.starts_with('R') || field.starts_with('C') {
                fields.next();
                fields.next()
            } else {
                fields.next()
            };
            let Some(path) = path else { break };
            out.push(PathChange {
                commit,
                author,
                now,
                path: path.to_owned(),
                status,
            });
        }
        Ok(out)
    }

    /// **The bytes at each `<rev>:<path>` spec, in input order** — `None` where
    /// the spec resolves to nothing (the path is absent from that tree, the rev
    /// does not exist, or the object is not a blob).
    ///
    /// ONE `git cat-file --batch` answers the whole slice, the way
    /// [`Repo::object_info`] does for `--batch-check`: a history walk over N
    /// writes asks for 2N sides and spawns git once, not 2N times.
    ///
    /// # Errors
    /// As [`Repo::blob_oid`]; [`GitFail::Unexpected`] when the stream ends
    /// mid-object or answers a different number of specs than were asked.
    pub fn blobs_at(&self, specs: &[&str]) -> Result<Vec<Option<Vec<u8>>>, GitFail> {
        Ok(self
            .blobs_with_oids_at(specs)?
            .into_iter()
            .map(|answer| answer.map(|blob| blob.bytes))
            .collect())
    }

    /// **The oid AND the bytes at each `<rev>:<path>` spec, in input order** —
    /// [`Repo::blobs_at`] without discarding the identity git already printed.
    ///
    /// `cat-file --batch` heads every answer with `<oid> <type> <size>`, so the
    /// object's id arrives in the SAME stream as its content. A caller that needs
    /// both — a history walk that recovers content and must then name the durable
    /// object carrying it — reads them from one pipe rather than re-deriving the
    /// oid with a second spawn per hit. Re-deriving is also not the same question:
    /// `hash-object --path` applies the clean filter, so a filtered repository
    /// would mint an id that is not the one history holds.
    ///
    /// # Errors
    /// As [`Repo::blobs_at`].
    pub fn blobs_with_oids_at(&self, specs: &[&str]) -> Result<Vec<Option<BlobAt>>, GitFail> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = String::new();
        for spec in specs {
            query.push_str(spec);
            query.push('\n');
        }

        let mut cmd = self.command();
        cmd.args(["cat-file", "--batch"]);
        let stdout = self.run_with_stdin(cmd, "cat-file --batch", query.into_bytes())?;
        let answers = parse_batch_stream(&stdout)?;
        if answers.len() != specs.len() {
            return Err(GitFail::Unexpected {
                what: "cat-file --batch".to_owned(),
                detail: format!(
                    "asked about {} specs, got {} answers",
                    specs.len(),
                    answers.len()
                ),
            });
        }
        Ok(answers)
    }

    // -----------------------------------------------------------------------
    // the INDEX — what a commit is about to record (F1)
    // -----------------------------------------------------------------------

    /// **What the INDEX carries, for exactly the paths where the worktree would
    /// answer about something else** — the bytes a commit is about to record.
    ///
    /// Each entry is a [`StagedPath`] — a path relative to [`Repo::root`], and
    /// what the index holds for it:
    ///
    /// - `Some(bytes)` — the index holds these bytes and a commit will record
    ///   them, whatever the worktree says;
    /// - `None` — this commit REMOVES the path, even though the worktree may
    ///   still carry a file there (`git rm --cached`).
    ///
    /// Paths where the index and the worktree agree are **absent**, so the answer
    /// is an OVERLAY: applied over a worktree read it yields the tree the commit
    /// will record for every tracked path, and the caller's own bytes everywhere
    /// else. An empty answer means the two intervals coincide — the caller's
    /// worktree read already spans the commit.
    ///
    /// # Why this is git's question and not the filesystem's
    /// Only git knows what the index holds — **including which index git means.**
    /// `git commit <pathspec>` builds a TEMPORARY index and hands the hook
    /// `GIT_INDEX_FILE`; every query here inherits that environment, so the answer
    /// is about the index that commit will write and never about a stale
    /// `.git/index`.
    ///
    /// # The bytes are the BLOB's, which is the point
    /// `cat-file blob` returns the content git will store, so a repository with a
    /// clean filter or eol conversion gets the bytes `git show HEAD:<path>` will
    /// print after the commit — the interval history carries — rather than the
    /// worktree's smudged form.
    ///
    /// # Unmerged paths are skipped, deliberately
    /// A path with no stage-0 index entry is mid-merge, and git refuses to commit
    /// with unmerged paths before any hook runs — so the fence never meets one.
    /// Skipping leaves the caller's worktree bytes in place for a state no commit
    /// can reach, instead of guessing which stage a merge will resolve to.
    ///
    /// # Errors
    /// As [`Repo::blob_oid`].
    pub fn staged_divergence(&self) -> Result<Vec<StagedPath>, GitFail> {
        self.require_repo()?;

        // git answers repo-relative; this handle may be rooted below the top
        // level, and the prefix is what maps one onto the other.
        let prefix = self.rev_parse("--show-prefix")?;

        let mut candidates: Vec<String> = Vec::new();
        // Worktree-vs-index: every path whose worktree bytes are not the index's,
        // including one deleted from the worktree while the index still carries it.
        candidates.extend(self.nul_paths(&["diff-files", "-z", "--name-only"], "diff-files")?);
        // Index-vs-HEAD deletions: a path this commit REMOVES. `diff-files` cannot
        // see it — once the entry leaves the index the worktree copy is untracked,
        // so nothing compares it against anything.
        if self.head_exists()? {
            candidates.extend(self.nul_paths(
                &[
                    "diff-index",
                    "--cached",
                    "-z",
                    "--diff-filter=D",
                    "--name-only",
                    "HEAD",
                ],
                "diff-index --cached",
            )?);
        }
        candidates.sort();
        candidates.dedup();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // git's OWN answer to "is this path in the index, and as which blob" —
        // never inferred from a `cat-file` failure, which would read a refusal for
        // any other reason as "this commit removes the file" and weaken the check.
        let staged = self.index_blobs(&candidates)?;

        let mut out = Vec::new();
        for path in candidates {
            let Some(rel) = strip_prefix(&path, &prefix) else {
                continue; // outside this handle's root — another workspace's path
            };
            match staged.iter().find(|(p, _)| *p == path) {
                Some((_, oid)) => out.push((rel, Some(self.blob_bytes(oid)?))),
                None => out.push((rel, None)),
            }
        }
        Ok(out)
    }

    /// Whether `HEAD` resolves — false on an unborn branch, where nothing can be
    /// removed from a commit that has no parent.
    fn head_exists(&self) -> Result<bool, GitFail> {
        let mut cmd = self.command();
        cmd.args(["rev-parse", "--verify", "--quiet", "HEAD"]);
        let out = cmd.output().map_err(|source| GitFail::Spawn {
            program: self.program.clone(),
            source,
        })?;
        Ok(out.status.success())
    }

    /// The stage-0 `(repo-relative path, blob oid)` index entries among `paths`.
    /// `:(top)` makes each pathspec repo-root-relative, so this answers the same
    /// way from a nested root as from the top level.
    fn index_blobs(&self, paths: &[String]) -> Result<Vec<(String, String)>, GitFail> {
        let mut cmd = self.command();
        cmd.args(["ls-files", "-s", "-z", "--"]);
        for path in paths {
            cmd.arg(format!(":(top){path}"));
        }
        let stdout = self.run(cmd, "ls-files -s")?;
        let mut out = Vec::new();
        for record in stdout.split(|b| *b == 0) {
            if record.is_empty() {
                continue;
            }
            let text = String::from_utf8_lossy(record);
            // `<mode> SP <oid> SP <stage> TAB <path>`
            let Some((meta, path)) = text.split_once('\t') else {
                continue;
            };
            let mut fields = meta.split(' ');
            let (Some(_mode), Some(oid), Some(stage)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if stage != "0" {
                continue; // unmerged — see the doc comment on `staged_divergence`
            }
            if !is_oid(oid) {
                return Err(GitFail::Unexpected {
                    what: "ls-files -s".to_owned(),
                    detail: format!("index entry oid is not an object id: {oid:?}"),
                });
            }
            out.push((path.to_owned(), oid.to_ascii_lowercase()));
        }
        Ok(out)
    }

    /// One blob's raw bytes, by oid.
    fn blob_bytes(&self, oid: &str) -> Result<Vec<u8>, GitFail> {
        if !is_oid(oid) {
            return Err(GitFail::BadOid {
                oid: oid.to_owned(),
            });
        }
        let mut cmd = self.command();
        cmd.args(["cat-file", "blob", oid]);
        self.run(cmd, "cat-file blob")
    }

    /// A NUL-separated `--name-only` answer as repo-relative paths. `-z` is not
    /// optional: without it git quotes and escapes unusual paths, and a quoted
    /// path matches no index entry.
    fn nul_paths(&self, args: &[&str], what: &str) -> Result<Vec<String>, GitFail> {
        let mut cmd = self.command();
        cmd.args(args);
        let stdout = self.run(cmd, what)?;
        Ok(stdout
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
            .map(|r| String::from_utf8_lossy(r).into_owned())
            .collect())
    }

    /// The **common** git directory — `git rev-parse --git-common-dir`, made
    /// absolute against [`Repo::root`].
    ///
    /// # Why the COMMON dir and not the git dir (U15 / D11)
    /// A linked worktree has its own `--git-dir`
    /// (`<main>/.git/worktrees/<name>`) and shares `--git-common-dir` with every
    /// other worktree of the same repository. **`hooks/` lives under the shared
    /// one**, so N worktrees are N meridian workspaces behind ONE hook
    /// directory. A caller that installed per `--git-dir` would write N hooks of
    /// which git runs exactly the one belonging to the worktree that is
    /// committing — and a caller that installed per worktree top-level would
    /// overwrite its own file N times.
    ///
    /// Note this answer **ignores `core.hooksPath`**: it is where git keeps the
    /// repository's own hooks, not necessarily where git will look for them.
    /// [`Repo::hooks_path`] is the other half, and a caller that means "where do
    /// hooks run from" needs both.
    ///
    /// # Errors
    /// As [`Repo::blob_oid`]; [`GitFail::Unexpected`] when git prints no path.
    pub fn common_dir(&self) -> Result<PathBuf, GitFail> {
        let text = self.rev_parse("--git-common-dir")?;
        Ok(self.absolutize(PathBuf::from(text)))
    }

    /// The top-level directory of the **worktree** this handle's root sits in —
    /// `git rev-parse --show-toplevel`.
    ///
    /// The comparison partner for a caller holding a workspace root of its own:
    /// when the two disagree, "this repository" and "this workspace" name
    /// different directories and any per-root install is guessing which one the
    /// operator meant.
    ///
    /// # Errors
    /// As [`Repo::common_dir`].
    pub fn top_level(&self) -> Result<PathBuf, GitFail> {
        let text = self.rev_parse("--show-toplevel")?;
        Ok(self.absolutize(PathBuf::from(text)))
    }

    /// The configured `core.hooksPath`, or `None` when the repository leaves it
    /// unset — `git config --get core.hooksPath`, resolved against
    /// [`Repo::root`] when it is relative (git resolves it against the worktree
    /// top-level, which is where it runs hooks from).
    ///
    /// **Set means git does not run `$GIT_COMMON_DIR/hooks` at all.** Anything
    /// written there is a silent no-op, so a caller installing a hook has to ask
    /// this before it writes, not after.
    ///
    /// # Errors
    /// As [`Repo::common_dir`]. `git config`'s documented "key not found" (exit
    /// 1, nothing on stderr) is the `None` answer and never an error — but a
    /// higher exit, which `git config` reserves for a malformed section or an
    /// unreadable config file, degrades typed like any other refusal.
    pub fn hooks_path(&self) -> Result<Option<PathBuf>, GitFail> {
        let mut cmd = self.command();
        cmd.args(["config", "--get", "core.hooksPath"]);
        let what = "config --get core.hooksPath";
        let out = cmd.output().map_err(|source| GitFail::Spawn {
            program: self.program.clone(),
            source,
        })?;
        // `git config --get` exits 1 for "the key is not set" and reserves 2+
        // for real failures (invalid section, unreadable file). Treating every
        // non-zero as absence would report a broken config as a clean repo.
        if out.status.code() == Some(1) && out.stderr.is_empty() {
            return Ok(None);
        }
        let stdout = self.harvest(out, what)?;
        let text = String::from_utf8_lossy(&stdout).trim().to_owned();
        if text.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.absolutize(PathBuf::from(text))))
    }

    /// The superproject's working tree when this repository is a **submodule**
    /// of one — `git rev-parse --show-superproject-working-tree` — and `None`
    /// when it is not.
    ///
    /// # This is the deciding artifact, and the neighbouring one answers wrong
    /// A `.git` that is a FILE rather than a directory is the tempting test and
    /// it is **not** the question: a linked worktree's `.git` is a file too, and
    /// a submodule cloned before git 1.7.8 has a real `.git` directory. This
    /// command asks git the question directly and answers empty for both of
    /// those, which is why the submodule refusal is wired to it.
    ///
    /// # Errors
    /// As [`Repo::common_dir`].
    pub fn superproject(&self) -> Result<Option<PathBuf>, GitFail> {
        let text = self.rev_parse("--show-superproject-working-tree")?;
        // Empty is the documented "not a submodule" answer, not a missing one.
        Ok((!text.is_empty()).then(|| self.absolutize(PathBuf::from(text))))
    }

    /// One `git rev-parse <flag>`, trimmed. The shared body of the three path
    /// queries above so they cannot drift in how they read git's answer.
    fn rev_parse(&self, flag: &str) -> Result<String, GitFail> {
        let mut cmd = self.command();
        cmd.args(["rev-parse", flag]);
        let stdout = self.run(cmd, &format!("rev-parse {flag}"))?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_owned())
    }

    /// Git answers `--git-common-dir` relative to the directory it ran in,
    /// which is [`Repo::root`] (every call is `git -C <root>`). An absolute
    /// answer is returned as given.
    fn absolutize(&self, path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        }
    }

    /// `git -C <root>` with a C locale, so a fatal's wording is the one
    /// [`NOT_A_REPO`] matches on any operator's machine.
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.arg("-C").arg(&self.root).env("LC_ALL", "C");
        cmd
    }

    fn hash_object(&self, path: &Path, write: bool) -> Result<String, GitFail> {
        // Read-only `hash-object` ANSWERS OUTSIDE A REPOSITORY: git happily
        // content-addresses loose bytes with no `.gitattributes` context, so the
        // oid it returns there is not "the blob this repo would store" — it is a
        // number about nothing, and a pin recording it claims a repo that does
        // not exist. Ask for the git dir first so a non-repo root degrades
        // typed. (`-w` already refuses without an object database; the guard
        // makes both paths answer alike.)
        self.require_repo()?;

        let mut cmd = self.command();
        cmd.arg("hash-object");
        if write {
            cmd.arg("-w");
        }
        cmd.arg("--").arg(self.relative(path));
        let what = if write {
            "hash-object -w"
        } else {
            "hash-object"
        };
        let stdout = self.run(cmd, what)?;
        let text = std::str::from_utf8(&stdout).unwrap_or_default().trim();
        if !is_oid(text) {
            return Err(GitFail::Unexpected {
                what: what.to_owned(),
                detail: format!("output is not an object id: {text:?}"),
            });
        }
        Ok(text.to_ascii_lowercase())
    }

    /// The one-spawn repository guard: `git rev-parse --git-dir` succeeds only
    /// inside a repository, so its failure is the typed
    /// [`GitFail::NotARepo`] every entry point degrades through.
    fn require_repo(&self) -> Result<(), GitFail> {
        let mut cmd = self.command();
        cmd.args(["rev-parse", "--git-dir"]);
        self.run(cmd, "rev-parse --git-dir")?;
        Ok(())
    }

    /// An absolute path inside the root becomes repo-relative; anything else is
    /// passed to git as given (git resolves it against `-C <root>`).
    fn relative<'p>(&self, path: &'p Path) -> &'p Path {
        path.strip_prefix(&self.root).unwrap_or(path)
    }

    fn run(&self, mut cmd: Command, what: &str) -> Result<Vec<u8>, GitFail> {
        let out = cmd.output().map_err(|source| GitFail::Spawn {
            program: self.program.clone(),
            source,
        })?;
        self.harvest(out, what)
    }

    fn run_with_stdin(
        &self,
        mut cmd: Command,
        what: &str,
        stdin: Vec<u8>,
    ) -> Result<Vec<u8>, GitFail> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| GitFail::Spawn {
                program: self.program.clone(),
                source,
            })?;
        // Feed stdin from a thread: a large query and a large answer would
        // otherwise deadlock on the pipe buffers (we write while git writes).
        let mut pipe = child.stdin.take().ok_or_else(|| GitFail::Unexpected {
            what: what.to_owned(),
            detail: "child stdin was not piped".to_owned(),
        })?;
        let feeder = std::thread::spawn(move || pipe.write_all(&stdin));
        let out = child.wait_with_output().map_err(|source| GitFail::Io {
            what: what.to_owned(),
            source,
        })?;
        match feeder.join() {
            Ok(Ok(())) => {}
            // A broken pipe here means git exited early; its own status and
            // stderr below are the honest report, so only a real write error
            // that outlived a SUCCESSFUL git run is surfaced as I/O.
            Ok(Err(source)) => {
                if out.status.success() {
                    return Err(GitFail::Io {
                        what: what.to_owned(),
                        source,
                    });
                }
            }
            Err(_) => {
                return Err(GitFail::Unexpected {
                    what: what.to_owned(),
                    detail: "the stdin feeder thread panicked".to_owned(),
                });
            }
        }
        self.harvest(out, what)
    }

    fn harvest(&self, out: Output, what: &str) -> Result<Vec<u8>, GitFail> {
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if stderr.contains(NOT_A_REPO) {
            return Err(GitFail::NotARepo {
                root: self.root.clone(),
            });
        }
        Err(GitFail::Refused {
            what: what.to_owned(),
            status: out.status.code(),
            stderr,
        })
    }
}

/// The set of objects reachable from every ref, from ONE
/// [`Repo::reachable_objects`] call. Membership is O(1); the whole point is
/// that the anchoring check never asks git per blob.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReachableSet {
    oids: HashSet<String>,
}

impl ReachableSet {
    /// Whether `oid` is reachable from a ref. Git spells object ids in
    /// lowercase hex; an uppercase spelling from a caller answers the same.
    #[must_use]
    pub fn contains(&self, oid: &str) -> bool {
        self.oids.contains(oid)
            || (oid.bytes().any(|b| b.is_ascii_uppercase())
                && self.oids.contains(&oid.to_ascii_lowercase()))
    }

    /// How many objects are reachable.
    #[must_use]
    pub fn len(&self) -> usize {
        self.oids.len()
    }

    /// Whether nothing is reachable — an empty repository, or one with no refs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.oids.is_empty()
    }
}

/// What git knows about one object in its database: its type (`blob`, `tree`,
/// `commit`, `tag`) and its size in bytes — the byte count the vibe-debt gauge
/// sums without a second git call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// Git's object type word, verbatim.
    pub kind: String,
    /// The object's size in bytes, as git reports it.
    pub size: u64,
}

/// One answered `<rev>:<path>` spec — the object's id AND its bytes, both taken
/// from the ONE `cat-file --batch` stream that answered the query
/// ([`Repo::blobs_with_oids_at`]).
///
/// The pair is one value because the two halves answer one question that
/// separates badly: a caller that recovers content from history and must then
/// name the durable object carrying it needs git's id for THESE bytes, and
/// re-hashing them locally asks a filtered question instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobAt {
    /// The object id git printed for this spec, verbatim.
    pub oid: String,
    /// The object's bytes, exactly as git stores them.
    pub bytes: Vec<u8>,
}

/// What one commit did to one path — the unit of recorded history.
///
/// Every field is git's own answer, quoted rather than derived: the engine does
/// not date a write, attribute one, or decide what changed. It asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    /// The full commit id that recorded this write.
    pub commit: String,
    /// The commit's author NAME, as git spells it — deliberately without the
    /// email.
    ///
    /// The name is the field that can equal an identity a page declares (a
    /// task's `owner:`, a reviewer handle). `Name <email>` never equals one, so
    /// carrying the email would leave every actor-vs-owner rule structurally
    /// unable to fire — a whole class of law silently disabled, which is worse
    /// than a coarse answer. The engine holds no email-to-handle mapping and does
    /// not invent one.
    pub author: String,
    /// The author date, ISO-8601 with offset (git's `%aI`).
    pub now: String,
    /// The repo-relative path this commit recorded. For a rename or a copy this
    /// is the DESTINATION — the path whose bytes exist at this commit.
    pub path: String,
    /// What the commit did to the path.
    pub status: ChangeStatus,
}

/// What a commit did to a path, collapsed to the three states a reader can act
/// on. Git's fuller alphabet (`R`, `C`, `T`, `U`, `X`) all land on
/// [`ChangeStatus::Modified`]: each leaves bytes at the destination path, which
/// is the fact a caller reading those bytes needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    /// The path did not exist before this commit (`A`).
    Added,
    /// The path existed before and after (`M`, and every other letter).
    Modified,
    /// The path existed before and not after (`D`).
    Deleted,
}

impl ChangeStatus {
    /// Read git's `--name-status` letter. The score suffix on `R100` / `C75` is
    /// ignored — similarity is not one of the three states.
    fn of(token: &str) -> ChangeStatus {
        match token.as_bytes().first() {
            Some(b'A') => ChangeStatus::Added,
            Some(b'D') => ChangeStatus::Deleted,
            _ => ChangeStatus::Modified,
        }
    }

    /// The word this status is spelled with on a rendered surface.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeStatus::Added => "create",
            ChangeStatus::Modified => "splice",
            ChangeStatus::Deleted => "remove",
        }
    }
}

/// A git call that did not produce an answer. Every variant is an honest
/// degradation: the caller learns what failed and never receives a sha the
/// engine made up.
#[derive(Debug)]
pub enum GitFail {
    /// Git could not be run at all — absent from `PATH`, or not executable.
    Spawn {
        /// The program that could not be run.
        program: OsString,
        /// The underlying spawn error.
        source: io::Error,
    },
    /// The handle's root is not a git repository (git's `not a git repository`
    /// fatal). The anchoring check degrades; it never fabricates a state.
    NotARepo {
        /// The root that is not a repository.
        root: PathBuf,
    },
    /// Git ran and refused: a missing path, a path outside the work tree, a
    /// corrupt object database.
    Refused {
        /// The plumbing call, as spelled on the command line.
        what: String,
        /// Git's exit status, or `None` when a signal killed it.
        status: Option<i32>,
        /// Git's stderr, trimmed.
        stderr: String,
    },
    /// The caller passed something that is not a hex object id. Refused
    /// **before** git is spawned: `cat-file` resolves revisions, so a ref name
    /// would silently answer a different question.
    BadOid {
        /// The rejected spelling.
        oid: String,
    },
    /// An I/O error while talking to a git process that otherwise succeeded.
    Io {
        /// The plumbing call being run.
        what: String,
        /// The underlying I/O error.
        source: io::Error,
    },
    /// Git succeeded but its output was not the documented shape — a wrapper
    /// script, or a git old enough to disagree with the parser.
    Unexpected {
        /// The plumbing call being run.
        what: String,
        /// What was wrong with the output.
        detail: String,
    },
}

impl fmt::Display for GitFail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                write!(f, "cannot run `{}` ({source})", program.to_string_lossy())
            }
            Self::NotARepo { root } => {
                write!(f, "not a git repository: {}", root.display())
            }
            Self::Refused {
                what,
                status,
                stderr,
            } => match status {
                Some(code) => write!(f, "`git {what}` failed (exit {code}): {stderr}"),
                None => write!(f, "`git {what}` was killed by a signal: {stderr}"),
            },
            Self::BadOid { oid } => write!(f, "not a git object id: {oid:?}"),
            Self::Io { what, source } => write!(f, "i/o error talking to `git {what}` ({source})"),
            Self::Unexpected { what, detail } => {
                write!(f, "unexpected `git {what}` output: {detail}")
            }
        }
    }
}

impl std::error::Error for GitFail {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// One `cat-file --batch-check` line: `<oid> <type> <size>`, or `<oid> missing`.
fn parse_batch_line(line: &str) -> Result<Option<ObjectInfo>, GitFail> {
    let mut parts = line.split(' ');
    let (Some(_oid), Some(second)) = (parts.next(), parts.next()) else {
        return Err(GitFail::Unexpected {
            what: "cat-file --batch-check".to_owned(),
            detail: format!("unparseable line: {line:?}"),
        });
    };
    if second == "missing" || second == "ambiguous" {
        return Ok(None);
    }
    let size = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| GitFail::Unexpected {
            what: "cat-file --batch-check".to_owned(),
            detail: format!("line carries no size: {line:?}"),
        })?;
    Ok(Some(ObjectInfo {
        kind: second.to_owned(),
        size,
    }))
}

/// The whole `cat-file --batch` stream: one answer per spec, in order. An
/// answer is `<oid> <type> <size>\n<size bytes>\n`, or a one-line
/// `<spec> missing` / `<spec> ambiguous` — which is a real answer ("that spec
/// resolves to nothing"), never a failure.
///
/// The size header is what makes this parseable at all: content can carry
/// newlines and NULs, so the byte count is read first and the contents are taken
/// by length, never scanned for a terminator.
fn parse_batch_stream(stream: &[u8]) -> Result<Vec<Option<BlobAt>>, GitFail> {
    let unexpected = |detail: String| GitFail::Unexpected {
        what: "cat-file --batch".to_owned(),
        detail,
    };
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < stream.len() {
        let end = stream[at..]
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| unexpected("stream ends mid-header".to_owned()))?
            + at;
        let header = String::from_utf8_lossy(&stream[at..end]).into_owned();
        at = end + 1;

        let mut parts = header.rsplitn(3, ' ');
        let (Some(size), Some(kind), oid) = (parts.next(), parts.next(), parts.next()) else {
            return Err(unexpected(format!("unparseable header: {header:?}")));
        };
        if kind == "missing" || kind == "ambiguous" || size == "missing" || size == "ambiguous" {
            out.push(None);
            continue;
        }
        let size: usize = size
            .parse()
            .map_err(|_| unexpected(format!("header carries no size: {header:?}")))?;
        let stop = at
            .checked_add(size)
            .filter(|stop| *stop <= stream.len())
            .ok_or_else(|| unexpected(format!("stream ends mid-object: {header:?}")))?;
        // The oid is git's own answer for this spec, taken from the header it
        // already printed — never re-derived (`Repo::blobs_with_oids_at` says why).
        let oid = oid
            .ok_or_else(|| unexpected(format!("header carries no oid: {header:?}")))?
            .to_owned();
        out.push(Some(BlobAt {
            oid,
            bytes: stream[at..stop].to_vec(),
        }));
        // git writes one newline after the contents.
        at = stop + 1;
    }
    Ok(out)
}

/// Whether `text` is a hex object id — sha1 (40) or sha256 (64), the two
/// spellings git mints. The guard that keeps a ref name out of `cat-file`.
#[must_use]
pub fn is_oid(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A repo-relative path as a path relative to a root whose own repo-relative
/// prefix is `prefix` (git's `--show-prefix`, empty at the top level).
///
/// `None` when the path lies OUTSIDE that root: a repository holding two
/// workspaces answers about both, and a path belonging to the other one may not
/// silently acquire this root's keys.
fn strip_prefix(repo_relative: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(repo_relative.to_owned());
    }
    repo_relative.strip_prefix(prefix).map(ToOwned::to_owned)
}

/// The git program a bare [`Repo::at`] handle runs, for a caller that wants to
/// name it in a diagnostic.
#[must_use]
pub fn default_program() -> &'static OsStr {
    OsStr::new(GIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_shapes_are_sha1_or_sha256_hex() {
        assert!(is_oid(&"a".repeat(40)));
        assert!(is_oid(&"0".repeat(64)));
        assert!(!is_oid("HEAD"));
        assert!(!is_oid(&"a".repeat(39)));
        assert!(!is_oid(&"z".repeat(40)));
        assert!(!is_oid(""));
    }

    #[test]
    fn batch_check_lines_parse_present_and_missing() {
        let present = parse_batch_line("fd3959e7e0fc6795a7d18e476c6e4e4fd386d8d5 blob 5")
            .expect("parseable")
            .expect("present");
        assert_eq!(present.kind, "blob");
        assert_eq!(present.size, 5);

        assert!(
            parse_batch_line("0000000000000000000000000000000000000000 missing")
                .expect("parseable")
                .is_none()
        );
        assert!(parse_batch_line("garbage").is_err());
    }

    #[test]
    fn reachable_set_answers_either_hex_case() {
        let mut oids = HashSet::new();
        oids.insert("fd3959e7e0fc6795a7d18e476c6e4e4fd386d8d5".to_owned());
        let set = ReachableSet { oids };
        assert!(set.contains("fd3959e7e0fc6795a7d18e476c6e4e4fd386d8d5"));
        assert!(set.contains("FD3959E7E0FC6795A7D18E476C6E4E4FD386D8D5"));
        assert!(!set.contains(&"0".repeat(40)));
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
    }

    /// The oid guard runs BEFORE git is spawned: a ref name is refused as
    /// [`GitFail::BadOid`], never resolved. Proven with a program name that
    /// does not exist — reaching git at all would produce `Spawn` instead.
    #[test]
    fn ref_names_are_refused_before_git_is_spawned() {
        let repo = Repo::at_with_program("/nonexistent", "definitely-not-git-b7f3");
        let fail = repo.object_exists("HEAD").expect_err("HEAD is not an oid");
        assert!(
            matches!(&fail, GitFail::BadOid { oid } if oid == "HEAD"),
            "expected BadOid, got {fail:?}"
        );
    }

    /// An empty query answers without spawning anything.
    #[test]
    fn empty_object_info_query_spawns_nothing() {
        let repo = Repo::at_with_program("/nonexistent", "definitely-not-git-b7f3");
        assert_eq!(repo.object_info(&[]).expect("no spawn"), Vec::new());
    }
}
