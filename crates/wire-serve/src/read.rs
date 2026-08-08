//! The read-op arms over borrowed parsed state — one implementation for both
//! hosts. Each arm takes already-built `model` state (a `&Document`, or the
//! `&CorpusIndex` + document map) plus the ambient root as data. Parsing is the
//! caller's, projection is `wire-map`'s, edges are `query`'s.

use std::collections::BTreeMap;

use wire::{ErrorBody, ErrorCode, NodeRev, Path, ResponseBody, Root, SecRef, Span};

use crate::bad_request;

/// The stored form ([`crate::write`]), read back into the agent plane. Lands on
/// the rendered face only.
///
/// `sections[].content` and `cat.content` stay raw and untranslated: they are
/// the verbatim bytes `sec_rev` was minted over, so each row is self-verifying.
///
/// A URI naming a vault this machine does not bind is left verbatim; a URI
/// naming a bound vault is the engine's, so a hand-edited one fails loudly
/// rather than resolving to something plausible.
///
/// # Errors
/// `bad_request` naming the URI, the non-canonical part and the canonical
/// spelling.
fn agent_plane_face(rendered: String) -> Result<String, Box<ErrorBody>> {
    if !crate::positions::may_carry_stored(&rendered) {
        return Ok(rendered);
    }
    let mounts = crate::positions::machine_mounts();
    crate::positions::to_agent_plane(&rendered, &mounts).map_err(|e| {
        bad_request(format!(
            "read: a stored cross-root link in this page cannot be read back — {e}"
        ))
    })
}

/// The claim-link decoration input, re-exported at the read seam so a host
/// wires it without taking a `render` dependency of its own.
pub use render::{ClaimLink, Decorations, NO_DECORATIONS};

/// wire §4.1 the map: header `file_rev` (the document root's rev over whole-file
/// bytes) + the ambient `root`, rows from the `wire-map` projection. `ambient`
/// is the corpus content-hash cursor the caller already holds.
#[must_use]
pub fn toc(doc: &model::Document, path: &Path, ambient: &Root) -> ResponseBody {
    ResponseBody::Toc {
        path: path.clone(),
        file_rev: NodeRev(doc.root.node_rev.0.clone()),
        root: ambient.clone(),
        nodes: wire_map::project_toc(doc),
    }
}

/// wire §4.2: full span bytes (heading-inclusive), rev over precisely those
/// bytes. `sec` absent → whole file + `file_rev` riding the `node_rev` slot.
///
/// # Errors
/// The `sec` names no node (`ref_not_found`) or names more than one
/// (`ambiguous_ref`), or a malformed anchor id (`bad_request`).
pub fn cat(doc: &model::Document, sec: Option<SecRef>) -> Result<ResponseBody, Box<ErrorBody>> {
    let Some(sec) = sec else {
        return Ok(ResponseBody::Cat {
            span: Span(0, doc.raw.len() as u64),
            node_rev: NodeRev(doc.root.node_rev.0.clone()),
            content: doc.raw.clone(),
        });
    };
    let target = model::resolve(doc, &to_model_ref(&sec)?).map_err(|e| {
        Box::new(match e {
            model::ResolveError::NotFound => cat_miss(&sec, doc),
            model::ResolveError::Ambiguous(candidates) => ambiguous(&sec, doc, &candidates),
        })
    })?;
    Ok(ResponseBody::Cat {
        span: Span(target.span.start as u64, target.span.end as u64),
        node_rev: NodeRev(target.node_rev.0),
        content: doc.raw[target.span].to_string(),
    })
}

/// wire §4.3: the full node inventory via the `wire-map` projection, `kinds`
/// filtered (values already validated against the closed enum at decode).
///
/// `enrich` (v3 sessions only): attach the host-face addressing facts — dewey
/// `n` and subtree `words` — to heading nodes. A v2 session never enriches:
/// the keys are v3-additive and the frozen v2 bytes stay byte-identical
/// (`contract_v2.rs`).
#[must_use]
pub fn extract(
    doc: &model::Document,
    path: &Path,
    kinds: Option<Vec<String>>,
    enrich: bool,
) -> ResponseBody {
    let mut nodes = wire_map::project(doc);
    if let Some(kinds) = kinds {
        let keep: Vec<wire::NodeKind> = kinds
            .iter()
            .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok())
            .collect();
        nodes.retain(|n| keep.contains(&n.kind));
    }
    if enrich {
        let facts = wire_map::facts::read_facts(&wire_map::project_toc(doc), doc.raw.as_bytes());
        let by_span: BTreeMap<(u64, u64), &wire_map::facts::ReadFact> = facts
            .iter()
            .filter(|f| f.depth > 0)
            .map(|f| ((f.span.0, f.span.1), f))
            .collect();
        for node in &mut nodes {
            if node.kind == wire::NodeKind::Heading
                && let Some(fact) = by_span.get(&(node.span.0, node.span.1))
            {
                node.n = Some(fact.n.clone());
                node.words = Some(fact.words);
            }
        }
    }
    ResponseBody::Nodes {
        path: path.clone(),
        nodes,
    }
}

/// The composed-read parameters, decoded from the v3-only `read` op — the host
/// face's read-tool vocabulary, engine-side.
#[derive(Debug, Clone, Default)]
pub struct ReadParams {
    /// The whole-call subtree scope, as segments.
    pub frag: Option<Vec<wire::HpathSeg>>,
    /// Document-absolute selectors in the tagged read grammar — and the mode
    /// itself: non-empty selects sections, absent/empty answers the toc.
    /// Nothing else picks the arm.
    pub sections: Option<Vec<wire::ReadSel>>,
    pub display_path: Option<String>,
    /// §9 read provenance: the daemon-derived actor, carried to the read-mint
    /// site. Never MCP-caller-settable; nothing below invents one.
    pub actor: Option<String>,
}

/// The `actor == None` no-mint door, in one place.
///
/// A read mints a receipt only for a real daemon-derived identity. The bare CLI
/// sends no actor and is local-operator-trusted — as `mrd put` skips the host's
/// authz — so it mints nothing and the pin gate is bypassed for it. A blank
/// actor is absent too: an empty string is not an identity, and admitting one
/// would open a bucket every actor-less caller shares.
pub(crate) fn mint_actor(actor: Option<&str>) -> Option<&str> {
    actor.map(str::trim).filter(|a| !a.is_empty())
}

/// The composed read op: addressing + content + render served from one
/// borrowed document snapshot — `file_rev`, the ambient `root`, the
/// toc/sections facts, and the rendered projection in one exchange. Refusal
/// messages are the Go host face's verbatim strings, so the thin proxy
/// forwards `error.message` without re-minting.
///
/// `mint` is the read-is-the-mint ledger: present only for a host that holds
/// a daemon-session layer (the registry daemon). A host with no session — the
/// bare CLI, any in-process caller — passes `None` and mints nothing.
/// Minting is sections-mode only.
///
/// `decorations` is the claim-link input, built by the caller
/// ([`page_decorations`]) and passed through to the renderer as data;
/// [`NO_DECORATIONS`] is the one spelling of "nothing to decorate". Decoration
/// lands in `rendered_text` alone: `sections[].content` stays the raw face a
/// put is built from, because a read-decorated view feeding a write is a
/// data-loss class.
///
/// # Errors
/// `bad_request` (fix): `frag` and `sections` both present; a section read
/// with no selectors. `ref_not_found` (fix): a toc `frag` naming no section;
/// all section selectors missing. `internal` carrying the typed
/// `render_failed` spelling when the walker refuses.
pub fn composed_read(
    doc: &model::Document,
    path: &Path,
    ambient: &Root,
    params: &ReadParams,
    mint: Option<&receipt::read_mint::ReadMintStore>,
    decorations: &Decorations,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let facts = wire_map::facts::read_facts(&wire_map::project_toc(doc), doc.raw.as_bytes());
    let file_rev = doc.root.node_rev.0.clone();
    let words_total: u64 = facts.iter().map(|f| f.words).sum();
    let display = params.display_path.as_deref().unwrap_or(path.0.as_str());
    let frag: &[wire::HpathSeg] = params.frag.as_deref().unwrap_or(&[]);
    // `sections`'s presence is the mode. Absent → the toc read; present → a
    // section read, empty included, so `sections: []` still meets the
    // "a section read needs a selector" refusal instead of answering a toc.
    let has_sections = params.sections.is_some();
    if !frag.is_empty() && has_sections {
        return Err(bad_request(format!(
            "read: pass either a #fragment on ref or sections[], not both, for {display} — \
             the fragment scopes the whole call; sections[] selects document-absolute \
             paths. Nothing was read and no rev was minted. Fix: drop the `#fragment` from \
             the ref and keep the document-absolute `sections[]`, or drop `sections[]` and \
             let the fragment scope the call."
        )));
    }
    let header = render::Header {
        display_path: display,
        file_rev: &file_rev,
        // The `--if-fingerprint` guard's own value: the same `ambient` this
        // body carries as `root`/`fingerprint`, so the rendered token and the
        // structured one cannot disagree.
        fingerprint: &ambient.0,
        words_total,
        decorations,
    };
    // The `^id` anchor plane — computed once, emitted by both modes, scoped by
    // the same `frag` that scopes the whole call. Its own array: a mixed-in
    // anchor row's `depth 0` once crashed a client renderer.
    let anchors: Vec<wire::ReadAnchor> = wire_map::facts::anchor_rows(&facts, frag)
        .iter()
        .filter_map(|f| read_anchor(f))
        .collect();
    // The frontmatter-properties plane — document-grain, so unlike `anchors`
    // it is never `frag`-scoped: frontmatter belongs to the document, not to
    // any subtree (wire-contract § A.3).
    let props = read_props(doc);

    if has_sections {
        let sels: Vec<wire::ReadSel> = params.sections.clone().unwrap_or_default();
        let (body, rendered_sections) = composed_sections(doc, &facts, &sels, header)?;
        // Read-is-the-mint: one receipt per section this call actually served
        // — actor, canonical selector, `sec_rev` — off the raw rows whose
        // bytes the caller received verbatim. A toc read mints nothing, and the
        // rev bound is the raw face's `sec_rev`, never anything derived from
        // the elided `rendered_text`. Unresolved selectors are absent from
        // these rows, so a miss mints nothing.
        if let (Some(store), Some(actor)) = (mint, mint_actor(params.actor.as_deref())) {
            // Keyed on the row's own tagged selector, the same structure the
            // pin gate looks up — so the two sides cannot drift into two
            // spellings of one address, and a heading named `1.2` cannot open
            // the gate for the dewey row of that name.
            for row in &rendered_sections {
                store.mint(actor, path.0.as_str(), &row.sel, &row.sec_rev.0);
            }
        }
        return Ok(ResponseBody::Read {
            path: path.clone(),
            file_rev: NodeRev(file_rev),
            root: ambient.clone(),
            words_total,
            toc: None,
            anchors,
            props,
            sections: Some(rendered_sections),
            truncated: body.notice.is_some().then_some(true),
            notice: body.notice,
            rendered_text: agent_plane_face(body.text)?,
        });
    }
    let rows = wire_map::facts::toc_rows(&facts, frag);
    if !frag.is_empty() && rows.is_empty() {
        let asked = wire::ReadSel::Hpath {
            hpath: frag.to_vec(),
        }
        .display();
        let mut e = ErrorBody::new(ErrorCode::RefNotFound);
        e.message = Some(format!(
            "read: no section at \"{asked}\" in {display}. Nothing was read and no \
             rev was minted. {}",
            crate::section_recovery(&asked, Some(display))
        ));
        return Err(Box::new(e));
    }
    // One row set for the heading plane: `rendered_text` renders these rows
    // and `toc` carries these rows, so the structured face never diverges
    // from the rendered one.
    let rendered_text = agent_plane_face(render::toc_toon(&header, &rows))?;
    Ok(ResponseBody::Read {
        path: path.clone(),
        file_rev: NodeRev(file_rev),
        root: ambient.clone(),
        words_total,
        toc: Some(rows.iter().map(|f| read_row(f)).collect()),
        anchors,
        props,
        sections: None,
        truncated: None,
        notice: None,
        rendered_text,
    })
}

/// One heading fact → one wire composed-read row: the addressing facts plus
/// the authz facts (`span`, `content_span`), carried verbatim off the fact —
/// this seam never re-derives an address.
fn read_row(f: &wire_map::facts::ReadFact) -> wire::ReadRow {
    wire::ReadRow {
        n: f.n.clone(),
        depth: f.depth,
        title: f.title.clone(),
        hpath: f.hpath.clone(),
        words: f.words,
        sec_rev: NodeRev(f.sec_rev.clone()),
        span: f.span,
        content_span: f.content_span,
    }
}

/// One anchor fact → one wire `anchors[]` entry: the block id and the
/// block-leaf span, which is everything the host's containment join consumes.
/// `None` is unreachable (`read_facts` mints an anchor fact only from an
/// anchor-bearing `list_item`) and is dropped rather than serialized as an
/// empty id.
fn read_anchor(f: &wire_map::facts::ReadFact) -> Option<wire::ReadAnchor> {
    f.anchor.as_ref().map(|id| wire::ReadAnchor {
        anchor: id.clone(),
        span: f.span,
    })
}

/// The frontmatter-properties plane (wire-contract § A.3): one row per
/// top-level key, document order, first occurrence wins. Keys and values come
/// off the model's flat parse (the keys authority); span and CAS token come
/// off the same `fm_key` grain resolution the write plane compares against,
/// so the served `prop_rev` and a later `if_node_rev` cannot be two
/// derivations of one fact.
///
/// `value` is the § A.6.1 DECODED scalar, not the stored bytes: this plane is
/// typed `string`, and a reader comparing `owner` against an id must not be
/// handed quote bytes it never asked about. `span`/`prop_rev` stay over the
/// stored form (§ A.6.2) — they answer a guard question, not a value one.
fn read_props(doc: &model::Document) -> Vec<wire::ReadProp> {
    let Some(map) = frontmatter_map(&doc.root) else {
        return Vec::new();
    };
    map.0
        .iter()
        .map(|(key, value)| {
            // A map key resolves by construction — both sides scan the same
            // block with the same key normalization, first occurrence wins.
            // A miss here is an engine bug; wrong data must not serve quietly.
            let target = model::resolve(doc, &model::Ref::FmKey(key.clone()))
                .expect("frontmatter map key resolves against its own document");
            wire::ReadProp {
                key: key.clone(),
                value: model::scalar::text(value),
                span: Span(target.span.start as u64, target.span.end as u64),
                prop_rev: NodeRev(target.node_rev.0),
            }
        })
        .collect()
}

/// The document's frontmatter node's parsed map, if any.
fn frontmatter_map(node: &model::Node) -> Option<&model::YamlMap> {
    if let model::NodeKind::Frontmatter { map } = &node.kind {
        return Some(map);
    }
    node.children.iter().find_map(frontmatter_map)
}

/// The claim-link decorations for one page: every `meridian-lock` pin the page
/// declares, matched to the links in the page's own body that address it,
/// carrying the shaped `@fp` token that pin's drift color mints.
///
/// The caller side of the render seam — outside `render`, which never reads a
/// lock and never computes a fingerprint. It needs the corpus, so only a host
/// that holds one calls it (the registry daemon); the bare CLI passes
/// [`render::NO_DECORATIONS`].
///
/// Four rules, each closing a way a decoration could lie:
///
/// - The color is the pin classifier's: [`model::selector::classify_pin`] is
///   the single compare `view::walk::lock_pin_colors` uses, so a decorated
///   link can never disagree with the same pin's row in `mrd walk`.
/// - The handle is the promoted `^slug`, never the pin's identity: a pin's
///   fingerprint covers the span its `ref` resolves to (the section), while an
///   anchor node's span is only its host line — treating the slug as the
///   identity would read green on every body edit.
/// - The link must actually address the pinned page: the body's spelling
///   (`guide`) and the lock's (`guide.md`) both go through the one linkpath
///   resolver before they are called a match.
/// - A pin it cannot honestly color is left undecorated — an unresolvable
///   target page, a refused lock, a `Malformed` token with no digest to show.
#[must_use]
pub fn page_decorations(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    path: &str,
) -> Decorations {
    let mut out = Decorations::new();
    let Some(doc) = docs.get(path) else {
        return out;
    };
    // A refused lock decorates nothing: it is already visible as the grey
    // `lock-refused` row.
    let Ok(Some(found)) = lock::find(doc) else {
        return out;
    };
    let mut links: Vec<(&String, &String)> = Vec::new();
    collect_links(&doc.root, &mut links);
    for pin in &found.lock.pins {
        let lock::Selector::Path(segments) = &pin.selector else {
            // The `properties` arm claims frontmatter keys: `Selector` has no
            // member for that, and frontmatter carries no `[[page#^block]]`
            // link to decorate — nothing to colour.
            continue;
        };
        if segments.is_empty() {
            continue; // `path: []` is the whole body — no block handle to decorate
        }
        let Some(target_path) = resolve_page(index, docs, &pin.object, path) else {
            continue;
        };
        // The anchor form: a path array whose sole element is a `^id`, decoded
        // exactly as `write::pin_row` encodes it. A mixed array is refused at
        // the write door, so a `^`-leading element inside a longer chain never
        // reaches this face.
        let selector = match segments.as_slice() {
            [only] if only.starts_with('^') => {
                model::selector::Selector::Block(only[1..].to_string())
            }
            _ => model::selector::Selector::Heading(segments.clone()),
        };
        // The handle a pin's target carries: for a section pin, the slug the
        // id promotion mints from the heading title — computed by the same
        // owner, so a decoration keys on what id promotion actually wrote. An
        // anchor pin is its own handle.
        let handle = match &selector {
            model::selector::Selector::Block(id) => Some(id.clone()),
            _ => segments.last().and_then(|h| crate::write::slug_id(h).ok()),
        };
        let Some(handle) = handle else {
            continue;
        };
        let color =
            model::selector::classify_pin(&selector, &pin.fingerprint, docs.get(&target_path));
        let Some(digest) =
            model::fingerprint::parse_fingerprint(&pin.fingerprint).map(|p| p.digest)
        else {
            continue; // Malformed: no digest exists to show
        };
        let Some(token) = syntax::fp_token(model::selector::color_tone(&color), &digest) else {
            continue;
        };
        for (target, block) in links.iter().filter(|(_, b)| **b == handle) {
            if resolve_page(index, docs, target, path).as_ref() == Some(&target_path) {
                out.insert(
                    ClaimLink {
                        target: (*target).clone(),
                        block: (*block).clone(),
                    },
                    token.clone(),
                );
            }
        }
    }
    out
}

/// Resolve a link/ref target spelling to a corpus path through the one address
/// owner ([`model::CorpusIndex::resolve_ref`]) — the same three rules, in the
/// same order, that `view::read_face::resolve_to_path` gives the walk plane. An
/// empty spelling is the page itself (a self-link `[[#^blk]]`).
///
/// It holds no precedence of its own: a private one drifted from the walk
/// plane's and coloured one pin two ways.
///
/// This face carries no mount table yet: it resolves against the ambient root
/// alone, so a `root:`-bearing address answers `None` (unresolved) rather than
/// the ambient root's same-basename file. `mrd walk` is where the unmounted
/// grey is rendered with its reason.
fn resolve_page(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    spelling: &str,
    from: &str,
) -> Option<String> {
    if spelling.is_empty() {
        return Some(from.to_owned());
    }
    index
        .resolve_ref(
            spelling,
            from,
            &model::RootedCorpus::ambient(docs),
            &addr::MountSet::default(),
        )
        .path()
        .map(str::to_owned)
}

/// Every `[[target#^block]]` link in the tree, as (target, block) borrowed off
/// the parsed nodes — the addresses a decoration can key on.
fn collect_links<'a>(node: &'a model::Node, out: &mut Vec<(&'a String, &'a String)>) {
    if let model::NodeKind::Wikilink {
        target,
        block: Some(block),
        ..
    }
    | model::NodeKind::Embed {
        target,
        block: Some(block),
        ..
    } = &node.kind
    {
        out.push((target, block));
    }
    for child in &node.children {
        collect_links(child, out);
    }
}

/// The rendered sections-mode pieces the `Read` body carries.
struct SectionsRender {
    text: String,
    notice: Option<String>,
}

/// One failed section selector with its honest reason (wire-contract A.3): a
/// miss (an anchor miss carries its Law A-3 teaching clause), or an ambiguity
/// carrying each candidate's machine address.
enum SelFail {
    Miss {
        display: String,
        /// The `^id` teaching clause ([`anchor_sel_teach`]); `None` on heading
        /// and dewey misses, whose teaching is the aggregate recovery clause.
        teach: Option<String>,
    },
    Ambiguous {
        display: String,
        candidates: Vec<String>,
    },
    /// A duplicated block id: >1 carrier, but unlike [`SelFail::Ambiguous`] no
    /// machine address exists per candidate — duplicate ids share one spelling
    /// and the anchor grammar has no occurrence index — so the entry counts
    /// the carriers and teaches the anchor remedy (wire-contract A.3, door
    /// symmetry over duplicate block ids).
    DupAnchor { display: String, count: usize },
}

impl SelFail {
    fn display(&self) -> &str {
        match self {
            SelFail::Miss { display, .. }
            | SelFail::Ambiguous { display, .. }
            | SelFail::DupAnchor { display, .. } => display,
        }
    }

    /// The all-fail refusal's per-selector clause. The miss arm keeps the
    /// standing single-miss spelling byte-by-byte as its prefix — an anchor
    /// miss appends its teaching rather than reshaping the sentence; the
    /// ambiguous arm never says "no section addressed" — two sections matched,
    /// and the honest answer names both and how to pin one (dogfood F4).
    fn phrase(&self) -> String {
        match self {
            SelFail::Miss {
                display,
                teach: None,
            } => format!("no section addressed by \"{display}\""),
            SelFail::Miss {
                display,
                teach: Some(teach),
            } => format!("no section addressed by \"{display}\" ({teach})"),
            SelFail::Ambiguous {
                display,
                candidates,
            } => format!(
                "\"{display}\" is ambiguous ({} matches — pin one occurrence by its machine \
                 address, or its dewey ordinal from the toc: {})",
                candidates.len(),
                candidates.join(" or ")
            ),
            SelFail::DupAnchor { display, count } => format!(
                "\"{display}\" is ambiguous ({count} blocks carry this id — give each a \
                 distinct id, or read the enclosing section by heading path)"
            ),
        }
    }

    /// The partial-read notice's per-selector entry — same facts as
    /// [`SelFail::phrase`], in the notice's established bare-selector shape.
    fn notice_entry(&self) -> String {
        match self {
            SelFail::Miss {
                display,
                teach: None,
            } => display.clone(),
            SelFail::Miss {
                display,
                teach: Some(teach),
            } => format!("{display} ({teach})"),
            SelFail::Ambiguous {
                display,
                candidates,
            } => format!(
                "{display} (ambiguous, {} matches: {})",
                candidates.len(),
                candidates.join(" or ")
            ),
            SelFail::DupAnchor { display, count } => {
                format!("{display} (ambiguous, {count} blocks carry this id)")
            }
        }
    }
}

/// Face-scoped `^id` miss teaching (Law A-3: a miss teaches before it
/// refuses). The composed read resolves anchors against the face's anchor
/// plane, which carries list-item hosts only (Go parity, golden-pinned), so a
/// miss has two honest shapes and each gets its own clause:
///
/// - the id exists in the parse tree but its host block kind is outside the
///   face's coverage — name the limit, never imply absence (the md-only-limit
///   pattern);
/// - the id is absent — name the nearest face-addressable ids, or say plainly
///   that the page carries none.
///
/// `None` for non-anchor selectors. The host-kind probe re-projects the toc
/// only on this error path, never on a served read.
fn anchor_sel_teach(
    doc: &model::Document,
    facts: &[wire_map::facts::ReadFact],
    sel: &wire::ReadSel,
) -> Option<String> {
    let wire::ReadSel::Anchor { anchor } = sel else {
        return None;
    };
    if let Some(row) = wire_map::project_toc(doc)
        .into_iter()
        .find(|r| r.anchor.as_deref() == Some(anchor.as_str()))
    {
        return Some(format!(
            "the anchor exists on this page, but its host block is a {} — outside the \
             kinds this read face addresses (plain list items only); read its enclosing \
             section by heading path instead",
            row.kind
        ));
    }
    let ids: Vec<String> = facts.iter().filter_map(|f| f.anchor.clone()).collect();
    if ids.is_empty() {
        return Some("this page carries no addressable block anchors".to_owned());
    }
    let shown: Vec<String> = model::selector::nearest(anchor, &ids)
        .iter()
        .take(model::selector::NEAREST_SHOWN)
        .map(|c| format!("^{c}"))
        .collect();
    Some(format!("nearest live block anchors: {}", shown.join(", ")))
}

/// A candidate's machine address — the verbatim `[{"h":…,"n":…}]` segment
/// array the read face publishes and `put`/`sections[]` take back.
fn machine_addr(hpath: &[wire::HpathSeg]) -> String {
    // hpathSeg is strings + ints; serialization is infallible.
    serde_json::to_string(hpath).unwrap_or_default()
}

/// The sections-mode leg of [`composed_read`]: selector resolution (first
/// match; partial-read notice), the walker-emitted content, and the rendered
/// text — refusal messages in the Go host face's verbatim spelling.
fn composed_sections(
    doc: &model::Document,
    facts: &[wire_map::facts::ReadFact],
    sels: &[wire::ReadSel],
    header: render::Header<'_>,
) -> Result<(SectionsRender, Vec<wire::ReadSectionOut>), Box<ErrorBody>> {
    let display = header.display_path;
    if sels.is_empty() {
        // Says what to pass, not what the caller "is in": a `#Fragment` is the
        // whole-call scope, not a member of `sections[]`.
        return Err(bad_request(format!(
            "read: a section read needs a selector, and none was given. Nothing was read \
             and no rev was minted. Fix: pass one or more section selectors (a heading \
             path, a dewey ordinal, or a ^anchor), or scope the whole read with a \
             `#Fragment` on the ref — or list this document's section paths with a toc \
             read of {display} (MCP read: mode:\"toc\"; CLI: no --section)."
        )));
    }
    let mut rows: Vec<render::SectionRow<'_>> = Vec::new();
    let mut failures: Vec<SelFail> = Vec::new();
    for sel in sels {
        let matches = wire_map::facts::selector_matches(facts, sel);
        match matches.as_slice() {
            &[fact] => rows.push(render::SectionRow { sel, fact }),
            [] => failures.push(SelFail::Miss {
                display: sel.display(),
                teach: anchor_sel_teach(doc, facts, sel),
            }),
            many => failures.push(match sel {
                // A duplicated block id has no per-candidate machine address
                // (anchor rows carry no hpath; the id is the shared spelling),
                // so its entry counts the carriers instead of listing
                // addresses (A.3, door symmetry over duplicate block ids).
                wire::ReadSel::Anchor { .. } => SelFail::DupAnchor {
                    display: sel.display(),
                    count: many.len(),
                },
                _ => SelFail::Ambiguous {
                    display: sel.display(),
                    candidates: many.iter().map(|f| machine_addr(&f.hpath)).collect(),
                },
            }),
        }
    }
    if rows.is_empty() && !failures.is_empty() {
        // A.3: the all-fail refusal names EVERY failed selector with its own
        // reason, symmetric with the partial-read notice below. All-ambiguous
        // is the fix-class `ambiguous_ref`; any miss keeps `ref_not_found`.
        let all_ambiguous = failures
            .iter()
            .all(|f| matches!(f, SelFail::Ambiguous { .. } | SelFail::DupAnchor { .. }));
        let mut e = ErrorBody::new(if all_ambiguous {
            ErrorCode::AmbiguousRef
        } else {
            ErrorCode::RefNotFound
        });
        let phrases: Vec<String> = failures.iter().map(SelFail::phrase).collect();
        e.message = Some(format!(
            "read: {} in {display}. Nothing was read and no rev was minted. {}",
            phrases.join("; "),
            crate::section_recovery(failures[0].display(), Some(display))
        ));
        return Err(Box::new(e));
    }
    let notice = (!failures.is_empty()).then(|| {
        let entries: Vec<String> = failures.iter().map(SelFail::notice_entry).collect();
        format!("unresolved selectors (no rev minted): {}", entries.join(", "))
    });
    let job = render::RenderJob::Sections {
        header,
        rows: &rows,
        notice: notice.as_deref(),
    };
    // The render face's production configuration — engine (`meridian-*`)
    // blocks are elided from rendered output (`rendered_text`) only.
    let rendered =
        render::Renderer::render(&render::ToonRenderer::with_meridian_elision(), doc, &job)
            .map_err(|e| {
                let mut err = ErrorBody::new(ErrorCode::Internal);
                err.message = Some(e.to_string());
                Box::new(err)
            })?;
    // `sections[].content` is the raw face — the verbatim bytes `sec_rev` was
    // minted over — so a put built from its content round-trips without
    // silently dropping an elided block. Elision lives in `rendered_text`
    // alone. `words` pairs with the raw content it describes.
    let sections: Vec<wire::ReadSectionOut> = rows
        .iter()
        .map(|row| {
            let content = wire_map::facts::section_content(row.fact, doc.raw.as_bytes());
            let content = String::from_utf8_lossy(&content).into_owned();
            let words = wire_map::gotext::fields_count(&content) as u64;
            wire::ReadSectionOut {
                sel: row.sel.clone(),
                hpath: row.fact.hpath.clone(),
                sec_rev: NodeRev(row.fact.sec_rev.clone()),
                words,
                content,
            }
        })
        .collect();
    Ok((
        SectionsRender {
            text: rendered.text,
            notice,
        },
        sections,
    ))
}

/// wire §10.2 the opt-in strictness guard for `links`: refuse when the caller
/// pinned a `require_root` the world no longer meets. Checked before the corpus
/// is built, so both hosts call this against the `as_of_root` they already
/// hold, before handing the built corpus to [`links`].
///
/// # Errors
/// `stale_view` when `require_root` is present and differs from `as_of_root`.
pub fn require_root_check(
    require_root: Option<&Root>,
    as_of_root: &Root,
) -> Result<(), Box<ErrorBody>> {
    if let Some(required) = require_root
        && required != as_of_root
    {
        let mut e = ErrorBody::new(ErrorCode::StaleView);
        e.required = Some(required.clone());
        e.as_of_root = Some(as_of_root.clone());
        // At the refusal point no live root was sampled; echo `as_of` for both
        // (wire §10.2 worked frame).
        e.live_root = Some(as_of_root.clone());
        return Err(Box::new(e));
    }
    Ok(())
}

/// wire §4.6 the corpus edge map under the §10.1 staleness triple, served from
/// `query` over the borrowed corpus. `as_of_root` folds the exact bytes the
/// answer parses; `live_root` is sampled after the computation — under a
/// concurrent write the two may differ, which is a legal frame, never an error
/// (§10.1: no lag bounds are promised). `live_root` is a closure so it is
/// sampled only on the success path. Call [`require_root_check`] before this.
///
/// # Errors
/// `path` names a file the corpus does not carry (`file_not_found`), or the
/// caller's `live_root` sample fails.
pub fn links(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    path: Option<&Path>,
    as_of_root: Root,
    changes_seq: u64,
    live_root: impl FnOnce() -> Result<Root, Box<ErrorBody>>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    if let Some(p) = path
        && !docs.contains_key(&p.0)
    {
        let mut e = ErrorBody::new(ErrorCode::FileNotFound);
        e.path = Some(p.clone());
        return Err(Box::new(e));
    }
    // `query::links`, never `links_rooted` with a default table: an empty
    // `MountSet` would turn "I did not consult a mount table" into "this
    // machine binds nothing". Rooted spellings come back `unresolved` here,
    // and the CLI degrades rather than serve that answer
    // (see `mrd::engine::answer_links`).
    let map = query::links(index, docs, path.map(|p| p.0.as_str()));
    let live = live_root()?;
    Ok(ResponseBody::Links {
        as_of_root,
        live_root: live,
        changes_seq,
        files: map.into_iter().map(|(p, e)| (p, into_wire(e))).collect(),
    })
}

/// [`links`] against a root-keyed corpus and a mount table — the cross-root
/// form.
///
/// The daemon does not call this: its warm state is one workspace corpus keyed
/// by one canonical path (`registry::warm_or_build`) and holds no mounted
/// corpora. The CLI refuses to let it answer a page that may carry a cross-root
/// position and degrades instead — see `mrd::engine::answer_links`.
///
/// # Errors
/// As [`links`].
#[allow(clippy::too_many_arguments)]
pub fn links_rooted(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
    path: Option<&Path>,
    as_of_root: Root,
    changes_seq: u64,
    live_root: impl FnOnce() -> Result<Root, Box<ErrorBody>>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    if let Some(p) = path
        && !docs.contains_key(&p.0)
    {
        let mut e = ErrorBody::new(ErrorCode::FileNotFound);
        e.path = Some(p.clone());
        return Err(Box::new(e));
    }
    let map = query::links_rooted(index, docs, corpus, mounts, path.map(|p| p.0.as_str()));
    let live = live_root()?;
    Ok(ResponseBody::Links {
        as_of_root,
        live_root: live,
        changes_seq,
        files: map.into_iter().map(|(p, e)| (p, into_wire(e))).collect(),
    })
}

/// One file's edges, as the wire carries them. Shared by both arms so the
/// answer's shape never depends on which of them served it.
fn into_wire(edges: query::FileLinks) -> wire::FileLinks {
    wire::FileLinks {
        resolved: edges.resolved,
        unresolved: edges.unresolved,
        resolved_rooted: edges
            .resolved_rooted
            .into_iter()
            .map(|(root, paths)| (root.to_string(), paths))
            .collect(),
        refused: edges
            .refused
            .into_iter()
            .map(|(link, edge)| {
                let rendered = render_refused_edge(&link, &edge);
                (link, rendered)
            })
            .collect(),
    }
}

/// Render one refused edge into its wire shape — the colour plane's verdict,
/// spelled in the colour plane's own words.
///
/// Nothing here re-classifies: the tone, the reason word and the detail come
/// from `model::selector`, the one owner of that vocabulary, and the teaching
/// refusal comes from the renderer that owns each class.
fn render_refused_edge(link: &str, edge: &query::RefusedEdge) -> wire::RefusedEdge {
    use model::selector::{Color, GreyReason, RedReason};

    // A spelling outside the address grammar has no colour — it is not an
    // address. It reuses `lock::LockError::BadRef`'s existing name for that
    // fact, and `AddrError`'s own `Display` is already a teaching refusal, so
    // it is carried verbatim.
    let color = match &edge.resolution {
        model::RefResolution::Malformed(err) => {
            return wire::RefusedEdge {
                color: "red".to_owned(),
                reason: "bad-ref".to_owned(),
                detail: None,
                message: err.to_string(),
                count: edge.count,
            };
        }
        model::RefResolution::Unmounted(root) => {
            Color::Grey(GreyReason::Unmounted { root: root.clone() })
        }
        model::RefResolution::PathUnseeable { root, path, detail } => {
            Color::Grey(GreyReason::PathUnseeable {
                root: root.clone(),
                path: path.clone(),
                detail: detail.clone(),
            })
        }
        model::RefResolution::NotFound {
            root: Some(root),
            path,
            selector,
        } => Color::Red(RedReason::FileNotFound {
            root: root.clone(),
            path: path.clone(),
            selector: selector.clone(),
        }),
        // Not refusals: `query::links_rooted` routes them to `unresolved` and
        // `resolved` respectively.
        model::RefResolution::Ambient(_)
        | model::RefResolution::Rooted { .. }
        | model::RefResolution::NotFound { root: None, .. } => {
            unreachable!("only refusals are carried in `refused`")
        }
    };

    wire::RefusedEdge {
        color: model::selector::color_tone(&color).to_owned(),
        reason: model::selector::color_reason(&color)
            .unwrap_or_default()
            .to_owned(),
        detail: model::selector::color_detail(&color),
        // The address as the page declared it, so a reader can find the string
        // on the page. The file-not-found renderer ignores it and joins its own
        // parts, so that class cannot name a root disagreeing with the root
        // that missed.
        message: model::selector::color_teaching(&color, link).unwrap_or_default(),
        count: edge.count,
    }
}

/// wire §4.5 the walk plane: best-effort app-compatible two-stage walk over the
/// corpus. Location facts only; `want_content` additionally returns the fragment
/// bytes, still no rev. The corpus here is the walk-plane superset (the caller
/// builds it skip-broken — the app indexes nothing it cannot read), which is a
/// different corpus from the §12 hash-domain fact plane [`links`] reads.
///
/// # Errors
/// The linkpath resolves to no file (`ref_not_found` stage 1) or the file is
/// found but the subpath is not (`ref_not_found` stage 2, with `dest`).
pub fn resolve(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    from: &Path,
    link: &str,
    want_content: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    match model::walk::walk(index, docs, &from.0, link) {
        Ok(loc) => {
            let content = want_content.then(|| docs[&loc.dest].raw[loc.span.clone()].to_string());
            Ok(ResponseBody::Resolve {
                dest: Path(loc.dest),
                span: Span(loc.span.start as u64, loc.span.end as u64),
                content,
            })
        }
        Err(miss) => {
            let mut e = ErrorBody::new(ErrorCode::RefNotFound);
            e.stage = Some(u32::from(miss.stage.number()));
            e.dest = miss.dest.map(Path);
            Err(Box::new(e))
        }
    }
}

/// The wire→model ref bridge (the crates never share a type — no-serde law).
/// `pub` because the host's write path (splice) resolves the same §2.1
/// targets through it — one bridge, not two.
///
/// The `@fp` strip is ordered here, and that ordering is the guarantee:
/// `model::Ref::anchor` is the one anchor-id mint guard, and this bridge is the
/// one place any wire `SecRef::Anchor` reaches it, so the strip cannot be
/// skipped by adding a put path. An `@` the shaped grammar does not recognize
/// survives into validation below and refuses `bad_request` — the block-id
/// charset has no `@`.
///
/// # Errors
/// An anchor id outside the block-id charset `[A-Za-z0-9-]` (`bad_request`).
pub fn to_model_ref(sec: &SecRef) -> Result<model::Ref, Box<ErrorBody>> {
    Ok(match sec {
        SecRef::Hpath { hpath } => model::Ref::Hpath(
            hpath
                .iter()
                .map(|s| model::HpathSeg {
                    h: s.h.clone(),
                    n: s.n,
                })
                .collect(),
        ),
        SecRef::Anchor { anchor } => {
            let (id, _fp) = syntax::split_fp(anchor);
            model::Ref::anchor(id.to_owned()).map_err(|bad| {
                bad_request(format!(
                    "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`",
                    id = bad.id
                ))
            })?
        }
        SecRef::FmKey { fm_key } => model::Ref::FmKey(fm_key.clone()),
    })
}

/// `ambiguous_ref` (§2.1: the strict plane never silently picks) — names each
/// duplicate by both addressable disambiguators: its node index (`n=`, the
/// §2.1 occurrence index) and its block id (`^block`) when it carries one.
/// `candidates` holds the machine-addressable `n=` forms. Duplicate block ids
/// share one spelling that cannot disambiguate them, so their `candidates`
/// stays `[]` — never prose inside the grammar field — and their message is
/// the anchor-plane refusal (count + the anchor-grammar remedy, never "rename
/// one heading"; A.3, door symmetry over duplicate block ids). `pub` because
/// the host's write path raises the same refusal against a splice target.
#[must_use]
pub fn ambiguous(sec: &SecRef, doc: &model::Document, candidates: &[model::Target]) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
    // The anchor plane: duplicate ids share one spelling and the anchor
    // grammar carries no occurrence index, so nothing machine-addressable
    // exists to name — `candidates` stays `[]` and the message counts the
    // carriers and teaches the anchor remedy, never "rename one heading"
    // (A.3, door symmetry over duplicate block ids).
    if let SecRef::Anchor { anchor } = sec {
        e.candidates = Some(Vec::new());
        e.message = Some(model::selector::render_anchor_ambiguity(
            &format!("^{anchor}"),
            candidates.len(),
        ));
        return e;
    }
    // One naming per duplicate: node index (1-based occurrence) + its ^block.
    let named: Vec<model::selector::AmbiguityCandidate> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| model::selector::AmbiguityCandidate {
            node_index: u32::try_from(i + 1).unwrap_or(u32::MAX),
            block: model::selector::first_anchor_in_span(doc, &c.span),
        })
        .collect();
    let display = match sec {
        SecRef::Hpath { hpath } => {
            // Machine-addressable `n=` forms, one per duplicate: the occurrence
            // index on the final segment resolves each uniquely (§2.1).
            e.candidates = Some(
                named
                    .iter()
                    .map(|c| {
                        let mut segs = hpath.clone();
                        if let Some(last) = segs.last_mut() {
                            last.n = Some(c.node_index);
                        }
                        SecRef::Hpath { hpath: segs }
                    })
                    .collect(),
            );
            hpath
                .iter()
                .map(|s| s.h.as_str())
                .collect::<Vec<_>>()
                .join("/")
        }
        SecRef::Anchor { .. } => unreachable!("the anchor arm returned above"),
        SecRef::FmKey { fm_key } => {
            e.candidates = Some(Vec::new());
            fm_key.clone()
        }
    };
    e.message = Some(model::selector::render_ambiguity(&display, &named));
    e
}

/// `ref_not_found` with a sentence — the sibling of [`ambiguous`] for the miss
/// case.
///
/// The hpath arm carries the one fact the read face cannot publish: heading
/// addresses are sanitized on the way out (`sanitize_heading`: `/` and U+0020
/// both become `-`) and `put` takes the raw heading text, so the map is
/// many-to-one. When the requested segments match some heading's sanitized
/// spelling, this names that heading's raw spelling — the address the write
/// actually needs.
#[must_use]
pub fn ref_not_found(sec: &SecRef, doc: &model::Document, display_path: &str) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::RefNotFound);
    let (display, recovery) = miss_parts(sec, doc, Some(display_path));
    e.message = Some(format!(
        "no node addressed by \"{display}\" in {display_path}. {} {recovery}",
        crate::NO_PARTIAL_WRITE_CLAUSE
    ));
    e
}

/// The strict READ miss — [`ref_not_found`]'s teaching with the read plane's
/// partial-state clause: `cat` applies no edit, so "the batch is refused
/// whole" would be the wrong voice. Before this arm existed, `cat` refused
/// with a bare code and no sentence — the one selector surface that failed
/// bare (Law A-3: a miss teaches before it refuses).
fn cat_miss(sec: &SecRef, doc: &model::Document) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::RefNotFound);
    let (display, recovery) = miss_parts(sec, doc, None);
    e.message = Some(format!(
        "no node addressed by \"{display}\". Nothing was read and no rev was minted. {recovery}"
    ));
    e
}

/// The display spelling and teaching clause for one missed [`SecRef`] —
/// shared by [`ref_not_found`] (write voice) and [`cat_miss`] (read voice),
/// which differ only in their partial-state clause. `display_path` is `None`
/// where the caller holds no path (`cat` serves one borrowed document).
fn miss_parts(
    sec: &SecRef,
    doc: &model::Document,
    display_path: Option<&str>,
) -> (String, String) {
    match sec {
        SecRef::Hpath { hpath } => {
            let asked = join_h(hpath);
            let recovery = raw_spelling_for(doc, hpath).map_or_else(
                || crate::section_recovery(&asked, display_path),
                |raw| {
                    format!(
                        "That IS this file's published address for a section, but `put` \
                         takes the RAW heading text, which `read` does not publish: \
                         address it as \"{raw}\"."
                    )
                },
            );
            (asked, recovery)
        }
        SecRef::Anchor { anchor } => {
            let asked = format!("^{anchor}");
            let recovery = format!(
                "{} {}",
                anchor_miss_teaching(doc, anchor),
                crate::section_recovery(&asked, display_path)
            );
            (asked, recovery)
        }
        SecRef::FmKey { fm_key } => (
            fm_key.clone(),
            "Fix: write the key with `at: upsert`, which creates it when absent; the \
             composed read's `props` plane lists the keys this page already has."
                .to_owned(),
        ),
    }
}

/// The strict plane resolves any parse-tree anchor regardless of host block
/// kind, so a miss here means the id is truly absent from the page. Teach the
/// nearest live ids — the same bigram rank the pin plane's dangling-anchor
/// hint uses — or say plainly that the page carries none, so nobody hunts a
/// page that has nothing to find (Law A-3: never fail bare).
fn anchor_miss_teaching(doc: &model::Document, id: &str) -> String {
    let live = model::selector::live_anchors(doc);
    if live.is_empty() {
        return "This page carries no block anchors.".to_owned();
    }
    let shown: Vec<String> = model::selector::nearest(id, &live)
        .iter()
        .take(model::selector::NEAREST_SHOWN)
        .map(|c| format!("^{c}"))
        .collect();
    format!("Nearest live block anchors: {}.", shown.join(", "))
}

/// The segments a caller asked for, in their own spelling.
fn join_h(hpath: &[wire::HpathSeg]) -> String {
    hpath
        .iter()
        .map(|s| s.h.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// The heading whose sanitized spelling is what the caller asked for, returned
/// in its raw spelling — `None` when nothing matches, or when the caller was
/// already raw-correct.
fn raw_spelling_for(doc: &model::Document, hpath: &[wire::HpathSeg]) -> Option<String> {
    let asked: Vec<String> = hpath
        .iter()
        .map(|s| wire_map::gotext::sanitize_heading(&s.h))
        .collect();
    for row in wire_map::project_toc(doc) {
        if row.kind.as_str() != "heading" {
            continue;
        }
        let segs = row.hpath.unwrap_or_default();
        let sanitized: Vec<String> = segs
            .iter()
            .map(|s| wire_map::gotext::sanitize_heading(&s.h))
            .collect();
        if sanitized == asked {
            let raw = join_h(&segs);
            if raw != join_h(hpath) {
                return Some(raw);
            }
        }
    }
    None
}

#[cfg(test)]
mod props_scalar_tests {
    //! § A.6 at the seam where the decode LIVES.
    //!
    //! The read half of the frontmatter scalar law was gated end-to-end at the
    //! script plane (`mrd/tests/a6_read_seam.rs`) because that is where the
    //! dogfood-season-1 incident was observed — a script comparing
    //! `card.fm["owner"]` against an id saw the stored quote bytes and silently
    //! matched nothing. It was never gated HERE, at [`read_props`], which is
    //! where the decode actually happens: `model::scalar` had unit tests for the
    //! codec, and nothing asserted the composed read applies it.
    //!
    //! So this is net-new coverage, not a relocation. A caller reading `props[]`
    //! is holding a VALUE, and every stored form below must arrive as one.

    use super::read_props;

    fn props_of(frontmatter: &str) -> Vec<(String, String)> {
        let raw = format!("---\n{frontmatter}---\n\n# Body\n");
        let doc = model::build(raw.clone(), syntax::parse(&raw));
        read_props(&doc)
            .into_iter()
            .map(|p| (p.key, p.value))
            .collect()
    }

    fn value_of(frontmatter: &str, key: &str) -> String {
        props_of(frontmatter)
            .into_iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("no prop {key} in {frontmatter:?}"))
            .1
    }

    /// The incident's own column: a double-quoted id reaches a reader as the
    /// id, not as the quoted source bytes.
    #[test]
    fn a_double_quoted_value_arrives_decoded() {
        assert_eq!(value_of("owner: \"3f9a1c07\"\n", "owner"), "3f9a1c07");
    }

    /// Single quotes decode too, including YAML's one escape (`''` → `'`).
    #[test]
    fn a_single_quoted_value_decodes_including_its_one_escape() {
        assert_eq!(value_of("owner: '3f9a1c07'\n", "owner"), "3f9a1c07");
        assert_eq!(value_of("note: 'it''s'\n", "note"), "it's");
    }

    /// A plain scalar is untouched — decoding is not parsing, and nothing here
    /// infers a type.
    #[test]
    fn plain_scalars_are_carried_through_unchanged() {
        assert_eq!(value_of("status: doing\n", "status"), "doing");
        assert_eq!(value_of("tags: [a, b]\n", "tags"), "[a, b]");
        assert_eq!(value_of("done: true\n", "done"), "true");
    }

    /// Malformed quoting is NOT repaired. A half-quoted value reaches the
    /// reader as it sits on disk, so a caller sees the corpus's real state
    /// rather than a guess about what was meant.
    #[test]
    fn malformed_quoting_reaches_the_reader_unchanged() {
        assert_eq!(value_of("owner: \"3f9a1c07\n", "owner"), "\"3f9a1c07");
    }

    /// The wikilink case the script plane's amendment names: a quoted wikilink
    /// arrives as the link text, so a comparison against the unquoted form
    /// matches.
    #[test]
    fn a_quoted_wikilink_arrives_as_its_link_text() {
        assert_eq!(value_of("owner: \"[[zt]]\"\n", "owner"), "[[zt]]");
    }

    /// Decoding is idempotent-unsafe by nature, which is why it happens exactly
    /// once and here: a value that is STILL quote-shaped after one decode must
    /// keep those quotes, or a second pass downstream would strip them.
    #[test]
    fn one_decode_and_only_one() {
        let once = value_of("owner: '\"quoted\"'\n", "owner");
        assert_eq!(once, "\"quoted\"", "the single quotes come off, once");
        assert_eq!(
            model::scalar::text(&once),
            "quoted",
            "a second decode WOULD strip again — proof the caller must not re-decode"
        );
    }
}
