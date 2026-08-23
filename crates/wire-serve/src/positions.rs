//! Positional address grammar (`docs/address-grammar.md` § 9) — one owner of
//! where an agent-plane address may occupy a document, and of agent/stored
//! translations. Cross-root law lives here.
//!
//! # Transform is positional, never a byte transform
//! `root:` is a live YAML frontmatter key in shipped preset/def grammar, so a
//! blanket token rewrite would corrupt defs and silently invalidate pins whose
//! fingerprint covers those bytes. Addresses are identified by position in the
//! candidate; only owned positions are translated (`strip_fp_candidate` shape).
//!
//! # Four positions; this module touches two
//!
//! | # | Position | This module |
//! |---|---|---|
//! | 1 | wikilink target `[[…]]` | **translates** |
//! | 2 | markdown link URL `[label](url)` | **translates** |
//! | 3 | `meridian-lock` `ref:` | **identity** (ratified) |
//! | 4 | `meridian-lock` `objects:` | **identity** (ratified) |
//!
//! 3–4 stay canonical `root:` form (agent plane, never URI); translating the
//! lock would break its ratified form.
//!
//! # Mask from the parser, never a second byte reading
//! Code samples and fences (`root:page.md` inside) are left alone (§ 9.4).
//! Position 1 is masked by `syntax::parse`; position 2 scans with the same
//! parse's Fence / `InlineCode` / Frontmatter spans — one parse, one masking
//! law.
//!
//! # Canonical agent-plane cross-root spelling is the wikilink
//! Stored form is always a markdown link (`[display](obsidian://…)`); reverse
//! picks the wikilink, so the address round-trips and surrounding markdown
//! converges on one spelling.

use std::ops::Range;

use addr::{Addr, MountSet, stored};

/// Which markdown form carried an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    /// Position 1 — `[[target]]` / `[[target|alias]]`.
    Wikilink,
    /// Position 1, transcluding — `![[target]]`. Refused: no `obsidian://` URI
    /// transcludes, so translating one would silently turn an embed into a
    /// link and change what the page says.
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
/// teaches the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TranslateError {
    /// The address names a root nothing declares here, so there is no vault
    /// name to store — grey's write-side twin.
    Unmounted { address: String, root: String },
    /// The address names a root the mount table declares and this machine
    /// cannot read — the path is absent, unreadable, or holds no corpus.
    ///
    /// A separate variant from [`TranslateError::Unmounted`] because the fix
    /// differs: the declaration the other refusal prescribes is already done.
    /// The reason word is [`addr::PATH_UNSEEABLE_REASON_WORD`].
    PathUnseeable {
        address: String,
        root: String,
        path: String,
        detail: String,
    },
    /// The root is bound but carries no Obsidian vault name — a plain-folder
    /// root. It has no `obsidian://` spelling at all.
    NoVault { address: String, root: String },
    /// The stored grammar refused ([`stored::StoredError`]).
    Stored {
        address: String,
        source: stored::StoredError,
    },
    /// A cross-root embed. No URI transcludes; translating would change the
    /// document's meaning rather than its spelling.
    Embed { address: String },
}

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslateError::Unmounted { address, root } => write!(
                f,
                "refused: '{address}' names root '{root}', which this machine does not bind — \
                 there is no vault name to store, so the link would land as bytes no reader can \
                 follow. Not a drift: nothing changed, you just cannot see from here. \
                 Fix: declare '{root}' in ~/MERIDIAN.md as a mount entry (name / path); \
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
/// either part, and an image (`![…](…)`) is not a link position (§ 9.1). A
/// shape this scanner declines to claim is left byte-untouched; the guard that
/// follows refuses anything a translation should have reached.
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

/// Every agent-plane cross-root address in an owned position of `raw`, span
/// order.
///
/// An address with no root is ambient and untouched — the overwhelming
/// majority of refs.
///
/// # The two positions do not ask the same question
/// In position 1 a rooted spelling is unambiguously an address, so an unbound
/// root refuses there. In position 2 it is not:
/// `Addr::parse("https://example.com")` succeeds with root `https`, as does
/// every other absolute URI. Position 2 therefore claims only what the mount
/// table declares; a rooted markdown URL naming a root nothing declares is left
/// untouched, indistinguishable from a URI whose scheme this engine does not
/// own.
///
/// # The predicate is declared, not bound
/// `is_bound` is false both for `https` (not ours — leave verbatim) and for a
/// root `~/MERIDIAN.md` declares that this machine has not checked out (ours,
/// no stored form — refuse). Skipping declared-but-unbound roots disarmed the
/// transform and [`crate::write::stored_form_guard`] together, letting
/// agent-plane bytes reach disk at exit 0. [`addr::MountSet::is_declared`]
/// keeps the two apart; an occupant on an unreachable root refuses through
/// [`stored_text`] as [`TranslateError::PathUnseeable`].
///
/// Both callers share this scan: the guard catches a door that reaches bytes
/// without the translation, so its population must be the translation's.
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
        // The declared test, not merely a rooted test — see the doc comment above.
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

/// The stored spelling of one occupant — `[display](obsidian://…)`.
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
    // Law A-2: the fragment is selector bytes to its end — `@` included — so
    // there is no fp lane to guard here. The shaped-token strip at the splice
    // door (S10) still keeps engine-minted decorations off stored bytes.
    let root = occupant
        .addr
        .root()
        .expect("an occupant carries a root by construction");
    if !mounts.is_bound(root) {
        // Two causes, two words: a root nobody declares, and a root the file
        // declares that this machine cannot read — whose prescribed fix is
        // already done.
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
    // An alias is a LOOKUP spelling and never a stored one (`address-grammar.md`
    // § 4.6a): the vault leg is keyed by the mount's own name, so `sessions:x`
    // must become the canonical name here or it would refuse `NoVault` on a root
    // that has one.
    let root = mounts.canonical(root).unwrap_or(root);
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
        // A bare wikilink's display is its address, so the round trip can tell
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
/// The first refusal, so the caller reports one actionable sentence.
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

/// Every stored cross-root form in `raw` this machine governs, with the
/// agent-plane text it reads back to — span order.
///
/// A URI naming a vault this machine does not bind is left verbatim — an
/// ordinary link to a vault meridian does not govern. A URI naming a bound
/// vault is the engine's, so a hand-edited one refuses loudly here rather than
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

/// Might this text carry a cross-root position at all? The cheap gate that
/// keeps the ordinary single-root corpus from ever paying for a mount table.
///
/// Deliberately a superset and lexical only: it decides whether to load the
/// table, never what gets translated. A false positive costs one small file
/// read; a false negative would skip the translation entirely.
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

/// Might this text carry a stored form? The read plane's gate — a plain byte
/// search for the scheme, which is exact for that direction: a stored form is
/// an `obsidian://` URI and nothing else is.
pub(crate) fn may_carry_stored(text: &str) -> bool {
    text.contains(stored::OBSIDIAN_SCHEME)
}

/// The mount table as this plane needs it: loaded from the machine's
/// `MERIDIAN.md`, lazily, and never fatally.
///
/// A missing, unparseable or unbindable config yields the empty projection, not
/// an error: absence is the topology working as designed (§ 8), and failing
/// here would brick every write on every single-root machine. With an empty
/// projection a cross-root wikilink refuses with [`TranslateError::Unmounted`]
/// and every ordinary write is untouched.
pub(crate) fn machine_mounts() -> MountSet {
    let Ok(resolution) = config::resolve(&config::Env::from_process()) else {
        return MountSet::default();
    };
    let Ok(table) = resolution.bind() else {
        return MountSet::default();
    };
    table.projection()
}

/// The `vault=` parameter of a URI the strict decode refused — the one fact
/// needed to decide whether the refusal is this engine's to raise.
///
/// Lexical and deliberately forgiving: it answers whose vault this is, not
/// whether the URI is canonical. The strict [`stored::decode`] answers the
/// second and is what the caller propagates.
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
/// [`stored::StoredError`] — a governed URI that is not canonical fails loudly
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
        // `assets` is bound WITHOUT a vault name — the plain-folder row.
    }

    /// § 9.4 — frontmatter and code are not address positions, asserted with
    /// the positive half beside it.
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

    /// The agent-plane form round-trips through the stored form
    /// byte-identically: the wikilink round-trips whole, and the markdown-link
    /// spelling converges on it.
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

        // § 9.4 — a markdown link carrying an agent-plane URL translates, and
        // reads back as the canonical wikilink.
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

    /// The stored form is an `obsidian://` URI carrying the vault name — not
    /// the agent-plane `root:` spelling, and not a device-local vault id.
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

        // The display of a bare wikilink is its agent-plane address: a display
        // field is not an owned position, and this is how read-back tells "no
        // alias" from "an alias that happens to read like the address".
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

    /// The two refusal classes the sibling test never watched fail:
    /// `PathUnseeable` and `Stored`.
    #[test]
    fn the_two_unwatched_refusal_classes_fire_and_name_what_they_refused() {
        // Path-unseeable — declared in the file, unreadable on this machine.
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

        // Stored — the stored grammar itself refuses. A path that is not a
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

    /// Every refusal class, each naming what it refused — with the acceptance
    /// half beside it.
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
        // Law A-2: an `@` in the fragment is selector bytes, so a spelling that
        // once refused as a fingerprint now translates with its whole fragment
        // verbatim — and round-trips.
        let at_bearing = to_stored("[[sessions:a.md#^claim@fp1.span2.b3.beef]]", &mounts)
            .expect("an `@`-bearing fragment is selector bytes and translates");
        assert!(
            !at_bearing.contains("[[sessions:"),
            "the owned position translated: {at_bearing}",
        );
        assert_eq!(
            to_agent_plane(&at_bearing, &mounts).expect("reads back"),
            "[[sessions:a.md#^claim@fp1.span2.b3.beef]]",
            "the whole fragment survives the round trip byte-identically",
        );
        // The corpus motive: an `@`-bearing HEADING is addressable cross-root
        // by its own spelling.
        let heading = to_stored("[[sessions:a.md#Deploy @ prod]]", &mounts)
            .expect("the design pair's heading translates");
        assert_eq!(
            to_agent_plane(&heading, &mounts).expect("reads back"),
            "[[sessions:a.md#Deploy @ prod]]",
        );
        // Acceptance: the ordinary corpus passes through byte-identically.
        // `https://example.com` parses as root `https` and `mailto:a@b.example`
        // as root `mailto` — this transform once refused both.
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
    /// this machine does not govern is left verbatim.
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

    /// Positions 3 and 4 are the identity: a `meridian-lock` block keeps the
    /// canonical `root:` form, never the URI. Pins the consequence, not the
    /// mask that implements it — a parser change that stopped masking the block
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
    //! Probe: is `may_carry_cross_root` a true superset of the spellings whose
    //! link-plane answer depends on the address plane?
    use super::*;

    #[test]
    fn may_carry_admits_every_spelling_whose_answer_depends_on_the_address_plane() {
        // `answerable` is a hand-authored oracle — does the address plane have
        // anything to say about this spelling, a resolution or a refusal? It
        // must never be computed by the function under test, or the slip lists
        // are empty by construction.
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

        // `may_carry_cross_root` is a superset of well-formed rooted spellings
        // only: a malformed rooted spelling has an address-plane answer — a
        // refusal — and this gate does not admit it.
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

        // The corrected gate is lexical, so it admits the refusals too — a
        // true superset, and the same predicate `resolve_linkpath`'s own guard
        // uses.
        assert!(
            lexical_slips.is_empty(),
            "the lexical gate must admit every address-plane answer: {lexical_slips:?}"
        );
        // Acceptance half — an ambient link and a colon-after-slash path must
        // both stay out, or the degrade fires on the whole ordinary corpus.
        assert!(
            lexical_overadmits.is_empty(),
            "the lexical gate must NOT admit an ambient spelling: {lexical_overadmits:?}"
        );
    }
}
