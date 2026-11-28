//! The reachable set, priced by what changed rather than by the history
//! (`docs/status.md` § How "a commit reaches it" is asked).
//!
//! The anchoring tips are every ref except git-annex's. The set they reach is
//! cached per repository at `<git common dir>/meridian/reach-v1.bin`: same tips
//! → read as is; every old tip still reached → extend with the objects the new
//! tips add; otherwise → rebuild. One process updates at a time under an
//! exclusive lock beside the cache; an unusable cache costs speed, never the
//! answer.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::{GitFail, ReachableSet, Repo, is_oid};

/// Refs that never anchor a pinned file: git-annex's location-log branches.
/// Passed as `--exclude` ahead of `--all`, so they apply to it alone.
pub(crate) const ANNEX_REFS: [&str; 5] = [
    "refs/heads/git-annex",
    "refs/heads/synced/git-annex",
    "refs/remotes/*/git-annex",
    "refs/remotes/*/synced/git-annex",
    "refs/annex/*",
];

/// The cache file's first line; a file that does not start with it is absent.
const MAGIC: &str = "meridian-reach v1";

/// One object id as raw bytes: SHA-1 left-aligned and zero-padded, so SHA-1 and
/// SHA-256 repositories share one fixed-size, sortable type.
pub(crate) type Oid = [u8; 32];

/// `hex` (either case) as an [`Oid`], or `None` when it is not a 40- or
/// 64-digit object id.
pub(crate) fn parse_oid(hex: &[u8]) -> Option<Oid> {
    if hex.len() != 40 && hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, [hi, lo]) in hex.as_chunks::<2>().0.iter().enumerate() {
        out[i] = (nibble(*hi)? << 4) | nibble(*lo)?;
    }
    Some(out)
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// What the cache file holds: the tips it was built from, and what they reach.
struct Cached {
    tips: Vec<String>,
    oids: Vec<Oid>,
}

impl Repo {
    /// The reachable set from the anchoring tips, through the per-repository
    /// cache. See the module docs.
    pub(crate) fn reachable_cached(&self) -> Result<ReachableSet, GitFail> {
        let tips = self.anchoring_tips()?;
        if tips.is_empty() {
            return Ok(ReachableSet::default());
        }
        let Some(dir) = self.cache_dir() else {
            return Ok(ReachableSet::from_unsorted(self.walk_all(&tips, &[])?));
        };
        // The lock serializes builders: a waiter reads what the holder wrote
        // instead of walking the same history beside it. No lock, no wait.
        let _lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("reach-v1.lock"))
            .ok()
            .filter(|f| f.lock().is_ok());
        let path = dir.join("reach-v1.bin");
        let cached = read_cache(&path).ok().flatten();
        let oids = match cached {
            Some(c) if c.tips == tips => return Ok(ReachableSet { oids: c.oids }),
            Some(c) if self.covers(&tips, &c.tips) => {
                let mut oids = c.oids;
                oids.extend(self.walk_all(&tips, &c.tips)?);
                oids
            }
            _ => self.walk_all(&tips, &[])?,
        };
        let set = ReachableSet::from_unsorted(oids);
        // A failed write leaves the old file or none; the answer stands either way.
        let _ = write_cache(&dir, &path, &tips, &set.oids);
        Ok(set)
    }

    /// The anchoring tips: `git rev-parse --all` minus [`ANNEX_REFS`], sorted
    /// and deduplicated, lowercase hex.
    fn anchoring_tips(&self) -> Result<Vec<String>, GitFail> {
        let mut cmd = self.command();
        cmd.arg("rev-parse");
        for pattern in ANNEX_REFS {
            cmd.arg(format!("--exclude={pattern}"));
        }
        cmd.arg("--all");
        let stdout = self.run(cmd, "rev-parse --all")?;
        let mut tips = Vec::new();
        for line in String::from_utf8_lossy(&stdout).lines() {
            let tip = line.trim();
            if !is_oid(tip) {
                return Err(GitFail::Unexpected {
                    what: "rev-parse --all".to_owned(),
                    detail: format!("line is not an object id: {tip:?}"),
                });
            }
            tips.push(tip.to_ascii_lowercase());
        }
        tips.sort();
        tips.dedup();
        Ok(tips)
    }

    /// `<git common dir>/meridian`, created; `None` when it cannot be.
    fn cache_dir(&self) -> Option<PathBuf> {
        let dir = self.common_dir().ok()?.join("meridian");
        fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// Whether every object `old` reaches is still reached from `new`: the
    /// walk `old ^new` lists nothing. Stops at the first object; any failure
    /// (a pruned old tip) answers no, and the caller rebuilds.
    fn covers(&self, new: &[String], old: &[String]) -> bool {
        matches!(self.walk(old, new, |_| false), Ok(true))
    }

    /// Every object `git rev-list --objects <include> ^<exclude>` lists.
    fn walk_all(&self, include: &[String], exclude: &[String]) -> Result<Vec<Oid>, GitFail> {
        let mut oids = Vec::new();
        self.walk(include, exclude, |oid| {
            oids.push(oid);
            true
        })?;
        Ok(oids)
    }

    /// Stream `git rev-list --objects --stdin` over `include` and `^exclude`
    /// into `on_oid`, never holding git's whole output. `on_oid` answering
    /// `false` stops the walk: the result is then `Ok(false)`, and `Ok(true)`
    /// means the walk listed nothing.
    fn walk(
        &self,
        include: &[String],
        exclude: &[String],
        mut on_oid: impl FnMut(Oid) -> bool,
    ) -> Result<bool, GitFail> {
        const WHAT: &str = "rev-list --objects";
        let mut cmd = self.command();
        // A repository carrying a reachability bitmap answers from it; one
        // without walks as it would unasked.
        cmd.args(["rev-list", "--use-bitmap-index", "--objects", "--stdin"]);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| GitFail::Spawn {
                program: self.program.clone(),
                source,
            })?;
        let mut revs = String::new();
        for tip in include {
            revs.push_str(tip);
            revs.push('\n');
        }
        for tip in exclude {
            revs.push('^');
            revs.push_str(tip);
            revs.push('\n');
        }
        let mut pipe = child
            .stdin
            .take()
            .ok_or_else(|| unexpected(WHAT, "stdin"))?;
        let feeder = std::thread::spawn(move || pipe.write_all(revs.as_bytes()));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| unexpected(WHAT, "stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| unexpected(WHAT, "stderr"))?;
        let drain = std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = stderr.read_to_end(&mut text);
            text
        });

        let listed = match read_oids(stdout, &mut on_oid) {
            Ok(listed) => listed,
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitFail::Io {
                    what: WHAT.to_owned(),
                    source,
                });
            }
        };
        let (empty, stopped, bad_line) = match listed {
            Listed::All { empty } => (empty, false, None),
            Listed::Stopped => (false, true, None),
            Listed::Bad(token) => (false, false, Some(token)),
        };
        if stopped || bad_line.is_some() {
            let _ = child.kill();
        }
        let status = child.wait().map_err(|source| GitFail::Io {
            what: WHAT.to_owned(),
            source,
        })?;
        let _ = feeder.join();
        let stderr = drain.join().unwrap_or_default();
        if let Some(token) = bad_line {
            return Err(GitFail::Unexpected {
                what: WHAT.to_owned(),
                detail: format!("line does not start with an object id: {token:?}"),
            });
        }
        if stopped {
            return Ok(false);
        }
        if !status.success() {
            return Err(GitFail::Refused {
                what: WHAT.to_owned(),
                status: status.code(),
                stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
            });
        }
        Ok(empty)
    }
}

/// How a [`read_oids`] pass ended.
enum Listed {
    /// git's output ran out; `empty` when it listed nothing.
    All { empty: bool },
    /// The consumer answered `false`.
    Stopped,
    /// A line that does not start with an object id, escaped.
    Bad(String),
}

/// Feed the leading object id of every `rev-list --objects` line to `on_oid`.
fn read_oids(stdout: impl Read, on_oid: &mut impl FnMut(Oid) -> bool) -> io::Result<Listed> {
    let mut empty = true;
    let mut reader = BufReader::with_capacity(1 << 20, stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(Listed::All { empty });
        }
        let token = line
            .split(|b| *b == b' ' || *b == b'\n')
            .next()
            .unwrap_or_default();
        if token.is_empty() {
            continue;
        }
        let Some(oid) = parse_oid(token) else {
            return Ok(Listed::Bad(token.escape_ascii().to_string()));
        };
        empty = false;
        if !on_oid(oid) {
            return Ok(Listed::Stopped);
        }
    }
}

fn unexpected(what: &str, pipe: &str) -> GitFail {
    GitFail::Unexpected {
        what: what.to_owned(),
        detail: format!("child {pipe} was not piped"),
    }
}

/// The cache at `path`: `Ok(None)` when absent or not in this format.
///
/// Layout: the [`MAGIC`] line, one `tip <hex>` line per tip, an
/// `objects <count>` line, then `count` raw 32-byte [`Oid`]s, sorted.
fn read_cache(path: &Path) -> io::Result<Option<Cached>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim_end() != MAGIC {
        return Ok(None);
    }
    let mut tips = Vec::new();
    let count = loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let text = line.trim_end();
        if let Some(tip) = text.strip_prefix("tip ") {
            tips.push(tip.to_owned());
        } else if let Some(n) = text.strip_prefix("objects ") {
            match n.parse::<usize>() {
                Ok(n) => break n,
                Err(_) => return Ok(None),
            }
        } else {
            return Ok(None);
        }
    };
    let mut oids: Vec<Oid> = vec![[0u8; 32]; count];
    for oid in &mut oids {
        reader.read_exact(oid)?;
    }
    if !oids.is_sorted() {
        return Ok(None);
    }
    Ok(Some(Cached { tips, oids }))
}

/// Write the cache through a temporary file and a rename, so a reader sees
/// the old file or the new one, never half of either.
fn write_cache(dir: &Path, path: &Path, tips: &[String], oids: &[Oid]) -> io::Result<()> {
    let tmp = dir.join(format!("reach-v1.bin.{}.tmp", std::process::id()));
    let result = (|| {
        let mut out = io::BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        writeln!(out, "{MAGIC}")?;
        for tip in tips {
            writeln!(out, "tip {tip}")?;
        }
        writeln!(out, "objects {}", oids.len())?;
        for oid in oids {
            out.write_all(oid)?;
        }
        out.into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
