//! Git plumbing: the one organ that shells out to `git` for content-addressing.
//!
//! # Charter
//! **Owns:** the operational blob-sha path (`git hash-object`, and the eager
//! `-w` write), the object-reachability probe (`git rev-list --objects --all`
//! computed ONCE into a [`ReachableSet`]), and object presence/size lookups
//! (one batched `git cat-file --batch-check`). Every call runs against a
//! [`Repo`] handle — a path plus the git program to run — so a later per-root
//! world constructs one handle per root and nothing here changes.
//!
//! **Never does:** compute an object id itself. **git owns content-addressing**
//! (the ratified law): this crate asks git and reports what git said. When git
//! is absent, or the handle's root is not a repository, every call returns a
//! typed [`GitFail`] — a fabricated or guessed oid is a correctness breach, not
//! a fallback (plan §5, S5 rescue row).
//!
//! **Never does (2):** name a workspace, resolve a root, or hold a global. The
//! caller supplies the root; there is no ambient "the repo" here.
//!
//! **Dependencies:** `std` only. The whole shell-out surface is one auditable
//! leaf, so `git`-invocation churn is a one-crate event (the corollary the
//! `syntax`/pulldown-cmark edge already sets), and every consumer — `mrd`,
//! `wire-serve`, `view` — can take the edge without dragging `model`/`wire` in.
//!
//! # This is NOT the fingerprint hasher
//! `model::fingerprint` owns the engine's content identity (`fp1.span2.b3.…`
//! over normalized span bytes) and `model::fingerprint::verify_object` is pure
//! equality against a git oid the engine did not compute. THIS crate is the
//! operational path that produces that oid for the `objects:` plane. The two
//! never merge: one is the engine's hash, the other is git's.
//!
//! # Filters are applied, deliberately
//! [`Repo::blob_oid`] hashes a work-tree path through git's default path-based
//! rules, so `.gitattributes` clean filters and eol conversion apply — the oid
//! it returns is **the blob git would store**, identical to `git rev-parse
//! HEAD:<path>` after a commit of the same bytes. `--no-filters` would return a
//! different id for a filtered path, and that id would never appear in the
//! object database, so every reachability answer over it would be wrong. The
//! gate proving this is `tests/plumbing.rs::blob_oid_matches_the_committed_blob_under_a_clean_filter`.
//!
//! # The three-state anchoring check
//! This crate gathers the facts; `receipt::anchor::ObjectAnchor` classifies
//! them (the same fact/classify split the origin-anchor axis already uses).
//! One check, one `rev-list`:
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

/// Whether `text` is a hex object id — sha1 (40) or sha256 (64), the two
/// spellings git mints. The guard that keeps a ref name out of `cat-file`.
#[must_use]
pub fn is_oid(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
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
