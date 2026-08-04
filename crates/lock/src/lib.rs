//! The `meridian-lock` fenced block — a machine-owned lockfile in the page
//! (decision `2026-07-24-meridian-lock-block`, "#8"; schema **R4**, settled in
//! session `86449b4e` 08-01 including the 17:20 ruling that removed the
//! top-level `objects:` table).
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
//! # The R4 schema (v2) — pins only
//! A lock is a version plus a list of pins. There is no retrieval-plane table:
//! the blob hash rides the pin row that needs it, so a hash can never outlive
//! the claim it was written for.
//!
//! ```meridian-lock
//! version: 2
//! pins:
//!   - object: "[[notes]]"
//!     hash: "13c3550f41b5796dd0"
//!     path: ["Scratch notes", "Findings"]
//!     fingerprint: "fp1.span2.b3.a8222f5a"
//! ```
//!
//! - **`object`** — a wiki link, resolved EXACTLY as Obsidian resolves it. The
//!   inner text is carried verbatim; resolution is the walk plane's job.
//! - **`hash`** — the git blob hash of the target file, NEVER omitted ("if
//!   hash is missing, we lost the explicit target meaning").
//! - **`path` XOR `properties`** ([`Selector`]) — the body arm or the
//!   frontmatter arm, never both, never neither. Both are **arrays only, no
//!   string form** ("machine lock file. convenient buys nothing but chaos"),
//!   and `[]` means *all* on either arm — the whole body without frontmatter,
//!   or every frontmatter key.
//! - **`fingerprint`** — a full `fp1.…` CID-token (`docs/norm-v2-spec.md` §2),
//!   carried VERBATIM by this crate. Classifying it (green / red / grey
//!   unverifiable) is the verify plane's job, never the parser's.
//! - **Free-form extra keys** (`claim:`, and any unknown legacy key) are
//!   allowed on a pin row and **engine-ignored**, carried verbatim (ZT 08-03:
//!   *"user can or can not use claim, its free to use anything"*).
//!
//! Settled semantics riding the schema: **latest only** (no `was` field —
//! history recovery rides git blobs), **self-lock is legal**, and `inputs` is
//! dead as vocabulary AND as a storage key (R1.3).
//!
//! # The three-states rule (R4 18a.2)
//! `absent` · `null` · `empty` are three different facts about a property and
//! **fingerprint differently**. The mechanism lives in `model::fingerprint`
//! (canonical keyed map, sorted keys, length-prefixed) because the
//! `Fingerprint` seal and the one blake3 hasher live there; this crate owns
//! the SELECTOR half — [`property_fingerprint`] refuses a duplicate selector
//! key rather than deduping it.
//!
//! # v1 is not readable here (plan decision P4)
//! [`VERSION`] is `2` and this crate reads v2 ONLY: a v1 file is
//! [`LockError::UnsupportedVersion`], naming the file. That is fail-loud BY
//! DESIGN — *refuse loudly, never guess*. The v1 parser lives in the migration
//! tool, quarantined, so no reader can drift back into reinterpreting an old
//! shape as a new one.
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

use model::{ByteSpan, Document, Node, NodeKind, YamlMap};
use std::collections::BTreeMap;

/// The reserved block language of the lock (#8 §1).
pub const LANG: &str = "meridian-lock";

/// The engine's reserved block-language namespace prefix (#8 §1).
pub const NAMESPACE_PREFIX: &str = "meridian-";

/// The one live root-object version — **R4's v2 shape**.
///
/// The bump from 1 is the `objects:` removal: a breaking shape change bumps the
/// schema version, and the field is schema-only, never a tool version. There is
/// no v1 read path here (P4).
pub const VERSION: u32 = 2;

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

// ── R4 §1: the selector — `path` XOR `properties` ──────────────────────────

/// Which arm of the target a pin claims: body path segments, or frontmatter
/// property keys. **Exactly one** — R4's `path` XOR `properties`.
///
/// The XOR is a TYPE here, not a validation rule, so a pin carrying both or
/// neither is unrepresentable rather than merely rejected. That is the same
/// shape as the historical `declared_ref: String → addr::Addr` retype: move the
/// discipline into the type and every construction site discharges it or fails
/// to compile.
///
/// Both arms are **arrays and only arrays** — R4 admits no string spelling on
/// either, because a machine surface gains nothing from a convenient form and
/// inherits every escaping question it brings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// The body arm: heading/anchor segments, root → leaf. `[]` is the whole
    /// body WITHOUT frontmatter.
    Path(Vec<String>),
    /// The frontmatter arm: property keys, a tag union. `[]` is ALL keys —
    /// symmetric with [`Selector::Path`]'s `[]`.
    ///
    /// Duplicate keys are refused, never deduped ([`property_fingerprint`]).
    Properties(Vec<String>),
}

impl Selector {
    /// The `path` field key.
    pub const PATH_KEY: &'static str = "path";
    /// The `properties` field key.
    pub const PROPERTIES_KEY: &'static str = "properties";

    /// The canonical field key this arm renders under.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Selector::Path(_) => Self::PATH_KEY,
            Selector::Properties(_) => Self::PROPERTIES_KEY,
        }
    }

    /// The array elements, whichever arm this is.
    #[must_use]
    pub fn elements(&self) -> &[String] {
        match self {
            Selector::Path(v) | Selector::Properties(v) => v,
        }
    }

    /// Is this the `[]` form — *all* of this arm?
    ///
    /// A pin pair of `Path([])` + `Properties([])` on one object is the
    /// **whole-file lock**: body and frontmatter, two co-existing rows. R4 has
    /// no single "whole file" spelling, and does not need one.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.elements().is_empty()
    }

    /// The first duplicate key, if this is a `properties` selector carrying one.
    ///
    /// `path` arrays may legitimately repeat — `["Design", "Design"]` is a
    /// section named `Design` nested under one — so the rule is the frontmatter
    /// arm's alone.
    #[must_use]
    pub fn duplicate_property(&self) -> Option<&str> {
        let Selector::Properties(keys) = self else {
            return None;
        };
        keys.iter().enumerate().find_map(|(i, k)| {
            keys[..i]
                .iter()
                .any(|earlier| earlier == k)
                .then_some(k.as_str())
        })
    }
}

/// One pin — the whole claim plane of R4. There is no second plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinEntry {
    /// The `object` — a wiki link's INNER text, verbatim (`notes` for
    /// `[[notes]]`).
    ///
    /// Carried, never resolved: R4 requires resolution to match Obsidian's own
    /// EXACTLY — *"100% match or it is a critical trust failure"* — and that is
    /// the walk plane's index, not a parser's guess. Storing the inner text
    /// verbatim is what lets this crate round-trip a link it cannot resolve.
    pub object: String,
    /// The `hash` — the git blob hash of the target FILE, verbatim.
    ///
    /// Never omitted: *"if hash is missing, we lost the explicit target
    /// meaning"*. It rides the pin row rather than a shared table precisely so
    /// it cannot outlive the claim it was written for.
    pub hash: String,
    /// `path` XOR `properties` — see [`Selector`].
    pub selector: Selector,
    /// The `fingerprint` — a full `fp1.…` token (norm-v2-spec §2), verbatim.
    pub fingerprint: String,
    /// Free-form extra keys, **engine-ignored and carried verbatim**: the
    /// value is the raw text after `: `, untouched.
    ///
    /// This is the `claim:` slot and every unknown legacy key at once. A
    /// `BTreeMap` because the canonical byte form admits no ordering freedom
    /// and duplicate keys are not representable — the same two properties the
    /// reader would otherwise have to enforce by hand.
    pub extra: BTreeMap<String, String>,
}

impl PinEntry {
    /// The reserved field keys — an extra key may not shadow one.
    pub const RESERVED_KEYS: [&'static str; 5] = [
        "object",
        "hash",
        Selector::PATH_KEY,
        Selector::PROPERTIES_KEY,
        "fingerprint",
    ];

    /// A pin with no extra keys — the shape the engine mints.
    #[must_use]
    pub fn new(object: &str, hash: &str, selector: Selector, fingerprint: &str) -> Self {
        PinEntry {
            object: object.to_string(),
            hash: hash.to_string(),
            selector,
            fingerprint: fingerprint.to_string(),
            extra: BTreeMap::new(),
        }
    }

    /// The identity a pin is upserted by: the object AND the selector.
    ///
    /// Both, because R4's whole-file lock is *two rows on one object* — keying
    /// on the object alone would make the second row silently replace the
    /// first, which is precisely the case the plan calls out to test.
    #[must_use]
    pub fn key(&self) -> (&str, &Selector) {
        (&self.object, &self.selector)
    }
}

/// The versioned root object. Field order is document order and IS the
/// canonical order — no maps, no sorting, deterministic bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    /// Root-object version; [`VERSION`] is the only live value.
    pub version: u32,
    /// The pins — R4's only plane.
    pub pins: Vec<PinEntry>,
}

impl Default for Lock {
    fn default() -> Self {
        Lock {
            version: VERSION,
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

    /// Upsert a pin by [`PinEntry::key`]: replace in place (position
    /// preserved), else append. The union discipline — an update never silently
    /// drops or reorders other entries.
    pub fn upsert_pin(&mut self, entry: PinEntry) {
        match self.pins.iter_mut().find(|p| p.key() == entry.key()) {
            Some(p) => *p = entry,
            None => self.pins.push(entry),
        }
    }

    /// Every pin claiming `object`, in document order — one for a body or
    /// frontmatter claim, two for the whole-file lock.
    pub fn pins_for(&self, object: &str) -> impl Iterator<Item = &PinEntry> {
        self.pins.iter().filter(move |p| p.object == object)
    }
}

/// Mint the fingerprint of a `properties` selector over a page's frontmatter
/// (R4 18a.2, the three-states rule).
///
/// This crate owns the SELECTOR half of the rule and `model::fingerprint` owns
/// the hashing half — see [`model::fingerprint::properties_fingerprint`] for
/// why the mint lives there (the `Fingerprint` seal, the one hasher).
///
/// # Errors
/// - [`LockError::DuplicateSelectorKey`] — R4 refuses duplicates, never
///   dedupes: a selector naming `status` twice is a bug in whoever wrote it,
///   and silently collapsing it would fingerprint a selection nobody asked for.
/// - [`LockError::WrongSelectorArm`] — the `path` arm has no property
///   fingerprint; its bytes are the body's, which is `model`'s span plane.
pub fn property_fingerprint(
    map: &YamlMap,
    selector: &Selector,
) -> Result<model::fingerprint::Fingerprint, LockError> {
    let Selector::Properties(keys) = selector else {
        return Err(LockError::WrongSelectorArm {
            found: Selector::PATH_KEY,
        });
    };
    if let Some(dup) = selector.duplicate_property() {
        return Err(LockError::DuplicateSelectorKey {
            line: 0,
            key: dup.to_string(),
        });
    }
    Ok(model::fingerprint::properties_fingerprint(map, keys))
}

/// Why a lock refused to parse. Fail-loud and line-addressed — a lockfile is
/// machine-written, so a deviation is damage to NAME, never to skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// The slice does not open with the ` ```meridian-lock ` fence.
    NotALockBlock,
    /// A version this reader does not implement — the upgrade slot working as
    /// designed (#8 §2): refuse loudly, never guess.
    ///
    /// **This is the v1 door, and it is shut on purpose** (P4). `file` names
    /// the page when the caller knew it ([`find`] does), because "some lock
    /// somewhere is v1" is not an actionable message for an operator running a
    /// migration over a vault.
    UnsupportedVersion {
        /// The version spelling found, verbatim.
        found: String,
        /// The page the lock was read from, when known.
        file: Option<String>,
    },
    /// Structurally outside the canonical grammar. `line` is 1-based within
    /// the block slice.
    Malformed { line: usize, reason: &'static str },
    /// More than one `meridian-lock` block on the page — sole-writer emits
    /// exactly one; two is corruption.
    MultipleBlocks,
    /// An `object` value that is not a `[[wiki link]]`. R4 types the field as a
    /// wiki link, and a machine file that disagrees is damage to NAME.
    BadObject {
        /// 1-based line within the block slice.
        line: usize,
        /// The rejected spelling, verbatim.
        found: String,
    },
    /// A `properties` selector naming the same key twice — **refused, never
    /// deduped** (R4 18a.2). `line` is 0 when the refusal came from a value in
    /// memory rather than from a parse.
    DuplicateSelectorKey { line: usize, key: String },
    /// A property fingerprint was asked of a `path` selector. The arms are not
    /// interchangeable; the body arm's bytes belong to the span plane.
    WrongSelectorArm {
        /// The arm that was passed.
        found: &'static str,
    },
}

/// The refusal reason, one line — THE spelling of why a lock is unreadable, so
/// a write refusal and a read face's `grey lock-refused` row name the damage
/// identically.
impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::NotALockBlock => write!(f, "not a meridian-lock block"),
            LockError::UnsupportedVersion { found, file } => {
                write!(f, "lock file version {found} was found, but this version")?;
                write!(f, " of meridian reads version {VERSION} only")?;
                match file {
                    Some(path) => write!(f, " (in `{path}`)"),
                    None => Ok(()),
                }
            }
            LockError::Malformed { line, reason } => {
                write!(f, "malformed at line {line}: {reason}")
            }
            LockError::MultipleBlocks => {
                write!(f, "more than one meridian-lock block on the page")
            }
            LockError::BadObject { line, found } => write!(
                f,
                "malformed object at line {line}: \"{found}\" — expected a [[wiki link]]"
            ),
            LockError::DuplicateSelectorKey { line, key } => write!(
                f,
                "duplicate properties selector key at line {line}: \"{key}\" \
                 — refused, not deduped"
            ),
            LockError::WrongSelectorArm { found } => {
                write!(f, "no property fingerprint for a `{found}` selector")
            }
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
/// A refusal is enriched with the page path when the document carries one, so
/// an operator sweeping a vault is told WHICH file to fix.
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
                lock: parse(slice).map_err(|err| name_the_file(err, doc))?,
            }))
        }
        _ => Err(LockError::MultipleBlocks),
    }
}

/// Attach the document's path to a refusal that carries a file slot.
fn name_the_file(err: LockError, doc: &Document) -> LockError {
    let LockError::UnsupportedVersion { found, file: None } = err else {
        return err;
    };
    let path = match &doc.root.kind {
        NodeKind::Document { path, .. } => Some(path.clone()),
        _ => None,
    };
    LockError::UnsupportedVersion { found, file: path }
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
    block_spans(doc)
        .into_iter()
        .filter_map(|span| doc.raw.get(span))
        .collect()
}

/// Every `meridian-lock` block's fence-to-fence SPAN, in document order.
///
/// The locating half of [`block_texts`], and it parses nothing — the spans come
/// from the fence info string alone. That is the whole point: a page carrying a
/// **v1** lock cannot be located through [`find`], because `find` reads the
/// grammar and a v1 body is [`LockError::UnsupportedVersion`] BY DESIGN (P4).
/// The migration door needs the span of a block it is not allowed to
/// understand, and this is how it gets one without a v1 read path existing
/// anywhere in the engine.
///
/// **This is not a parser and must never become one.** It answers *"which bytes
/// are a lock block"*, never *"what does the lock say"* — the second question
/// has exactly one answer-giver, [`parse`], and it speaks v2 only.
#[must_use]
pub fn block_spans(doc: &Document) -> Vec<ByteSpan> {
    let mut spans: Vec<ByteSpan> = Vec::new();
    collect_lock_spans(&doc.root, &mut spans);
    spans.sort_by_key(|s| s.start);
    spans
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
            file: None,
        });
    }

    let mut lock = Lock::new();

    if it.peek().is_some_and(|(_, l)| *l == "pins:") {
        it.next();
        parse_pins(&mut it, &mut lock.pins)?;
    }

    if let Some((n, _)) = it.next() {
        return Err(LockError::Malformed {
            line: n,
            reason: "unrecognized line (canonical order: version, pins)",
        });
    }
    Ok(lock)
}

/// Body-line cursor: `(1-based line, text)` pairs of a lock slice.
type BodyIter<'a> = std::iter::Peekable<std::vec::IntoIter<(usize, &'a str)>>;

/// The pin rows. Each opens with `  - object: "[[…]]"`; its continuation lines
/// are indented four spaces and run until the next row or the end of the plane.
fn parse_pins(it: &mut BodyIter<'_>, out: &mut Vec<PinEntry>) -> Result<(), LockError> {
    const OPEN: &str = "  - object: ";
    while let Some((_, l)) = it.peek() {
        if !l.starts_with(OPEN) {
            break;
        }
        let (n, l) = it.next().unwrap_or((0, ""));
        let (raw_object, tail) = read_quoted(&l[OPEN.len()..], n)?;
        if !tail.is_empty() {
            return Err(LockError::Malformed {
                line: n,
                reason: "trailing bytes after object",
            });
        }
        let object = unwrap_wikilink(&raw_object).ok_or(LockError::BadObject {
            line: n,
            found: raw_object.clone(),
        })?;

        // Continuations, in canonical order: hash, path|properties,
        // fingerprint, then the free-form extras. `n` is the row's own line —
        // a row TRUNCATED by the closing fence has no line of its own to blame,
        // so the refusal names the row that is incomplete.
        let hash = read_field(it, "hash", n)?;
        let (selector, sel_line) = read_selector(it, n)?;
        if let Some(dup) = selector.duplicate_property() {
            return Err(LockError::DuplicateSelectorKey {
                line: sel_line,
                key: dup.to_string(),
            });
        }
        let fingerprint = read_field(it, "fingerprint", n)?;
        let extra = read_extras(it)?;

        out.push(PinEntry {
            object,
            hash,
            selector,
            fingerprint,
            extra,
        });
    }
    Ok(())
}

/// `[[target]]` → `target`. Anything else is not a wiki link.
fn unwrap_wikilink(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix("[[")?.strip_suffix("]]")?;
    (!inner.is_empty()).then(|| inner.to_string())
}

/// The refusal a pin row earns by omitting `hash`. Named because d2 §5.3's
/// carrier asserts this exact spelling — a refusal test that does not name its
/// refusal cannot tell its own case from a neighbouring one.
pub const MISSING_HASH: &str = "pin row needs `    hash: \"…\"` after its object";

/// The refusal a pin row earns by omitting `fingerprint`. See [`MISSING_HASH`].
pub const MISSING_FINGERPRINT: &str = "pin row needs `    fingerprint: \"…\"` after its selector";

/// One required `    <name>: "<value>"` continuation line. `row` is the line
/// the pin row opened on — what a truncated row is blamed on.
fn read_field(it: &mut BodyIter<'_>, name: &'static str, row: usize) -> Result<String, LockError> {
    let Some((n, line)) = it.next() else {
        return Err(LockError::Malformed {
            line: row,
            reason: "pin row ended before its required fields",
        });
    };
    let prefix = format!("    {name}: ");
    let Some(rest) = line.strip_prefix(&prefix) else {
        return Err(LockError::Malformed {
            line: n,
            reason: match name {
                "hash" => MISSING_HASH,
                _ => MISSING_FINGERPRINT,
            },
        });
    };
    let (value, tail) = read_quoted(rest, n)?;
    if !tail.is_empty() {
        return Err(LockError::Malformed {
            line: n,
            reason: "trailing bytes after a pin field",
        });
    }
    Ok(value)
}

/// The one selector line — `path` XOR `properties`, array form only.
fn read_selector(it: &mut BodyIter<'_>, row: usize) -> Result<(Selector, usize), LockError> {
    let Some((n, line)) = it.next() else {
        return Err(LockError::Malformed {
            line: row,
            reason: "pin row ended before its selector",
        });
    };
    if let Some(rest) = line.strip_prefix("    path: ") {
        return Ok((Selector::Path(read_array(rest, n)?), n));
    }
    if let Some(rest) = line.strip_prefix("    properties: ") {
        return Ok((Selector::Properties(read_array(rest, n)?), n));
    }
    Err(LockError::Malformed {
        line: n,
        reason: "pin row needs exactly one of `    path: [...]` or `    properties: [...]`",
    })
}

/// `["a", "b"]` → the elements; `[]` → empty. No string form is accepted on
/// either arm — that is R4's rule, not a convenience this reader may relax.
fn read_array(s: &str, line: usize) -> Result<Vec<String>, LockError> {
    let inner = s
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .ok_or(LockError::Malformed {
            line,
            reason: "selector must be an ARRAY — `[...]`, never a string",
        })?;
    let mut out = Vec::new();
    let mut rest = inner;
    if rest.is_empty() {
        return Ok(out);
    }
    loop {
        let (value, tail) = read_quoted(rest, line)?;
        out.push(value);
        match tail.strip_prefix(", ") {
            Some(next) => rest = next,
            None if tail.is_empty() => return Ok(out),
            None => {
                return Err(LockError::Malformed {
                    line,
                    reason: "array elements are separated by `, `",
                });
            }
        }
    }
}

/// The free-form tail of a pin row: any `    key: <raw>` line that is not a
/// reserved field. Values are carried VERBATIM — the engine ignores them, so
/// it has no business normalizing them either.
fn read_extras(it: &mut BodyIter<'_>) -> Result<BTreeMap<String, String>, LockError> {
    let mut out = BTreeMap::new();
    while let Some((n, line)) = it.peek().copied() {
        let Some(rest) = line.strip_prefix("    ") else {
            break;
        };
        let Some(colon) = rest.find(": ") else {
            break;
        };
        let key = &rest[..colon];
        if key.is_empty() || key.contains(' ') {
            break;
        }
        if PinEntry::RESERVED_KEYS.contains(&key) {
            return Err(LockError::Malformed {
                line: n,
                reason: "a reserved pin field appears out of canonical order",
            });
        }
        it.next();
        if out
            .insert(key.to_string(), rest[colon + 2..].to_string())
            .is_some()
        {
            return Err(LockError::Malformed {
                line: n,
                reason: "duplicate extra key on a pin row",
            });
        }
    }
    Ok(out)
}

/// Render the canonical fence-to-fence bytes (no trailing newline — the
/// splice caller owns surrounding terminators). Deterministic: one byte form
/// per [`Lock`] value; an empty pin plane is omitted entirely.
#[must_use]
pub fn render(lock: &Lock) -> String {
    let mut out = String::from("```");
    out.push_str(LANG);
    out.push('\n');
    out.push_str("version: ");
    out.push_str(&lock.version.to_string());
    out.push('\n');
    if !lock.pins.is_empty() {
        out.push_str("pins:\n");
        for pin in &lock.pins {
            out.push_str("  - object: ");
            push_quoted(&mut out, &format!("[[{}]]", pin.object));
            out.push('\n');
            out.push_str("    hash: ");
            push_quoted(&mut out, &pin.hash);
            out.push('\n');
            out.push_str("    ");
            out.push_str(pin.selector.key());
            out.push_str(": ");
            push_array(&mut out, pin.selector.elements());
            out.push('\n');
            out.push_str("    fingerprint: ");
            push_quoted(&mut out, &pin.fingerprint);
            out.push('\n');
            // `BTreeMap` order: sorted, so one byte form per value. The engine
            // ignores these keys, but it still owns the file's canonical form.
            for (key, value) in &pin.extra {
                out.push_str("    ");
                out.push_str(key);
                out.push_str(": ");
                out.push_str(value);
                out.push('\n');
            }
        }
    }
    out.push_str("```");
    out
}

/// Append `["a", "b"]`, or `[]`.
fn push_array(out: &mut String, elements: &[String]) {
    out.push('[');
    for (i, element) in elements.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        push_quoted(out, element);
    }
    out.push(']');
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
    use model::fingerprint::PropValue;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    fn fp(tag: &str) -> String {
        format!("fp1.span2.b3.{}", tag.repeat(32))
    }

    fn sample() -> Lock {
        let mut l = Lock::new();
        l.upsert_pin(PinEntry::new(
            "notes",
            "13c3550f41b5796dd0",
            Selector::Path(vec!["Scratch notes".into(), "Findings".into()]),
            &fp("ab"),
        ));
        l
    }

    /// Frontmatter as `model` parses it — the real input a property
    /// fingerprint is taken over, never a hand-built map.
    fn frontmatter(raw: &str) -> YamlMap {
        fn find_fm(node: &Node) -> Option<YamlMap> {
            if let NodeKind::Frontmatter { map } = &node.kind {
                return Some(map.clone());
            }
            node.children.iter().find_map(find_fm)
        }
        find_fm(&doc(raw).root).expect("the fixture has frontmatter")
    }

    /// **The carrier of d2 §5.3, after the board arm that used to carry it.**
    ///
    /// "An ungated close renders grey, never green" was enforced by a board
    /// detection arm — a declared-but-unpinned edge coloured grey. R1.3 retired
    /// the `^inputs` plane that was the only way to reach it, and under R4 the
    /// state that arm described is not merely unobserved but UNREPRESENTABLE:
    /// the pin row IS the declaration, so there is no "declared without a minted
    /// pin" to detect. The law did not retire; its enforcement changed category,
    /// from board detection to SCHEMA REFUSAL.
    ///
    /// So the grammar is what holds the line, and this is the test that watches
    /// it. It is mutation-provable in BOTH directions by construction: the
    /// complete row parses, and dropping either mandatory field from that SAME
    /// row makes it refuse. If the schema ever grows an optional-fingerprint or
    /// optional-hash arm, this screams — which is exactly when the law would
    /// need re-ruling rather than silently lapsing.
    ///
    /// `hash` is mandatory because *"if hash is missing, we lost the explicit
    /// target meaning"* (R4); `fingerprint` is mandatory because it is the
    /// claim — a row without one asserts nothing and could only ever be granted
    /// a colour, never compute one.
    #[test]
    fn a_pin_row_missing_a_mandatory_field_refuses_at_parse() {
        // TWO pins, and the mutation is always applied to the FIRST — so the
        // dropped field is followed by MORE ROW, never by the end of the block.
        //
        // This is load-bearing, and it was measured rather than assumed. With a
        // ONE-pin fixture, `fingerprint` is the row's LAST field, so dropping it
        // refuses with `pin row ended before its required fields` — the generic
        // TRUNCATION rule, which is a different law from "fingerprint is
        // mandatory". The test still went red, so it looked like a proof; it was
        // resting on the wrong rule and could not have told the two apart.
        let complete = format!(
            "```meridian-lock\n\
             version: 2\n\
             pins:\n\
             \x20 - object: \"[[notes]]\"\n\
             \x20   hash: \"13c3550f41b5796dd0\"\n\
             \x20   path: [\"Findings\"]\n\
             \x20   fingerprint: \"{}\"\n\
             \x20 - object: \"[[scratch]]\"\n\
             \x20   hash: \"9f2c1a77b0e4d3a512\"\n\
             \x20   path: [\"Notes\"]\n\
             \x20   fingerprint: \"{}\"\n\
             ```",
            fp("ab"),
            fp("cd")
        );

        // The positive control: the complete block parses, and to TWO pins.
        let ok = parse(&complete).expect("the complete R4 rows are legal");
        assert_eq!(ok.pins.len(), 2, "the control block parses to two pins");

        // Drop `fingerprint` from that same row — it must refuse, not default.
        let no_fingerprint = complete
            .lines()
            .filter(|l| !l.trim_start().starts_with("fingerprint:"))
            .collect::<Vec<_>>()
            .join("\n");
        // NAMING the refusal, not just demanding one: a bare `is_err()` passes
        // when the fixture trips some OTHER refusal, which is a refusal test
        // that cannot tell its own case from a neighbouring one. The reason
        // string is what pins this to the fingerprint requirement.
        match parse(&no_fingerprint) {
            Err(LockError::Malformed { reason, .. }) => assert_eq!(
                reason, MISSING_FINGERPRINT,
                "the refusal must name the FINGERPRINT requirement, not some \
                 other parse failure that happens to also refuse: {no_fingerprint}"
            ),
            other => panic!(
                "a pin row without a fingerprint asserts nothing and must refuse, \
                 never parse to a row a reader could colour: {other:?}"
            ),
        }

        // Drop `hash` from that same row — it must refuse too.
        let no_hash = complete
            .lines()
            .filter(|l| !l.trim_start().starts_with("hash:"))
            .collect::<Vec<_>>()
            .join("\n");
        match parse(&no_hash) {
            Err(LockError::Malformed { reason, .. }) => assert_eq!(
                reason, MISSING_HASH,
                "the refusal must name the HASH requirement: {no_hash}"
            ),
            other => panic!(
                "\"if hash is missing, we lost the explicit target meaning\" — the \
                 grammar refuses rather than pinning a target it cannot name: {other:?}"
            ),
        }
    }

    /// The canonical byte form, pinned literal — R4's v2 shape.
    #[test]
    fn render_canonical_bytes() {
        let expected = format!(
            "```meridian-lock\n\
             version: 2\n\
             pins:\n\
             \x20 - object: \"[[notes]]\"\n\
             \x20   hash: \"13c3550f41b5796dd0\"\n\
             \x20   path: [\"Scratch notes\", \"Findings\"]\n\
             \x20   fingerprint: \"{}\"\n\
             ```",
            fp("ab")
        );
        assert_eq!(render(&sample()), expected);
    }

    /// Round-trip of BOTH pin arms, extras included — value → bytes → value →
    /// bytes is a fixpoint.
    #[test]
    fn round_trip_of_both_arms() {
        let mut l = sample();
        let mut props = PinEntry::new(
            "r-000042",
            "9e3a12f4",
            Selector::Properties(vec!["status".into(), "description".into()]),
            &fp("cd"),
        );
        // A free-form key rides along, engine-ignored and verbatim.
        props
            .extra
            .insert("claim".into(), "owns the verdict".into());
        l.upsert_pin(props);

        let bytes = render(&l);
        let back = parse(&bytes).expect("canonical bytes parse");
        assert_eq!(back, l, "both arms and the extra key survive");
        assert_eq!(render(&back), bytes, "render is deterministic");
        assert_eq!(
            back.pins[1].extra.get("claim").map(String::as_str),
            Some("owns the verdict"),
            "an engine-ignored key is carried VERBATIM, not dropped"
        );

        // Empty plane: version-only lock.
        let empty = Lock::new();
        let b = render(&empty);
        assert_eq!(b, "```meridian-lock\nversion: 2\n```");
        assert_eq!(parse(&b).expect("empty lock parses"), empty);
    }

    /// **The three-states rule (R4 18a.2) — the gate that matters most.**
    /// absent · null · empty are three different facts, and all three
    /// pairwise-differ. A fixed-column table would collapse the first two.
    #[test]
    fn three_states_fingerprint_differently() {
        let absent = frontmatter("---\ntitle: A\n---\n\nbody\n");
        let null = frontmatter("---\ntitle: A\nstatus:\n---\n\nbody\n");
        let empty = frontmatter("---\ntitle: A\nstatus: \"\"\n---\n\nbody\n");

        // The classifier sees three states before any hashing happens.
        assert_eq!(
            model::fingerprint::classify_property(&absent, "status"),
            PropValue::Absent
        );
        assert_eq!(
            model::fingerprint::classify_property(&null, "status"),
            PropValue::Null
        );
        assert_eq!(
            model::fingerprint::classify_property(&empty, "status"),
            PropValue::Scalar("\"\"".into()),
            "an empty string is a VALUE — neither absent nor null"
        );

        let sel = Selector::Properties(vec!["status".into()]);
        let f = |m: &YamlMap| {
            property_fingerprint(m, &sel)
                .expect("a well-formed selector mints")
                .into_string()
        };
        let (fa, fn_, fe) = (f(&absent), f(&null), f(&empty));

        assert_ne!(fa, fn_, "absent must not fingerprint as null");
        assert_ne!(fn_, fe, "null must not fingerprint as empty");
        assert_ne!(fa, fe, "absent must not fingerprint as empty");

        // PINNED GOLDENS. The inequalities above are satisfied by any three
        // distinct digests, including three produced by a serialization that
        // silently changed meaning. These literals make the canonical form
        // itself the thing under test: a change to key sorting, to the
        // length-prefix encoding, to `PROPS_DOMAIN`, or to the codec token
        // fails HERE, loudly, instead of silently re-pinning the whole corpus.
        assert_eq!(
            fa,
            "fp1.props1.b3.c36eba93c2f4218e860e9d796c4db60af76d38f1c57faddc3e3027db2ca8c7a5"
        );
        assert_eq!(
            fn_,
            "fp1.props1.b3.df4ae00cc4076665fd8b31c4e77134f29ec9aa177604970ccf51e5f356e24c35"
        );
        assert_eq!(
            fe,
            "fp1.props1.b3.fb6e32302d213b345faa57d7d9e29124def0c7fc343fae2d3864cc09e85ccd15"
        );
    }

    /// **Pinned parser behavior**: bare `status:` is NULL, and so is every
    /// other YAML null spelling. This crate hand-parses frontmatter, so the
    /// rule is ours to keep true — the assertion is what survives a swap.
    #[test]
    fn bare_key_parses_as_null() {
        for spelling in ["status:", "status: ~", "status: null", "status: NULL"] {
            let map = frontmatter(&format!("---\n{spelling}\n---\n\nbody\n"));
            assert_eq!(
                model::fingerprint::classify_property(&map, "status"),
                PropValue::Null,
                "`{spelling}` is YAML null"
            );
        }
        // And the near-misses are NOT null — a classifier that answered Null
        // for everything would pass the loop above.
        for spelling in ["status: \"\"", "status: nullish", "status: 0"] {
            let map = frontmatter(&format!("---\n{spelling}\n---\n\nbody\n"));
            assert!(
                matches!(
                    model::fingerprint::classify_property(&map, "status"),
                    PropValue::Scalar(_)
                ),
                "`{spelling}` is a value, not null"
            );
        }
    }

    /// A duplicate selector key is REFUSED, never deduped (R4) — at the mint,
    /// and at the parse.
    #[test]
    fn duplicate_selector_key_is_refused() {
        let map = frontmatter("---\nstatus: open\n---\n\nbody\n");
        let dup = Selector::Properties(vec!["status".into(), "status".into()]);
        assert_eq!(
            property_fingerprint(&map, &dup),
            Err(LockError::DuplicateSelectorKey {
                line: 0,
                key: "status".into()
            })
        );

        let block = format!(
            "```meridian-lock\nversion: 2\npins:\n  - object: \"[[a]]\"\n    \
             hash: \"h\"\n    properties: [\"status\", \"status\"]\n    \
             fingerprint: \"{}\"\n```",
            fp("ab")
        );
        assert_eq!(
            parse(&block),
            Err(LockError::DuplicateSelectorKey {
                line: 6,
                key: "status".into()
            })
        );

        // A repeated PATH segment is legal — a section nested under a section
        // of the same name. The refusal is the frontmatter arm's alone.
        let path_block = block.replace(
            "properties: [\"status\", \"status\"]",
            "path: [\"Design\", \"Design\"]",
        );
        assert!(parse(&path_block).is_ok(), "path arrays may repeat");
    }

    /// Selector ORDER does not change the fingerprint — the canonical keyed map
    /// sorts, so order-independence is by construction, not by discipline.
    #[test]
    fn selector_order_does_not_change_the_fingerprint() {
        let map = frontmatter("---\nstatus: open\ndescription: d\ntitle: T\n---\n\nbody\n");
        let one = Selector::Properties(vec!["status".into(), "description".into()]);
        let two = Selector::Properties(vec!["description".into(), "status".into()]);
        assert_eq!(
            property_fingerprint(&map, &one).expect("mints"),
            property_fingerprint(&map, &two).expect("mints")
        );
        // Acceptance half: a DIFFERENT selection really does differ, so the
        // equality above is not a fingerprint that ignores its input.
        let three = Selector::Properties(vec!["status".into(), "title".into()]);
        assert_ne!(
            property_fingerprint(&map, &one).expect("mints"),
            property_fingerprint(&map, &three).expect("mints")
        );
    }

    /// `[]` means ALL on either arm, and the two arms are symmetric about it.
    #[test]
    fn empty_array_means_all_on_both_arms() {
        assert!(Selector::Path(vec![]).is_all());
        assert!(Selector::Properties(vec![]).is_all());
        assert!(!Selector::Path(vec!["A".into()]).is_all());

        // The properties arm's `[]` really selects every key: it moves when a
        // key nobody named moves.
        let all = Selector::Properties(vec![]);
        let before = frontmatter("---\nstatus: open\ntitle: T\n---\n\nbody\n");
        let after = frontmatter("---\nstatus: open\ntitle: CHANGED\n---\n\nbody\n");
        assert_ne!(
            property_fingerprint(&before, &all).expect("mints"),
            property_fingerprint(&after, &all).expect("mints"),
            "`[]` covers every key, including ones no selector names"
        );
        // And it equals the explicit enumeration of the same keys.
        let explicit = Selector::Properties(vec!["title".into(), "status".into()]);
        assert_eq!(
            property_fingerprint(&before, &all).expect("mints"),
            property_fingerprint(&before, &explicit).expect("mints"),
        );
    }

    /// **The whole-file lock is TWO co-existing rows** — `path: []` plus
    /// `properties: []` on one object. R4 has no single "whole file" spelling,
    /// so the upsert key must be (object, selector) or the second row silently
    /// replaces the first.
    #[test]
    fn whole_file_lock_is_two_rows_on_one_object() {
        let mut l = Lock::new();
        l.upsert_pin(PinEntry::new(
            "notes",
            "blob",
            Selector::Path(vec![]),
            &fp("ab"),
        ));
        l.upsert_pin(PinEntry::new(
            "notes",
            "blob",
            Selector::Properties(vec![]),
            &fp("cd"),
        ));
        assert_eq!(l.pins.len(), 2, "the arms co-exist, they do not overwrite");
        assert_eq!(l.pins_for("notes").count(), 2);

        let bytes = render(&l);
        assert!(bytes.contains("    path: []"));
        assert!(bytes.contains("    properties: []"));
        assert_eq!(parse(&bytes).expect("round-trips"), l);

        // Upserting the SAME arm again replaces in place — the key is the pair,
        // so this is an update, not a third row.
        l.upsert_pin(PinEntry::new(
            "notes",
            "blob2",
            Selector::Path(vec![]),
            &fp("ef"),
        ));
        assert_eq!(l.pins.len(), 2);
        assert_eq!(l.pins[0].hash, "blob2");
    }

    /// The v1 door is SHUT (P4), and the refusal names the file.
    #[test]
    fn version_one_fails_loud_naming_the_file() {
        // Bare slice: no document, so no file to name.
        let v1 = "```meridian-lock\nversion: 1\nobjects:\n  \"k\": \"sha\"\n```";
        assert_eq!(
            parse(v1),
            Err(LockError::UnsupportedVersion {
                found: "1".into(),
                file: None
            })
        );

        // Through `find`, the page IS known and the message says so.
        let page = format!("# A\n\n{v1}\n");
        let err = find(&doc(&page)).expect_err("a v1 lock must refuse");
        let LockError::UnsupportedVersion { found, file } = &err else {
            panic!("expected UnsupportedVersion, got {err:?}");
        };
        assert_eq!(found, "1");
        let named = file.as_deref().expect("find names the page");
        assert!(
            err.to_string().contains(named) && err.to_string().contains("version 1"),
            "the refusal names the file and the version: {err}"
        );

        // A FUTURE version refuses identically — the slot guesses in neither
        // direction.
        assert!(matches!(
            parse("```meridian-lock\nversion: 3\n```"),
            Err(LockError::UnsupportedVersion { .. })
        ));
    }

    /// An `object` that is not a wiki link is damage to NAME.
    #[test]
    fn object_must_be_a_wikilink() {
        let block = |obj: &str| {
            format!(
                "```meridian-lock\nversion: 2\npins:\n  - object: \"{obj}\"\n    \
                 hash: \"h\"\n    path: []\n    fingerprint: \"{}\"\n```",
                fp("ab")
            )
        };
        // ACCEPTANCE first — without it, a predicate refusing everything passes.
        let ok = parse(&block("[[sessions/notes]]")).expect("a wiki link parses");
        assert_eq!(ok.pins[0].object, "sessions/notes", "inner text, verbatim");

        for bad in ["notes", "[[notes]", "[notes]]", "[[]]"] {
            match parse(&block(bad)).expect_err(bad) {
                LockError::BadObject { found, .. } => {
                    assert_eq!(found, bad, "the refusal names what it refused");
                }
                other => panic!("{bad}: expected BadObject, got {other:?}"),
            }
        }
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

        let broken = "# A\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
        let err = find(&doc(broken)).expect_err("broken lock must refuse");
        assert!(matches!(err, LockError::Malformed { .. }), "{err:?}");

        let two = format!("# A\n\n{block}\n\n{block}\n");
        assert_eq!(find(&doc(&two)), Err(LockError::MultipleBlocks));
    }

    /// Every structural deviation is a line-addressed refusal.
    #[test]
    fn malformed_cases_name_their_line() {
        let head = "```meridian-lock\nversion: 2\npins:\n  - object: \"[[a]]\"\n";
        // Lines are 1-based over the SLICE — the opening fence is line 1.
        for (slice, want_line) in [
            ("```yaml\nversion: 2\n```".to_string(), 0), // not a lock (line unused)
            ("```meridian-lock\n```".to_string(), 1),    // no version
            ("```meridian-lock\nversion: x\n```".to_string(), 2), // non-integer
            ("```meridian-lock\nversion: 2\nwhat\n```".to_string(), 3), // unrecognized
            ("```meridian-lock\nversion: 2".to_string(), 2), // no closing fence
            // A row TRUNCATED by the closing fence is blamed on the line the
            // ROW opened on (4) — there is no later line to point at.
            (format!("{head}```"), 4), // pin row without its hash
            (format!("{head}    hash: \"h\"\n```"), 4), // no selector
            // a STRING selector — R4 admits arrays only
            (format!("{head}    hash: \"h\"\n    path: \"A/B\"\n```"), 6),
            // ragged array separator
            (
                format!("{head}    hash: \"h\"\n    path: [\"A\",\"B\"]\n```"),
                6,
            ),
            // selector present, fingerprint missing — same truncation rule
            (format!("{head}    hash: \"h\"\n    path: []\n```"), 4),
        ] {
            let err = parse(&slice).expect_err(&slice);
            match err {
                LockError::NotALockBlock => assert_eq!(want_line, 0, "{slice}"),
                LockError::Malformed { line, .. } => assert_eq!(line, want_line, "{slice}"),
                other => panic!("{slice}: unexpected {other:?}"),
            }
        }
    }

    /// Escapes round-trip: quotes, backslashes, and newlines in values — in
    /// array elements too, which is where a naive `, `-split would break.
    #[test]
    fn escaping_round_trips() {
        let mut l = Lock::new();
        l.upsert_pin(PinEntry::new(
            "we\"ird\\link",
            "sha",
            Selector::Path(vec!["a, b".into(), "c\"d".into(), "e\nf".into()]),
            &fp("ab"),
        ));
        let bytes = render(&l);
        assert_eq!(parse(&bytes).expect("parses"), l);
        // One line per field even with embedded newlines: fence + version +
        // pins: + object + hash + path + fingerprint + fence = 8.
        assert_eq!(bytes.lines().count(), 8);
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
        l.upsert_pin(PinEntry::new(
            "notes",
            "13c3550f41b5796dd0",
            Selector::Path(vec!["Scratch notes".into(), "Findings".into()]),
            &fp("cd"),
        ));
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
    /// not write.
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
        // (This fixture used to read `yaml ^inputs`. `inputs` is dead as
        // vocabulary AND as a storage key — R1.3 — so it does not survive even
        // as a test string; the property under test is the trailing token.)
        assert!(!is_meridian_lang("yaml extra-token"));
        assert!(!is_meridian_lang(""));
        assert!(is_lock_lang("meridian-lock"));
        assert!(!is_lock_lang("meridian-journal"));
    }
}
