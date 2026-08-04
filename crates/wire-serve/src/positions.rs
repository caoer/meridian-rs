//! **The positional address grammar** (`docs/address-grammar.md` § 9) — the ONE
//! owner of *where an agent-plane address may occupy a document*, and of the two
//! translations across the agent/stored seam.
//!
//! # The transform is POSITIONAL, never a byte transform (A-1)
//!
//! `root:` is **already a live YAML frontmatter key** in the shipped preset/def
//! grammar — `crates/preset/src/lib.rs:217` reads it, fixtures carry
//! `root: SESSION.md` verbatim. A blanket transform over the token would rewrite
//! that line, corrupt the def, and — because it changes bytes inside a span —
//! **silently invalidate every pin whose fingerprint covers it.** Frontmatter is
//! not an address position and the shipped code already says so one refusal over:
//! *"frontmatter is not a claim-link position (S10/R22)"*.
//!
//! So this module identifies each address **by its position in the candidate**
//! and translates only the ones in the owned positions — the `strip_fp_candidate`
//! shape, copied structurally rather than by analogy.
//!
//! # The four positions, and the two this module touches (A-4)
//!
//! | # | Position | This module |
//! |---|---|---|
//! | 1 | a wikilink target — the `dest` of `[[…]]` | **translates** |
//! | 2 | a markdown link URL — the URL of `[label](url)` | **translates** |
//! | 3 | `meridian-lock` `ref:` values | **identity**, by ratified law |
//! | 4 | `meridian-lock` `objects:` keys | **identity**, by ratified law |
//!
//! Positions 3 and 4 stay in the canonical `root:` form —
//! *"Lock `ref:` and `objects:` keys use the canonical `root:` form (agent
//! plane), never the URI"* (`2026-07-24-cross-root-addressing.md` §2). They are
//! address positions, so U10's TYPE reaches them; they are not stored-form
//! positions, so this transform is the identity there. A worker reading "the
//! positional grammar U12's transform may touch" without A-4 would translate the
//! lock and break the ratified stored form.
//!
//! # The mask comes from the parser, never from a second reading of the bytes
//!
//! `root:page.md` inside a code span or a fenced block is a **code sample** the
//! document law leaves alone (§ 9.4 P2). Wikilink positions come from
//! `syntax::parse`, which already masks fences and inline code, so position 1 is
//! masked by construction. Position 2 has no node in the tree — `syntax::parse`
//! emits `Wikilink`/`Embed`, never an inline `[x](y)` — so its scan derives its
//! mask from the SAME parse's `Fence` / `InlineCode` / `Frontmatter` spans. One
//! parse, one masking law; a second reading of the bytes would be a second
//! answer to *"is this code?"*.
//!
//! # The canonical agent-plane spelling of a cross-root ref is the WIKILINK
//!
//! The stored form is always a markdown link (`[display](obsidian://…)`), so the
//! reverse translation must pick a spelling. It picks the wikilink, because the
//! ratified decision spells the agent plane's cross-root ref that way
//! (`2026-07-24-cross-root-addressing.md` §5: *"a cross-root stored link
//! (`[[sessions:...]]`)"*). Consequence, stated rather than discovered: a
//! **markdown link** carrying an agent-plane cross-root URL (§ 9.4 P4)
//! translates on write as U3 rules, and reads back as the canonical wikilink
//! with the same address and the same display. The ADDRESS round-trips byte for
//! byte; the surrounding markdown converges on one spelling, and
//! [`crate::positions::tests`] asserts that convergence rather than leaving it
//! to be discovered.

use std::ops::Range;

use addr::{Addr, MountSet, stored};

/// Which markdown form carried an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    /// Position 1 — `[[target]]` / `[[target|alias]]`.
    Wikilink,
    /// Position 1, transcluding — `![[target]]`. Refused: no `obsidian://` URI
    /// transcludes, so translating one would silently turn an embed into a
    /// link and change what the page SAYS.
    Embed,
    /// Position 2 — `[label](url)`.
    Markdown,
}

/// One address occupying an owned position, with the whole-link span a
/// translation replaces.
#[derive(Debug, Clone)]
pub(crate) struct Occupant {
    /// The whole link's byte span in the text scanned.
    pub span: Range<usize>,
    /// The parsed agent-plane address.
    pub addr: Addr,
    /// The display text — the wikilink's alias or the markdown link's label.
    /// `None` for a bare wikilink, whose display IS its address.
    pub display: Option<String>,
    /// Which form carried it.
    pub form: Form,
}

impl Occupant {
    /// The agent-plane spelling this occupant renders back to — the canonical
    /// wikilink (module docs).
    fn agent_plane_text(&self, address: &str) -> String {
        match &self.display {
            Some(display) if display != address => format!("[[{address}|{display}]]"),
            _ => format!("[[{address}]]"),
        }
    }
}

/// Why a translation refused. Every variant names the address it refused and
/// teaches the fix — a refusal a human cannot act on is worse than none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TranslateError {
    /// The address names a root **nothing declares here**, so there is no vault
    /// name to store. Grey's write-side twin: you cannot store a link to a
    /// place you cannot see from here.
    Unmounted { address: String, root: String },
    /// The address names a root the mount table **declares** and this machine
    /// **cannot read** — the path is absent, unreadable, or holds no corpus.
    ///
    /// A separate variant from [`TranslateError::Unmounted`] because the fix is
    /// a different action (S3-R43/S3-R50): telling a user to declare a root
    /// their config already declares prescribes a COMPLETED action, spends
    /// their trust and their time, and leaves no signal pointing at the real
    /// cause. The reason word is [`addr::PATH_UNSEEABLE_REASON_WORD`], REUSED
    /// from the one source rather than re-spelled here.
    PathUnseeable {
        address: String,
        root: String,
        path: String,
        detail: String,
    },
    /// The root is bound but carries no Obsidian vault name — a `git-folder`
    /// root. It has no `obsidian://` spelling at all.
    NoVault { address: String, root: String },
    /// The stored grammar refused ([`stored::StoredError`]).
    Stored {
        address: String,
        source: stored::StoredError,
    },
    /// A cross-root EMBED. No URI transcludes; translating would change the
    /// document's meaning rather than its spelling.
    Embed { address: String },
    /// An `@fp` decoration reached a position about to be stored. The
    /// fingerprint never appears in stored bytes or a display field — the lock
    /// is its sole store.
    Fingerprint { address: String, fp: String },
}

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslateError::Unmounted { address, root } => write!(
                f,
                "refused: '{address}' names root '{root}', which this machine does not bind — \
                 there is no vault name to store, so the link would land as bytes no reader can \
                 follow. Not a drift: nothing changed, you just cannot see from here. \
                 Fix: declare '{root}' in ~/MERIDIAN.md as a mount entry (name / path / kind); \
                 see [[address-grammar]]."
            ),
            TranslateError::PathUnseeable {
                address,
                root,
                path,
                detail,
            } => write!(
                f,
                "refused({word}): '{address}' names root '{root}', which ~/MERIDIAN.md declares at \
                 {path} — and this machine cannot read it there: {detail}. The mount entry is \
                 already correct, so there is nothing to declare. \
                 Fix: check the path, then re-run; see [[address-grammar]].",
                word = addr::PATH_UNSEEABLE_REASON_WORD,
            ),
            TranslateError::NoVault { address, root } => write!(
                f,
                "refused: '{address}' names root '{root}', which is bound as a non-vault root — \
                 it has no Obsidian vault name, and the stored form is spelled in vault names. \
                 Fix: address the file from a vault root, or give '{root}' a vault entry; \
                 see [[address-grammar]]."
            ),
            TranslateError::Stored { address, source } => {
                write!(f, "refused: '{address}' has no stored form — {source}")
            }
            TranslateError::Embed { address } => write!(
                f,
                "refused: '{address}' is a cross-root EMBED, and no `obsidian://` URI \
                 transcludes — storing it as a link would change what this page says, not how it \
                 is spelled. Fix: write it as a link `[[{address}]]`, or copy the content; \
                 see [[address-grammar]]."
            ),
            TranslateError::Fingerprint { address, fp } => write!(
                f,
                "refused: '{address}' carries the render-face decoration '@{fp}' into a position \
                 that is about to be STORED — a fingerprint never appears in stored bytes or in a \
                 display field; the lock is its sole store. \
                 Fix: write the plain address; the tone and digest are computed, never authored."
            ),
        }
    }
}

/// The masked regions of `raw`: fenced blocks, inline code and frontmatter,
/// taken from the parse rather than re-derived.
fn code_mask(nodes: &[syntax::DialectNode]) -> Vec<Range<usize>> {
    let mut mask: Vec<Range<usize>> = nodes
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                syntax::DialectKind::Fence { .. }
                    | syntax::DialectKind::InlineCode
                    | syntax::DialectKind::Frontmatter { .. }
            )
        })
        .map(|n| n.span.clone())
        .collect();
    mask.sort_by_key(|r| r.start);
    mask
}

fn masked(mask: &[Range<usize>], at: usize) -> bool {
    mask.iter().any(|r| r.start <= at && at < r.end)
}

/// Rebuild the agent-plane address spelling a wikilink node carried. The parser
/// splits the dest into `(target, heading, block)`; this puts it back exactly as
/// it was written, so [`Addr::parse`] sees the bytes the document holds.
fn wikilink_spelling(target: &str, heading: Option<&str>, block: Option<&str>) -> String {
    match (heading, block) {
        (Some(h), _) => format!("{target}#{h}"),
        (_, Some(b)) => format!("{target}#^{b}"),
        _ => target.to_string(),
    }
}

/// Every markdown link `[label](url)` in `raw`, outside the mask, as
/// `(whole span, label, url)`.
///
/// Deliberately conservative: no nested brackets in the label, no newline in
/// either part, and an image (`![…](…)`) is NOT a link position — § 9.1 names
/// four positions and an image URL is not among them. A shape this scanner
/// declines to claim is left byte-untouched, which is the safe direction: the
/// guard that follows refuses anything a translation should have reached.
fn markdown_links(raw: &str, mask: &[Range<usize>]) -> Vec<(Range<usize>, String, String)> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'[' || masked(mask, i) {
            i += 1;
            continue;
        }
        // `![…](…)` is an image, not a link position.
        if i > 0 && bytes[i - 1] == b'!' {
            i += 1;
            continue;
        }
        // `[[…]]` is a wikilink — position 1, owned by the parser.
        if bytes.get(i + 1) == Some(&b'[') {
            i += 1;
            continue;
        }
        let Some(close) = bytes[i + 1..]
            .iter()
            .position(|b| matches!(b, b']' | b'[' | b'\n'))
            .map(|p| i + 1 + p)
        else {
            i += 1;
            continue;
        };
        if bytes[close] != b']' || bytes.get(close + 1) != Some(&b'(') {
            i += 1;
            continue;
        }
        let Some(end) = bytes[close + 2..]
            .iter()
            .position(|b| matches!(b, b')' | b'\n'))
            .map(|p| close + 2 + p)
        else {
            i += 1;
            continue;
        };
        if bytes[end] != b')' {
            i += 1;
            continue;
        }
        let label = raw[i + 1..close].to_string();
        let url = raw[close + 2..end].to_string();
        out.push((i..end + 1, label, url));
        i = end + 1;
    }
    out
}

/// Every AGENT-PLANE cross-root address in an owned position of `raw`, span
/// order.
///
/// An address with no root is ambient and untouched — the overwhelming majority
/// of refs, and the acceptance half that keeps this grammar from swallowing the
/// ordinary corpus.
///
/// # The two positions do NOT ask the same question, and the reason is measured
///
/// **In position 1 a rooted spelling is unambiguously an address.** `[[x:y]]` is
/// a wikilink and nothing else, so an unbound root REFUSES there — the write
/// side of the grey rule: you cannot store a link to a place you cannot see
/// from here.
///
/// **In position 2 it is not.** Measured while building this scan:
/// `Addr::parse("https://example.com")` SUCCEEDS with root `https` and path
/// `//example.com` — `https` is a well-formed [`addr::MountName`]
/// (`[a-z0-9-]`), and the colon law has no fallback by design. So does
/// `mailto:someone@example.com`, and so does every other absolute URI. A scan
/// that refused an unbound root in a markdown URL would therefore **refuse every
/// write containing an ordinary external link** — an instrument that cries wolf
/// on the majority case, which S3-R23(1) rules is deleted by the next person it
/// inconveniences, leaving the invariant with no guard at all.
///
/// **So in position 2 the engine claims only what its mount table DECLARES.**
/// This is the same principle [`stored_occupants`] applies from the other side
/// (a URI naming a vault this machine does not bind is left verbatim), and it is
/// stated here rather than discovered: a rooted markdown URL naming a root
/// **nothing declares** is left untouched, because at that position it is
/// indistinguishable from a URI whose scheme this engine does not own.
///
/// # The predicate is DECLARED, not BOUND — and the difference was a false green
///
/// **Position 2 asked [`addr::MountSet::is_bound`] and that answered the wrong
/// question.** `is_bound` is false for `https` — a scheme nobody declares — and
/// equally false for a root `~/MERIDIAN.md` DECLARES that this machine has not
/// checked out. **The two are opposite obligations wearing one answer:** the
/// first is not ours and must be left verbatim; the second is ours, has no vault
/// name, and therefore has no stored form at all.
///
/// So a declared-but-unbound root was `continue`d — never collected, never
/// translated. And because [`crate::write::stored_form_guard`] scans with THIS
/// FUNCTION, the same skip removed it from the guard's population too: **one
/// `continue` disarmed the transform and the guard together**, and agent-plane
/// bytes reached disk at exit 0 with no signal. The ordinary laptop case, not a
/// corner: `mrd config` one command away named the root's path correctly the
/// whole time.
///
/// [`addr::MountSet::is_declared`] is the question that keeps the two apart.
/// Everything downstream already existed: an occupant on an unreachable root
/// refuses through [`stored_text`] as [`TranslateError::PathUnseeable`], naming
/// the PATH to check rather than prescribing a declaration that is already made
/// (S3-R43/S3-R50). **The vocabulary for refusing this was built before the
/// scanner could ever hand it a root** — the fix is the population, not a new
/// word.
///
/// **The two callers share this scan deliberately.** The guard exists to catch a
/// door that reaches bytes WITHOUT the translation, so its population must be
/// the translation's population — two definitions would drift, and the arm that
/// drifted would be the silent one. Sharing is the invariant; the defect was
/// that the shared predicate answered a question neither caller was asking.
pub(crate) fn agent_plane_occupants(raw: &str, mounts: &MountSet) -> Vec<Occupant> {
    let nodes = syntax::parse(raw);
    let mask = code_mask(&nodes);
    let mut out: Vec<Occupant> = Vec::new();

    for node in &nodes {
        let (target, heading, block, alias, form) = match &node.kind {
            syntax::DialectKind::Wikilink {
                target,
                heading,
                block,
                alias,
            } => (target, heading, block, alias, Form::Wikilink),
            syntax::DialectKind::Embed {
                target,
                heading,
                block,
                alias,
            } => (target, heading, block, alias, Form::Embed),
            _ => continue,
        };
        let spelling = wikilink_spelling(target, heading.as_deref(), block.as_deref());
        let Ok(addr) = Addr::parse(&spelling) else {
            continue;
        };
        if addr.root().is_none() {
            continue;
        }
        out.push(Occupant {
            span: node.span.clone(),
            addr,
            display: alias.clone(),
            form,
        });
    }

    for (span, label, url) in markdown_links(raw, &mask) {
        let Ok(addr) = Addr::parse(&url) else {
            continue;
        };
        // The DECLARED test, not merely a rooted test — see the doc comment above.
        if !addr.root().is_some_and(|root| mounts.is_declared(root)) {
            continue;
        }
        out.push(Occupant {
            span,
            addr,
            display: Some(label),
            form: Form::Markdown,
        });
    }

    out.sort_by_key(|o| o.span.start);
    out
}

/// The STORED spelling of one occupant — `[display](obsidian://…)`.
///
/// # Errors
/// [`TranslateError`], each naming the address it refused.
pub(crate) fn stored_text(
    occupant: &Occupant,
    mounts: &MountSet,
) -> Result<String, TranslateError> {
    let address = occupant.addr.target();
    if occupant.form == Form::Embed {
        return Err(TranslateError::Embed { address });
    }
    // The `@fp` decoration never reaches stored bytes OR a display field. The
    // splice door has already stripped every token this write introduces
    // (`strip_fp_candidate` runs first); this is the artifact-grain backstop for
    // any door that reaches a stored position another way.
    if let Some(fp) = occupant.addr.fp() {
        return Err(TranslateError::Fingerprint {
            address,
            fp: fp.to_string(),
        });
    }
    let root = occupant
        .addr
        .root()
        .expect("an occupant carries a root by construction");
    if !mounts.is_bound(root) {
        // TWO causes, TWO words (S3-R43): a root nobody declares, and a root
        // the file declares that this machine cannot read. The second one's
        // prescribed fix is already done, so sharing one refusal would send the
        // user to edit a config that is already right.
        return Err(match mounts.unreachable(root) {
            Some(u) => TranslateError::PathUnseeable {
                address,
                root: root.to_string(),
                path: u.path.clone(),
                detail: u.detail.clone(),
            },
            None => TranslateError::Unmounted {
                address,
                root: root.to_string(),
            },
        });
    }
    let vault = mounts
        .vault_of(root)
        .ok_or_else(|| TranslateError::NoVault {
            address: address.clone(),
            root: root.to_string(),
        })?;
    let selector = occupant
        .addr
        .has_selector()
        .then(|| occupant.addr.selector());
    let uri = stored::encode(vault, occupant.addr.path(), selector).map_err(|source| {
        TranslateError::Stored {
            address: address.clone(),
            source,
        }
    })?;
    let display = occupant.display.clone().unwrap_or_else(|| {
        // A bare wikilink's display IS its address, so the round trip can tell
        // "no alias" from "an alias that happens to read like the address".
        match selector {
            Some(sel) => format!("{address}#{sel}"),
            None => address.clone(),
        }
    });
    Ok(format!("[{display}]({uri})"))
}

/// `raw` with every agent-plane cross-root address in an owned position
/// replaced by its stored form.
///
/// # Errors
/// The FIRST refusal, so the caller reports one actionable sentence rather than
/// a list a human has to triage.
pub(crate) fn to_stored(raw: &str, mounts: &MountSet) -> Result<String, TranslateError> {
    let occupants = agent_plane_occupants(raw, mounts);
    if occupants.is_empty() {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    for occupant in &occupants {
        let text = stored_text(occupant, mounts)?;
        out.push_str(&raw[cursor..occupant.span.start]);
        out.push_str(&text);
        cursor = occupant.span.end;
    }
    out.push_str(&raw[cursor..]);
    Ok(out)
}

/// Every STORED cross-root form in `raw` this machine governs, with the
/// agent-plane text it reads back to — span order.
///
/// A URI naming a vault this machine does not bind is **left verbatim**: it is
/// an ordinary link a human wrote to a vault meridian does not govern, and
/// claiming it would make every such page unreadable. A URI naming a BOUND
/// vault IS the engine's, so a hand-edited one refuses loudly here rather than
/// resolving to something plausible.
///
/// # Errors
/// [`stored::StoredError`] for a governed URI that is not canonical.
pub(crate) fn stored_occupants(
    raw: &str,
    mounts: &MountSet,
) -> Result<Vec<(Range<usize>, String)>, stored::StoredError> {
    let nodes = syntax::parse(raw);
    let mask = code_mask(&nodes);
    let mut out = Vec::new();
    for (span, label, url) in markdown_links(raw, &mask) {
        if !stored::is_stored_uri(&url) {
            continue;
        }
        // Peek at the vault name before judging the URI: a malformed URI to a
        // vault we do not govern is not ours to refuse.
        let parsed = stored::decode(&url);
        let vault = match &parsed {
            Ok(r) => r.vault.clone(),
            Err(_) => match governed_vault_guess(&url) {
                Some(v) => v,
                None => continue,
            },
        };
        if mounts.name_of_vault(&vault).is_none() {
            continue;
        }
        let parsed = parsed?;
        let name = mounts
            .name_of_vault(&parsed.vault)
            .expect("checked bound just above");
        let address = match &parsed.selector {
            Some(sel) => format!("{name}:{}#{sel}", parsed.path),
            None => format!("{name}:{}", parsed.path),
        };
        let occupant = Occupant {
            span: span.clone(),
            addr: Addr::parse(&address).map_err(|_| stored::StoredError::BadPath {
                found: parsed.path.clone(),
            })?,
            display: Some(label),
            form: Form::Markdown,
        };
        out.push((span, occupant.agent_plane_text(&address)));
    }
    Ok(out)
}

/// **Might this text carry a cross-root position at all?** The cheap gate that
/// keeps the ordinary single-root corpus from ever paying for a mount table.
///
/// Deliberately a SUPERSET and lexical only: it decides whether to LOAD the
/// table, never what gets translated. `https://example.com` fires it (its head
/// parses as root `https` — the measurement on [`agent_plane_occupants`]);
/// nothing is translated when the table then says `https` is not bound. A false
/// positive costs one small file read; a false negative would skip the
/// translation entirely, so the bias is deliberate and one-directional.
pub(crate) fn may_carry_cross_root(raw: &str) -> bool {
    if raw.contains(stored::OBSIDIAN_SCHEME) {
        return true;
    }
    let nodes = syntax::parse(raw);
    let rooted = |spelling: &str| Addr::parse(spelling).is_ok_and(|a| a.root().is_some());
    for node in &nodes {
        let (syntax::DialectKind::Wikilink {
            target,
            heading,
            block,
            ..
        }
        | syntax::DialectKind::Embed {
            target,
            heading,
            block,
            ..
        }) = &node.kind
        else {
            continue;
        };
        if rooted(&wikilink_spelling(
            target,
            heading.as_deref(),
            block.as_deref(),
        )) {
            return true;
        }
    }
    let mask = code_mask(&nodes);
    markdown_links(raw, &mask)
        .iter()
        .any(|(_, _, url)| rooted(url))
}

/// **Might this text carry a STORED form?** The read plane's gate — a plain
/// byte search for the scheme, which is exact for that direction: a stored form
/// is an `obsidian://` URI and nothing else is.
pub(crate) fn may_carry_stored(text: &str) -> bool {
    text.contains(stored::OBSIDIAN_SCHEME)
}

/// The mount table as this plane needs it: **loaded from the machine's
/// `MERIDIAN.md`, lazily, and never fatally.**
///
/// A missing, unparseable or unbindable config yields the EMPTY projection, not
/// an error: absence is the topology working as designed (§ 8 M6), and failing
/// here would brick every write on every single-root machine on the planet. The
/// consequence is stated rather than implied — with an empty projection a
/// cross-root wikilink REFUSES with [`TranslateError::Unmounted`], which is the
/// honest answer (there is no vault name to store), and every ordinary write is
/// untouched.
pub(crate) fn machine_mounts() -> MountSet {
    let Ok(resolution) = config::resolve(&config::Env::from_process()) else {
        return MountSet::default();
    };
    let Ok(table) = resolution.bind() else {
        return MountSet::default();
    };
    table.projection()
}

/// The `vault=` parameter of a URI the strict decode REFUSED — the one fact
/// needed to decide whether the refusal is this engine's to raise.
///
/// Lexical and deliberately forgiving: it answers *"whose vault is this?"*, not
/// *"is this URI canonical?"*. The strict [`stored::decode`] answers the second
/// and is what the caller propagates.
fn governed_vault_guess(url: &str) -> Option<String> {
    let rest = url.strip_prefix(stored::OBSIDIAN_SCHEME)?;
    let (_, query) = rest.split_once('?')?;
    let raw = query
        .split('&')
        .find_map(|p| p.strip_prefix("vault="))
        .filter(|v| !v.is_empty())?;
    stored::decode_component(raw).or_else(|| Some(raw.to_string()))
}

/// `raw` with every governed stored form read back into its agent-plane
/// spelling.
///
/// # Errors
/// [`stored::StoredError`] — a governed URI that is not canonical fails LOUDLY
/// rather than resolving to something plausible.
pub(crate) fn to_agent_plane(raw: &str, mounts: &MountSet) -> Result<String, stored::StoredError> {
    let occupants = stored_occupants(raw, mounts)?;
    if occupants.is_empty() {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    for (span, text) in &occupants {
        out.push_str(&raw[cursor..span.start]);
        out.push_str(text);
        cursor = span.end;
    }
    out.push_str(&raw[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mounts() -> MountSet {
        let sessions = addr::MountName::parse("sessions").expect("a name");
        let assets = addr::MountName::parse("assets").expect("a name");
        MountSet::new([sessions.clone(), assets.clone()]).with_vault(sessions, "field-notes-sessions")
        // `assets` is bound WITHOUT a vault name — the git-folder row.
    }

    /// § 9.4 P1/P2 — frontmatter and code are NOT address positions, asserted
    /// with the positive half (P3) in the same breath so the untouched
    /// assertion cannot be satisfied by a transform that touches nothing.
    #[test]
    fn frontmatter_and_code_are_untouched_and_the_wikilink_is_not() {
        let raw = "---\nroot: SESSION.md\ntitle: x\n---\n\n\
                   Inline `root:page.md` and a fence:\n\n\
                   ```\n[[sessions:fenced.md]]\nroot: SESSION.md\n```\n\n\
                   A real one: [[sessions:notes.md#Design]]\n";
        let out = to_stored(raw, &mounts()).expect("translates");
        assert!(
            out.contains("---\nroot: SESSION.md\ntitle: x\n---"),
            "the frontmatter span must be byte-identical:\n{out}",
        );
        assert!(
            out.contains("`root:page.md`"),
            "an inline code span is a code sample, never an address:\n{out}",
        );
        assert!(
            out.contains("```\n[[sessions:fenced.md]]\nroot: SESSION.md\n```"),
            "a fenced block is untouched:\n{out}",
        );
        assert!(
            out.contains(
                "[sessions:notes.md#Design](obsidian://advanced-uri\
                 ?vault=field-notes-sessions&filepath=notes.md&heading=Design)"
            ),
            "the ONE real position must translate — otherwise this test passes \
             for a transform that does nothing:\n{out}",
        );
        assert!(
            !out.contains("[[sessions:notes.md#Design]]"),
            "no agent-plane cross-root spelling survives in an owned position:\n{out}",
        );
    }

    /// The gate: the agent-plane form round-trips through the stored form
    /// byte-identically. The wikilink IS the canonical agent-plane spelling
    /// (module docs), so it round-trips whole; the markdown-link spelling
    /// converges on it, and that convergence is ASSERTED rather than left to be
    /// discovered.
    #[test]
    fn the_agent_plane_form_round_trips_byte_identically() {
        let mounts = mounts();
        for agent in [
            "[[sessions:notes.md]]",
            "[[sessions:24-01-retro/notes.md#Design]]",
            "[[sessions:a/b.md#^claim-1]]",
            "[[sessions:notes.md|the retro]]",
            "[[sessions:a b/ünï code.md#A Heading|odd bytes]]",
            "prose [[sessions:x.md]] between [[sessions:y.md#H]] links\n",
            "[[ambient.md]] and [[ambient.md#H|alias]] are untouched\n",
        ] {
            let stored = to_stored(agent, &mounts).expect("translates");
            let back = to_agent_plane(&stored, &mounts).expect("reads back");
            assert_eq!(
                back, agent,
                "round trip must be byte-identical via {stored}"
            );
        }

        // § 9.4 P4 — a markdown link carrying an agent-plane URL translates (U3
        // rules it), and reads back as the canonical wikilink: same address,
        // same display, one spelling.
        let stored = to_stored("[the retro](sessions:notes.md)", &mounts).expect("translates");
        assert_eq!(
            stored,
            "[the retro](obsidian://open?vault=field-notes-sessions&file=notes.md)",
        );
        assert_eq!(
            to_agent_plane(&stored, &mounts).expect("reads back"),
            "[[sessions:notes.md|the retro]]",
        );
    }

    /// The positive half of criterion 4: the stored form IS an `obsidian://`
    /// URI carrying the VAULT NAME — not the agent-plane `root:` spelling, and
    /// not a device-local vault id.
    #[test]
    fn the_stored_form_carries_the_vault_name_not_the_root_name() {
        let out = to_stored("[[sessions:notes.md#Design]]", &mounts()).expect("translates");
        assert_eq!(
            out,
            "[sessions:notes.md#Design](obsidian://advanced-uri\
             ?vault=field-notes-sessions&filepath=notes.md&heading=Design)",
        );
        assert!(
            !out.contains("vault=sessions"),
            "the canonical ROOT name is the agent plane's; the stored plane is \
             spelled in vault names: {out}",
        );

        // **The DISPLAY of a bare wikilink is its agent-plane address, and that
        // is deliberate.** A-3 forbids an agent-plane `root:` spelling in
        // positions 1 and 2 — the wikilink TARGET and the markdown link URL. A
        // display field is neither, and it is what makes the round trip exact:
        // it is how read-back tells "no alias" from "an alias that happens to
        // read like the address". What criterion 4 forbids in a display field is
        // the FINGERPRINT, asserted separately.
        let (display, url) = out
            .split_once("](")
            .expect("the stored form is a markdown link");
        assert_eq!(
            display, "[sessions:notes.md#Design",
            "the display is the address"
        );
        assert!(
            !url.contains("sessions:notes.md"),
            "position 2 — the URL — carries NO agent-plane spelling: {url}",
        );
    }

    /// **The two refusal classes the shipped pin never watched fail** (U21).
    ///
    /// `the_refusals_name_the_address_and_the_ordinary_corpus_still_passes`
    /// covers `Unmounted`, `NoVault`, `Embed` and `Fingerprint`. It covers
    /// neither `PathUnseeable` nor `Stored` — so two refusals shipped that had
    /// never been observed red, and **a refusal never observed red is not known
    /// to fire.** Both are constructible today; here they are, each named.
    #[test]
    fn the_two_unwatched_refusal_classes_fire_and_name_what_they_refused() {
        // PATH-UNSEEABLE — declared in the file, unreadable on this machine.
        // Distinct from Unmounted BECAUSE THE FIX IS DIFFERENT: the mount entry
        // is already right, so telling the user to declare it sends them to
        // edit a config that is already correct.
        let declared = addr::MountName::parse("declared").expect("a name");
        let table = MountSet::new([]).with_unreachable(
            declared.clone(),
            "/nowhere/declared",
            "No such file or directory (os error 2)",
        );
        assert_eq!(
            to_stored("[[declared:notes.md]]", &table),
            Err(TranslateError::PathUnseeable {
                address: "declared:notes.md".to_string(),
                root: "declared".to_string(),
                path: "/nowhere/declared".to_string(),
                detail: "No such file or directory (os error 2)".to_string(),
            }),
            "a DECLARED but unreadable root refuses with the PATH, never with              the declare-it teaching whose action is already done",
        );

        // STORED — the stored grammar itself refuses. A path that is not a
        // confined corpus path has no stored spelling at all.
        let escape = to_stored("[[sessions:../escape.md]]", &mounts());
        assert!(
            matches!(
                &escape,
                Err(TranslateError::Stored {
                    address,
                    source: stored::StoredError::BadPath { found },
                }) if address == "sessions:../escape.md" && found == "../escape.md"
            ),
            "the stored plane's own refusal is propagated, naming the address              AND the part the grammar refused: {escape:?}",
        );
    }

    /// Every refusal class, each naming what it refused — and the ACCEPTANCE
    /// half beside it, so none of these is satisfied by a transform that
    /// refuses everything.
    #[test]
    fn the_refusals_name_the_address_and_the_ordinary_corpus_still_passes() {
        let mounts = mounts();
        assert_eq!(
            to_stored("[[unbound:notes.md]]", &mounts),
            Err(TranslateError::Unmounted {
                address: "unbound:notes.md".to_string(),
                root: "unbound".to_string(),
            }),
        );
        assert_eq!(
            to_stored("[[assets:media/logo.png]]", &mounts),
            Err(TranslateError::NoVault {
                address: "assets:media/logo.png".to_string(),
                root: "assets".to_string(),
            }),
            "a bound root with no vault name has no stored spelling",
        );
        assert_eq!(
            to_stored("![[sessions:notes.md]]", &mounts),
            Err(TranslateError::Embed {
                address: "sessions:notes.md".to_string(),
            }),
            "no URI transcludes",
        );
        assert!(matches!(
            to_stored("[[sessions:a.md#^claim@fp1.span2.b3.beef]]", &mounts),
            Err(TranslateError::Fingerprint { .. }),
        ));
        // ACCEPTANCE: the ordinary corpus passes through byte-identically.
        //
        // The URL rows are the ones that matter and they are MEASURED, not
        // assumed: `https://example.com` parses as an `Addr` with root `https`,
        // and `mailto:a@b.example` as root `mailto`. Before the bound test, this
        // transform refused BOTH — every external link in the corpus. The rows
        // stay here as the standing control on that defect.
        for ordinary in [
            "# A page\n\nno addresses at all\n",
            "[[ambient.md]] and [[ambient.md#H]]\n",
            "[a link](https://example.com) and [another](./rel.md)\n",
            "[mail](mailto:a@b.example) and [tel](tel:+15550100)\n",
            "[unbound](unbound:notes.md) — indistinguishable from a scheme we do not own\n",
            "---\nroot: SESSION.md\n---\n\n[[ambient.md]]\n",
        ] {
            assert_eq!(
                to_stored(ordinary, &mounts).expect("the ordinary corpus translates"),
                ordinary,
                "an untouched document must be byte-identical",
            );
        }
    }

    /// A hand-edited stored URI fails loudly on read-back; a URI naming a vault
    /// this machine does not govern is left verbatim, because claiming it would
    /// make every page carrying an ordinary obsidian link unreadable.
    #[test]
    fn read_back_refuses_a_governed_hand_edit_and_ignores_a_foreign_vault() {
        let mounts = mounts();
        let hand_edited =
            "[x](obsidian://adv-uri?vault=field-notes-sessions&filepath=a.md&block=claim)";
        assert!(
            to_agent_plane(hand_edited, &mounts).is_err(),
            "a governed URI that is not canonical must refuse, never resolve to \
             something plausible",
        );
        let non_canonical = "[x](obsidian://open?vault=field-notes-sessions&file=a%2Fb.md)";
        assert!(to_agent_plane(non_canonical, &mounts).is_err());

        for foreign in [
            "[x](obsidian://open?vault=someone-else&file=a.md)",
            "[x](obsidian://adv-uri?vault=someone-else&filepath=a.md&block=c)",
            "[x](obsidian://open?vault=&file=)",
        ] {
            assert_eq!(
                to_agent_plane(foreign, &mounts).expect("a foreign vault is not ours to judge"),
                foreign,
                "a link to a vault this machine does not bind is left verbatim",
            );
        }
    }

    /// Positions 3 and 4 are the IDENTITY (A-4): a `meridian-lock` block keeps
    /// the canonical `root:` form, never the URI. The block is fenced, so the
    /// mask already covers it — this test pins the CONSEQUENCE rather than the
    /// mechanism, because a future parser change that stopped masking the block
    /// would break the ratified stored form silently.
    #[test]
    fn the_lock_block_keeps_the_canonical_root_form() {
        let raw = "# Page\n\n[[sessions:notes.md]]\n\n\
                   ```meridian-lock\nversion: 2\npins:\n  - object: \"[[sessions:notes]]\"\n    \
                   hash: \"9ae3f1deadbeef\"\n    path: [\"Design\"]\n    \
                   fingerprint: \"fp1.span2.b3.a8222f5a\"\n```\n";
        let out = to_stored(raw, &mounts()).expect("translates");
        assert!(
            out.contains("object: \"[[sessions:notes]]\""),
            "the lock pin's `object` stays agent-plane: {out}",
        );
        assert!(
            out.contains("path: [\"Design\"]") && out.contains("hash: \"9ae3f1deadbeef\""),
            "the rest of the pin row is byte-identical too: {out}",
        );
        assert!(
            out.contains(
                "[sessions:notes.md](obsidian://open?vault=field-notes-sessions\
                          &file=notes.md)"
            ),
            "and the body link DID translate — otherwise this passes for a \
             transform that does nothing: {out}",
        );
    }
}

#[cfg(test)]
mod superset_probe {
    //! GATE 1 probe (U21): is `may_carry_cross_root` a TRUE SUPERSET of the
    //! spellings whose LINK-PLANE answer depends on the address plane?
    use super::*;

    // The predicate is imported UNDER ITS OWN NAME. It was aliased here as
    // `head_has_colon`, and a second NAME for one predicate is the cheapest way
    // to lose the sharing this probe exists to protect: the whole finding is
    // that nothing TESTS the sharing, so a reader who meets two names concludes
    // there are two functions. One question, one predicate, one name.

    #[test]
    fn may_carry_admits_every_spelling_whose_answer_depends_on_the_address_plane() {
        // `answerable` is a HAND-AUTHORED ORACLE: does the address plane have
        // anything to say about this spelling — a resolution OR a refusal?
        //
        // **It must never be computed by the function under test.** It was:
        // both `lexical_gate` and `answerable` called the same predicate, so
        // `answerable && !lexical_gate` and `!answerable && lexical_gate` were
        // false BY CONSTRUCTION and neither list could ever be populated. The
        // acceptance half — the assertion that this gate is not an
        // admit-everything gate — therefore could not fail. A test comparing a
        // function against itself passes for the same reason a test that never
        // runs passes.
        let cases = [
            (
                "sessions:notes.md",
                true,
                "well-formed rooted — the ordinary case",
            ),
            ("sessions:24-01/notes.md", true, "rooted with subdirs"),
            (
                "Sessions:notes.md",
                true,
                "UPPERCASE root — Addr::parse REFUSES (BadMountName)",
            ),
            (
                "My Notes:draft.md",
                true,
                "space in root — Addr::parse REFUSES (BadMountName)",
            ),
            (
                "a:b:c.md",
                true,
                "two head colons — REFUSES (AmbiguousColon)",
            ),
            (":notes.md", true, "empty root — REFUSES (EmptyMountName)"),
            ("sessions:", true, "empty path — REFUSES (EmptyPath)"),
            ("ambient.md", false, "no root at all — must NOT be admitted"),
            (
                "dir/a:b.md",
                false,
                "colon after the first slash — a path byte",
            ),
        ];
        let mut translate_slips = Vec::new();
        let mut lexical_slips = Vec::new();
        let mut lexical_overadmits = Vec::new();
        for (target, answerable, why) in cases {
            let raw = format!("# P\n\n[[{target}]]\n");
            let translate_gate = may_carry_cross_root(&raw);
            let lexical_gate = addr::head_carries_root_separator(target);
            println!(
                "may_carry={translate_gate:<5} lexical={lexical_gate:<5} \
                 answerable={answerable:<5} {target:<24} {why}"
            );
            if answerable && !translate_gate {
                translate_slips.push(target);
            }
            if answerable && !lexical_gate {
                lexical_slips.push(target);
            }
            if !answerable && lexical_gate {
                lexical_overadmits.push(target);
            }
        }

        // MEASURED, and this is the gate-1 finding: `may_carry_cross_root` is a
        // superset of WELL-FORMED rooted spellings only. A malformed rooted
        // spelling has an address-plane answer — a REFUSAL — and this gate does
        // not admit it.
        assert_eq!(
            translate_slips,
            [
                "Sessions:notes.md",
                "My Notes:draft.md",
                "a:b:c.md",
                ":notes.md",
                "sessions:"
            ],
            "the translation gate's measured blind spot must not drift silently",
        );

        // THE CORRECTED GATE. Lexical, so it admits the refusals too — a TRUE
        // superset, and it is the SAME predicate `resolve_linkpath`'s own C-3
        // guard uses (`model/src/lib.rs:1692`), which is the guard whose `None`
        // the degrade exists to compensate for. One question, one predicate.
        assert!(
            lexical_slips.is_empty(),
            "the lexical gate must admit every address-plane answer: {lexical_slips:?}"
        );
        // ACCEPTANCE HALF — a gate that admits everything is not a gate. An
        // ambient link and a colon-after-slash path must both stay OUT, or the
        // degrade fires on the whole ordinary corpus.
        assert!(
            lexical_overadmits.is_empty(),
            "the lexical gate must NOT admit an ambient spelling: {lexical_overadmits:?}"
        );
    }
}
