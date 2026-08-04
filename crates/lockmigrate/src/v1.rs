//! **THE QUARANTINE** — the one place the dead v1 `meridian-lock` grammar is
//! spelled in engine Rust, and it lives in a crate that deletes itself.
//!
//! `crates/lock` reads **v2 only** and answers a v1 body with
//! `LockError::UnsupportedVersion`, naming the file (plan decision P4). That is
//! fail-loud BY DESIGN: if the live reader could also interpret v1, every
//! reader in the engine would carry two shapes and one of them would eventually
//! be read as the other. So the old grammar is not in the reader — it is here,
//! behind a crate whose retirement is in U9b's definition of done.
//!
//! # The v1 shape (schema #8 §2, superseded by R4)
//!
//! ````text
//! ```meridian-lock
//! version: 1
//! objects:
//!   "sessions:notes.md": "9ae3f1deadbeef"
//! pins:
//!   - ref: "sessions:notes.md#Design/Findings"
//!     fingerprint: "fp1.span2.b3.…"
//! ```
//! ````
//!
//! Two planes. `objects:` is the retrieval plane — whole-file git blob shas,
//! keyed by the pin target. `pins:` is the claim plane — a `ref` (one agent-plane
//! address carrying the selector as a `/`-joined suffix) plus a fingerprint.
//! R4 collapsed the two: the blob hash moved ONTO the pin row so it can never
//! outlive the claim it was written for, and the `/`-joined selector became a
//! real array.
//!
//! # This reader is deliberately MORE PERMISSIVE than the historical one
//!
//! The shipped v1 parser accepted exactly two lines per pin row and refused a
//! third. This one accepts any further `    key: value` continuation and carries
//! the raw bytes. **A migration that drops a key it did not recognise is data
//! loss** — R4 allows free-form extra keys on a pin row, engine-ignored (ZT
//! 2026-08-03: *"user can or can not use claim, its free to use anything"*), so
//! an unknown legacy key has somewhere to land and no excuse to die. Refusing
//! the row instead would be equally wrong: it would strand the page.

use std::collections::BTreeMap;

/// The v1 claim-plane row opener — the one token that appears in a v1 lock
/// body and nowhere else. The quarantine gate greps for exactly this.
const OPEN: &str = "  - ref: ";

/// The version this module — and only this module — knows how to read.
pub const V1: u32 = 1;

/// One v1 pin row: the declared address, its fingerprint, and any further keys
/// carried VERBATIM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinV1 {
    /// The `ref` — the declared address, as written.
    pub declared_ref: String,
    /// The `fingerprint` — a full `fp1.…` token, verbatim.
    pub fingerprint: String,
    /// Every other `    key: value` continuation, raw text after `: `.
    pub extra: BTreeMap<String, String>,
    /// The 1-based line the row opened on, for line-addressed refusals.
    pub line: usize,
}

/// A parsed v1 lock: the retrieval plane and the claim plane.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockV1 {
    /// `objects:` — `(key, blob_sha)` in document order.
    pub objects: Vec<(String, String)>,
    /// `pins:` — the claim plane.
    pub pins: Vec<PinV1>,
}

impl LockV1 {
    /// The blob sha recorded for `key`, if the retrieval plane carries one.
    #[must_use]
    pub fn blob_of(&self, key: &str) -> Option<&str> {
        self.objects
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, sha)| sha.as_str())
    }
}

/// Why a v1 slice would not parse. Line-addressed, 1-based within the slice —
/// the same discipline the live reader keeps, because a lock is machine-written
/// and a deviation is damage to NAME.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub reason: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ParseError {}

fn err(line: usize, reason: impl Into<String>) -> ParseError {
    ParseError {
        line,
        reason: reason.into(),
    }
}

/// Read the `version:` of a fence-to-fence `meridian-lock` slice WITHOUT
/// committing to either grammar.
///
/// This is how the sweep tells a v1 page (migrate) from a v2 page (already
/// done, skip) without asking the live reader to look at bytes it refuses.
///
/// # Errors
/// [`ParseError`] when the slice is not a lock block, or carries no readable
/// `version:` line.
pub fn peek_version(slice: &str) -> Result<u32, ParseError> {
    let mut lines = slice.lines();
    let first = lines.next().ok_or_else(|| err(1, "empty slice"))?;
    if !first.starts_with("```meridian-lock") {
        return Err(err(1, "not a meridian-lock block"));
    }
    let second = lines.next().ok_or_else(|| err(1, "no body"))?;
    let raw = second
        .strip_prefix("version: ")
        .ok_or_else(|| err(2, "first body line must be `version: N`"))?;
    raw.trim()
        .parse()
        .map_err(|_| err(2, format!("version `{}` is not an integer", raw.trim())))
}

/// **Parse a v1 lock block.** The slice is fence-to-fence, exactly what
/// `lock::block_spans` locates.
///
/// # Errors
/// [`ParseError`] — including when the block is not v1 at all, which is a
/// caller bug: the sweep gates on [`peek_version`] first.
pub fn parse(slice: &str) -> Result<LockV1, ParseError> {
    let mut numbered = slice.lines().enumerate();

    let (_, first) = numbered.next().ok_or_else(|| err(1, "empty slice"))?;
    if !first.starts_with("```meridian-lock") {
        return Err(err(1, "not a meridian-lock block"));
    }

    let mut body: Vec<(usize, &str)> = numbered.map(|(i, l)| (i + 1, l)).collect();
    match body.pop() {
        Some((_, close)) if close.trim_end() == "```" => {}
        Some((n, _)) => return Err(err(n, "missing closing fence")),
        None => return Err(err(1, "missing closing fence")),
    }

    let mut it = body.into_iter().peekable();

    let (vline, vtext) = it.next().ok_or_else(|| err(1, "empty lock body"))?;
    let vraw = vtext
        .strip_prefix("version: ")
        .ok_or_else(|| err(vline, "first body line must be `version: N`"))?;
    let version: u32 = vraw
        .trim()
        .parse()
        .map_err(|_| err(vline, "version is not an integer"))?;
    if version != V1 {
        return Err(err(
            vline,
            format!("this reader parses version {V1} only, found {version}"),
        ));
    }

    let mut out = LockV1::default();

    if it.peek().is_some_and(|(_, l)| *l == "objects:") {
        it.next();
        while let Some((_, l)) = it.peek() {
            if !l.starts_with("  \"") {
                break;
            }
            let (n, l) = it.next().unwrap_or((0, ""));
            let (key, rest) = read_quoted(&l[2..], n)?;
            let rest = rest
                .strip_prefix(": ")
                .ok_or_else(|| err(n, "objects entry needs `\"key\": \"sha\"`"))?;
            let (sha, tail) = read_quoted(rest, n)?;
            if !tail.is_empty() {
                return Err(err(n, "trailing bytes after objects entry"));
            }
            out.objects.push((key, sha));
        }
    }

    if it.peek().is_some_and(|(_, l)| *l == "pins:") {
        it.next();
        while let Some((_, l)) = it.peek() {
            if !l.starts_with(OPEN) {
                break;
            }
            let (n, l) = it.next().unwrap_or((0, ""));
            let (declared_ref, tail) = read_quoted(&l[OPEN.len()..], n)?;
            if !tail.is_empty() {
                return Err(err(n, "trailing bytes after ref"));
            }
            let (fline, fl) = it
                .next()
                .ok_or_else(|| err(n, "pin row missing its fingerprint line"))?;
            let frest = fl
                .strip_prefix("    fingerprint: ")
                .ok_or_else(|| err(fline, "pin continuation must be `    fingerprint: \"…\"`"))?;
            let (fingerprint, ftail) = read_quoted(frest, fline)?;
            if !ftail.is_empty() {
                return Err(err(fline, "trailing bytes after fingerprint"));
            }
            // The permissive tail — see the module docs. Unknown keys are the
            // reason this reader exists in a migration tool rather than being a
            // copy of the shipped one.
            let mut extra = BTreeMap::new();
            while let Some((n, line)) = it.peek().copied() {
                let Some(rest) = line.strip_prefix("    ") else {
                    break;
                };
                let Some(colon) = rest.find(": ") else { break };
                let key = &rest[..colon];
                if key.is_empty() || key.contains(' ') {
                    break;
                }
                it.next();
                if extra
                    .insert(key.to_string(), rest[colon + 2..].to_string())
                    .is_some()
                {
                    return Err(err(n, format!("duplicate extra key `{key}` on a pin row")));
                }
            }
            out.pins.push(PinV1 {
                declared_ref,
                fingerprint,
                extra,
                line: n,
            });
        }
    }

    if let Some((n, l)) = it.next() {
        return Err(err(n, format!("unrecognized line: `{l}`")));
    }
    Ok(out)
}

/// Read a leading `"…"` scalar; return `(unescaped, rest)`. The three canonical
/// escapes, matching the v2 reader byte for byte — the quoting law did not
/// change between the schemas, only the field layout did.
fn read_quoted(s: &str, line: usize) -> Result<(String, &str), ParseError> {
    let inner = s
        .strip_prefix('"')
        .ok_or_else(|| err(line, "expected a double-quoted scalar"))?;
    let mut value = String::new();
    let mut chars = inner.char_indices();
    while let Some((i, ch)) = chars.next() {
        match ch {
            '"' => return Ok((value, &inner[i + 1..])),
            '\\' => match chars.next() {
                Some((_, '\\')) => value.push('\\'),
                Some((_, '"')) => value.push('"'),
                Some((_, 'n')) => value.push('\n'),
                _ => return Err(err(line, "unknown escape in quoted scalar")),
            },
            _ => value.push(ch),
        }
    }
    Err(err(line, "unterminated quoted scalar"))
}
