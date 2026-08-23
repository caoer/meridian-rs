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

/// The read plane's own size ceiling: the words ONE sections call may serve.
///
/// Counted in WORDS, not bytes, because the face already speaks words — the
/// banner's `words_total` and every toc row's `words:N` — so a caller reads
/// the cost of a section before asking for it, and the ceiling is
/// discoverable BEFORE it refuses (laws.md § the face-honesty law, clause 2).
/// A byte bound would be invisible until tripped.
///
/// The number is a product knob (leader call, 2026-08-15, card
/// `read-budget-refusal-missing`): the bound exists to fire before the MCP
/// host clips the result, and hosts in this fleet clip tool output near ~25k
/// tokens — roughly 18–19k English words — so 20 000 is the largest round
/// bound that still beats the clip in the common case. Tunable in one line.
pub const READ_MAX_WORDS: u64 = 20_000;

/// The § A.8 fan-out ceiling every face list carries, at the read door: the
/// DISTINCT selectors one `sections[]` call may name. Reused, not invented —
/// the same 64 the run plane's `targets[]` carries.
pub const READ_MAX_SELECTORS: usize = 64;

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
    /// The whole-call subtree scope: ONE tagged selector, resolved to one
    /// section — a heading path or a dewey ordinal; the anchor arm refuses
    /// (a block has no subtree). Mutually exclusive with `sections`.
    pub toc: Option<wire::ReadSel>,
    /// Document-absolute selectors in the tagged read grammar — and the mode
    /// itself: non-empty selects sections, absent/empty answers the toc.
    /// Nothing else picks the arm.
    pub sections: Option<Vec<wire::ReadSel>>,
    pub display_path: Option<String>,
}

/// The composed read op: addressing + content + render served from one
/// borrowed document snapshot — `file_rev`, the ambient `root`, the
/// toc/sections facts, and the rendered projection in one exchange. Refusal
/// messages are the Go host face's verbatim strings, so the thin proxy
/// forwards `error.message` without re-minting.
///
/// A read is identity-free and side-effect-free (§ A.3 proof law): it mints
/// nothing and records nothing, on every host alike. What a sections-mode
/// read serves instead is each section's own `fp1.…` fingerprint — the
/// proof token a later `splice.pin` of that section carries back.
///
/// `decorations` is the claim-link input, built by the caller
/// ([`page_decorations`]) and passed through to the renderer as data;
/// [`NO_DECORATIONS`] is the one spelling of "nothing to decorate". Decoration
/// lands in `rendered_text` alone: `sections[].content` stays the raw face a
/// put is built from, because a read-decorated view feeding a write is a
/// data-loss class.
///
/// # Errors
/// `bad_request` (fix): `toc` and `sections` both present ("pass one"); a
/// `toc` anchor arm (a block has no subtree); a section read with no
/// selectors; a section read past either of this plane's bounds — more than
/// [`READ_MAX_SELECTORS`] distinct selectors, or a resolved set that would
/// serve more than [`READ_MAX_WORDS`] words. `ref_not_found` (fix): a `toc` selector naming no section; all
/// section selectors missing. `ambiguous_ref` (fix): a `toc` selector
/// matching more than one section. `internal` carrying the typed
/// `render_failed` spelling when the walker refuses.
pub fn composed_read(
    doc: &model::Document,
    path: &Path,
    ambient: &Root,
    params: &ReadParams,
    decorations: &Decorations,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let facts = wire_map::facts::read_facts(&wire_map::project_toc(doc), doc.raw.as_bytes());
    let file_rev = doc.root.node_rev.0.clone();
    // The file's own word count, off its raw bytes — never a sum of the toc
    // rows. A row's `words` is subtree-inclusive, so summing rows counts every
    // descendant once per ancestor level: the ~2x banner a reader budgets
    // against (D-USER r2 F3). One counter: `facts::words_total`.
    let words_total: u64 = wire_map::facts::words_total(doc.raw.as_bytes());
    let display = params.display_path.as_deref().unwrap_or(path.0.as_str());
    // `sections`'s presence is the mode. Absent → the toc read; present → a
    // section read, empty included, so `sections: []` still meets the
    // "a section read needs a selector" refusal instead of answering a toc.
    let has_sections = params.sections.is_some();
    if params.toc.is_some() && has_sections {
        return Err(bad_request(format!(
            "read: toc and sections[] were both passed for {display} — pass one: toc \
             answers a subtree's MAP, sections[] answers chosen sections' CONTENT, and \
             one call answers one question. Nothing was read and no rev was minted. \
             Fix: keep toc for the scoped shape table, or keep sections[] for the \
             content."
        )));
    }
    // The scope resolves BEFORE either mode serves: an anchor arm, a miss, or
    // a duplicate refuses here, so no plane is consulted under a scope that
    // does not name exactly one section.
    let scope_fact = match &params.toc {
        None => None,
        Some(sel) => Some(resolve_toc_scope(&facts, sel, display)?),
    };
    let scope = scope_fact.map(|f| &f.span);
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
    // The `^id` anchor plane — computed once, emitted by both modes, bounded
    // by the same resolved scope that bounds the whole call. Its own array: a
    // mixed-in anchor row's `depth 0` once crashed a client renderer.
    let anchors: Vec<wire::ReadAnchor> = wire_map::facts::anchor_rows(&facts, scope)
        .iter()
        .filter_map(|f| read_anchor(f))
        .collect();
    // The frontmatter-properties plane — document-grain, so unlike `anchors`
    // it is never `toc`-scoped: frontmatter belongs to the document, not to
    // any subtree (wire-contract § A.3).
    let props = read_props(doc);

    if has_sections {
        let sels: Vec<wire::ReadSel> = params.sections.clone().unwrap_or_default();
        let (body, rendered_sections) = composed_sections(doc, &facts, &sels, header)?;
        return Ok(ResponseBody::Read {
            path: path.clone(),
            file_rev: NodeRev(file_rev),
            root: ambient.clone(),
            words_total,
            toc: None,
            anchors,
            props,
            sections: Some(rendered_sections),
            // `truncated` means rows are MISSING from the answer, which is
            // the unresolved plane alone. A collapsed repeat is carried by
            // the notice beside it and is not a truncation: every distinct
            // selector the caller named is served.
            truncated: (!body.unresolved.is_empty()).then_some(true),
            notice: body.notice,
            unresolved: body.unresolved,
            rendered_text: agent_plane_face(body.text)?,
        });
    }
    // A resolved scope always yields rows (its own section is a row), so the
    // old empty-scope arm is gone: a selector naming nothing already refused
    // at resolve_toc_scope, in its own lane's words.
    let rows = wire_map::facts::toc_rows(&facts, scope);
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
        unresolved: Vec::new(),
        rendered_text,
    })
}

/// The toc-scope resolver: ONE selector → exactly one heading row, or the
/// refusal that names what actually went wrong in its own lane's words.
///
/// The tagged grammar killed the season-1 lane confusion at the type: the
/// caller STATES hpath or dewey, so a miss is a miss of the lane that ran,
/// never a `^id` dressed as literal heading text. The anchor arm refuses
/// before any resolution — a block has no subtree, so no scope exists for it
/// to name — and a duplicate heading refuses with each candidate's machine
/// address instead of silently merging the siblings' subtrees, the same
/// never-silently-picks law the sections plane holds (§2.1).
fn resolve_toc_scope<'a>(
    facts: &'a [wire_map::facts::ReadFact],
    sel: &wire::ReadSel,
    display: &str,
) -> Result<&'a wire_map::facts::ReadFact, Box<ErrorBody>> {
    if let wire::ReadSel::Anchor { .. } = sel {
        let asked = sel.display();
        return Err(bad_request(format!(
            "read: toc:\"{asked}\" cannot scope the shape table — a block has no \
             subtree, so there is no map under it. Nothing was read and no rev was \
             minted. Fix: read the block's content with sections:[\"{asked}\"], or \
             scope the toc with a heading path or a dewey ordinal from a bare read \
             of {display}."
        )));
    }
    match wire_map::facts::selector_matches(facts, sel).as_slice() {
        &[fact] => Ok(fact),
        [] => {
            let mut e = ErrorBody::new(ErrorCode::RefNotFound);
            e.message = Some(toc_miss_message(sel, display));
            Err(Box::new(e))
        }
        many => {
            // The sections plane's own ambiguity spelling and its published
            // remedy (§2.1 never-silently-picks; AMBIGUITY_FIX byte-shared
            // with the write door) — one voice for one failure across faces.
            let candidates: Vec<Vec<wire::HpathSeg>> =
                many.iter().map(|f| f.hpath.clone()).collect();
            let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
            e.message = Some(format!(
                "read: toc \"{}\" is ambiguous ({} matches: {}) in {display}. Nothing \
                 was read and no rev was minted. {}",
                sel.display(),
                candidates.len(),
                candidate_addrs(&candidates).join(" or "),
                model::selector::AMBIGUITY_FIX
            ));
            Err(Box::new(e))
        }
    }
}

/// The toc-scope miss, per lane. The heading arm keeps the standing
/// section-miss spelling byte-for-byte (the discovery recovery teaches the
/// ROOT-ANCHORED law and the nearest candidate); the dewey arm is honest
/// about its own plane — ordinals are positional, so the remedy is the toc
/// that lists them, never a candidate search over names.
fn toc_miss_message(sel: &wire::ReadSel, display: &str) -> String {
    let asked = sel.display();
    match sel {
        wire::ReadSel::Dewey { .. } => format!(
            "read: no section at \"{asked}\" in {display} — dewey ordinals are the toc's \
             own first column, positional and re-minted per read, so an ordinal the \
             current toc does not list addresses nothing. Nothing was read and no rev \
             was minted. Fix: read {display} bare (no toc, no sections) and take the \
             ordinal — or the hpath — from the table it answers."
        ),
        _ => format!(
            "read: no section at \"{asked}\" in {display}. Nothing was read and no \
             rev was minted. {}",
            crate::section_recovery(&asked, Some(display))
        ),
    }
}

/// One section selector → the cat door's `SecRef`, resolving the dewey lane
/// through the SAME `selector_matches` the composed read uses (one
/// resolution, every door — read alignment, script-effects ruling
/// 2026-08-13). Hpath and anchor pass through; a dewey ordinal resolves to
/// its row's hpath, refusing on a miss or an ambiguity in the composed
/// read's own words.
///
/// # Errors
/// The refusal phrase, ready for a host's typed fault.
pub fn selector_to_secref(doc: &model::Document, sel: &wire::ReadSel) -> Result<SecRef, String> {
    match sel {
        wire::ReadSel::Hpath { hpath } => Ok(SecRef::Hpath {
            hpath: hpath.clone(),
        }),
        wire::ReadSel::Anchor { anchor } => Ok(SecRef::Anchor {
            anchor: anchor.clone(),
        }),
        wire::ReadSel::Dewey { .. } => {
            let facts =
                wire_map::facts::read_facts(&wire_map::project_toc(doc), doc.raw.as_bytes());
            let matches = wire_map::facts::selector_matches(&facts, sel);
            match matches.as_slice() {
                &[fact] => Ok(SecRef::Hpath {
                    hpath: fact.hpath.clone(),
                }),
                [] => Err(format!("no section addressed by \"{}\"", sel.display())),
                many => Err(format!(
                    "\"{}\" is ambiguous ({} matches)",
                    sel.display(),
                    many.len()
                )),
            }
        }
    }
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

/// One anchor fact → one wire `anchors[]` entry: the block id, the block-leaf
/// span the host's containment join consumes, and the leaf's CAS token — the
/// §4.2 "anchors with their revs" half of the complete write kit (W-2: a host
/// autofills a rev-less block write from this, the same way it does from a
/// heading row's `sec_rev`). `None` is unreachable (`read_facts` mints an
/// anchor fact only from an anchor-bearing row) and is dropped rather than
/// serialized as an empty id.
fn read_anchor(f: &wire_map::facts::ReadFact) -> Option<wire::ReadAnchor> {
    f.anchor.as_ref().map(|id| wire::ReadAnchor {
        anchor: id.clone(),
        span: f.span,
        rev: NodeRev(f.sec_rev.clone()),
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
///
/// **A block scalar is the one shape this decode must NOT touch** (§ A.6.1a):
/// the model map already holds its folded/literal text, and `scalar::decode`
/// opens with `value.trim()`, which would eat the trailing newline clip
/// chomping produced, the leading newline a leading blank produced, and the
/// leading spaces an explicit indent indicator preserved. `model::fm_publish`
/// owns that branch for every seam that publishes a value.
///
/// The § A.3 props plane of one document — `read_props` published for the
/// § A.7 in-process serve, which builds the script toc face from the same
/// arms the composed read serves (one § A.6 decode, one spelling per lane).
#[must_use]
pub fn props_of(doc: &model::Document) -> Vec<wire::ReadProp> {
    read_props(doc)
}

/// The composed read's own whole-file word count, published for the § A.7
/// in-process serve — the same `facts::words_total` derivation
/// `composed_read` serves, so the script face's `words` agrees across lanes.
#[must_use]
pub fn words_of(doc: &model::Document) -> usize {
    usize::try_from(wire_map::facts::words_total(doc.raw.as_bytes())).unwrap_or(usize::MAX)
}

/// The rows themselves — see `props_of` above for the plane's contract.
fn read_props(doc: &model::Document) -> Vec<wire::ReadProp> {
    let Some(map) = frontmatter_map(&doc.root) else {
        return Vec::new();
    };
    // The frontmatter BLOCK, not just the map: a block scalar's shape is a
    // fact about the key LINE, and only the block still carries it.
    let block = frontmatter_span(&doc.root)
        .and_then(|span| doc.raw.get(span))
        .unwrap_or_default();
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
                // One owner for "decode, or already decoded" (§ A.6.1a).
                value: model::fm_publish(block, key, value),
                span: Span(target.span.start as u64, target.span.end as u64),
                prop_rev: NodeRev(target.node_rev.0),
            }
        })
        .collect()
}

/// The document's frontmatter node's raw span, if any — the bytes a value seam
/// needs to tell a block scalar from a colon remainder.
fn frontmatter_span(node: &model::Node) -> Option<std::ops::Range<usize>> {
    if matches!(node.kind, model::NodeKind::Frontmatter { .. }) {
        return Some(node.span.clone());
    }
    node.children.iter().find_map(frontmatter_span)
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
pub fn page_decorations(index: &model::CorpusIndex, docs: &model::Docs, path: &str) -> Decorations {
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
        // reaches this face. Heading segments are read through the R4
        // occurrence spelling (`"Dup#2"` → `n: Some(2)`), the same door the
        // walk plane uses.
        let selector = match segments.as_slice() {
            [only] if only.starts_with('^') => {
                model::selector::Selector::Block(only[1..].to_string())
            }
            _ => model::selector::Selector::Heading(
                segments
                    .iter()
                    .map(|seg| {
                        let (h, n) = lock::parse_occurrence(seg);
                        model::HpathSeg {
                            h: h.to_string(),
                            n,
                        }
                    })
                    .collect(),
            ),
        };
        // The handle a pin's target carries: for a section pin, the id the
        // promotion mints from the heading title and occurrence — computed by
        // the same owner ([`crate::write::occurrence_slug`]), so a decoration
        // keys on what id promotion actually wrote. An anchor pin is its own
        // handle.
        let handle = match &selector {
            model::selector::Selector::Block(id) => Some(id.clone()),
            model::selector::Selector::Heading(segs) => segs
                .last()
                .and_then(|s| crate::write::occurrence_slug(&s.h, s.n).ok()),
            _ => None,
        };
        let Some(handle) = handle else {
            continue;
        };
        let color = model::selector::classify_pin(
            &selector,
            &pin.fingerprint,
            docs.get(&target_path).map(|d| &**d),
        );
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
    docs: &model::Docs,
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
    /// The unresolved plane (wire-contract § A.3): the machine tense of
    /// `notice`, built from the same [`SelFail`] set as the prose.
    unresolved: Vec<wire::ReadUnresolved>,
}

/// One failed section selector with its honest reason (wire-contract A.3): a
/// miss (an anchor miss carries its Law A-3 teaching clause), or an ambiguity
/// carrying each candidate's machine address. One value, three tenses: the
/// all-fail [`Self::phrase`], the notice [`Self::notice_entry`], and the
/// structured [`Self::row`] — derived from one resolution pass so they
/// cannot disagree.
enum SelFail {
    Miss {
        sel: wire::ReadSel,
        /// The `^id` or heading-path teaching ([`anchor_sel_teach`]); `None`
        /// on dewey misses, whose teaching is the aggregate recovery clause.
        teach: Option<AnchorTeach>,
    },
    Ambiguous {
        sel: wire::ReadSel,
        candidates: Vec<Vec<wire::HpathSeg>>,
    },
    /// A duplicated block id: >1 carrier, but unlike [`SelFail::Ambiguous`] no
    /// machine address exists per candidate — duplicate ids share one spelling
    /// and the anchor grammar has no occurrence index — so the entry counts
    /// the carriers and teaches the anchor remedy (wire-contract A.3, door
    /// symmetry over duplicate block ids).
    DupAnchor { sel: wire::ReadSel, count: usize },
}

impl SelFail {
    fn sel(&self) -> &wire::ReadSel {
        match self {
            SelFail::Miss { sel, .. }
            | SelFail::Ambiguous { sel, .. }
            | SelFail::DupAnchor { sel, .. } => sel,
        }
    }

    fn display(&self) -> String {
        self.sel().display()
    }

    /// The selector's row on the `unresolved` plane — same facts as the two
    /// prose tenses, in the § A.3 row shape.
    fn row(&self) -> wire::ReadUnresolved {
        let bare = |reason| wire::ReadUnresolved {
            sel: self.sel().clone(),
            reason,
            candidates: Vec::new(),
            count: None,
            host: None,
            nearest: Vec::new(),
        };
        match self {
            SelFail::Miss { teach: None, .. } => bare(wire::UnresolvedReason::NoMatch),
            SelFail::Miss {
                teach: Some(teach), ..
            } => match &teach.host {
                Some(kind) => wire::ReadUnresolved {
                    host: Some(kind.clone()),
                    ..bare(wire::UnresolvedReason::UnaddressableHost)
                },
                None => wire::ReadUnresolved {
                    nearest: teach.nearest.clone(),
                    ..bare(wire::UnresolvedReason::NoMatch)
                },
            },
            SelFail::Ambiguous { candidates, .. } => wire::ReadUnresolved {
                candidates: candidates.clone(),
                ..bare(wire::UnresolvedReason::Ambiguous)
            },
            SelFail::DupAnchor { count, .. } => wire::ReadUnresolved {
                count: Some(*count as u64),
                ..bare(wire::UnresolvedReason::DuplicateAnchor)
            },
        }
    }

    /// The all-fail refusal's per-selector clause. The miss arm keeps the
    /// standing single-miss spelling byte-by-byte as its prefix — an anchor
    /// miss appends its teaching rather than reshaping the sentence; the
    /// ambiguous arm never says "no section addressed" — two sections matched,
    /// and the honest answer names both and how to pin one (dogfood F4).
    fn phrase(&self) -> String {
        let display = self.display();
        match self {
            SelFail::Miss { teach: None, .. } => format!("no section addressed by \"{display}\""),
            SelFail::Miss {
                teach: Some(teach), ..
            } => format!("no section addressed by \"{display}\" ({})", teach.clause),
            // The parenthetical carries FACTS; the remedy is the message's own
            // `Fix:`, from the one published constant per plane. It used to
            // carry the remedy here and let a discovery clause take the `Fix:`
            // label — two clauses disagreeing about what to do next, with the
            // authoritative-looking one circular.
            SelFail::Ambiguous { candidates, .. } => format!(
                "\"{display}\" is ambiguous ({} matches: {})",
                candidates.len(),
                candidate_addrs(candidates).join(" or ")
            ),
            SelFail::DupAnchor { count, .. } => {
                format!("\"{display}\" is ambiguous ({count} blocks carry this id)")
            }
        }
    }

    /// The partial-read notice's per-selector entry — same facts as
    /// [`SelFail::phrase`], in the notice's established bare-selector shape.
    fn notice_entry(&self) -> String {
        let display = self.display();
        match self {
            SelFail::Miss { teach: None, .. } => display,
            SelFail::Miss {
                teach: Some(teach), ..
            } => format!("{display} ({})", teach.clause),
            SelFail::Ambiguous { candidates, .. } => format!(
                "{display} (ambiguous, {} matches: {})",
                candidates.len(),
                candidate_addrs(candidates).join(" or ")
            ),
            SelFail::DupAnchor { count, .. } => {
                format!("{display} (ambiguous, {count} blocks carry this id)")
            }
        }
    }
}

/// The prose spelling of an ambiguity's candidate list — each machine
/// address rendered verbatim, refusal order.
fn candidate_addrs(candidates: &[Vec<wire::HpathSeg>]) -> Vec<String> {
    candidates.iter().map(|c| machine_addr(c)).collect()
}

/// One `^id` miss teaching: the parenthetical clause the refusal carries,
/// plus the unaddressable HOST kind when the id exists on the page — the
/// all-fail Fix branches on it, because `anchors[]` excludes frontmatter
/// hosts and cannot list such an id (dogfood P2-c: a Fix must be servable).
struct AnchorTeach {
    clause: String,
    /// `Some(kind)` = the id exists but its host is outside the face's anchor
    /// plane; `None` = the id is absent from the page.
    host: Option<String>,
    /// Absent-id arm only: the nearest live ids over the whole-page pool,
    /// host kinds included — the `unresolved` row's `nearest` and the prose
    /// clause render these same rows (§ A.3).
    nearest: Vec<wire::ReadNearestAnchor>,
}

/// Heading-lane miss teaching (dogfood r3 gap 6a — parity with the anchor
/// lane's nearest offer). A heading path is ROOT-ANCHORED, so the two honest
/// near-misses each get their own clause, both measured off the toc the
/// engine already projected — never invented:
///
/// - the asked segments match a live path's TAIL — the caller held a real
///   heading without its ancestry; name the root-anchoring rule and the live
///   full path(s);
/// - otherwise — the nearest live heading paths, ranked by the leaf heading's
///   spelling (the same hint machinery the anchor lane rides).
///
/// `None` when the page has no headings — the aggregate recovery clause
/// stands alone there.
fn hpath_miss_teaching(doc: &model::Document, hpath: &[wire::HpathSeg]) -> Option<String> {
    if hpath.is_empty() {
        return None;
    }
    let live: Vec<Vec<wire::HpathSeg>> = wire_map::project_toc(doc)
        .into_iter()
        .filter(|row| row.kind.as_str() == "heading")
        .filter_map(|row| row.hpath)
        .collect();
    if live.is_empty() {
        return None;
    }
    // (a) Tail match on raw segment text: the asked path IS on the page,
    // just not anchored at the root heading.
    let tails: Vec<String> = live
        .iter()
        .filter(|segs| {
            segs.len() > hpath.len()
                && segs[segs.len() - hpath.len()..]
                    .iter()
                    .zip(hpath)
                    .all(|(seg, want)| seg.h == want.h)
        })
        .map(|segs| format!("\"{}\"", join_h(segs)))
        .take(model::selector::NEAREST_SHOWN)
        .collect();
    if !tails.is_empty() {
        return Some(format!(
            "a heading path is root-anchored — \"{}\" is on this page at {}; address it \
             by its full path from the root heading",
            join_h(hpath),
            tails.join(" or ")
        ));
    }
    // (b) Nearest by leaf heading text, rendered as full paths.
    let leaves: Vec<String> = live
        .iter()
        .filter_map(|segs| segs.last().map(|s| s.h.clone()))
        .collect();
    let want = &hpath[hpath.len() - 1].h;
    let mut shown: Vec<String> = Vec::new();
    for leaf in model::selector::nearest(want, &leaves) {
        for segs in &live {
            if segs.last().map(|s| s.h.as_str()) == Some(leaf.as_str()) {
                let rendered = format!("\"{}\"", join_h(segs));
                if !shown.contains(&rendered) {
                    shown.push(rendered);
                }
            }
        }
        if shown.len() >= model::selector::NEAREST_SHOWN {
            break;
        }
    }
    shown.truncate(model::selector::NEAREST_SHOWN);
    Some(format!("nearest live heading paths: {}", shown.join(", ")))
}

/// Face-scoped `^id` miss teaching (Law A-3: a miss teaches before it
/// refuses). The composed read resolves anchors against the face's anchor
/// plane, which since F-R4 carries every body-hosted block id Obsidian
/// addresses (paragraph, list item, task, callout, table, fence, heading) —
/// the one exclusion is a frontmatter caret, which is literal YAML, not a
/// block. A miss has two honest shapes and each gets its own clause:
///
/// - the id exists in the parse tree but its host is the frontmatter — name
///   the limit truthfully and the servable way in: the `props` plane, which
///   any composed read already serves (there is no enclosing section to
///   offer). Never imply absence (the md-only-limit pattern);
/// - the id is absent — name the nearest live ids, or say plainly that the
///   page carries none.
///
/// `None` for non-anchor selectors. The host-kind probe re-projects the toc
/// only on this error path, never on a served read.
fn anchor_sel_teach(doc: &model::Document, sel: &wire::ReadSel) -> Option<AnchorTeach> {
    // Heading-lane parity (gap 6a): an hpath miss carries the same measured
    // teaching the write door speaks — prose clause only, no wire rows.
    if let wire::ReadSel::Hpath { hpath } = sel {
        return hpath_miss_teaching(doc, hpath).map(|clause| AnchorTeach {
            clause,
            host: None,
            nearest: Vec::new(),
        });
    }
    let wire::ReadSel::Anchor { anchor } = sel else {
        return None;
    };
    let toc = wire_map::project_toc(doc);
    if let Some(row) = toc
        .iter()
        .find(|r| r.anchor.as_deref() == Some(anchor.as_str()))
    {
        let clause = if row.kind == "frontmatter" {
            "the anchor exists on this page, but its host is the frontmatter — a caret \
             tail is literal YAML there, not a block, and any composed read already \
             serves the frontmatter keys on its `props` plane"
                .to_owned()
        } else {
            // Unreachable while every body host is face-addressable — kept
            // truthful in case a kind ever leaves the plane again.
            format!(
                "the anchor exists on this page, but its host block is a {} — outside \
                 this read face's anchor plane; read its enclosing section by heading \
                 path instead",
                row.kind
            )
        };
        return Some(AnchorTeach {
            clause,
            host: Some(row.kind.clone()),
            nearest: Vec::new(),
        });
    }
    // The candidate pool spans every `^id` on the page, non-addressable hosts
    // included (§ A.3): the ids that would explain a near-miss typo must not
    // be excluded just because this face cannot serve them. First occurrence
    // wins on a duplicated id — the pool ranks spellings, not carriers.
    let mut pool: Vec<(String, String)> = Vec::new();
    for row in &toc {
        if let Some(id) = &row.anchor
            && !pool.iter().any(|(a, _)| a == id)
        {
            pool.push((id.clone(), row.kind.clone()));
        }
    }
    if pool.is_empty() {
        return Some(AnchorTeach {
            clause: "this page carries no block anchors".to_owned(),
            host: None,
            nearest: Vec::new(),
        });
    }
    let ids: Vec<String> = pool.iter().map(|(a, _)| a.clone()).collect();
    let nearest: Vec<wire::ReadNearestAnchor> = model::selector::nearest(anchor, &ids)
        .iter()
        .take(model::selector::NEAREST_SHOWN)
        .map(|c| wire::ReadNearestAnchor {
            anchor: c.clone(),
            kind: pool
                .iter()
                .find(|(a, _)| a == c)
                .map(|(_, k)| k.clone())
                .unwrap_or_default(),
        })
        .collect();
    let shown: Vec<String> = nearest.iter().map(nearest_teach).collect();
    Some(AnchorTeach {
        clause: format!("nearest live block anchors: {}", shown.join(", ")),
        host: None,
        nearest,
    })
}

/// One nearest candidate's prose spelling. A face-addressable id is offered
/// bare — since F-R4 that is every body host; a frontmatter caret carries
/// the gate and the servable way in (§ A.3: teach the gate, never imply
/// absence).
fn nearest_teach(row: &wire::ReadNearestAnchor) -> String {
    match row.kind.as_str() {
        "frontmatter" => format!(
            "^{} (frontmatter-hosted — its keys are on the `props` plane)",
            row.anchor
        ),
        _ => format!("^{}", row.anchor),
    }
}

/// The servable Fix for an anchor whose host is outside the face's anchor
/// plane — since F-R4, the frontmatter caret alone. The standing `^`
/// recovery points at `anchors[]`, where the very id this refusal reports is
/// absent by construction (dogfood P2-c): a frontmatter caret-tail gets the
/// `props` plane (it has no enclosing section); the defensive non-frontmatter
/// arm keeps the section-map lane should a kind ever leave the plane again.
fn unaddressable_fix(host: &str, display: &str) -> String {
    if host == "frontmatter" {
        format!(
            "Fix: frontmatter is document-grain — any composed read of {display} serves \
             every key on its `props` plane (CLI: `mrd read {display}` with no --section)."
        )
    } else {
        format!(
            "Fix: find the enclosing section's heading path with a toc read of {display} \
             (MCP read: sections[] omitted; CLI: `mrd read {display}` with no --section), \
             then read that section."
        )
    }
}

/// Collapse identical selectors, keeping first-occurrence order, and report
/// how many were dropped.
///
/// Identity is the selector's own serialized form: two selectors are the same
/// question only when they are the same spelling. Two DIFFERENT spellings that
/// land on one node stay two rows — the caller asked twice in two grammars and
/// each row carries its own `sel` back, so collapsing them would answer a
/// question that was not asked.
///
/// Keyed through a set rather than a linear scan so a pathological list is
/// `O(n log n)`: the count ceiling is applied to the DISTINCT set, so this
/// runs before anything bounds the input length.
fn dedupe_selectors(sels: &[wire::ReadSel]) -> (Vec<wire::ReadSel>, usize) {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut out: Vec<wire::ReadSel> = Vec::with_capacity(sels.len());
    for sel in sels {
        // Selectors are strings and ints; serialization is infallible.
        let key = serde_json::to_string(sel).unwrap_or_default();
        if seen.insert(key) {
            out.push(sel.clone());
        }
    }
    let repeats = sels.len() - out.len();
    (out, repeats)
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
///
/// The third element is the mint plane: one canonical receipt key
/// ([`wire_map::facts::canonical_sel`]) per served section, parallel to the
/// served rows — derived from the same resolved fact as the row it keys, so
/// the minted key and the served bytes cannot disagree.
#[allow(clippy::too_many_lines)]
fn composed_sections(
    doc: &model::Document,
    facts: &[wire_map::facts::ReadFact],
    sels: &[wire::ReadSel],
    header: render::Header<'_>,
) -> Result<(SectionsRender, Vec<wire::ReadSectionOut>), Box<ErrorBody>> {
    let display = header.display_path;
    if sels.is_empty() {
        // Says what to pass, not what the caller "is in": `toc` is the
        // whole-call scope, not a member of `sections[]`.
        return Err(bad_request(format!(
            "read: a section read needs a selector, and none was given. Nothing was read \
             and no rev was minted. Fix: pass one or more section selectors (a heading \
             path, a dewey ordinal, or a ^anchor), or scope the shape table with `toc` — \
             or list this document's section paths with a bare toc read of {display} \
             (MCP read: sections[] omitted; CLI: no --section)."
        )));
    }
    // Repeats first, ceiling second. An identical selector is one question
    // asked twice: it resolves to the same node, so its row, its bytes and its
    // rev are byte-identical — 65 copies is the r9 F1 receipt B waste, not 65
    // answers. Collapsing before counting also means a caller who repeats
    // themselves is never refused for a fan-out they never asked for.
    let (distinct, repeats) = dedupe_selectors(sels);
    if distinct.len() > READ_MAX_SELECTORS {
        return Err(bad_request(format!(
            "read: {} distinct section selectors were passed for {display} — past the \
             ceiling of {READ_MAX_SELECTORS} per call. Nothing was read and no rev was \
             minted.\n  → split the ask: pass at most {READ_MAX_SELECTORS} selectors per \
             read and repeat the call for the rest.",
            distinct.len()
        )));
    }
    let mut rows: Vec<render::SectionRow<'_>> = Vec::new();
    let mut failures: Vec<SelFail> = Vec::new();
    for sel in &distinct {
        let matches = wire_map::facts::selector_matches(facts, sel);
        match matches.as_slice() {
            &[fact] => rows.push(render::SectionRow { sel, fact }),
            [] => failures.push(SelFail::Miss {
                sel: sel.clone(),
                teach: anchor_sel_teach(doc, sel),
            }),
            many => failures.push(match sel {
                // A duplicated block id has no per-candidate machine address
                // (anchor rows carry no hpath; the id is the shared spelling),
                // so its entry counts the carriers instead of listing
                // addresses (A.3, door symmetry over duplicate block ids).
                wire::ReadSel::Anchor { .. } => SelFail::DupAnchor {
                    sel: sel.clone(),
                    count: many.len(),
                },
                _ => SelFail::Ambiguous {
                    sel: sel.clone(),
                    candidates: many.iter().map(|f| f.hpath.clone()).collect(),
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
        // The aggregate Fix follows the first failure, as before — and it is
        // chosen by that failure's KIND, because the remedy for a miss is not
        // the remedy for an ambiguity.
        //
        // `section_recovery` teaches DISCOVERY: how to find a selector that
        // exists. That is the remedy for a MISS. An ambiguity is the opposite
        // failure — the caller's selector resolved, twice — so a discovery
        // clause sends them to look up an address they already typed, and the
        // one actionable sentence in the message is not the one labelled
        // `Fix:`. Both ambiguity planes therefore answer with their own
        // published remedy, byte-shared with the exemplar the write door
        // renders, so the two doors cannot drift apart again.
        //
        // The unaddressable-host miss keeps its own arm: `anchors[]` cannot
        // list the id it just named (dogfood P2-c) — a Fix must be servable.
        let fix = match &failures[0] {
            SelFail::Miss {
                teach:
                    Some(AnchorTeach {
                        host: Some(host), ..
                    }),
                ..
            } => unaddressable_fix(host, display),
            SelFail::DupAnchor { .. } => model::selector::ANCHOR_AMBIGUITY_FIX.to_owned(),
            SelFail::Ambiguous { .. } => model::selector::AMBIGUITY_FIX.to_owned(),
            other @ SelFail::Miss { .. } => {
                crate::section_recovery(&other.display(), Some(display))
            }
        };
        e.message = Some(format!(
            "read: {} in {display}. Nothing was read and no rev was minted. {fix}",
            phrases.join("; "),
        ));
        return Err(Box::new(e));
    }
    // The size bound, measured on the rows this call actually resolved — so
    // the number the refusal names is the number the caller would have been
    // served, never an estimate off the whole file. Nested selections are
    // counted as served: a parent and its child both selected DO carry the
    // child's bytes twice.
    let served_words: u64 = rows
        .iter()
        .map(|row| wire_map::facts::section_words(row.fact, doc.raw.as_bytes()))
        .sum();
    if served_words > READ_MAX_WORDS {
        return Err(bad_request(format!(
            "read: this call would serve {served_words} words from {} section(s) of \
             {display} — past the read ceiling of {READ_MAX_WORDS} words per call: \
             refused, never truncated. Nothing was read and no rev was minted.\n  \
             → narrow the ask: a bare toc \
             read of {display} lists every section with its own `words:N`, so you can \
             pick the ones that fit; then pass those in `sections[]`, or scope the shape \
             table to one subtree first with `toc:`.",
            rows.len()
        )));
    }
    let notice = {
        let mut entries: Vec<String> = Vec::new();
        if !failures.is_empty() {
            let fails: Vec<String> = failures.iter().map(SelFail::notice_entry).collect();
            entries.push(format!(
                "unresolved selectors (no rev minted): {}",
                fails.join(", ")
            ));
        }
        // Marked, never silent: a collapsed repeat is a served answer that
        // does not match the shape of the ask, which is exactly what
        // face-honesty clause 1 exists to state out loud.
        if repeats > 0 {
            entries.push(format!(
                "collapsed {repeats} repeated selector(s): each section is served once — \
                 a repeat resolves to the same node, so its bytes and its rev are identical"
            ));
        }
        (!entries.is_empty()).then(|| entries.join("; "))
    };
    let unresolved: Vec<wire::ReadUnresolved> = failures.iter().map(SelFail::row).collect();
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
    // alone. `words` pairs with the raw content it describes. `fingerprint`
    // is the § A.3 proof half: the section's own `fp1.…` token, computed with
    // the same anchor removals the pin door's live recompute uses, so the
    // token a read serves is the token a pin compare accepts.
    let removals = syntax::anchor_removals(&doc.raw);
    let sections: Vec<wire::ReadSectionOut> = rows
        .iter()
        .map(|row| {
            let content = wire_map::facts::section_content(row.fact, doc.raw.as_bytes());
            let content = String::from_utf8_lossy(&content).into_owned();
            let words = wire_map::facts::section_words(row.fact, doc.raw.as_bytes());
            let span = usize::try_from(row.fact.span.0).unwrap_or(usize::MAX)
                ..usize::try_from(row.fact.span.1).unwrap_or(usize::MAX);
            let fingerprint = model::fingerprint::fingerprint_span(doc, &span, &removals)
                .map(model::fingerprint::Fingerprint::into_string)
                .ok();
            wire::ReadSectionOut {
                sel: row.sel.clone(),
                hpath: row.fact.hpath.clone(),
                sec_rev: NodeRev(row.fact.sec_rev.clone()),
                fingerprint,
                words,
                content,
            }
        })
        .collect();
    Ok((
        SectionsRender {
            text: rendered.text,
            notice,
            unresolved,
        },
        sections,
    ))
}

/// wire §10.2 the opt-in strictness guard for `links`: refuse when the caller
/// pinned a `require_root` the world no longer meets. Checked before the corpus
/// is built, so the host calls this against the `as_of_root` it already
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

/// The §52 per-file refusal for an UNSERVED corpus member (in the hash domain,
/// not UTF-8 — node-rev-merkle-spec §3 per-file degradation): the typed
/// `invalid_utf8` naming the member, its condition, and where its bytes stand.
/// One mint for every read face (`docs/laws.md` Law 3) — a face
/// that answers `file_not_found` for a member that exists on disk serves an
/// answers-miss, not the ruled degradation.
#[must_use]
pub fn unserved_refusal(path: &Path, condition: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::InvalidUtf8);
    e.path = Some(path.clone());
    e.message = Some(format!(
        "{} {condition} — the file serves no spans/nodes; its bytes stay under the root",
        path.0
    ));
    Box::new(e)
}

/// The links doors' shared miss split, resolved the way §12.1 binds a door that
/// was asked about ONE named path: a member answers from the corpus (`None`); an
/// unserved member is its per-file `invalid_utf8` ([`unserved_refusal`]); a real
/// file the domain excludes is LOADED and returned (`Some`), because corpus
/// residency is not an admission test; only a path with no file under the root
/// is `file_not_found`.
fn links_nonmember(
    root: &fs::WorkspaceRoot,
    docs: &model::Docs,
    unserved: &BTreeMap<String, String>,
    path: Option<&Path>,
) -> Result<Option<model::Document>, Box<ErrorBody>> {
    let Some(p) = path else { return Ok(None) };
    if docs.contains_key(&p.0) {
        return Ok(None);
    }
    if let Some(condition) = unserved.get(&p.0) {
        return Err(unserved_refusal(p, condition));
    }
    crate::load_doc(root, p).map(Some)
}

/// wire §4.6 the corpus edge map under the §10.1 staleness triple, served from
/// `query` over the borrowed corpus. `as_of_root` folds the exact bytes the
/// answer parses; `live_root` is sampled after the computation — under a
/// concurrent write the two may differ, which is a legal frame, never an error
/// (§10.1: no lag bounds are promised). `live_root` is a closure so it is
/// sampled only on the success path. Call [`require_root_check`] before this.
///
/// # Errors
/// `path` names an unserved member (per-file `invalid_utf8`, §52) or no file
/// under the workspace root (`file_not_found`), or the caller's `live_root`
/// sample fails. A real file OUTSIDE the hash domain is served, never refused
/// (§12.1: the door family binds every door the caller names a path at).
#[allow(clippy::too_many_arguments)]
pub fn links(
    root: &fs::WorkspaceRoot,
    index: &model::CorpusIndex,
    docs: &model::Docs,
    unserved: &BTreeMap<String, String>,
    path: Option<&Path>,
    as_of_root: Root,
    changes_seq: u64,
    live_root: impl FnOnce() -> Result<Root, Box<ErrorBody>>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let nonmember = links_nonmember(root, docs, unserved, path)?;
    // `query::links`, never `links_rooted` with a default table: an empty
    // `MountSet` would turn "I did not consult a mount table" into "this
    // machine binds nothing". Rooted spellings come back `unresolved` here,
    // and the CLI degrades rather than serve that answer
    // (see `mrd::engine::answer_links`).
    let map = match (&nonmember, path) {
        (Some(doc), Some(p)) => one_file_map(
            &query::file_links(index, &p.0, doc, &model::RootedCorpus::ambient(docs), None),
            p,
        ),
        _ => query::links(index, docs, path.map(|p| p.0.as_str())),
    };
    let live = live_root()?;
    let domain = fs::domain::Domain::load(root).unwrap_or_default();
    let probe = fs::domain::LinkTargetProbe::new(root, &domain);
    Ok(ResponseBody::Links {
        as_of_root,
        live_root: live,
        changes_seq,
        files: map
            .into_iter()
            .map(|(p, e)| (p, into_wire_with_reasons(&probe, e)))
            .collect(),
        excluded: excluded_members(root, docs, unserved, path),
    })
}

/// The domain-excluded population of an ENUMERATION: markdown under the root
/// that the corpus does not hold, so `files` is published beside what it left
/// out rather than as the whole vault (§4.6, §12.1 enumerator clause). Empty
/// for the NAMED form — a named path is served, so nothing was left out.
fn excluded_members(
    root: &fs::WorkspaceRoot,
    docs: &model::Docs,
    unserved: &BTreeMap<String, String>,
    path: Option<&Path>,
) -> Vec<String> {
    if path.is_some() {
        return Vec::new();
    }
    fs::walk(root)
        .unwrap_or_default()
        .iter()
        .filter_map(|rel| rel.to_str().map(str::to_owned))
        .filter(|rel| !docs.contains_key(rel) && !unserved.contains_key(rel))
        .collect()
}

/// The one-entry answer for a named page the corpus does not carry: its edges
/// resolve against the corpus it is not in, and it is the only file in the map
/// because the caller named it.
fn one_file_map(edges: &query::FileLinks, path: &Path) -> BTreeMap<String, query::FileLinks> {
    let mut map = BTreeMap::new();
    map.insert(path.0.clone(), edges.clone());
    map
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
    root: &fs::WorkspaceRoot,
    index: &model::CorpusIndex,
    docs: &model::Docs,
    unserved: &BTreeMap<String, String>,
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
    path: Option<&Path>,
    as_of_root: Root,
    changes_seq: u64,
    live_root: impl FnOnce() -> Result<Root, Box<ErrorBody>>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let nonmember = links_nonmember(root, docs, unserved, path)?;
    let map = match (&nonmember, path) {
        (Some(doc), Some(p)) => one_file_map(
            &query::file_links(index, &p.0, doc, corpus, Some(mounts)),
            p,
        ),
        _ => query::links_rooted(index, docs, corpus, mounts, path.map(|p| p.0.as_str())),
    };
    let live = live_root()?;
    let domain = fs::domain::Domain::load(root).unwrap_or_default();
    let probe = fs::domain::LinkTargetProbe::new(root, &domain);
    Ok(ResponseBody::Links {
        as_of_root,
        live_root: live,
        changes_seq,
        files: map
            .into_iter()
            .map(|(p, e)| (p, into_wire_with_reasons(&probe, e)))
            .collect(),
        excluded: excluded_members(root, docs, unserved, path),
    })
}

/// This file's unresolved edges that name a real out-of-domain file, keyed as
/// `unresolved` keys them (§4.6, session decision 0034).
///
/// The classification is `fs::domain::LinkTargetProbe` — the one mint the
/// `sql link` projection also asks through, so the two planes cannot name
/// one rule differently. Nothing is decided here. The probe is the caller's,
/// minted ONCE per serve: its lazy fallback index is a whole-tree walk, and a
/// per-file mint re-walked the tree for every file with a bare-name miss
/// (measured 571 walks ≈ 88 s on an 11.5k-file vault; `sql_op.rs` precedent).
fn unresolved_reasons(
    probe: &fs::domain::LinkTargetProbe<'_>,
    edges: &query::FileLinks,
) -> BTreeMap<String, String> {
    edges
        .unresolved
        .keys()
        .filter_map(|target| {
            probe
                .exclusion(target)
                .map(|why| (target.clone(), why.word().to_owned()))
        })
        .collect()
}

/// One file's edges, as the wire carries them. Shared by both arms so the
/// answer's shape never depends on which of them served it.
/// [`into_wire`] with the §4.6 exclusion reasons attached — the ONE mint both
/// link doors use, so the ambient and the rooted answer can never disagree
/// about why one edge is unresolved.
fn into_wire_with_reasons(
    probe: &fs::domain::LinkTargetProbe<'_>,
    edges: query::FileLinks,
) -> wire::FileLinks {
    let reasons = unresolved_reasons(probe, &edges);
    let mut out = into_wire(edges);
    out.unresolved_reason = reasons;
    out
}

fn into_wire(edges: query::FileLinks) -> wire::FileLinks {
    wire::FileLinks {
        unresolved_reason: BTreeMap::new(),
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
            root: Some(root.clone()),
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
    docs: &model::Docs,
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
fn miss_parts(sec: &SecRef, doc: &model::Document, display_path: Option<&str>) -> (String, String) {
    match sec {
        SecRef::Hpath { hpath } => {
            let asked = join_h(hpath);
            let recovery = raw_spelling_for(doc, hpath).map_or_else(
                || match hpath_miss_teaching(doc, hpath) {
                    Some(clause) => {
                        format!(
                            "{clause}. {}",
                            crate::section_recovery(&asked, display_path)
                        )
                    }
                    None => crate::section_recovery(&asked, display_path),
                },
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
            format!(
                "Fix: write the key with `at: upsert`, which creates it when absent. \
                 Upsert is a value-plane door (§ A.6.3a): its `text` is the VALUE alone \
                 and the engine composes the `{fm_key}: <value>` line itself, so a `text` \
                 that repeats the key writes it doubled. The composed read's `props` \
                 plane lists the keys this page already has."
            ),
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
    //! `card["fm"]["owner"]` against an id saw the stored quote bytes and silently
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
