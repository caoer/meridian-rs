//! The read-op arms over BORROWED parsed state — one implementation for both
//! hosts. Each arm takes already-built `model` state (a `&Document`, or the
//! `&CorpusIndex` + document map) plus the ambient root as data; the caller
//! obtains that state (the sidecar builds it per request from `fs`; the daemon
//! reuses its warm engine). Parsing is the caller's, projection is `wire-map`'s,
//! edges are `query`'s — these arms only wire model facts into wire bodies.

use std::collections::BTreeMap;

use wire::{ErrorBody, ErrorCode, NodeRev, Path, ResponseBody, Root, SecRef, Span};

use crate::bad_request;

/// **U12 — the stored form, read back into the agent plane.**
///
/// `put` translates an agent-plane `root:` address into the `obsidian://` stored
/// form ([`crate::write`]); this is the other direction, and it lands on the
/// **RENDERED face only** — exactly where the S10 `@fp` decoration lands, and
/// for the same reason.
///
/// **`sections[].content` and `cat.content` stay RAW and are deliberately NOT
/// translated.** They are *"the verbatim bytes `sec_rev` was minted over, so
/// each row is self-verifying"* (the op-owner ruling this arm already carries).
/// A translated content row would no longer hash to the rev shipped beside it,
/// which is the A-K1 class the raw face exists to prevent. The agent plane is a
/// VIEW of stored bytes; the raw face IS the stored bytes.
///
/// A URI naming a vault this machine does not bind is left verbatim — an
/// ordinary link a human wrote, not the engine's to claim. A URI naming a BOUND
/// vault IS the engine's, so a hand-edited one **fails loudly here** rather than
/// resolving to something plausible.
///
/// # Errors
/// `bad_request` naming the URI, the non-canonical part and the canonical
/// spelling, so the fix is one edit.
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

/// The stage-2 S10 claim-link decoration input, re-exported at the read seam so
/// a host wires it without taking a `render` dependency of its own.
pub use render::{ClaimLink, Decorations, NO_DECORATIONS};

/// wire §4.1 the map: header `file_rev` (the document root's rev over whole-file
/// bytes) + the ambient `root`, rows from the `wire-map` projection. `ambient`
/// is the corpus content-hash cursor the caller already holds (the sidecar folds
/// it from disk; the daemon reads its warm engine's fingerprint).
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
            model::ResolveError::NotFound => ErrorBody::new(ErrorCode::RefNotFound),
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
/// `enrich` (M1 U2, v3 sessions ONLY): attach the host-face addressing facts
/// — dewey `n` and subtree `words` — to heading nodes, so `extract` serves
/// every addressing fact ccc-statusd re-derived host-side. A v2 session never
/// enriches: the keys are v3-additive and the frozen v2 bytes stay
/// byte-identical (`contract_v2.rs`).
///
/// **U14 dropped `hpath_text` from this enrichment.** It was the sanitized
/// joined ADDRESS, and the node it decorated already carries `hpath` as
/// segments — the address that round-trips — so the string added no fact,
/// only a second spelling of one, lossy and un-invertible. Nothing is lost
/// here: a v3 `extract` still serves the whole address, structurally.
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

/// The composed-read parameters (M1 U4a2), decoded from the v3-only `read`
/// op — the host face's read-tool vocabulary, engine-side.
#[derive(Debug, Clone, Default)]
pub struct ReadParams {
    pub mode: Option<String>,
    /// The whole-call subtree scope, as SEGMENTS (U14).
    pub frag: Option<Vec<wire::HpathSeg>>,
    /// Document-absolute selectors in the tagged read grammar (U14).
    pub sections: Option<Vec<wire::ReadSel>>,
    pub display_path: Option<String>,
    /// §9 read provenance (D-Actor/B, review C4): the daemon-derived actor,
    /// carried to THIS seam — the stage-2 read-mint site (the one place
    /// holding the document, each `sec_rev`, and the actor together). M1 left
    /// it UNREAD; stage-2 S6 reads it here, additively, with no op re-shape.
    /// It stays DAEMON-derived and never MCP-caller-settable (D13): the wire
    /// `Op::Read.actor` field is not widened, and nothing below invents one.
    pub actor: Option<String>,
}

/// The `actor == None` no-mint door (D16), in ONE place.
///
/// A read mints a receipt only for a real daemon-derived identity. The bare
/// CLI sends no actor (`read_cmd.rs`: "§9 read provenance is the DAEMON's to
/// stamp") and is local-operator-trusted — exactly as `mrd put` skips the
/// host's authz — so it mints nothing and the pin gate is bypassed for it. A
/// blank actor is treated as absent too: an empty string is not an identity,
/// and admitting one would open a bucket every actor-less caller shares.
pub(crate) fn mint_actor(actor: Option<&str>) -> Option<&str> {
    actor.map(str::trim).filter(|a| !a.is_empty())
}

/// The COMPOSED read op (M1 U4a2, decision D6): addressing + content +
/// render served from ONE borrowed document snapshot — `file_rev`, the
/// ambient `root`, the toc/sections facts, and the `readText` projection in
/// one exchange. Refusal messages are the Go host face's VERBATIM strings,
/// so the thin proxy (U8a) forwards `error.message` without re-minting.
///
/// `mint` is the stage-2 S6 read-is-the-mint ledger (D9): present only for a
/// host that HOLDS a daemon-session layer (the registry daemon). A host with
/// no session — the per-request sidecar, the bare CLI — passes `None` and
/// mints nothing, which is honest: there is no session for a receipt to live
/// in. Minting is SECTIONS-mode only; see [`composed_read`]'s mint block.
///
/// `decorations` is the stage-2 S10 claim-link input (D10), built by the caller
/// ([`page_decorations`]) and passed through to the renderer as data. It rides
/// the render HEADER, so it is an input to the read as a whole rather than to a
/// mode, and [`NO_DECORATIONS`] is the ONE spelling of "nothing to decorate" —
/// a host with no corpus cannot color a cross-page pin and says so by passing
/// it. Decoration lands in `rendered_text` ALONE: `sections[].content` stays
/// the raw face a put is built from, because a read-decorated view feeding a
/// write is the A-K1 data-loss class.
///
/// # Errors
/// `bad_request` (fix): `frag` and `sections` both present; sections mode
/// with no selectors; an unknown `mode` (decode already gates it — this is
/// the belt). `ref_not_found` (fix): a toc `frag` naming no section; ALL
/// sections-mode selectors missing. `internal` carrying the typed
/// `render_failed` spelling (G1) when the walker refuses.
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
    let has_sections = params.sections.as_ref().is_some_and(|s| !s.is_empty());
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
        words_total,
        decorations,
    };
    // The `^id` ANCHOR plane (stage-2 s1c) — computed once, emitted by BOTH
    // modes, scoped by the same `frag` that scopes the whole call. It rides
    // its own array because a consumer iterating `toc` must not be able to
    // meet a second row class: S1 mixed the two and ccc-statusd's `readText`
    // panicked on an anchor row's `depth 0` ("negative Repeat count").
    let anchors: Vec<wire::ReadAnchor> = wire_map::facts::anchor_rows(&facts, frag)
        .iter()
        .filter_map(|f| read_anchor(f))
        .collect();

    match params.mode.as_deref().unwrap_or("toc") {
        "toc" => {
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
            // ONE row set for the heading plane: `rendered_text` renders these
            // rows and `toc` carries these rows, so the captured Go toc bytes
            // stay frozen and the structured face never diverges from the
            // rendered one. The authz facts (`span`, `content_span`) ride the
            // same rows; the anchor plane is `anchors`.
            let rendered_text = agent_plane_face(render::toc_toon(&header, &rows))?;
            Ok(ResponseBody::Read {
                path: path.clone(),
                file_rev: NodeRev(file_rev),
                root: ambient.clone(),
                words_total,
                toc: Some(rows.iter().map(|f| read_row(f)).collect()),
                anchors,
                sections: None,
                truncated: None,
                notice: None,
                rendered_text,
            })
        }
        "sections" => {
            let sels: Vec<wire::ReadSel> = if frag.is_empty() {
                params.sections.clone().unwrap_or_default()
            } else {
                vec![wire::ReadSel::Hpath {
                    hpath: frag.to_vec(),
                }]
            };
            let (body, rendered_sections) = composed_sections(doc, &facts, &sels, header)?;
            // S6 read-IS-the-mint (D9): one receipt per section this call
            // actually served — actor, canonical selector, `sec_rev`.
            //
            // SECTIONS mode only, and off the RAW rows: `rendered_sections`
            // are the `sections[].content` rows whose bytes the caller
            // received verbatim, so each receipt is a true "this was in your
            // context" fact. Two things follow deliberately. A toc-mode read
            // mints NOTHING — it serves the section map, not content, so it
            // must never gate an attestation over bytes nobody saw. And the
            // rev bound is the raw face's `sec_rev`, never anything derived
            // from the elided `rendered_text` — a read-elided view must never
            // be what a write is authorized against (the A-K1 data-loss
            // class).
            //
            // Unresolved selectors are absent from these rows (they carry the
            // PARTIAL-read notice instead), so a miss mints nothing.
            if let (Some(store), Some(actor)) = (mint, mint_actor(params.actor.as_deref())) {
                // U14: the receipt is keyed on the row's own TAGGED selector,
                // the same structure the pin gate looks up — so the two sides
                // of this coupling cannot drift into two spellings of one
                // address, and a heading named `1.2` can no longer open the
                // gate for the dewey row of that name.
                for row in &rendered_sections {
                    store.mint(actor, path.0.as_str(), &row.sel, &row.sec_rev.0);
                }
            }
            Ok(ResponseBody::Read {
                path: path.clone(),
                file_rev: NodeRev(file_rev),
                root: ambient.clone(),
                words_total,
                toc: None,
                anchors,
                sections: Some(rendered_sections),
                truncated: body.notice.is_some().then_some(true),
                notice: body.notice,
                rendered_text: agent_plane_face(body.text)?,
            })
        }
        other => Err(bad_request(format!("read: invalid mode \"{other}\""))),
    }
}

/// One heading fact → one wire composed-read row: the M1 addressing facts
/// plus the stage-2 S1 authz facts (`span`, `content_span`), carried verbatim
/// off the fact — the engine has ONE hpath owner
/// (`model::gotext::sanitize_heading`, through `wire_map::facts`), and this
/// seam never re-derives an address.
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

/// One anchor fact → one wire `anchors[]` entry (stage-2 s1c): the block id
/// and the block-leaf span, which is everything the host's containment join
/// consumes. `None` is unreachable (`read_facts` mints an anchor fact only
/// from an anchor-bearing `list_item`) and is DROPPED rather than serialized
/// as an empty id — an entry no caller can address is worse than no entry.
fn read_anchor(f: &wire_map::facts::ReadFact) -> Option<wire::ReadAnchor> {
    f.anchor.as_ref().map(|id| wire::ReadAnchor {
        anchor: id.clone(),
        span: f.span,
    })
}

/// The claim-link decorations for ONE page (stage-2 S10): every `meridian-lock`
/// pin the page declares, matched to the links in the page's own body that
/// address it, carrying the shaped `@fp` token that pin's drift color mints.
///
/// This is the CALLER side of the render seam — deliberately outside `render`,
/// which never reads a lock and never computes a fingerprint (D10). It needs
/// the corpus, so only a host that holds one calls it (the registry daemon);
/// the per-request sidecar and the bare CLI pass [`render::NO_DECORATIONS`].
///
/// Four rules, each closing a way a decoration could lie:
///
/// - **The color is S9's, per pin.** [`model::selector::classify_pin`] is the
///   single compare `view::walk::lock_pin_colors` uses, so a decorated link can
///   never disagree with the same pin's row in `mrd walk` / `mrd status`.
/// - **The handle is the promoted `^slug`, never the pin's identity.** A pin's
///   fingerprint covers the span its `ref` RESOLVES to (the section); an anchor
///   node's span is only its host line, so treating the slug as the identity
///   would read green on every body edit. Here the slug is only a lookup key.
/// - **The link must actually address the pinned page.** The body's spelling
///   (`guide`) and the lock's (`guide.md`) are two spellings of one address, so
///   both go through the ONE linkpath resolver before they are called a match.
/// - **A pin it cannot honestly color is left undecorated** — an unresolvable
///   target page, a refused lock, a `Malformed` token with no digest to show.
///   An undecorated link claims nothing; a decorated one claims the ledger
///   measured it.
///
/// **D12 (U10):** the pinned page is resolved through the corpus index, never
/// by string surgery. The `root:` prefix no longer *rides inside a spelling
/// this function re-splits* — `lock` parsed the ref into an [`addr::Addr`] at
/// its own door, and this function READS the parts. It was FINDING 02's site
/// A3: the same `lock::find` bytes, re-split and re-parsed here.
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
    // `lock-refused` row, and a token minted from bytes nobody could parse
    // would be a claim nobody measured.
    let Ok(Some(found)) = lock::find(doc) else {
        return out;
    };
    let mut links: Vec<(&String, &String)> = Vec::new();
    collect_links(&doc.root, &mut links);
    for pin in &found.lock.pins {
        // R4 arms, and what each can honestly decorate. `path` segments map
        // STRAIGHT across to `Selector::Heading` — array to array, nothing
        // joined and nothing re-parsed, which is the whole point of the machine
        // surface carrying structure.
        let lock::Selector::Path(segments) = &pin.selector else {
            // The `properties` arm claims frontmatter keys. `Selector` has no
            // member for that, and frontmatter carries no `[[page#^block]]`
            // link to decorate either — so there is nothing here to colour, not
            // a mapping left undone.
            continue;
        };
        if segments.is_empty() {
            continue; // `path: []` is the whole body — no block handle to decorate
        }
        let Some(target_path) = resolve_page(index, docs, &pin.object, path) else {
            continue;
        };
        // R4's anchor form: a path array whose SOLE element is a `^id`. Decoded
        // here exactly as `write::pin_row` encodes it, and the sole-element rule
        // is what keeps the two planes apart — a mixed array is refused at the
        // write door, so a `^`-leading element inside a longer chain never
        // reaches this face to be read as either grain.
        let selector = match segments.as_slice() {
            [only] if only.starts_with('^') => {
                model::selector::Selector::Block(only[1..].to_string())
            }
            _ => model::selector::Selector::Heading(segments.clone()),
        };
        // The handle a pin's target carries: for a section pin, the D15 slug
        // S7's promotion mints from the heading title — computed by the SAME
        // owner, so a decoration keys on the id promotion actually wrote. An
        // anchor pin IS its own handle and needs no promotion.
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

/// Resolve a link/ref target spelling to a corpus path through the ONE address
/// owner ([`model::CorpusIndex::resolve_ref`]) — the same three rules, in the
/// same order, that `view::read_face::resolve_to_path` gives the walk plane. An
/// empty spelling is the page itself (a self-link `[[#^blk]]`).
///
/// It holds no precedence of its own on purpose. When it did, it lacked the
/// key + `.md` rule the walk plane had, so a ref `a/b` in a corpus carrying both
/// `a/b.md` and `b.md` decorated a link with a tone minted over `b.md` while the
/// walk colored `a/b.md` — one pin, two documents, two answers.
///
/// **U11 — this face carries no mount table yet.** It resolves against the
/// AMBIENT root alone, so a `root:`-bearing address answers `None` (unresolved)
/// rather than the ambient root's same-basename file. That is FINDING 03's fix
/// reaching this plane: an unresolved decoration is first-class here, and a
/// wrong one is not. Per-item VERDICTS are not this surface's to give (plan
/// §9.1: its only colour channel is daemon-only, carries tone but no reason, and
/// is skipped exactly in the unresolvable-target case) — `mrd walk` is where the
/// unmounted grey is rendered with its reason.
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

/// The sections-mode leg of [`composed_read`]: selector resolution (FIRST
/// match; PARTIAL-read notice), the walker-emitted content, and the rendered
/// text — refusal messages in the Go host face's verbatim spelling.
fn composed_sections(
    doc: &model::Document,
    facts: &[wire_map::facts::ReadFact],
    sels: &[wire::ReadSel],
    header: render::Header<'_>,
) -> Result<(SectionsRender, Vec<wire::ReadSectionOut>), Box<ErrorBody>> {
    let display = header.display_path;
    if sels.is_empty() {
        // Says what to PASS, not what the caller "is in": the reader who lands
        // here selected no mode — `--section` implied one — so naming the mode
        // back at them describes an internal state they never chose (issue-05).
        // A `#Fragment` is the WHOLE-call scope, not a member of `sections[]`;
        // the old wording called it a sections-mode selector, which is exactly
        // what it is not (gaps § 3.3).
        return Err(bad_request(format!(
            "read: a section read needs a selector, and none was given. Nothing was read \
             and no rev was minted. Fix: pass one or more section selectors (a heading \
             path, a dewey ordinal, or a ^anchor), or scope the whole read with a \
             `#Fragment` on the ref — or run `mrd read {display}` with no --section to \
             list this document's section paths."
        )));
    }
    let mut rows: Vec<render::SectionRow<'_>> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for sel in sels {
        match wire_map::facts::resolve_selector(facts, sel) {
            Some(fact) => rows.push(render::SectionRow { sel, fact }),
            None => missing.push(sel.display()),
        }
    }
    if rows.is_empty() && !missing.is_empty() {
        let mut e = ErrorBody::new(ErrorCode::RefNotFound);
        e.message = Some(format!(
            "read: no section addressed by \"{}\" in {display}. Nothing was read and no \
             rev was minted. {}",
            missing[0],
            crate::section_recovery(&missing[0], Some(display))
        ));
        return Err(Box::new(e));
    }
    let notice = (!missing.is_empty()).then(|| {
        format!(
            "unresolved selectors (no rev minted): {}",
            missing.join(", ")
        )
    });
    let job = render::RenderJob::Sections {
        header,
        rows: &rows,
        notice: notice.as_deref(),
    };
    // U4b: the render face's production configuration — engine (`meridian-*`)
    // blocks are elided from RENDERED output (`rendered_text`) only.
    let rendered =
        render::Renderer::render(&render::ToonRenderer::with_meridian_elision(), doc, &job)
            .map_err(|e| {
                let mut err = ErrorBody::new(ErrorCode::Internal);
                err.message = Some(e.to_string());
                Box::new(err)
            })?;
    // Op-owner ruling (2026-07-24, D concurring, pin #4): `sections[].content`
    // is the RAW face — the verbatim bytes `sec_rev` was minted over — so each
    // row is self-verifying and a put built from its content round-trips
    // without silently dropping an elided block (the A-K1 data-loss class).
    // Elision lives in `rendered_text` alone; the composed op carries content
    // and render as DISTINCT legs (D6). `words` pairs with the raw content it
    // describes (Go `renderSectionsSidecar` semantics, golden structured
    // parity).
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
/// pinned a `require_root` the world no longer meets — retryable, never silent.
/// Checked BEFORE the corpus is built (the sidecar's timing: refuse a stale view
/// without paying the parse), so both hosts call this against the `as_of_root`
/// they already hold, before handing the built corpus to [`links`].
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
/// `query` over the borrowed corpus. `as_of_root` folds the EXACT bytes the
/// answer parses; `live_root` is sampled AFTER the computation — under a
/// concurrent write the two may differ, which is a legal frame, never an error
/// (§10.1: no lag bounds are promised). `live_root` is a closure so it is
/// sampled only on the success path (a `file_not_found` never pays a second
/// fold). Call [`require_root_check`] before this — the §10.2 refusal is
/// checked before the corpus is built, so it is not this arm's.
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
    // **`query::links`, never `links_rooted` with a default table.** An empty
    // `MountSet` is a machine that binds NOTHING, and the address plane will
    // rightly answer `grey(unmounted)` against it. This caller has not consulted
    // a mount table at all, so handing one in would turn "I did not look" into
    // "this machine binds nothing" — a claim it has no standing to make, and on
    // any multi-root machine a false one. Rooted spellings therefore come back
    // `unresolved` here, exactly as before U21, and the CLI degrades rather
    // than serve that answer (see `mrd::engine::answer_links`).
    let map = query::links(index, docs, path.map(|p| p.0.as_str()));
    let live = live_root()?;
    Ok(ResponseBody::Links {
        as_of_root,
        live_root: live,
        changes_seq,
        files: map.into_iter().map(|(p, e)| (p, into_wire(e))).collect(),
    })
}

/// [`links`] against a ROOT-KEYED corpus and a mount table — the cross-root
/// form (U21).
///
/// **The daemon does not call this, and that asymmetry is stated rather than
/// smoothed.** The resident engine's warm state is ONE workspace corpus keyed
/// by one canonical path (`registry::warm_or_build`); it holds no mounted
/// corpora, so for this one op it is knowingly LESS CAPABLE than the
/// in-process path. The CLI therefore refuses to let it answer a page that may
/// carry a cross-root position and degrades instead — see
/// `mrd::engine::answer_links`. Making the daemon root-aware end to end means N
/// mounted corpora per workspace with per-root fingerprint invalidation,
/// residency and reap: a designed subsystem, recorded as the named successor,
/// not a wiring change.
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
/// answer's SHAPE never depends on which of them served it — only its content
/// does, and only where the two genuinely know different things.
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
/// Nothing here re-classifies. The tone, the reason word and the detail come
/// from `model::selector`, which is the one owner of that vocabulary, and the
/// teaching refusal comes from the renderer that owns each class. A `match` of
/// my own over `RefResolution` producing words would be a second answer waiting
/// to disagree with the walk plane about one address.
fn render_refused_edge(link: &str, edge: &query::RefusedEdge) -> wire::RefusedEdge {
    use model::selector::{Color, GreyReason, RedReason};

    // A spelling outside the address grammar has no COLOUR — nothing was
    // measured and nothing was declined; it is not an address. It reuses
    // `lock::LockError::BadRef`'s existing name for that fact rather than
    // minting a synonym, and `AddrError`'s own `Display` is already a
    // four-property teaching refusal, so it is carried verbatim.
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
        // An ambient miss and a resolution are not refusals and never reach
        // this map — `query::links_rooted` routes them to `unresolved` and
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
        // The address as the PAGE declared it — the refusal echoes what the
        // author wrote, not what resolution made of it, so a reader can find
        // the string on the page. The file-not-found renderer ignores it and
        // joins its own parts, which is the one way that class cannot name a
        // root disagreeing with the root that missed.
        message: model::selector::color_teaching(&color, link).unwrap_or_default(),
        count: edge.count,
    }
}

/// wire §4.5 the walk plane: best-effort app-compatible two-stage walk over the
/// corpus. Location facts only; `want_content` additionally returns the fragment
/// bytes, still no rev. The corpus here is the walk-plane SUPERSET (the caller
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
/// The anchor form re-passes the mint-guard; the strict decode already refused
/// out-of-charset ids, so this is the belt to that suspender. `pub` because the
/// sidecar's write path (splice) resolves the same §2.1 targets through it — one
/// bridge, not two.
///
/// **Stage-2 S10 — the `@fp` strip is ORDERED HERE, and that ordering is the
/// guarantee.** `model::Ref::anchor` is this workspace's ONE anchor-id mint
/// guard, and this bridge is the ONE place any wire `SecRef::Anchor` reaches
/// it — every read arm and every write arm (native edits and the lowered plan
/// batch alike) funnels through this function. So the strip cannot be skipped
/// by adding a put path: a path that skips it has no `model::Ref` to write
/// with. A well-formed agent-plane address (`^leaders-guideline@green.b3af12cd`)
/// therefore addresses exactly what its stored spelling addresses, while an
/// `@` the shaped grammar does NOT recognize survives into validation below and
/// refuses `bad_request` — the block-id charset has no `@`.
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

/// `ambiguous_ref` (§2.1: the strict plane never silently picks) — the U2.2
/// refuse-ambiguous-only refusal, naming EACH duplicate by both addressable
/// disambiguators the teaching refusal offers: its node index (`n=`, the §2.1
/// occurrence index) and its block id (`^block`) when it carries one. `candidates`
/// holds the machine-addressable `n=` forms (hpath duplicates are nameable
/// exactly by occurrence index on the final segment; duplicate block ids share
/// one id that cannot disambiguate them, so their `candidates` stays type-level
/// EMPTY — `[]`, never prose inside the grammar field). `message` carries the d1
/// teaching refusal verbatim (both spellings interpolated, "Unambiguous writes to
/// this file remain served"). `pub` because the sidecar's write path raises the
/// same refusal against a splice target — one spelling of it, shared.
#[must_use]
pub fn ambiguous(sec: &SecRef, doc: &model::Document, candidates: &[model::Target]) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
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
        SecRef::Anchor { anchor } => {
            // Duplicate block ids share one id — it cannot disambiguate them, so
            // `candidates` stays EMPTY (no exact §2.1 spelling per target); the
            // message names them by node index (d1: "by ... node index").
            e.candidates = Some(Vec::new());
            format!("^{anchor}")
        }
        SecRef::FmKey { fm_key } => {
            e.candidates = Some(Vec::new());
            fm_key.clone()
        }
    };
    e.message = Some(model::selector::render_ambiguity(&display, &named));
    e
}

/// `ref_not_found` with a SENTENCE — the sibling of [`ambiguous`] for the miss
/// case. Both refusals answer "your target did not resolve"; only one of them
/// used to say anything, so a `put` against a stale address printed the bare
/// code `ref_not_found` and named no file, no target, and no way forward
/// (issue-08, the worst message the 08-01 dogfood recorded).
///
/// The hpath arm carries the one fact the read face cannot publish. Heading
/// addresses are SANITIZED on the way out (`sanitize_heading`: `/` and U+0020
/// both become `-`) and `put` takes the RAW heading text, so the map is
/// many-to-one and no caller can recover the pre-image from a `read`. When the
/// requested segments match some heading's sanitized spelling, this names that
/// heading's RAW spelling — the address the write actually needs.
#[must_use]
pub fn ref_not_found(sec: &SecRef, doc: &model::Document, display_path: &str) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::RefNotFound);
    let (display, recovery) = match sec {
        SecRef::Hpath { hpath } => {
            let asked = join_h(hpath);
            let recovery = raw_spelling_for(doc, hpath).map_or_else(
                || crate::section_recovery(&asked, Some(display_path)),
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
            let recovery = crate::section_recovery(&asked, Some(display_path));
            (asked, recovery)
        }
        SecRef::FmKey { fm_key } => (
            fm_key.clone(),
            // Names ONLY the door that was probed open. An earlier draft sent the
            // reader to `mrd read --json` for the key list — but the composed
            // read body carries no frontmatter plane at all
            // (`anchors`/`file_rev`/`fingerprint`/`path`/`rendered_text`/`toc`/
            // `words_total`), so that clause was this card's own `takeover`: a
            // refusal pointing at a door that does not open. `at: upsert` is
            // verified to create an absent key; the page's own frontmatter block
            // is where the existing keys are.
            "Fix: write the key with `at: upsert`, which creates it when absent; the \
             page's own frontmatter block lists the keys it already has."
                .to_owned(),
        ),
    };
    e.message = Some(format!(
        "no node addressed by \"{display}\" in {display_path}. {} {recovery}",
        crate::NO_PARTIAL_WRITE_CLAUSE
    ));
    e
}

/// The segments a caller asked for, in their own spelling.
fn join_h(hpath: &[wire::HpathSeg]) -> String {
    hpath
        .iter()
        .map(|s| s.h.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// The heading whose SANITIZED spelling is what the caller asked for, returned
/// in its RAW spelling — `None` when nothing matches, or when the caller was
/// already raw-correct (then the address is not the sanitize trap and naming it
/// back would only restate the request).
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
