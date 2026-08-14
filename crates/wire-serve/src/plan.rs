//! Lower `splice.plan_edits` to native wire edits (Go `buildSpliceEdit` /
//! `buildPropertyEdits` emulation). Byte-faithful to the deleted host arms so
//! downstream validate/CAS/armed/reparse behave identically by construction —
//! EXCEPT the three body doors (`append`, `replace_section`, `create`), whose
//! composition follows the splice-hygiene law instead (wire-contract § A.3,
//! N-1, 2026-08-12): exactly one blank line at every block and section
//! boundary the splice touches, and a list-item payload joins a trailing
//! list flush. The hygiene law supersedes byte-faithfulness at those doors.
//!
//! Two target planes, one fact table (door symmetry, wire-contract A.3): a
//! heading path resolves against the same `read_facts` rows the read face
//! publishes, and a `^id` block ref resolves against that table's ANCHOR plane
//! (W-2, D-B face gate a: every toc-listed anchor is readable AND writeable by
//! its id; F-R4 widened the plane to every body host Obsidian addresses). The
//! one host outside the face's anchor law — a frontmatter caret, literal YAML
//! — is unlisted on the read door and therefore unresolvable on this one:
//! never a wider write door than the read door beside it.

use wire::{Edit, EditShape, ErrorBody, ErrorCode, HpathSeg, NodeRev, PutAt, SecRef};

use crate::bad_request;

/// One resolved heading in the host-face index (`tocIndex` `tocNode` lift).
struct HeadingFacts {
    /// The published RAW address — carries `n` only where ambiguous, exactly as
    /// the read face publishes it. Rides verbatim into the native edit target.
    raw_hpath: Vec<HpathSeg>,
    /// Heading level (create child depth).
    level: u32,
    /// Full node span (subtree-inclusive).
    span: (usize, usize),
    /// Heading-excluded content span, when present.
    content_span: Option<(usize, usize)>,
    /// The node's live CAS token, same mint the read face publishes — the
    /// containment refusal folds it in when the caller's rev is also stale
    /// (one refusal, both facts; wire-contract §A.3 containment law).
    sec_rev: String,
}

/// One face-addressable block anchor (the fact table's anchor plane).
struct AnchorFacts {
    /// The block id, without the `^` marker.
    id: String,
    /// The block-leaf span (terminator-excluded, §1 leaf span law).
    span: (usize, usize),
}

/// Host-face put index: the read face's own address table + fm keys in order.
///
/// The index is the same [`wire_map::facts::read_facts`] table the read plane
/// resolves against, so one address grammar and one occurrence law span both
/// planes. (A sanitized-join key map made `# A/B` and `# A B` one key,
/// last-wins, so a write could land on the wrong section silently.)
struct PlanIndex {
    headings: Vec<HeadingFacts>,
    anchors: Vec<AnchorFacts>,
    /// Every `^id` the DOCUMENT carries, whatever its host — the kernel's
    /// anchor plane, wider than `anchors` (the face's). The difference is the
    /// host-excluded set — since F-R4, frontmatter carets alone — and a miss
    /// splits its teaching on it: an id that exists but is host-excluded is
    /// taught the `props` plane, an absent id is taught discovery (W-2
    /// acceptance: one message for both sent callers in a circle).
    doc_anchor_ids: Vec<String>,
    fm_keys: Vec<String>,
}

/// Why an address resolved to no single section.
enum Miss {
    NotFound,
    /// The address matched more than one section — it abstained on `n` where
    /// the document is ambiguous.
    Ambiguous(usize),
}

impl PlanIndex {
    fn new(doc: &model::Document) -> Self {
        let rows = wire_map::project_toc(doc);
        let fm_keys = rows
            .iter()
            .filter(|r| r.kind == "frontmatter")
            .flat_map(|r| r.keys.clone().unwrap_or_default())
            .collect();
        let mut headings = Vec::new();
        let mut anchors = Vec::new();
        for f in wire_map::facts::read_facts(&rows, doc.raw.as_bytes()) {
            if let Some(id) = &f.anchor {
                anchors.push(AnchorFacts {
                    id: id.clone(),
                    span: (span_usize(f.span.0), span_usize(f.span.1)),
                });
                continue;
            }
            if f.hpath.is_empty() {
                continue;
            }
            headings.push(HeadingFacts {
                raw_hpath: f.hpath,
                level: f.depth,
                span: (span_usize(f.span.0), span_usize(f.span.1)),
                content_span: f
                    .content_span
                    .map(|cs| (span_usize(cs.0), span_usize(cs.1))),
                sec_rev: f.sec_rev,
            });
        }
        let mut doc_anchor_ids = Vec::new();
        collect_anchor_ids(&doc.root, &mut doc_anchor_ids);
        PlanIndex {
            headings,
            anchors,
            doc_anchor_ids,
            fm_keys,
        }
    }

    /// Resolve a block id against the anchor plane — the same rows
    /// [`wire_map::facts::selector_matches`] answers the read door with, so
    /// the two doors give one answer for one id (A.3): unlisted (absent, or a
    /// host outside the anchor law) is a miss on both; duplicated is loud on
    /// both.
    fn anchor(&self, id: &str) -> Result<&AnchorFacts, Miss> {
        let mut hits = self.anchors.iter().filter(|a| a.id == id);
        match (hits.next(), hits.count()) {
            (None, _) => Err(Miss::NotFound),
            (Some(only), 0) => Ok(only),
            (Some(_), rest) => Err(Miss::Ambiguous(rest + 1)),
        }
    }

    /// Resolve an address to exactly one section, or say why not.
    ///
    /// The occurrence law is `model::resolve_hpath_node`'s, not the read face's
    /// first-wins: a selector segment with `n: None` demands uniqueness, and an
    /// ambiguous address refuses — the write plane never silently picks.
    fn get(&self, addr: &[HpathSeg]) -> Result<&HeadingFacts, Miss> {
        if addr.is_empty() {
            return Err(Miss::NotFound);
        }
        let mut hits = self.headings.iter().filter(|f| seg_chain_matches(addr, f));
        match (hits.next(), hits.count()) {
            (None, _) => Err(Miss::NotFound),
            (Some(only), 0) => Ok(only),
            (Some(_), rest) => Err(Miss::Ambiguous(rest + 1)),
        }
    }
}

/// Per-segment address equality against a published address: same length, raw
/// text byte-equal, and an occurrence that either abstains (`n: None`) or names
/// the section's own. A published `n: None` means "unique among its siblings",
/// i.e. occurrence 1 — so `n: Some(1)` against a unique heading matches, as it
/// does natively.
fn seg_chain_matches(addr: &[HpathSeg], f: &HeadingFacts) -> bool {
    addr.len() == f.raw_hpath.len()
        && addr.iter().zip(&f.raw_hpath).all(|(sel, pub_seg)| {
            sel.h == pub_seg.h && sel.n.is_none_or(|k| k == pub_seg.n.unwrap_or(1))
        })
}

/// The section-miss refusal, shared by every addressing arm.
///
/// The `^` arm steers the message only, never the resolution: an anchor-shaped
/// address is sent to `anchors[]` rather than to a section listing.
fn section_miss(addr: &[HpathSeg], miss: &Miss) -> Box<ErrorBody> {
    let shown = crate::display_hpath(addr);
    if let Miss::Ambiguous(n) = miss {
        return bad_request(format!(
            "address {} matches {n} sections. {} Fix: pass the occurrence — the read face \
             publishes it as `n` on the ambiguous segment.",
            policy::defs::go_quote(&shown),
            crate::NO_PARTIAL_WRITE_CLAUSE,
        ));
    }
    let anchor_shaped = block_ref(addr).is_some()
        || matches!(addr, [only] if only.h.starts_with('^') || only.h.starts_with("#^"));
    bad_request(format!(
        "no section addressed by {}. {} {}",
        policy::defs::go_quote(&shown),
        crate::NO_PARTIAL_WRITE_CLAUSE,
        crate::section_recovery(if anchor_shaped { "^" } else { "" }, None)
    ))
}

/// The block-ref arm of a plan address: a `^id` / `#^id` block is a SINGLE
/// segment by construction (a block id is `[A-Za-z0-9-]`, so it can carry no
/// path) — a multi-segment address is never one. Both spellings are the
/// pre-flight's (`check_write` `strip_fp_address`), so an address the
/// pre-flight resolves is never one the committer reads as a heading. The
/// `@fp` decoration peels here, the same `syntax::split_fp` the native door
/// applies in `read::to_model_ref` — a decorated ref resolves in both entry
/// points or neither.
fn block_ref(addr: &[HpathSeg]) -> Option<&str> {
    let [only] = addr else { return None };
    let id = only
        .h
        .strip_prefix("#^")
        .or_else(|| only.h.strip_prefix('^'))?;
    Some(syntax::split_fp(id).0)
}

/// Every anchor id the document carries, whatever the host kind — the walk
/// behind [`PlanIndex::doc_anchor_ids`].
fn collect_anchor_ids(node: &model::Node, out: &mut Vec<String>) {
    if let model::NodeKind::Anchor { name } = &node.kind {
        out.push(name.clone());
    }
    for child in &node.children {
        collect_anchor_ids(child, out);
    }
}

/// Resolve a block id for a write arm: the anchor-plane hit, or the arm's
/// refusal. A miss splits its teaching honestly (W-2 acceptance — one message
/// for both cases sent callers in a circle): an id the DOCUMENT carries but
/// the anchor plane excludes — since F-R4, a frontmatter caret alone (every
/// body host resolves) — is taught the `props` plane, the same answer the
/// read door's unaddressable-host teaching gives; an id the document does
/// not carry at all keeps the discovery miss (`section_miss`, whose `^` arm
/// points at the `anchors[]` plane where listed ids actually live). A
/// duplicate speaks the anchor-plane ambiguity voice (A.3 door symmetry over
/// duplicate block ids: count the carriers, teach the anchor remedy, name no
/// candidates — nothing machine-addressable exists), and an id outside the
/// one §2.4 charset refuses at the mint-guard exactly as the native decode
/// door does.
fn resolve_block<'a>(
    idx: &'a PlanIndex,
    addr: &[HpathSeg],
    id: &str,
) -> Result<&'a AnchorFacts, Box<ErrorBody>> {
    if model::Ref::anchor(id.to_owned()).is_err() {
        return Err(bad_request(format!(
            "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`"
        )));
    }
    idx.anchor(id).map_err(|miss| match miss {
        Miss::NotFound if idx.doc_anchor_ids.iter().any(|a| a == id) => bad_request(format!(
            "no section addressed by {shown}. {clause} Fix: `^{id}` exists in this \
                 document, but its host is the frontmatter — a caret there is literal \
                 YAML, not a block; frontmatter keys are written through the `props` \
                 plane, not a block ref.",
            shown = policy::defs::go_quote(&crate::display_hpath(addr)),
            clause = crate::NO_PARTIAL_WRITE_CLAUSE,
        )),
        Miss::NotFound => section_miss(addr, &Miss::NotFound),
        Miss::Ambiguous(n) => {
            let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
            e.candidates = Some(Vec::new());
            e.message = Some(model::selector::render_anchor_ambiguity(
                &format!("^{id}"),
                n,
            ));
            Box::new(e)
        }
    })
}

/// Wire `u64` spans → `usize` checked (never lossy `as`; saturated miss hits Go bounds guards).
fn span_usize(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// One lowered plan batch: native edits + index-aligned birth annotations.
/// `born[i]` is `Some(Born)` exactly when edit `i` lowered from a `create`
/// row — the armed-fact builder reports the BORN section for those (the
/// engine's armed-fact law, wire-contract § A.3 create door); every other
/// edit arms its own target.
#[derive(Debug)]
pub struct Lowered {
    pub edits: Vec<Edit>,
    pub born: Vec<Option<Born>>,
}

/// A `create` row's birth annotation: the born title plus where its heading
/// line starts INSIDE the lowered edit's text. The hygiene composition makes
/// the leading bytes variable (separators are derived from the document, and
/// a boundary needing surgery lowers to a content rewrite), so the armed-fact
/// builder can no longer assume "one `\n`, then the heading" — the lowering
/// states the offset instead of the reader guessing it.
#[derive(Debug, Clone)]
pub struct Born {
    pub title: String,
    /// Byte offset of the `#` opening the born heading, within the lowered
    /// edit's `text`.
    pub heading_offset: usize,
}

/// Lower one plan-level batch to native edits: properties first as one group,
/// then body ops in request order. Returned batch feeds the native splice path.
///
/// # Errors
/// First failing plan edit: `bad_request` teaching (host arms minus `put: ` prefix).
pub fn lower(
    doc: &model::Document,
    plan_edits: &[wire::PlanEdit],
) -> Result<Lowered, Box<ErrorBody>> {
    let idx = PlanIndex::new(doc);
    let raw = doc.raw.as_bytes();

    // Properties first (host order); last value per key wins; keys sorted.
    let mut props: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for e in plan_edits {
        if let wire::PlanEdit::SetProperty { key, value, .. } = e {
            props.insert(key, value);
        }
    }
    let mut edits = if props.is_empty() {
        Vec::new()
    } else {
        lower_property_group(doc, &idx, &props)?
    };
    let mut born: Vec<Option<Born>> = vec![None; edits.len()];

    for e in plan_edits {
        match e {
            wire::PlanEdit::SetProperty { .. } => {}
            wire::PlanEdit::Append { hpath, body, rev } => {
                edits.push(lower_append(&idx, doc, hpath, body, rev.as_deref())?);
                born.push(None);
            }
            wire::PlanEdit::Match {
                hpath,
                old,
                new,
                all,
                rev,
            } => {
                edits.push(lower_match(
                    &idx,
                    raw,
                    hpath,
                    old,
                    new,
                    *all,
                    rev.as_deref(),
                )?);
                born.push(None);
            }
            wire::PlanEdit::ReplaceSection { hpath, body, rev } => {
                edits.push(lower_replace_section(
                    &idx,
                    doc,
                    hpath,
                    body,
                    rev.as_deref(),
                )?);
                born.push(None);
            }
            wire::PlanEdit::Create {
                parent_hpath,
                title,
                body,
                rev,
            } => {
                let (edit, heading_offset) =
                    lower_create(&idx, doc, parent_hpath, title, body, rev.as_deref())?;
                edits.push(edit);
                born.push(Some(Born {
                    title: title.clone(),
                    heading_offset,
                }));
            }
        }
    }
    Ok(Lowered { edits, born })
}

/// The read-face published address of the section whose node span starts at
/// `heading_start` — the same `read_facts` table every plane resolves
/// against, so the returned segments carry `n` exactly where the document is
/// ambiguous and a read-back lands. `None` when no section starts there.
pub(crate) fn published_hpath_at(
    doc: &model::Document,
    heading_start: usize,
) -> Option<Vec<HpathSeg>> {
    PlanIndex::new(doc)
        .headings
        .iter()
        .find(|f| f.span.0 == heading_start)
        .map(|f| f.raw_hpath.clone())
}

// --- splice hygiene (wire-contract § A.3, N-1) ------------------------------

/// Whether byte `off` sits inside a fenced code block anywhere on its
/// containment chain. Plain list items are not a dialect fact (the model
/// materializes tasks and fences, not bullets), so the flush-join rule reads
/// the tail LINE's shape — but a fence interior can LOOK like a list item,
/// and the parsed tree is what says so (the containment spec's case-8
/// principle: fence interiors never drive structure decisions).
fn offset_inside_fence(node: &model::Node, off: usize) -> bool {
    if !node.span.contains(&off) {
        return false;
    }
    if matches!(node.kind, model::NodeKind::CodeBlock { .. }) {
        return true;
    }
    node.children.iter().any(|c| offset_inside_fence(c, off))
}

/// Whether the payload's first non-blank line opens a list item — bullet
/// (`-`/`*`/`+`) or ordered (`1.`/`1)`), marker then space or end of line.
fn opens_list_item(payload: &str) -> bool {
    let Some(line) = payload.lines().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let t = line.trim_start_matches([' ', '\t']);
    if let Some(rest) = t.strip_prefix(['-', '*', '+']) {
        return rest.is_empty() || rest.starts_with(' ');
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return false;
    }
    match t[digits..].strip_prefix(['.', ')']) {
        Some(rest) => rest.is_empty() || rest.starts_with(' '),
        None => false,
    }
}

/// Payload edge normalization (§ A.3 hygiene: boundaries are the engine's,
/// interior bytes the caller's): leading blank lines dropped, trailing
/// whitespace collapsed to one terminator. Empty in, empty out.
fn normalize_payload(body: &str) -> String {
    let mut s = body;
    while let Some(nl) = s.find('\n') {
        if s[..nl].trim().is_empty() {
            s = &s[nl + 1..];
        } else {
            break;
        }
    }
    let s = s.trim_end();
    if s.is_empty() {
        String::new()
    } else {
        format!("{s}\n")
    }
}

/// Compose the canonical content-span bytes for `payload` landing at a
/// section's subtree end, and pick the mechanism (§ A.3 splice hygiene).
///
/// The composition keeps the existing content through its last non-blank
/// line, joins the payload across one canonical boundary — a blank line for
/// a block boundary, flush when a list-item payload continues a trailing
/// list — and ends with exactly one blank line before a following heading
/// (a single terminator at EOF).
///
/// Mechanism is derived, never declared: a composition that purely extends
/// the existing content bytes lowers to `Put{end}` (zero-width insert, the
/// pre-hygiene shape); one that must move a boundary byte (a separator to
/// remove or collapse) lowers to `Put{content}`. Returns `(at, text,
/// payload_offset_in_text)`.
fn compose_at_subtree_end(
    doc: &model::Document,
    content_span: (usize, usize),
    payload: &str,
) -> (PutAt, String, usize) {
    let (cs, e) = content_span;
    let raw = doc.raw.as_bytes();
    let content = &doc.raw[cs..e];
    let followed = e < raw.len();

    let mut out = String::with_capacity(content.len() + payload.len() + 4);
    match content.rfind(|c: char| !c.is_whitespace()) {
        None => {
            // No content. A bare, unterminated heading line needs its own
            // terminator before the blank line under the heading.
            if cs > 0 && raw[cs - 1] != b'\n' {
                out.push('\n');
            }
            out.push('\n');
        }
        Some(i) => {
            let ink_end = i + content[i..].chars().next().map_or(1, char::len_utf8);
            if let Some(nl) = content[ink_end..].find('\n') {
                // Keep through the last non-blank line, terminator included.
                out.push_str(&content[..=ink_end + nl]);
            } else {
                // Bare final line: keep its ink, terminate it ourselves.
                out.push_str(&content[..ink_end]);
                out.push('\n');
            }
            let tail_line_start = content[..i].rfind('\n').map_or(0, |p| p + 1);
            let flush = opens_list_item(payload)
                && opens_list_item(&content[tail_line_start..ink_end])
                && !offset_inside_fence(&doc.root, cs + i);
            if !flush {
                out.push('\n');
            }
        }
    }
    let payload_offset = out.len();
    out.push_str(payload);
    if followed {
        out.push('\n');
    }

    if out.as_bytes().starts_with(content.as_bytes()) {
        let text = out[content.len()..].to_string();
        (PutAt::End, text, payload_offset - content.len())
    } else {
        (PutAt::Content, out, payload_offset)
    }
}

/// Append arm: block targets refuse; payload lands at the section's subtree
/// end across canonical boundaries (§ A.3 splice hygiene). Lowered to
/// `Put{end}` when the composition purely extends the content, `Put{content}`
/// when a boundary needs surgery. An empty payload appends nothing (a no-op
/// edit — never a stray blank line).
fn lower_append(
    idx: &PlanIndex,
    doc: &model::Document,
    hpath: &[HpathSeg],
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    // The op-class refusal outranks resolution: an append at a block refuses
    // whether or not the id resolves — a line grows through `match` /
    // `replace_section`, a NEW line belongs to the enclosing section.
    if block_ref(hpath).is_some() || matches!(hpath, [only] if only.h.starts_with('^')) {
        return Err(bad_request(format!(
            "append to a block anchor {} is not supported — append targets a section (the containing heading path)",
            policy::defs::go_quote(&crate::display_hpath(hpath))
        )));
    }
    let node = idx.get(hpath).map_err(|m| section_miss(hpath, &m))?;
    let payload = normalize_payload(body);
    let (at, text) = if payload.is_empty() {
        (PutAt::End, String::new())
    } else {
        let (at, text, _) = compose_at_subtree_end(doc, content_span_of(node), &payload);
        (at, text)
    };
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: node.raw_hpath.clone(),
        },
        edit: EditShape::Put { at, text },
        if_node_rev: rev
            .filter(|r| !r.is_empty())
            .map(|r| NodeRev(r.to_string())),
    })
}

/// A node's content span with the defensive full-span fallback (a headingless
/// fact has no separate content span).
fn content_span_of(node: &HeadingFacts) -> (usize, usize) {
    match node.content_span {
        Some((cs, ce)) if cs >= node.span.0 && ce <= node.span.1 => (cs, ce),
        _ => node.span,
    }
}

/// Replace arm: unique-anchor Match, or all:true RMW over heading-excluded
/// content written back as `Put{content}`. At a `^id` block target the same
/// two shapes land on the block-leaf bytes (W-2): all:false as a native
/// anchor-target Match, all:true as an RMW written back `Put{all}` — the
/// content span IS the full span at a leaf, and the kernel's span-escape and
/// identity gates judge what the replacement did to the marker.
fn lower_match(
    idx: &PlanIndex,
    raw: &[u8],
    hpath: &[HpathSeg],
    old: &str,
    new: &str,
    all: bool,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    // Peel `@fp` from `old` (needle in stored bytes) before search; `new` is payload.
    let peeled = &*syntax::strip_fp(old);
    let if_node_rev = rev
        .filter(|r| !r.is_empty())
        .map(|r| NodeRev(r.to_string()));
    if let Some(id) = block_ref(hpath) {
        let blk = resolve_block(idx, hpath, id)?;
        if !all {
            return Ok(Edit {
                target: SecRef::Anchor { anchor: id.into() },
                edit: EditShape::Match {
                    old: peeled.to_string(),
                    new: new.to_string(),
                },
                if_node_rev,
            });
        }
        let (s, e) = blk.span;
        let content = String::from_utf8_lossy(&raw[s..e]);
        if !content.contains(peeled) {
            return Err(bad_request(format!(
                "replace anchor {} not found in {}",
                policy::defs::go_quote(peeled),
                policy::defs::go_quote(&crate::display_hpath(hpath))
            )));
        }
        return Ok(Edit {
            target: SecRef::Anchor { anchor: id.into() },
            edit: EditShape::Put {
                at: PutAt::All,
                text: content.replace(peeled, new),
            },
            if_node_rev,
        });
    }
    let node = idx.get(hpath).map_err(|m| section_miss(hpath, &m))?;
    let old = peeled;
    if all {
        // Go stripHeading: content-span offset; defensive full-span fallbacks.
        let (s, e) = node.span;
        let full = String::from_utf8_lossy(&raw[s..e]);
        let content = match node.content_span {
            Some((cs, _)) if cs >= s && cs <= e => String::from_utf8_lossy(&raw[cs..e]),
            _ => full,
        };
        if !content.contains(old) {
            return Err(bad_request(format!(
                "replace anchor {} not found in {}",
                policy::defs::go_quote(old),
                policy::defs::go_quote(&crate::display_hpath(hpath))
            )));
        }
        let new_content = content.replace(old, new);
        return Ok(Edit {
            target: SecRef::Hpath {
                hpath: node.raw_hpath.clone(),
            },
            edit: EditShape::Put {
                at: PutAt::Content,
                text: new_content,
            },
            if_node_rev,
        });
    }
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: node.raw_hpath.clone(),
        },
        edit: EditShape::Match {
            old: old.to_string(),
            new: new.to_string(),
        },
        if_node_rev,
    })
}

/// `replace_section`: rev required; payload containment (wire-contract §A.3
/// containment law) with the first-line address-echo normalization; the
/// contained body then composes with the § A.3 hygiene boundaries — one
/// blank line under the section's own heading, one before a following
/// heading, a bare terminator at EOF (an empty body keeps the boundary blank
/// alone). `Put{content}` + `if_node_rev`.
///
/// At a `^id` block target the op keeps its face meaning — the payload is the
/// CONTENT, the address is preserved by construction (`lower_replace_block`):
/// `Put{content}` preserves a section's heading because the content span
/// excludes it; a block leaf has no such slot (its content span IS its full
/// span), so the block arm composes the address back instead — trailing
/// newlines trimmed (a leaf span excludes its terminator), the ` ^id` marker
/// re-affixed line-final unless the payload already echoes it (the caller
/// repeating the address — the same case-4 normalization family as the
/// heading echo above). An empty body refuses: a bare marker hosting nothing
/// has no clean meaning, and removing a block is the containing section's
/// write.
fn lower_replace_section(
    idx: &PlanIndex,
    doc: &model::Document,
    hpath: &[HpathSeg],
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    if let Some(id) = block_ref(hpath) {
        return lower_replace_block(idx, hpath, id, body, rev);
    }
    let node = idx.get(hpath).map_err(|m| section_miss(hpath, &m))?;
    let rev = rev.unwrap_or("");
    if rev.is_empty() {
        return Err(bad_request(format!(
            "replace_section on {} requires a fresh rev (a whole-section rewrite is destructive) — read the section and pass its rev",
            policy::defs::go_quote(&crate::display_hpath(hpath))
        )));
    }
    // Containment first (the gate speaks the caller's own payload
    // coordinates), then hygiene composes the contained remainder — the echo
    // strip leaves a leading blank line and normalize_payload collapses it.
    let body = contain_replace_payload(node, hpath, body, rev)?;
    let (cs, e) = content_span_of(node);
    let raw = doc.raw.as_bytes();
    let followed = e < raw.len();
    let payload = normalize_payload(body);
    let mut text = String::with_capacity(payload.len() + 3);
    // A bare, unterminated heading line needs its own terminator first.
    if cs == e && cs > 0 && raw[cs - 1] != b'\n' {
        text.push('\n');
    }
    if !payload.is_empty() {
        text.push('\n');
        text.push_str(&payload);
    }
    if followed {
        text.push('\n');
    }
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: node.raw_hpath.clone(),
        },
        edit: EditShape::Put {
            at: PutAt::Content,
            text,
        },
        if_node_rev: Some(NodeRev(rev.to_string())),
    })
}

/// One payload heading as the dialect parse sees it (ATX only; `#`-lines
/// inside fences are Fence content and never reach here — the containment
/// law's "judged on the PARSED payload, never line-regex").
struct PayloadHeading {
    /// 1-based line in the caller's own payload — refusals must speak the
    /// caller's coordinates, so this is computed on the ORIGINAL body.
    line: usize,
    level: u32,
    text: String,
}

/// The §A.3 containment gate + echo normalization for a `replace_section`
/// payload. Returns the body to splice — the original, or its remainder after
/// the first-line address echo is stripped.
///
/// The invariant: the target's subtree is exactly the payload, so a payload
/// heading at or above the target's own level cannot nest and refuses whole.
/// The ONE normalization: a first line echoing the target's own heading (same
/// level, same title) is the caller repeating the address — stripped
/// silently. Judged before any strip, so refusal line numbers are the
/// caller's own.
///
/// # Errors
/// `bad_request` in the `payload_escapes_section` grammar: offending line,
/// both levels, honest alternative — folding in the stale-rev fact when
/// `rev_given` no longer matches the target (one refusal, both facts; W-4).
fn contain_replace_payload<'b>(
    node: &HeadingFacts,
    hpath: &[HpathSeg],
    body: &'b str,
    rev_given: &str,
) -> Result<&'b str, Box<ErrorBody>> {
    let title = node
        .raw_hpath
        .last()
        .map(|s| s.h.as_str())
        .unwrap_or_default();
    let headings = syntax::parse(body).into_iter().filter_map(|n| {
        if let syntax::DialectKind::Heading { level, text } = n.kind {
            Some(PayloadHeading {
                line: 1 + body[..n.span.start].matches('\n').count(),
                level: u32::from(level),
                text,
            })
        } else {
            None
        }
    });

    let mut echo = false;
    for h in headings {
        let own_name = h.level == node.level && h.text == title;
        if own_name && h.line == 1 {
            // The address echo — normalized away below, never an escape.
            echo = true;
            continue;
        }
        if h.level <= node.level {
            return Err(escape_refusal(node, hpath, &h, own_name, rev_given));
        }
    }

    Ok(if echo {
        body.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        body
    })
}

/// The `payload_escapes_section` refusal (wire-contract §A.3 containment law;
/// grammar from the ratified spec): names the offending body line, both
/// levels, and the honest alternative — restructuring is a write to the
/// parent. When the caller's rev is ALSO stale, the same refusal carries the
/// CAS fact with the current rev as the resend token, so a stale CAS can
/// never mask the structural refusal into a two-error teaching loop (W-4).
fn escape_refusal(
    node: &HeadingFacts,
    hpath: &[HpathSeg],
    h: &PayloadHeading,
    duplicate_name: bool,
    rev_given: &str,
) -> Box<ErrorBody> {
    let mut msg = format!(
        "payload_escapes_section — body line {} is a level-{} heading ({}{}); target {} is \
         level {}, so payload headings must be level {}+. replace_section replaces the \
         target's subtree only. To create a sibling, target the parent section or use \
         create_section.",
        h.line,
        h.level,
        policy::defs::go_quote(&h.text),
        if duplicate_name {
            " — the target's own name, so it would mint a duplicate sibling"
        } else {
            ""
        },
        policy::defs::go_quote(&crate::display_hpath(hpath)),
        node.level,
        node.level + 1,
    );
    if duplicate_name {
        msg.push_str(
            " An echo of the target's heading is normalized away only as the payload's \
             FIRST line.",
        );
    }
    if rev_given != node.sec_rev {
        use std::fmt::Write as _;
        // Infallible on String; the write! form is the workspace's
        // format-push convention (clippy::format_push_string).
        let _ = write!(
            msg,
            " Separately, the rev you passed is stale: you sent {} but the section is now \
             {} — fix the payload and resend with the current rev.",
            policy::defs::go_quote(rev_given),
            policy::defs::go_quote(&node.sec_rev),
        );
    }
    bad_request(msg)
}

/// The block arm of `replace_section` — see [`lower_replace_section`].
fn lower_replace_block(
    idx: &PlanIndex,
    hpath: &[HpathSeg],
    id: &str,
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    let shown = crate::display_hpath(hpath);
    let rev = rev.unwrap_or("");
    if rev.is_empty() {
        return Err(bad_request(format!(
            "replace_section on {} requires a fresh rev (a whole-block rewrite is destructive) — read the block (sections:[{}]) and pass its rev",
            policy::defs::go_quote(&shown),
            policy::defs::go_quote(&shown),
        )));
    }
    resolve_block(idx, hpath, id)?;
    // Echo strip, then recompose: a payload ending with the target's own
    // marker is the caller repeating the address — stripped first so the
    // empty-body law sees the CONTENT ("marker only, nothing else" is the
    // empty case), then the address is re-affixed exactly once.
    let mut content = body.trim_end_matches('\n');
    let marker = format!("^{id}");
    if content.ends_with(&marker)
        && content[..content.len() - marker.len()]
            .chars()
            .next_back()
            .is_none_or(|c| c == ' ' || c == '\t' || c == '\n')
    {
        content = content[..content.len() - marker.len()].trim_end_matches([' ', '\t']);
    }
    if content.is_empty() {
        return Err(bad_request(format!(
            "replace_section on {} with an empty body would leave a bare `^` marker hosting \
             nothing. {} To remove the block, write through the containing section (its \
             heading path).",
            policy::defs::go_quote(&shown),
            crate::NO_PARTIAL_WRITE_CLAUSE,
        )));
    }
    let text = format!("{content} {marker}");
    Ok(Edit {
        target: SecRef::Anchor { anchor: id.into() },
        edit: EditShape::Put {
            at: PutAt::All,
            text,
        },
        if_node_rev: Some(NodeRev(rev.to_string())),
    })
}

/// Create: parent-append with § A.3 hygiene boundaries (one blank line before
/// the born heading, one between it and its body, one before whatever
/// follows); top-level / parent-miss refuse. `rev` is the PARENT's node-grain
/// token, threaded to the lowered edit's `if_node_rev` — one rev derivation,
/// no second comparison rule (§ A.3). Returns the edit plus the born
/// heading's byte offset within its text (see [`Born`]).
///
/// The parent target is the LOWERING's mechanism only: the armed fact for
/// this row names the BORN section (the § A.3 create-door law; A.6.3a′ is
/// the precedent), carried there by [`Lowered::born`] — the fact must name
/// what the caller addressed, and the caller addressed the birth.
fn lower_create(
    idx: &PlanIndex,
    doc: &model::Document,
    parent_hpath: &[HpathSeg],
    title: &str,
    body: &str,
    rev: Option<&str>,
) -> Result<(Edit, usize), Box<ErrorBody>> {
    let full = if parent_hpath.is_empty() {
        title.to_string()
    } else {
        format!("{}/{title}", crate::display_hpath(parent_hpath))
    };
    let cannot_place = || {
        bad_request(format!(
            "cannot place new section {} — its parent is not in the document",
            policy::defs::go_quote(&full)
        ))
    };
    if parent_hpath.is_empty() {
        return Err(cannot_place());
    }
    // An ambiguous parent says so by name: `cannot_place` would claim the
    // parent is absent when the document holds several of it.
    let parent = idx.get(parent_hpath).map_err(|m| match m {
        Miss::NotFound => cannot_place(),
        m @ Miss::Ambiguous(_) => section_miss(parent_hpath, &m),
    })?;
    let level = (parent.level + 1) as usize;
    let body = normalize_payload(body);
    let payload = if body.is_empty() {
        format!("{} {title}\n", "#".repeat(level))
    } else {
        format!("{} {title}\n\n{body}", "#".repeat(level))
    };
    let (at, text, heading_offset) = compose_at_subtree_end(doc, content_span_of(parent), &payload);
    Ok((
        Edit {
            target: SecRef::Hpath {
                hpath: parent.raw_hpath.clone(),
            },
            edit: EditShape::Put { at, text },
            if_node_rev: rev
                .filter(|r| !r.is_empty())
                .map(|r| NodeRev(r.to_string())),
        },
        heading_offset,
    ))
}

/// Property group: ONE edit per key, each targeting its OWN `fm_key` — an
/// existing key as `Put{all}` over its line, an absent key as `Put{upsert}`
/// (the § A.6.3a create-or-replace door, which addresses a key that does not
/// exist yet). Keys/values only through `yaml_safe_key`/
/// `yaml_preserve_or_encode` — an existing key's stored line feeds the
/// § A.6.3c no-op preservation, so a write-back of the served value recomposes
/// the stored spelling byte-identically.
///
/// One edit per key is the ARMED-FACT law, not a style choice (§6.1: armed
/// facts carry op, target identities and rev transitions; §7.1: a node entry
/// names the deepest node containing each changed byte range). The former
/// shape folded every absent key into ONE edit targeting the LAST EXISTING key
/// with `Put{end}`, so a batch setting `owner` and `status` on a document whose
/// frontmatter holds `title` armed — and receipted — `title put:end
/// <rev>-><same rev>`: an identity the batch never wrote, an op nobody asked
/// for, and a no-op transition beside two keys that landed. The facts are the
/// normative receipt content (§6.4), so that collapse made every props write
/// unauditable.
///
/// The absent arm passes the caller's RAW value: the upsert door encodes it
/// (§ A.6.3a), with an absent key's empty before-span selecting the fresh
/// encode — the same bytes this function's own encode produces. The encode
/// above still runs for every key, because its newline refusal teaches in the
/// `set_property` door's words.
fn lower_property_group(
    doc: &model::Document,
    idx: &PlanIndex,
    props: &std::collections::BTreeMap<&str, &str>,
) -> Result<Vec<Edit>, Box<ErrorBody>> {
    // Key owner first (same refusal surface as rebuild::plan_set_property).
    let mut keyed: std::collections::BTreeMap<policy::defs::SafeKey<'_>, &str> =
        std::collections::BTreeMap::new();
    for (k, v) in props {
        let key = policy::defs::yaml_safe_key(k).map_err(|_| {
            bad_request(format!(
                "invalid frontmatter key {} — a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')",
                policy::defs::go_quote(k)
            ))
        })?;
        keyed.insert(key, v);
    }

    let fm_key_set: std::collections::HashSet<&str> =
        idx.fm_keys.iter().map(String::as_str).collect();
    if idx.fm_keys.is_empty() {
        for k in keyed.keys() {
            if !fm_key_set.contains(k.as_str()) {
                return Err(bad_request(
                    "cannot set a new property — the file has no frontmatter to anchor it (add a '---' block first)",
                ));
            }
        }
    }
    let mut quoted: std::collections::BTreeMap<policy::defs::SafeKey<'_>, String> =
        std::collections::BTreeMap::new();
    for (k, v) in &keyed {
        // An existing key's stored line, for § A.6.3c preservation. A resolve
        // miss (e.g. a duplicate key) just falls back to the fresh encode —
        // preservation is byte quiet, never a new refusal surface.
        let stored_line = if fm_key_set.contains(k.as_str()) {
            model::resolve(doc, &model::Ref::FmKey(k.as_str().to_string()))
                .ok()
                .map(|t| doc.raw[t.span].to_string())
        } else {
            None
        };
        // A newline in a value forges frontmatter keys — refuse, never sanitize.
        let safe = policy::defs::yaml_preserve_or_encode(stored_line.as_deref(), v).map_err(|_| {
            bad_request(format!(
                "property value for {} contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section",
                policy::defs::go_quote(k.as_str())
            ))
        })?;
        quoted.insert(*k, safe);
    }
    let line = |k: policy::defs::SafeKey<'_>| format!("{k}: {}", quoted[&k]);

    let fm_put = |key: &str, at: PutAt, text: String| Edit {
        target: SecRef::FmKey {
            fm_key: key.to_string(),
        },
        edit: EditShape::Put { at, text },
        if_node_rev: None,
    };

    let mut edits = Vec::new();
    for k in quoted.keys().copied() {
        if fm_key_set.contains(k.as_str()) {
            edits.push(fm_put(k.as_str(), PutAt::All, line(k)));
        } else {
            edits.push(fm_put(k.as_str(), PutAt::Upsert, keyed[&k].to_string()));
        }
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use wire::{EditShape, HpathSeg, PlanEdit, PutAt, SecRef};

    fn doc(raw: &str) -> model::Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    fn lower1(raw: &str, e: PlanEdit) -> Result<wire::Edit, Box<wire::ErrorBody>> {
        super::lower(&doc(raw), &[e]).map(|mut l| l.edits.remove(0))
    }

    fn put_text(e: &wire::Edit) -> (&PutAt, &str) {
        match &e.edit {
            EditShape::Put { at, text } => (at, text),
            EditShape::Match { .. } => panic!("expected put"),
        }
    }

    /// Append hygiene discipline (§ A.3): a new block lands across one blank
    /// line; a bare final line is terminated first. Both stay pure `Put{end}`
    /// extensions.
    #[test]
    fn append_hygiene_discipline() {
        let e = lower1(
            "# Memo\n\nline\n",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "Memo".into(),
                    n: None,
                }],
                body: "added".into(),
                rev: None,
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::End);
        assert_eq!(
            text, "\nadded\n",
            "a new block gets its blank-line boundary, trailing NL ensured"
        );

        let e = lower1(
            "# Memo\n\nline",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "Memo".into(),
                    n: None,
                }],
                body: "added\n".into(),
                rev: None,
            },
        )
        .expect("lowers");
        let (_, text) = put_text(&e);
        assert_eq!(
            text, "\n\nadded\n",
            "a bare final line is terminated, then the blank-line boundary"
        );
    }

    /// A task-hosted id lowers since F-R4 (every body host is in the anchor
    /// plane); the one host-excluded refusal left is the frontmatter caret,
    /// whose teaching names the `props` plane.
    #[test]
    fn task_hosted_block_lowers_fm_caret_refuses_toward_props() {
        let lowered = lower1(
            "# Tasks\n\n- [ ] one ^task1\n",
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "^task1".into(),
                    n: None,
                }],
                old: "one".into(),
                new: "two".into(),
                all: false,
                rev: None,
            },
        )
        .expect("a task-hosted id lowers (F-R4)");
        assert_eq!(
            lowered.target,
            SecRef::Anchor {
                anchor: "task1".into()
            },
            "the lowered edit targets the block by its id"
        );

        let err = lower1(
            "---\ntitle: x ^fm-c\n---\n# Tasks\n\n- item ^t1\n",
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "^fm-c".into(),
                    n: None,
                }],
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect_err("a frontmatter caret does not resolve");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"^fm-c\". No edit was applied; the batch is \
                 refused whole. Fix: `^fm-c` exists in this document, but its host is \
                 the frontmatter — a caret there is literal YAML, not a block; \
                 frontmatter keys are written through the `props` plane, not a block \
                 ref."
            )
        );
    }

    /// Append-to-block refusal arm string (arm-faithful for direct callers).
    #[test]
    fn append_to_block_refuses_arm_string() {
        let err = lower1(
            "# Tasks\n\n- item ^t1\n",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "^t1".into(),
                    n: None,
                }],
                body: "x".into(),
                rev: None,
            },
        )
        .expect_err("block append refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                r#"append to a block anchor "^t1" is not supported — append targets a section (the containing heading path)"#
            )
        );
    }

    /// Append section-miss includes toc remedy tail.
    #[test]
    fn append_miss_names_a_runnable_recovery() {
        let err = lower1(
            "# A\n\nx\n",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "Ghost".into(),
                    n: None,
                }],
                body: "x".into(),
                rev: None,
            },
        )
        .expect_err("miss refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"Ghost\". No edit was applied; the batch is \
                 refused whole. Fix: list the document's section paths with a toc read \
                 (MCP read: sections[] omitted; CLI: a read with no --section), then feed its \
                 row back in one of the two addressing forms: the row's raw heading \
                 segments as an hpath array (one entry per heading, no joining), or its \
                 dewey ordinal (CLI: `--section 1.2`). The joined selector string splits \
                 on `/`, so a heading whose raw text carries one is reachable only by \
                 those two forms."
            )
        );
    }

    /// Top-level create refuses.
    #[test]
    fn create_top_level_refuses() {
        let err = lower1(
            "# A\n\nx\n",
            PlanEdit::Create {
                parent_hpath: vec![],
                title: "Brand".into(),
                body: "b".into(),
                rev: None,
            },
        )
        .expect_err("top-level create refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(r#"cannot place new section "Brand" — its parent is not in the document"#)
        );
    }

    /// Create parent-append shape: `Put{end}` on parent, child depth = parent+1.
    #[test]
    fn create_parent_append_shape() {
        let e = lower1(
            "# A\n\nx\n\n## B\n\ny\n",
            PlanEdit::Create {
                parent_hpath: vec![
                    HpathSeg {
                        h: "A".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "B".into(),
                        n: None,
                    },
                ],
                title: "New Kid".into(),
                body: "hello".into(),
                rev: None,
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::End);
        assert_eq!(text, "\n### New Kid\n\nhello\n");
        let SecRef::Hpath { hpath } = &e.target else {
            panic!("hpath target")
        };
        assert_eq!(
            hpath.iter().map(|s| s.h.as_str()).collect::<Vec<_>>(),
            vec!["A", "B"],
            "targets the RAW parent chain"
        );
    }

    /// match all:true RMW over heading-excluded content; not-found arm host-verbatim.
    #[test]
    fn match_all_rmw_and_not_found() {
        let raw = "# Todo\n\n- item a\n- item b\n";
        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Todo".into(),
                    n: None,
                }],
                old: "item".into(),
                new: "task".into(),
                all: true,
                rev: Some("deadbeefdeadbeef".into()),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::Content);
        // Content span starts after heading (blank separator rides content) — Go stripHeading.
        assert_eq!(text, "\n- task a\n- task b\n");
        assert_eq!(
            e.if_node_rev.as_ref().map(|r| r.0.as_str()),
            Some("deadbeefdeadbeef")
        );

        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Todo".into(),
                    n: None,
                }],
                old: "ghost".into(),
                new: "x".into(),
                all: true,
                rev: None,
            },
        )
        .expect_err("not found refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(r#"replace anchor "ghost" not found in "Todo""#)
        );
    }

    /// `replace_section`: rev required; trailing-NL; empty body empty; `if_node_rev`.
    #[test]
    fn replace_section_rev_and_payload() {
        let raw = "# Notes\n\nold\n";
        let err = lower1(
            raw,
            PlanEdit::ReplaceSection {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
                body: "new".into(),
                rev: None,
            },
        )
        .expect_err("rev-less refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                r#"replace_section on "Notes" requires a fresh rev (a whole-section rewrite is destructive) — read the section and pass its rev"#
            )
        );

        let e = lower1(
            raw,
            PlanEdit::ReplaceSection {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
                body: "new".into(),
                rev: Some("cafebabecafebabe".into()),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::Content);
        assert_eq!(
            text, "\nnew\n",
            "the blank line under the section's own heading rides the composition (§ A.3)"
        );
        assert_eq!(
            e.if_node_rev.as_ref().map(|r| r.0.as_str()),
            Some("cafebabecafebabe")
        );
    }

    /// The address echo strips to a suffix slice: the lowered `Put{content}`
    /// carries the remainder verbatim (blank separator kept), and an
    /// echo-only payload lowers to the empty content write.
    #[test]
    fn replace_section_echo_strip_lowers_the_remainder() {
        let raw = "# Notes\n\nold\n";
        let lowered = |body: &str| {
            lower1(
                raw,
                PlanEdit::ReplaceSection {
                    hpath: vec![HpathSeg {
                        h: "Notes".into(),
                        n: None,
                    }],
                    body: body.into(),
                    rev: Some("cafebabecafebabe".into()),
                },
            )
            .expect("lowers")
        };
        let e = lowered("# Notes\n\nnew body\n");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::Content);
        assert_eq!(
            text, "\nnew body\n",
            "echo line stripped, remainder verbatim"
        );

        let e = lowered("# Notes\n");
        let (_, text) = put_text(&e);
        assert_eq!(text, "", "echo-only payload lowers to the empty write");
    }

    /// Property dance: ONE edit per key, each on its OWN `fm_key` — existing as
    /// `Put{all}` over its line, absent as `Put{upsert}` carrying the caller's
    /// raw value for the § A.6.3a door to encode. The identity a receipt reads
    /// back is the key that was written, never the last key of the block.
    #[test]
    fn property_group_dance() {
        let raw = "---\nstatus: open\nowner: d\n---\n# A\n\nx\n";

        let edits = super::lower(
            &doc(raw),
            &[PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed".into(),
                rev: None,
            }],
        )
        .expect("lowers")
        .edits;
        assert_eq!(edits.len(), 1);
        let (at, text) = put_text(&edits[0]);
        assert_eq!((*at, text), (PutAt::All, "status: closed"));
        assert!(matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "status"));

        let edits = super::lower(
            &doc(raw),
            &[PlanEdit::SetProperty {
                key: "zeta".into(),
                value: "1".into(),
                rev: None,
            }],
        )
        .expect("lowers")
        .edits;
        assert_eq!(edits.len(), 1);
        let (at, text) = put_text(&edits[0]);
        assert_eq!(
            (*at, text),
            (PutAt::Upsert, "1"),
            "an absent key is created through its own upsert door, raw value"
        );
        assert!(
            matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "zeta"),
            "the created key is the target — never the last key of the block"
        );

        let edits = super::lower(
            &doc(raw),
            &[
                PlanEdit::SetProperty {
                    key: "owner".into(),
                    value: "e".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "zeta".into(),
                    value: "1".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed".into(),
                    rev: None,
                },
            ],
        )
        .expect("lowers")
        .edits;
        // Three keys, three edits — key-sorted, each naming what it wrote.
        assert_eq!(edits.len(), 3, "one armed edit per key the caller set");
        let named = |e: &wire::Edit| match &e.target {
            SecRef::FmKey { fm_key } => fm_key.clone(),
            other => panic!("fm_key target, got {other:?}"),
        };
        assert_eq!(
            edits.iter().map(named).collect::<Vec<_>>(),
            vec!["owner", "status", "zeta"]
        );
        assert_eq!(put_text(&edits[0]), (&PutAt::All, "owner: e"));
        assert_eq!(put_text(&edits[1]), (&PutAt::All, "status: closed"));
        assert_eq!(put_text(&edits[2]), (&PutAt::Upsert, "1"));
    }

    /// No frontmatter to anchor a new key refuses.
    #[test]
    fn property_no_frontmatter_refuses() {
        let err = super::lower(
            &doc("# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "status".into(),
                value: "open".into(),
                rev: None,
            }],
        )
        .expect_err("no fm refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "cannot set a new property — the file has no frontmatter to anchor it (add a '---' block first)"
            )
        );
    }

    /// Values with `: ` quote through shared `yaml_safe_value` predicate — in
    /// the § A.6.3 double-quoted spelling, since the predicate is the one owner.
    #[test]
    fn property_value_quotes_through_shared_predicate() {
        let edits = super::lower(
            &doc("---\nnote: x\n---\n# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "note".into(),
                value: "a: b".into(),
                rev: None,
            }],
        )
        .expect("lowers")
        .edits;
        let (_, text) = put_text(&edits[0]);
        assert_eq!(text, "note: \"a: b\"");
    }

    /// Empty value keeps the trailing space (never the model-upsert `k:`) and
    /// lands the § A.6.3 empty STRING: this plane is typed `string`, so a bare
    /// `k: ` would emit a null the caller has no way to mean.
    #[test]
    fn property_empty_value_keeps_trailing_space() {
        let edits = super::lower(
            &doc("---\nnote: x\n---\n# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "note".into(),
                value: String::new(),
                rev: None,
            }],
        )
        .expect("lowers")
        .edits;
        let (_, text) = put_text(&edits[0]);
        assert_eq!(text, "note: \"\"");
    }

    /// Duplicate headings refuse without an occurrence rather than silently
    /// picking one.
    #[test]
    fn duplicate_headings_refuse_without_an_occurrence() {
        let raw = "# Notes\n\nfirst\n\n# Notes\n\nsecond\n";
        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect_err("an ambiguous address refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "address \"Notes\" matches 2 sections. No edit was applied; the batch is \
                 refused whole. Fix: pass the occurrence — the read face publishes it as \
                 `n` on the ambiguous segment."
            )
        );

        // The occurrence the read face publishes reaches exactly one of them.
        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: Some(2),
                }],
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect("`n: 2` names the second");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\n2nd\n");
    }

    /// `sanitize_heading` is non-injective (`# A/B` and `# A B` both sanitize
    /// to `A-B`), so a sanitize-keyed index made one of them unaddressable.
    /// Each heading must have an address that reaches it and nothing else.
    #[test]
    fn each_colliding_heading_keeps_its_own_address() {
        let raw = "# A/B\n\nfirst\n\n# A B\n\nsecond\n";
        // Pinned premise: under a sanitize-keyed grammar these two headings
        // were one key.
        assert_eq!(wire_map::gotext::sanitize_heading("A/B"), "A-B");
        assert_eq!(wire_map::gotext::sanitize_heading("A B"), "A-B");

        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "A/B".into(),
                    n: None,
                }],
                old: "first".into(),
                new: "1st".into(),
                all: true,
                rev: None,
            },
        )
        .expect("the `A/B` section is addressable");
        let (_, text) = put_text(&e);
        // Trailing blank line rides `A/B`'s content span (it precedes `# A B`).
        assert_eq!(text, "\n1st\n\n", "reaches `A/B`, not `A B`");

        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "A B".into(),
                    n: None,
                }],
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect("the `A B` section is addressable");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\n2nd\n", "reaches `A B`, not `A/B`");
    }

    /// A write must land on the section the caller addressed — never on a
    /// different one while reporting success. The sanitized-join grammar made
    /// this reachable and silent: an edit addressed to `A/B` lowered to a
    /// target on `A B` and returned `Ok`.
    #[test]
    fn a_write_addressed_to_one_section_never_lands_on_another() {
        // The anchor text is present in both sections, so a wrong-section write
        // succeeds instead of refusing — the silent path, not the loud one.
        let raw = "# A/B\n\nnote here\n\n# A B\n\nnote here\n";

        for (addressed, other) in [("A/B", "A B"), ("A B", "A/B")] {
            let e = lower1(
                raw,
                PlanEdit::Match {
                    hpath: vec![HpathSeg {
                        h: addressed.into(),
                        n: None,
                    }],
                    old: "note".into(),
                    new: "NOTE".into(),
                    all: true,
                    rev: None,
                },
            )
            .unwrap_or_else(|e| panic!("{addressed} is addressable: {:?}", e.message));

            let SecRef::Hpath { hpath } = &e.target else {
                panic!("hpath target")
            };
            let landed = hpath.iter().map(|s| s.h.as_str()).collect::<Vec<_>>();
            assert_eq!(
                landed,
                vec![addressed],
                "addressed {addressed:?} but the edit targets {landed:?} — a write \
                 aimed at one section landed on {other:?} and reported success"
            );
        }
    }

    /// The plan face speaks raw segments: the raw title is the address, and
    /// the sanitized spelling names no heading.
    #[test]
    fn raw_title_addresses_and_the_sanitized_spelling_misses() {
        let raw = "# My Section\n\nx\n";
        lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "My Section".into(),
                    n: None,
                }],
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect("the raw title IS the address");

        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "My-Section".into(),
                    n: None,
                }],
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect_err("the sanitized spelling names no heading");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"My-Section\". No edit was applied; the batch \
                 is refused whole. Fix: list the document's section paths with a toc \
                 read (MCP read: sections[] omitted; CLI: a read with no --section), then feed \
                 its row back in one of the two addressing forms: the row's raw heading \
                 segments as an hpath array (one entry per heading, no joining), or its \
                 dewey ordinal (CLI: `--section 1.2`). The joined selector string splits \
                 on `/`, so a heading whose raw text carries one is reachable only by \
                 those two forms."
            )
        );
    }
}
