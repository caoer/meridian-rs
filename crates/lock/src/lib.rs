//! The `meridian-lock` fenced block — a machine-owned lockfile in the page
//! (decision `2026-07-24-meridian-lock-block`, "#8"; M1 plan U11, the FORMAT
//! half — the write-path integration lives in `wire-serve`).
//!
//! # Nature (#8): a lockfile
//! Cargo.lock's species: machine-written, hand-editing is a bug, humans open
//! it only when something is broken. Consequences carried here:
//!
//! - **Engine sole-writer** (#8 §3): [`render`] emits ONE canonical byte form
//!   — deterministic field order, quoting, indentation. There is no "style".
//! - **Strict reader, fail-loud**: [`parse`] accepts exactly the canonical
//!   grammar; anything else is a typed [`LockError`] naming the line — a
//!   malformed lock is corruption (or a hand edit), never something to guess
//!   through. Absence of a lock is `Ok(None)` from [`find`]; a PRESENT but
//!   broken lock is never silently treated as absent.
//! - **Lock-is-content** (#8 §5): the block sits inside the page's span, so
//!   the page's fingerprint covers its lock — that containment is what makes
//!   the transitive pin graph complete, and it is a property of WHERE the
//!   block lives, not of this crate's code (fixtured all the same).
//!
//! # Namespace (#8 §1) — and readership, which is a different question (U36)
//! The whole `meridian-*` prefix is reserved as the engine's block-language
//! namespace — [`is_meridian_lang`] is the one predicate for *"is this ours?"*,
//! uniform over the prefix, so tooling never grows an enumerated list.
//!
//! **Reservation is not readership.** *"Should a reader see it?"* is answered by
//! [`is_engine_emitted`], derived from the registered canonical writers
//! ([`EngineEmitted`], [`engine_emits!`]). The two answers come apart exactly
//! where it matters: the engine writes a `meridian-lock`, so it is elided from
//! the render face; a human writes a `meridian-mount` and `config` only PARSES
//! it, so it renders. One predicate serving both is what made a config file
//! render as a config file that declares nothing.
//!
//! # Planes (#8 §2)
//! `objects:` — whole-file blob shas, the retrieval plane (git's world, the
//! object-class rev). `pins:` — `(ref, fingerprint)` claims, the claim plane;
//! fingerprint values are full `fp1.…` CID-tokens (`docs/norm-v2-spec.md`
//! §2), carried VERBATIM by this crate — classifying them (green / red / grey
//! unverifiable) is the verify plane's job, never the parser's.
//!
//! # Scope (M1)
//! Format + reader/writer + mutators only. No lock is minted in M1 — the pin
//! BEHAVIOR (read-mint gate, atomic put+lock) is stage 2.

use model::{ByteSpan, Document, Node, NodeKind};

/// The reserved block language of the lock (#8 §1).
pub const LANG: &str = "meridian-lock";

/// The engine's reserved block-language namespace prefix (#8 §1).
pub const NAMESPACE_PREFIX: &str = "meridian-";

/// The one live root-object version.
pub const VERSION: u32 = 1;

/// Is this fence info string an ENGINE block (`meridian-*`)? The first
/// whitespace token decides — one law for elision and any future engine
/// block, never an enumerated list (#8 §1).
#[must_use]
pub fn is_meridian_lang(lang: &str) -> bool {
    lang.split_whitespace()
        .next()
        .is_some_and(|tok| tok.starts_with(NAMESPACE_PREFIX))
}

/// Is this fence info string THE lock block? First-token exact.
#[must_use]
pub fn is_lock_lang(lang: &str) -> bool {
    lang.split_whitespace().next() == Some(LANG)
}

// ── The engine-emits property: READERSHIP, per language (U36) ──────────────
//
// A namespace answers *"is this ours?"* — [`is_meridian_lang`], uniform over
// `meridian-*`, and it does not move. Elision answers a DIFFERENT question,
// *"should a reader see it?"*, and the two answers come apart exactly where it
// matters: the engine writes a `meridian-lock`, so no human authored it and the
// render face elides it as noise; a human writes a `meridian-mount`, and
// `config` only PARSES it, so the render face must show it. One predicate
// serving both questions is what made a config file render as a config file
// that declares nothing, with no signal.
//
// The property is *"does the ENGINE EMIT this block's bytes?"* — never *"does a
// crate NAME this language?"*. `config` names `meridian-mount` in a `pub const`
// and parses it; a mention-keyed predicate would elide it and reproduce the
// defect wearing derivation clothes.
//
// It is derived, never listed. `#8 §1` bars an enumerated list wherever it
// lives, because a list silently omits new members. Here the set is assembled
// by the LINKER from declarations that live beside their writers, so there is
// no list to omit from — and a declaration cannot be written without a writer,
// which is what [`EngineEmittedLang::of`]'s bound enforces.

/// A block language whose bytes THE ENGINE EMITS: the engine owns its one
/// canonical writer (#8 §3, engine sole-writer).
///
/// Implementing this is what makes a language engine-emitted, and therefore
/// render-elided. A language the engine only PARSES has no impl and is not
/// elided — the user authored those bytes on purpose.
pub trait EngineEmitted {
    /// The fence info-string first token this writer emits under.
    const LANG: &'static str;

    /// Emit the canonical block bytes. This method is the property: a type that
    /// cannot produce the bytes cannot claim the engine emits them.
    fn emit_canonical(&self) -> String;
}

/// One registered engine-emitted language — the declaration a canonical writer
/// leaves beside itself, collected by the linker.
///
/// The field is private and [`of`](Self::of) is the only constructor, so this
/// value cannot be minted for a language that has no writer. That seal is what
/// separates this from a list of names: a registration is *evidence of a
/// writer*, not an assertion about one.
#[derive(Debug)]
pub struct EngineEmittedLang {
    lang: &'static str,
}

impl EngineEmittedLang {
    /// Declare `T`'s language engine-emitted. The `T: EngineEmitted` bound is
    /// the seal — no writer, no declaration.
    #[must_use]
    pub const fn of<T: EngineEmitted>() -> Self {
        EngineEmittedLang { lang: T::LANG }
    }

    /// The declared language.
    #[must_use]
    pub const fn lang(&self) -> &'static str {
        self.lang
    }
}

inventory::collect!(EngineEmittedLang);

#[doc(hidden)]
pub use inventory;

/// Declare `$t` the engine's canonical writer for its block language, and
/// register that language as engine-emitted — ONE motion, invoked beside the
/// writer, so a new engine block's readership consequence cannot be forgotten
/// separately from the block.
///
/// ```
/// struct MyBlock;
///
/// impl lock::EngineEmitted for MyBlock {
///     const LANG: &'static str = "meridian-myblock";
///     fn emit_canonical(&self) -> String {
///         "```meridian-myblock\nversion: 1\n```".to_string()
///     }
/// }
/// lock::engine_emits!(MyBlock);
///
/// fn main() {
///     // The declaration is what makes it engine-emitted, so the render face
///     // elides it — with no elision predicate edited anywhere.
///     assert!(lock::is_engine_emitted("meridian-myblock"));
///     // A reserved language nobody writes stays visible to readers.
///     assert!(!lock::is_engine_emitted("meridian-unwritten"));
///     assert!(lock::is_meridian_lang("meridian-unwritten"));
/// }
/// ```
#[macro_export]
macro_rules! engine_emits {
    ($t:ty) => {
        $crate::inventory::submit! { $crate::EngineEmittedLang::of::<$t>() }
    };
}

impl EngineEmitted for Lock {
    const LANG: &'static str = LANG;

    fn emit_canonical(&self) -> String {
        render(self)
    }
}

engine_emits!(Lock);

/// Does the ENGINE EMIT this fence info string's block? The first whitespace
/// token decides, matching every other reader.
///
/// This is the elision law (#8 §1 as applied per-language by U36) and it is
/// **not** [`is_meridian_lang`]. Reservation is uniform; readership is not.
/// A `meridian-*` language with no registered writer is a human's bytes — a
/// declaration the engine parses, or one no reader claims yet — and a reader
/// sees it.
#[must_use]
pub fn is_engine_emitted(lang: &str) -> bool {
    let Some(tok) = lang.split_whitespace().next() else {
        return false;
    };
    inventory::iter::<EngineEmittedLang>
        .into_iter()
        .any(|e| e.lang == tok)
}

/// One claim-plane entry (#8 §2): the declared address and the full
/// fingerprint CID-token, both verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinEntry {
    /// The `ref` — the declared address, PARSED ([`addr::Addr`]).
    ///
    /// **The retype that establishes the address owner (U10).** It was a
    /// `String` sixteen sites re-split; the type makes every one of them
    /// discharge the parse or fail to compile (R5). It renders back
    /// byte-identically, so the canonical lock bytes are unmoved.
    pub declared_ref: addr::Addr,
    /// The `fingerprint` — a full `fp1.…` token (norm-v2-spec §2), verbatim.
    pub fingerprint: String,
}

/// The versioned root object (#8 §2). Field order is document order and IS
/// the canonical order — no maps, no sorting, deterministic bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    /// Root-object version; [`VERSION`] is the only live value.
    pub version: u32,
    /// Retrieval plane: `(key, blob_sha)` — whole-file blob shas, git's world.
    pub objects: Vec<(String, String)>,
    /// Claim plane: the pins.
    pub pins: Vec<PinEntry>,
}

impl Default for Lock {
    fn default() -> Self {
        Lock {
            version: VERSION,
            objects: Vec::new(),
            pins: Vec::new(),
        }
    }
}

impl Lock {
    /// An empty version-current lock.
    #[must_use]
    pub fn new() -> Self {
        Lock::default()
    }

    /// Upsert an `objects:` entry by key: replace the sha in place (position
    /// preserved), else append.
    pub fn set_object(&mut self, key: &str, blob_sha: &str) {
        match self.objects.iter_mut().find(|(k, _)| k == key) {
            Some((_, sha)) => blob_sha.clone_into(sha),
            None => self.objects.push((key.to_string(), blob_sha.to_string())),
        }
    }

    /// Upsert a pin by `declared_ref`: replace in place (position preserved),
    /// else append. The union discipline — an update never silently drops or
    /// reorders other entries.
    pub fn upsert_pin(&mut self, entry: PinEntry) {
        match self
            .pins
            .iter_mut()
            .find(|p| p.declared_ref == entry.declared_ref)
        {
            Some(p) => *p = entry,
            None => self.pins.push(entry),
        }
    }
}

/// Why a lock refused to parse. Fail-loud and line-addressed — a lockfile is
/// machine-written, so a deviation is damage to NAME, never to skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// The slice does not open with the ` ```meridian-lock ` fence.
    NotALockBlock,
    /// A version this reader does not implement — the upgrade slot working
    /// as designed (#8 §2): refuse loudly, never guess a future format.
    UnsupportedVersion { found: String },
    /// Structurally outside the canonical grammar. `line` is 1-based within
    /// the block slice.
    Malformed { line: usize, reason: &'static str },
    /// More than one `meridian-lock` block on the page — sole-writer emits
    /// exactly one; two is corruption.
    MultipleBlocks,
    /// A `ref` outside the address grammar (`docs/address-grammar.md` § 4).
    /// Carries the offending spelling and the parse refusal, so the damage is
    /// named rather than reported as "something was wrong".
    BadRef {
        /// 1-based line within the block slice.
        line: usize,
        /// The rejected spelling, verbatim.
        found: String,
        /// Why the address grammar refused it.
        reason: addr::AddrError,
    },
}

/// The refusal reason, one line — THE spelling of why a lock is unreadable, so
/// a write refusal and a read face's `grey lock-refused` row name the damage
/// identically.
impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::NotALockBlock => write!(f, "not a meridian-lock block"),
            LockError::UnsupportedVersion { found } => {
                write!(f, "unsupported lock version: {found}")
            }
            LockError::Malformed { line, reason } => {
                write!(f, "malformed at line {line}: {reason}")
            }
            LockError::MultipleBlocks => {
                write!(f, "more than one meridian-lock block on the page")
            }
            LockError::BadRef {
                line,
                found,
                reason,
            } => write!(f, "malformed ref at line {line}: \"{found}\" — {reason}"),
        }
    }
}

impl std::error::Error for LockError {}

/// A lock located in a document: the fence-to-fence byte span (the model
/// `CodeBlock` span, final terminator excluded — splice-ready) + the parsed
/// root object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub span: ByteSpan,
    pub lock: Lock,
}

/// Find THE page lock: `Ok(None)` when the page has no `meridian-lock` block;
/// `Err` when one exists but is malformed, or when more than one exists. A
/// present-but-broken lock is never reported as absent.
///
/// # Errors
/// [`LockError`] per its variants.
pub fn find(doc: &Document) -> Result<Option<Found>, LockError> {
    let mut spans: Vec<ByteSpan> = Vec::new();
    collect_lock_spans(&doc.root, &mut spans);
    match spans.as_slice() {
        [] => Ok(None),
        [span] => {
            let slice = doc.raw.get(span.clone()).ok_or(LockError::Malformed {
                line: 1,
                reason: "lock span out of bounds",
            })?;
            Ok(Some(Found {
                span: span.clone(),
                lock: parse(slice)?,
            }))
        }
        _ => Err(LockError::MultipleBlocks),
    }
}

/// Every `meridian-lock` block's RAW bytes in document order — the lock
/// ARTIFACT as it sits on disk, with no parse in the way.
///
/// [`find`] answers "what does the lock SAY", which needs the grammar and
/// therefore refuses corruption. This answers "which bytes ARE the lock", which
/// a guard comparing two documents needs to work even when one of them is
/// malformed or carries two blocks: a diff over parsed values could not see a
/// change from one broken block to a different broken block, and byte-identity
/// is the property the artifact guard states (advisor R25 — guard the artifact,
/// not the verb).
#[must_use]
pub fn block_texts(doc: &Document) -> Vec<&str> {
    let mut spans: Vec<ByteSpan> = Vec::new();
    collect_lock_spans(&doc.root, &mut spans);
    spans.sort_by_key(|s| s.start);
    spans
        .into_iter()
        .filter_map(|span| doc.raw.get(span))
        .collect()
}

fn collect_lock_spans(node: &Node, out: &mut Vec<ByteSpan>) {
    if let NodeKind::CodeBlock { lang, .. } = &node.kind
        && is_lock_lang(lang)
    {
        out.push(node.span.clone());
    }
    for child in &node.children {
        collect_lock_spans(child, out);
    }
}

/// Parse a fence-to-fence lock slice (what [`find`] hands over, or what
/// [`render`] emitted). Strict canonical grammar only.
///
/// # Errors
/// [`LockError`] — see [`find`].
pub fn parse(slice: &str) -> Result<Lock, LockError> {
    let mut lines = slice.lines().enumerate();

    let Some((_, first)) = lines.next() else {
        return Err(LockError::NotALockBlock);
    };
    if !first.starts_with("```") || !is_lock_lang(&first[3..]) {
        return Err(LockError::NotALockBlock);
    }

    let mut body: Vec<(usize, &str)> = lines.map(|(i, l)| (i + 1, l)).collect();
    match body.pop() {
        Some((_, close)) if close.trim_end() == "```" => {}
        Some((n, _)) => {
            return Err(LockError::Malformed {
                line: n,
                reason: "missing closing fence",
            });
        }
        None => {
            return Err(LockError::Malformed {
                line: 1,
                reason: "missing closing fence",
            });
        }
    }

    let mut it = body.into_iter().peekable();

    // version — required first body line.
    let Some((vline, vtext)) = it.next() else {
        return Err(LockError::Malformed {
            line: 1,
            reason: "empty lock body (version required)",
        });
    };
    let Some(vraw) = vtext.strip_prefix("version: ") else {
        return Err(LockError::Malformed {
            line: vline,
            reason: "first body line must be `version: N`",
        });
    };
    let version: u32 = vraw.trim().parse().map_err(|_| LockError::Malformed {
        line: vline,
        reason: "version is not an integer",
    })?;
    if version != VERSION {
        return Err(LockError::UnsupportedVersion {
            found: vraw.trim().to_string(),
        });
    }

    let mut lock = Lock::new();

    if it.peek().is_some_and(|(_, l)| *l == "objects:") {
        it.next();
        parse_objects(&mut it, &mut lock.objects)?;
    }
    if it.peek().is_some_and(|(_, l)| *l == "pins:") {
        it.next();
        parse_pins(&mut it, &mut lock.pins)?;
    }

    if let Some((n, _)) = it.next() {
        return Err(LockError::Malformed {
            line: n,
            reason: "unrecognized line (canonical order: version, objects, pins)",
        });
    }
    Ok(lock)
}

/// Body-line cursor: `(1-based line, text)` pairs of a lock slice.
type BodyIter<'a> = std::iter::Peekable<std::vec::IntoIter<(usize, &'a str)>>;

/// The `objects:` plane — `  "key": "sha"` lines, before `pins:`.
fn parse_objects(it: &mut BodyIter<'_>, out: &mut Vec<(String, String)>) -> Result<(), LockError> {
    while let Some((_, l)) = it.peek() {
        if !l.starts_with("  \"") {
            break;
        }
        let (n, l) = it.next().unwrap_or((0, ""));
        let (key, rest) = read_quoted(&l[2..], n)?;
        let rest = rest.strip_prefix(": ").ok_or(LockError::Malformed {
            line: n,
            reason: "objects entry needs `\"key\": \"sha\"`",
        })?;
        let (sha, tail) = read_quoted(rest, n)?;
        if !tail.is_empty() {
            return Err(LockError::Malformed {
                line: n,
                reason: "trailing bytes after objects entry",
            });
        }
        out.push((key, sha));
    }
    Ok(())
}

/// The `pins:` plane — two-line `- ref:` / `fingerprint:` entries, last.
fn parse_pins(it: &mut BodyIter<'_>, out: &mut Vec<PinEntry>) -> Result<(), LockError> {
    while let Some((_, l)) = it.peek() {
        if !l.starts_with("  - ref: ") {
            break;
        }
        let (n, l) = it.next().unwrap_or((0, ""));
        let (declared_ref, tail) = read_quoted(&l["  - ref: ".len()..], n)?;
        if !tail.is_empty() {
            return Err(LockError::Malformed {
                line: n,
                reason: "trailing bytes after ref",
            });
        }
        // THE door the retype opened: a lock is machine-written, so a `ref`
        // outside the address grammar is damage to NAME, never to carry. The
        // string convention could only hand it on.
        let declared_ref = addr::Addr::parse(&declared_ref).map_err(|err| LockError::BadRef {
            line: n,
            found: declared_ref.clone(),
            reason: err,
        })?;
        let Some((fn_line, fl)) = it.next() else {
            return Err(LockError::Malformed {
                line: n,
                reason: "pin entry missing its fingerprint line",
            });
        };
        let Some(frest) = fl.strip_prefix("    fingerprint: ") else {
            return Err(LockError::Malformed {
                line: fn_line,
                reason: "pin continuation must be `    fingerprint: \"…\"`",
            });
        };
        let (fingerprint, ftail) = read_quoted(frest, fn_line)?;
        if !ftail.is_empty() {
            return Err(LockError::Malformed {
                line: fn_line,
                reason: "trailing bytes after fingerprint",
            });
        }
        out.push(PinEntry {
            declared_ref,
            fingerprint,
        });
    }
    Ok(())
}

/// Render the canonical fence-to-fence bytes (no trailing newline — the
/// splice caller owns surrounding terminators). Deterministic: one byte form
/// per [`Lock`] value; empty planes are omitted entirely.
#[must_use]
pub fn render(lock: &Lock) -> String {
    let mut out = String::from("```");
    out.push_str(LANG);
    out.push('\n');
    out.push_str("version: ");
    out.push_str(&lock.version.to_string());
    out.push('\n');
    if !lock.objects.is_empty() {
        out.push_str("objects:\n");
        for (key, sha) in &lock.objects {
            out.push_str("  ");
            push_quoted(&mut out, key);
            out.push_str(": ");
            push_quoted(&mut out, sha);
            out.push('\n');
        }
    }
    if !lock.pins.is_empty() {
        out.push_str("pins:\n");
        for pin in &lock.pins {
            out.push_str("  - ref: ");
            push_quoted(&mut out, &pin.declared_ref.to_string());
            out.push('\n');
            out.push_str("    fingerprint: ");
            push_quoted(&mut out, &pin.fingerprint);
            out.push('\n');
        }
    }
    out.push_str("```");
    out
}

/// Append `"…"` with the three canonical escapes: `\\`, `\"`, and `\n` as
/// `\n`-escape — every value stays one line, and round-trips exactly.
fn push_quoted(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

/// Read a leading `"…"` scalar off `s`; return `(unescaped, rest)`.
fn read_quoted(s: &str, line: usize) -> Result<(String, &str), LockError> {
    let inner = s.strip_prefix('"').ok_or(LockError::Malformed {
        line,
        reason: "expected a double-quoted scalar",
    })?;
    let mut value = String::new();
    let mut chars = inner.char_indices();
    while let Some((i, ch)) = chars.next() {
        match ch {
            '"' => return Ok((value, &inner[i + 1..])),
            '\\' => match chars.next() {
                Some((_, '\\')) => value.push('\\'),
                Some((_, '"')) => value.push('"'),
                Some((_, 'n')) => value.push('\n'),
                _ => {
                    return Err(LockError::Malformed {
                        line,
                        reason: "unknown escape in quoted scalar",
                    });
                }
            },
            _ => value.push(ch),
        }
    }
    Err(LockError::Malformed {
        line,
        reason: "unterminated quoted scalar",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    /// A fixture address. Every fixture spelling in this module is a legal
    /// address, so a panic here is a fixture bug, never a grammar surprise.
    fn a(spelling: &str) -> addr::Addr {
        addr::Addr::parse(spelling).expect("a fixture ref is a well-formed address")
    }

    fn sample() -> Lock {
        let mut l = Lock::new();
        l.set_object("sessions:24-01-retro/notes.md", "9ae3f1deadbeef");
        l.upsert_pin(PinEntry {
            declared_ref: a("sessions:24-01-retro/notes.md#Design"),
            fingerprint: format!("fp1.span2.b3.{}", "ab".repeat(32)),
        });
        l
    }

    /// The canonical byte form, pinned literal — the #8 §2 shape.
    #[test]
    fn render_canonical_bytes() {
        let expected = format!(
            "```meridian-lock\n\
             version: 1\n\
             objects:\n\
             \x20 \"sessions:24-01-retro/notes.md\": \"9ae3f1deadbeef\"\n\
             pins:\n\
             \x20 - ref: \"sessions:24-01-retro/notes.md#Design\"\n\
             \x20   fingerprint: \"fp1.span2.b3.{}\"\n\
             ```",
            "ab".repeat(32)
        );
        assert_eq!(render(&sample()), expected);
    }

    /// Sole-writer determinism: value → bytes → value → bytes is a fixpoint.
    #[test]
    fn round_trip_is_byte_stable() {
        let l = sample();
        let bytes = render(&l);
        let back = parse(&bytes).expect("canonical bytes parse");
        assert_eq!(back, l);
        assert_eq!(render(&back), bytes, "render is deterministic");

        // Empty planes: version-only lock.
        let empty = Lock::new();
        let b = render(&empty);
        assert_eq!(b, "```meridian-lock\nversion: 1\n```");
        assert_eq!(parse(&b).expect("empty lock parses"), empty);
    }

    /// [`find`]: absent = `Ok(None)`; present = parsed with the `CodeBlock`
    /// span; a present-but-broken lock is an ERROR, never "absent".
    #[test]
    fn find_absent_present_broken() {
        assert_eq!(find(&doc("# A\n\nplain page\n")), Ok(None));

        let block = render(&sample());
        let raw = format!("# A\n\nintro\n\n{block}\n");
        let d = doc(&raw);
        let found = find(&d).expect("ok").expect("present");
        assert_eq!(&raw[found.span.clone()], block, "span is fence-to-fence");
        assert_eq!(found.lock, sample());

        let broken = "# A\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n";
        let err = find(&doc(broken)).expect_err("broken lock must refuse");
        assert!(matches!(err, LockError::Malformed { .. }), "{err:?}");

        let two = format!("# A\n\n{block}\n\n{block}\n");
        assert_eq!(find(&doc(&two)), Err(LockError::MultipleBlocks));
    }

    /// The upgrade slot refuses loudly (#8 §2).
    #[test]
    fn unsupported_version_refuses() {
        let err = parse("```meridian-lock\nversion: 2\n```").expect_err("v2 refused");
        assert_eq!(
            err,
            LockError::UnsupportedVersion {
                found: "2".to_string()
            }
        );
    }

    /// Every structural deviation is a line-addressed refusal.
    #[test]
    fn malformed_cases_name_their_line() {
        // Lines are 1-based over the SLICE — the opening fence is line 1.
        for (slice, want_line) in [
            ("```yaml\nversion: 1\n```", 0),          // not a lock (line unused)
            ("```meridian-lock\n```", 1),             // no version
            ("```meridian-lock\nversion: x\n```", 2), // non-integer
            (
                "```meridian-lock\nversion: 1\npins:\n  - ref: \"a\"\n```",
                4,
            ), // pin without fingerprint
            (
                "```meridian-lock\nversion: 1\nobjects:\n  \"k\": bare\n```",
                4,
            ), // unquoted sha
            ("```meridian-lock\nversion: 1\nwhat\n```", 3), // unrecognized line
            (
                "```meridian-lock\nversion: 1\nobjects:\n  \"k\": \"v\" x\n```",
                4,
            ), // trailing bytes
            ("```meridian-lock\nversion: 1", 2),      // no closing fence
        ] {
            let err = parse(slice).expect_err(slice);
            match err {
                LockError::NotALockBlock => assert_eq!(want_line, 0, "{slice}"),
                LockError::Malformed { line, .. } => assert_eq!(line, want_line, "{slice}"),
                other => panic!("{slice}: unexpected {other:?}"),
            }
        }
    }

    /// U10 — the door the `declared_ref` retype opened. A lock is
    /// machine-written, so a `ref` outside the address grammar is damage to
    /// NAME (`LockError::BadRef`), never a string to carry on.
    ///
    /// **The acceptance half is asserted in the same breath** (S3-R8(c)): the
    /// rooted, colon-bearing ref that this crate has always round-tripped must
    /// still parse and still render byte-identically. A guard proven only by
    /// what it blocks is indistinguishable from one that blocks everything —
    /// and here that would mean refusing every cross-root pin ever written.
    #[test]
    fn a_ref_outside_the_address_grammar_is_named_damage_not_carried() {
        let block = |r#ref: &str| {
            format!(
                "```meridian-lock\nversion: 1\npins:\n  - ref: \"{ref}\"\n    \
                 fingerprint: \"fp1.span2.b3.00\"\n```"
            )
        };

        // ACCEPTANCE — the cross-root spelling this crate is honestly
        // prefix-transparent about (FINDING 02, "where the claim IS true").
        let ok =
            parse(&block("sessions:24-01-retro/notes.md#Design")).expect("a rooted ref parses");
        assert_eq!(
            ok.pins[0].declared_ref.root().map(addr::MountName::as_str),
            Some("sessions"),
            "the root is now STRUCTURAL, not glued to the path",
        );
        assert_eq!(
            render(&ok),
            block("sessions:24-01-retro/notes.md#Design"),
            "the canonical bytes are unmoved by the retype",
        );
        assert!(
            parse(&block("plain/page.md#Design")).is_ok(),
            "the ambient majority case is untouched",
        );

        // REFUSAL — each class named, never conflated with `Malformed`.
        for (r#ref, want) in [
            (
                "Sessions:notes.md",
                addr::AddrError::BadMountName {
                    found: "Sessions".to_string(),
                },
            ),
            (":notes.md", addr::AddrError::EmptyMountName),
            (
                "a:b:c.md",
                addr::AddrError::AmbiguousColon {
                    found: "a:b:c.md".to_string(),
                },
            ),
        ] {
            match parse(&block(r#ref)).expect_err(r#ref) {
                LockError::BadRef { found, reason, .. } => {
                    assert_eq!(found, r#ref, "the refusal names what it refused");
                    assert_eq!(reason, want, "{ref} — the class is carried, not flattened");
                }
                other => panic!("{ref}: expected BadRef, got {other:?}"),
            }
        }
    }

    /// Escapes round-trip: quotes, backslashes, and newlines in values.
    #[test]
    fn escaping_round_trips() {
        let mut l = Lock::new();
        l.set_object("we\"ird\\key\nline", "sha");
        l.upsert_pin(PinEntry {
            declared_ref: a("a\"b"),
            fingerprint: "fp1.span2.b3.00".to_string(),
        });
        let bytes = render(&l);
        assert_eq!(parse(&bytes).expect("parses"), l);
        // One line per value even with embedded newlines: fence + version +
        // objects: + 1 entry + pins: + ref + fingerprint + fence = 8.
        assert_eq!(bytes.lines().count(), 8);
    }

    /// Mutators upsert in place — position preserved, nothing dropped.
    #[test]
    fn mutators_upsert_in_place() {
        let mut l = sample();
        l.upsert_pin(PinEntry {
            declared_ref: a("other#Ref"),
            fingerprint: "fp1.span2.b3.11".to_string(),
        });
        assert_eq!(l.pins.len(), 2);
        // Update the FIRST pin: position stays, value changes.
        l.upsert_pin(PinEntry {
            declared_ref: a("sessions:24-01-retro/notes.md#Design"),
            fingerprint: "fp1.span2.b3.22".to_string(),
        });
        assert_eq!(l.pins.len(), 2);
        assert_eq!(l.pins[0].fingerprint, "fp1.span2.b3.22");
        assert_eq!(l.pins[1].declared_ref.to_string(), "other#Ref");

        l.set_object("sessions:24-01-retro/notes.md", "newsha");
        assert_eq!(
            l.objects,
            vec![(
                "sessions:24-01-retro/notes.md".to_string(),
                "newsha".to_string()
            )]
        );
    }

    /// LOCK-IS-CONTENT (#8 §5): the block sits inside the page span, so the
    /// PAGE fingerprint covers the lock — updating a pinned fingerprint moves
    /// the page's own fingerprint (transitive drift by construction). Also:
    /// the lock rides inside a FENCE, so nothing in it can ever scan as an
    /// anchor (norm-v2 masks fenced code) — lock bytes are hash-domain-stable.
    #[test]
    fn lock_is_content_moves_page_fingerprint() {
        let mut l = sample();
        let page_v1 = format!("# Page\n\nbody\n\n{}\n", render(&l));
        l.upsert_pin(PinEntry {
            declared_ref: a("sessions:24-01-retro/notes.md#Design"),
            fingerprint: format!("fp1.span2.b3.{}", "cd".repeat(32)),
        });
        let page_v2 = format!("# Page\n\nbody\n\n{}\n", render(&l));

        let (d1, d2) = (doc(&page_v1), doc(&page_v2));
        assert_ne!(
            model::fingerprint::fingerprint(&d1, &d1.root),
            model::fingerprint::fingerprint(&d2, &d2.root),
            "a lock update must move the page fingerprint"
        );
        // And the pinned tokens themselves parse as fingerprints.
        let found = find(&d1).expect("ok").expect("present");
        for pin in &found.lock.pins {
            assert!(
                model::fingerprint::parse_fingerprint(&pin.fingerprint).is_some(),
                "lock fingerprints are CID-tokens"
            );
        }
    }

    /// The engine-emits property (U36): READERSHIP, derived from the registered
    /// canonical writers — not from the namespace, and not from a list of names.
    ///
    /// The two predicates are asserted against each other here, because the
    /// whole point is that they DISAGREE for a reserved language the engine does
    /// not write. `meridian-journal` is the live example: reserved, and — as of
    /// this commit, measured — emitted by nothing in `src/`.
    #[test]
    fn engine_emits_is_not_the_namespace() {
        // Emitted: `Lock` implements `EngineEmitted` and declares itself.
        assert!(is_engine_emitted(LANG));
        assert!(is_engine_emitted("meridian-lock trailing-token"));
        assert_eq!(<Lock as EngineEmitted>::LANG, LANG);
        // The impl really emits — the property is the writer, not the name.
        let l = Lock::new();
        assert_eq!(l.emit_canonical(), render(&l));

        // Reserved but NOT emitted: no writer claims these, so a reader sees them.
        for lang in ["meridian-mount", "meridian-tool", "meridian-journal"] {
            assert!(is_meridian_lang(lang), "`{lang}` is reserved");
            assert!(
                !is_engine_emitted(lang),
                "`{lang}` has no canonical writer, so the engine does not emit it"
            );
        }

        // Outside the namespace: neither predicate fires (the acceptance half —
        // without it, "not emitted" is satisfied by a predicate returning false
        // for everything).
        for lang in ["rust", "yaml", "meridian", ""] {
            assert!(!is_meridian_lang(lang));
            assert!(!is_engine_emitted(lang));
        }
    }

    /// The namespace law (#8 §1): one predicate for every engine block. This is
    /// RESERVATION and it did not move — U36 changed readership only.
    #[test]
    fn namespace_predicate() {
        assert!(is_meridian_lang("meridian-lock"));
        assert!(is_meridian_lang("meridian-journal"));
        assert!(is_meridian_lang("meridian-lock extra-token"));
        assert!(!is_meridian_lang("meridian"));
        assert!(!is_meridian_lang("yaml ^inputs"));
        assert!(!is_meridian_lang(""));
        assert!(is_lock_lang("meridian-lock"));
        assert!(!is_lock_lang("meridian-journal"));
    }
}
