//! Conformance candidate rebuild — Go `body.ApplyForConformance` parity.
//! Rebuilds the candidate body under def rules so the verdict compares
//! normalized forms. Rev neutralization and string fidelity are load-bearing.

use super::BodyError;
use super::go_fmt::go_quote;

/// One address segment — the policy-boundary twin of `wire::HpathSeg` (C-08:
/// the address is the array). `n` is the optional 1-based occurrence among
/// same-parent siblings sharing the same RAW heading text — the same basis
/// `model::resolve_hpath_node` counts and the read face publishes, so the
/// occurrence the read face minted resolves here to the same section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seg {
    pub h: String,
    pub n: Option<u32>,
}

/// Render segments for a HUMAN to read — refusal strings and context values
/// only, never an address anything resolves against (C-08: the address is the
/// array).
///
/// The escaping encoding (`/` inside a segment writes `\/`, a literal `\`
/// doubles) stays injective, so a refusal names exactly one address. An
/// occurrence rides as `#n` on the segment carrying one (the same render
/// `wire-serve::display_hpath` uses).
fn hpath_display(segs: &[Seg]) -> String {
    segs.iter()
        .map(|s| {
            let text = s.h.replace('\\', r"\\").replace('/', r"\/");
            match s.n {
                Some(n) => format!("{text}#{n}"),
                None => text,
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// One put-plan edit (Go `body.Edit` as `plansToBodyEdits` builds it: the
/// daemon face's vocabulary; `Old` is never set on this face).
#[derive(Debug, Clone, Default)]
pub struct PlanEdit {
    /// replace | append | `replace_section` | `create_section` | `set_property`
    pub op: String,
    /// The address as SEGMENTS (C-08): a heading path one segment per heading,
    /// or a single segment carrying a `^id` / `#^id` block, a new section name,
    /// or a frontmatter key. Never a joined string (see the wire twin
    /// `wire::CheckWriteEdit::at`).
    pub target: Vec<Seg>,
    pub find: String,
    pub body: String,
    pub rev: String,
    pub all: bool,
}

/// Rebuild the candidate: apply `edits` to `prev` in memory. An all-no-op
/// batch returns the prev raw unchanged. Errors are the substrate-law
/// refusals only (resolve misses, anchor misses, ECAS teachings, would-corrupt).
pub fn rebuild(
    prev: &model::Document,
    edits: &[PlanEdit],
    build_doc: &dyn Fn(&str) -> model::Document,
) -> Result<model::Document, BodyError> {
    if edits.is_empty() {
        return Err(BodyError {
            code: "E_FAIL_LOUD".to_string(),
            message: "ApplyForConformance called with no edits".to_string(),
            remedy: "pass at least one Edit".to_string(),
            context: Vec::new(),
        });
    }
    let view = DocView::new(prev);
    let mut ops: Vec<SpliceOp> = Vec::new();
    let mut pending_nl: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for e in edits {
        let mut e = e.clone();
        let resolved = view.resolve_edit_target(&e);
        // Rev neutralization: the host's CAS domain is not ours to validate.
        if !e.rev.is_empty()
            && let Ok((Some(sec), _)) = &resolved
        {
            e.rev.clone_from(&sec.rev);
        }
        let new_ops = plan_edit_resolved(&view, &e, resolved, &pending_nl)?;
        for op in &new_ops {
            if op.start == op.end && op.replacement.last() == Some(&b'\n') {
                pending_nl.insert(op.start);
            }
        }
        ops.extend(new_ops);
    }
    if ops.is_empty() {
        return Ok(build_doc(&prev.raw));
    }
    assert_disjoint(&ops)?;
    let out = apply_splices(prev.raw.as_bytes(), &ops);
    let out = String::from_utf8(out).map_err(|_| BodyError {
        code: "E_WOULD_CORRUPT".to_string(),
        message: "the spliced result would not parse: invalid UTF-8".to_string(),
        remedy: "the edit was refused; the file is unchanged. Fix the content and retry"
            .to_string(),
        context: Vec::new(),
    })?;
    // Reparse gate: the engine's build is total (Go's parse-error branch is
    // unreachable here); the fence-at-EOF delta is the live half of the gate.
    if fence_open_at_eof(out.as_bytes()) && !fence_open_at_eof(prev.raw.as_bytes()) {
        return Err(BodyError {
            code: "E_WOULD_CORRUPT".to_string(),
            message: "the edit would leave an unterminated code fence that swallows the rest of the document".to_string(),
            remedy: "the edit was refused; the file is unchanged. Close the code fence (matching ``` or ~~~) and retry".to_string(),
            context: Vec::new(),
        });
    }
    Ok(build_doc(&out))
}

// --- the section/block/frontmatter view (Go Document surface this needs) ---

#[derive(Debug, Clone)]
struct SecX {
    /// dewey ordinal ("1.2") — display/address convenience
    n: String,
    /// The containment chain as RAW heading text, one entry per ancestor
    /// (self last). Was a `sanitize_heading`-joined string; see
    /// [`DocView::resolve_section`].
    hpath: Vec<String>,
    /// Per-chain-entry 1-based occurrence among same-parent siblings sharing
    /// the same raw text — `resolve_hpath_node`'s counting basis, so a
    /// selector `n` here names the section the committer would pick.
    occ: Vec<u32>,
    title: String,
    /// content span [start,end): heading line excluded, subtree-inclusive
    start: usize,
    end: usize,
    /// 1-based first content line
    start_line: usize,
    /// Go pkg/body section rev: xxhash64/8-hex over the content span
    rev: String,
}

#[derive(Debug, Clone)]
struct BlockX {
    id: String,
    /// content span excluding the trailing " ^id" marker (Go block index)
    start: usize,
    end: usize,
    /// 1-based line
    line: usize,
}

struct FmKeyX {
    key: String,
    /// value span on the key line (the "key: " prefix stays outside)
    start: usize,
    end: usize,
}

struct DocView<'a> {
    raw: &'a str,
    sections: Vec<SecX>,
    blocks: Vec<BlockX>,
    fm: Vec<FmKeyX>,
    has_fm: bool,
    /// insertion offset for a new key: the closing "---" line start
    fm_end: usize,
}

impl<'a> DocView<'a> {
    fn new(doc: &'a model::Document) -> Self {
        let raw = &doc.raw;
        let mut sections: Vec<SecX> = Vec::new();
        let mut ord: Vec<usize> = Vec::new();
        let mut odepth: Vec<u8> = Vec::new();
        let mut hstack: Vec<(u8, String)> = Vec::new();
        // Section index of each hstack ancestor, so occurrence counting keys on
        // the PARENT'S IDENTITY, not its depth — two same-titled sections under
        // different parents are not occurrences of each other.
        let mut istack: Vec<usize> = Vec::new();
        let mut occ_counts: std::collections::HashMap<(Option<usize>, String), u32> =
            std::collections::HashMap::new();
        for sec in super::check::doc_sections(doc) {
            // dewey ordinal (pkg/body map.go algorithm)
            while odepth.last().is_some_and(|d| *d > sec.depth) {
                ord.pop();
                odepth.pop();
            }
            if odepth.last() == Some(&sec.depth) {
                *ord.last_mut().unwrap() += 1;
            } else {
                ord.push(1);
                odepth.push(sec.depth);
            }
            // raw hpath chain (C-08: no sanitize, the address is the array)
            while hstack.last().is_some_and(|(d, _)| *d >= sec.depth) {
                hstack.pop();
                istack.pop();
            }
            // 1-based occurrence among this parent's same-raw-text children —
            // `resolve_hpath_node`'s basis (never document order, never
            // position among ALL siblings).
            let parent = istack.last().copied();
            let occ = occ_counts
                .entry((parent, sec.title.clone()))
                .and_modify(|c| *c += 1)
                .or_insert(1);
            let mut occ_chain = parent.map_or_else(Vec::new, |p| sections[p].occ.clone());
            occ_chain.push(*occ);
            hstack.push((sec.depth, sec.title.clone()));
            istack.push(sections.len());
            sections.push(SecX {
                n: ord
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("."),
                hpath: hstack.iter().map(|(_, s)| s.clone()).collect(),
                occ: occ_chain,
                title: sec.title.clone(),
                start: sec.content_span.start,
                end: sec.content_span.end,
                start_line: sec.start_line,
                rev: rev8(&raw.as_bytes()[sec.content_span.clone()]),
            });
        }
        let mut blocks = Vec::new();
        collect_blocks(&doc.root, raw, &mut blocks);
        let (fm_keys, has_fm, fm_end) = fm_index(raw);
        DocView {
            raw,
            sections,
            blocks,
            fm: fm_keys,
            has_fm,
            fm_end,
        }
    }

    /// Go `resolveSection`, re-grained to SEGMENTS (C-08): the address matches a
    /// section's full containment chain, byte-exactly on RAW heading text, or —
    /// as the single-segment shorthand the Go face has always had — its bare
    /// title. Zero = `E_NO_MATCH`; >1 = `E_AMBIGUOUS` with candidate ordinals.
    ///
    /// Comparison is segment-wise and never re-derives a boundary from text, so
    /// a heading whose own text contains `/` stays addressable (`["A/B"]`) and
    /// distinct from the nested path `["A","B"]` — the u14/R5 collision class,
    /// closed here rather than merely bounded.
    ///
    /// The occurrence law is `model::resolve_hpath_node`'s: a segment's
    /// `n: None` abstains here and ambiguity refuses below (never silently
    /// picks); `n: Some(k)` demands the k-th same-raw-text sibling under the
    /// same parent, so the address the read face publishes over duplicate
    /// headings resolves to the section the committer would pick. The
    /// bare-title shorthand carries no occurrence: a lone segment with `n` set
    /// matches only as a full depth-1 chain — anything else falls closed.
    fn resolve_section(&self, hpath: &[Seg]) -> Result<&SecX, BodyError> {
        let bare = match hpath {
            [only] if only.n.is_none() => Some(only.h.as_str()),
            _ => None,
        };
        let chain_matches = |s: &SecX| {
            s.hpath.len() == hpath.len()
                && hpath
                    .iter()
                    .enumerate()
                    .all(|(i, sel)| sel.h == s.hpath[i] && sel.n.is_none_or(|k| k == s.occ[i]))
        };
        let hits: Vec<&SecX> = self
            .sections
            .iter()
            .filter(|s| chain_matches(s) || bare.is_some_and(|b| s.title == b))
            .collect();
        match hits.len() {
            0 => Err(BodyError {
                code: "E_NO_MATCH".to_string(),
                message: format!(
                    "no section addressed by {}",
                    go_quote(&hpath_display(hpath))
                ),
                remedy: "address the section by its `hpath` segments, exactly as the read \
                     face's toc publishes them — one array entry per heading, raw text, \
                     no joining"
                    .to_string(),
                context: vec![("hpath".to_string(), hpath_display(hpath))],
            }),
            1 => Ok(hits[0]),
            n => {
                let cands: Vec<String> = hits
                    .iter()
                    .map(|h| format!("{} (line {})", h.n, h.start_line))
                    .collect();
                Err(BodyError {
                    code: "E_AMBIGUOUS".to_string(),
                    message: format!(
                        "{} is ambiguous: {} sections share this heading",
                        go_quote(&hpath_display(hpath)),
                        n
                    ),
                    remedy: format!(
                        "qualify with the full heading path, or pass the occurrence — the \
                         read face publishes it as `n` on the ambiguous segment; candidates: {}",
                        cands.join(", ")
                    ),
                    context: vec![
                        ("hpath".to_string(), hpath_display(hpath)),
                        ("candidates".to_string(), cands.join(", ")),
                    ],
                })
            }
        }
    }

    /// Go `resolveBlock`: the section view of a `^id` block.
    fn resolve_block(&self, id: &str) -> Result<SecX, BodyError> {
        let hits: Vec<&BlockX> = self.blocks.iter().filter(|b| b.id == id).collect();
        match hits.len() {
            0 => Err(BodyError {
                code: "E_NO_MATCH".to_string(),
                message: format!("no block ^{id}"),
                remedy: "block ids inside fences are not addressable; check the id".to_string(),
                context: vec![("block".to_string(), id.to_string())],
            }),
            1 => {
                let b = hits[0];
                Ok(SecX {
                    n: format!("^{id}"),
                    hpath: vec![format!("^{id}")],
                    occ: vec![1],
                    title: id.to_string(),
                    start: b.start,
                    end: b.end,
                    start_line: b.line,
                    rev: rev8(&self.raw.as_bytes()[b.start..b.end]),
                })
            }
            _ => {
                let lines: Vec<String> = hits.iter().map(|b| b.line.to_string()).collect();
                Err(BodyError {
                    code: "E_AMBIGUOUS".to_string(),
                    message: format!(
                        "block ^{} is declared {} times (lines {})",
                        id,
                        hits.len(),
                        lines.join(", ")
                    ),
                    remedy: "block ids must be unique; rename the duplicates".to_string(),
                    context: vec![
                        ("block".to_string(), id.to_string()),
                        ("lines".to_string(), lines.join(", ")),
                    ],
                })
            }
        }
    }

    /// Go `resolveEditTarget`: (section view, home policy-section name).
    #[allow(clippy::type_complexity)]
    fn resolve_edit_target(&self, e: &PlanEdit) -> Result<(Option<SecX>, String), BodyError> {
        if e.op == "create_section" {
            return Ok((None, hpath_display(&e.target)));
        }
        if e.op == "set_property" {
            return Ok((None, String::new()));
        }
        if let Some(id) = block_ref(&e.target) {
            let sec = self.resolve_block(id)?;
            let home = self.innermost_section(sec.start);
            return Ok((Some(sec), home));
        }
        let sec = self.resolve_section(&e.target)?;
        Ok((Some(sec.clone()), sec.title.clone()))
    }

    /// The innermost section whose content span contains `at`.
    fn innermost_section(&self, at: usize) -> String {
        let mut best: Option<&SecX> = None;
        for s in &self.sections {
            if s.start <= at
                && at < s.end
                && best.is_none_or(|b| s.end - s.start <= b.end - b.start)
            {
                best = Some(s);
            }
        }
        best.map(|s| s.title.clone()).unwrap_or_default()
    }
}

fn collect_blocks(node: &model::Node, raw: &str, out: &mut Vec<BlockX>) {
    if let model::NodeKind::Anchor { name } = &node.kind {
        // The model anchor node spans its host LINE; the Go block index span
        // excludes the trailing " ^id" marker.
        let line = &raw[node.span.clone()];
        let trimmed = line.trim_end_matches([' ', '\t']);
        let marker = format!("^{name}");
        let mut end = node.span.start + trimmed.len();
        if trimmed.ends_with(&marker) {
            end = node.span.start + trimmed.len() - marker.len();
            let head = &raw[node.span.start..end];
            end = node.span.start + head.trim_end_matches([' ', '\t']).len();
        }
        out.push(BlockX {
            id: name.clone(),
            start: node.span.start,
            end,
            line: raw.as_bytes()[..node.span.start]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1,
        });
    }
    for child in &node.children {
        collect_blocks(child, raw, out);
    }
}

/// The block-ref arm of an address: a `^id` / `#^id` block is a SINGLE segment
/// by construction (a block id is `[A-Za-z0-9-]`, so it can carry no path), and
/// a multi-segment address is therefore never one — checking only the lone
/// segment keeps a heading literally named `^x` nested under a parent from
/// being read as a block.
fn block_ref(target: &[Seg]) -> Option<&str> {
    let [only] = target else { return None };
    if let Some(id) = only.h.strip_prefix("#^") {
        return Some(id);
    }
    only.h.strip_prefix('^')
}

/// The frontmatter key index: per top-level key, the VALUE span on its line
/// (the "key: " prefix outside), plus the closing-`---` insertion offset.
fn fm_index(raw: &str) -> (Vec<FmKeyX>, bool, usize) {
    let Some(rest) = raw
        .strip_prefix("---")
        .and_then(|r| r.strip_prefix('\r').or(Some(r)))
        .and_then(|r| r.strip_prefix('\n'))
    else {
        return (Vec::new(), false, 0);
    };
    let base = raw.len() - rest.len();
    let mut keys = Vec::new();
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if text == "---" {
            return (keys, true, base + offset);
        }
        if !line.starts_with([' ', '\t', '#'])
            && let Some(colon) = text.find(':')
        {
            let key = text[..colon].trim().to_string();
            if !key.is_empty() {
                let mut vstart = colon + 1;
                if text.as_bytes().get(vstart) == Some(&b' ') {
                    vstart += 1;
                }
                keys.push(FmKeyX {
                    key,
                    start: base + offset + vstart.min(text.len()),
                    end: base + offset + text.len(),
                });
            }
        }
        offset += line.len();
    }
    (Vec::new(), false, 0) // unterminated fm: Go hasFM=false path
}

// --- the plan ops (planEditResolved switch) ---

#[derive(Debug, Clone)]
struct SpliceOp {
    start: usize,
    end: usize,
    replacement: Vec<u8>,
}

fn plan_edit_resolved(
    view: &DocView<'_>,
    e: &PlanEdit,
    resolved: Result<(Option<SecX>, String), BodyError>,
    pending_nl: &std::collections::HashSet<usize>,
) -> Result<Vec<SpliceOp>, BodyError> {
    let (sec, section) = resolved?;
    match e.op.as_str() {
        "append" => plan_append(view, e, sec.as_ref(), &section, pending_nl),
        "replace_section" => plan_replace_section(e, sec.as_ref(), &section),
        "create_section" => plan_create_section(view, e),
        "set_property" => plan_set_property(view, e),
        "replace" => plan_anchored(view, e, sec.as_ref(), &section),
        other => Err(BodyError {
            code: "E_FAIL_LOUD".to_string(),
            message: format!("unknown edit op {}", go_quote(other)),
            remedy: "use one of: replace, append, replace_section, create_section, set_property"
                .to_string(),
            context: Vec::new(),
        }),
    }
}

fn plan_append(
    view: &DocView<'_>,
    e: &PlanEdit,
    sec: Option<&SecX>,
    section: &str,
    pending_nl: &std::collections::HashSet<usize>,
) -> Result<Vec<SpliceOp>, BodyError> {
    if let Some(id) = block_ref(&e.target) {
        return Err(BodyError {
            code: "E_FAIL_LOUD".to_string(),
            message: format!(
                "append to a block anchor {} is not supported: it would splice before the block's \" ^id\" marker and orphan it",
                go_quote(&hpath_display(&e.target))
            ),
            remedy: "append targets a section (pass the containing heading path); to change the block's line use replace with a Find anchor".to_string(),
            context: vec![
                ("block".to_string(), hpath_display(&e.target)),
                ("section".to_string(), section.to_string()),
            ],
        })
        .inspect_err(|_err| {
            let _ = id;
        });
    }
    let sec = sec.expect("append resolved a section");
    let at = sec.end;
    let mut payload = ensure_trailing_nl(&e.body);
    if at > 0 && view.raw.as_bytes()[at - 1] != b'\n' && !pending_nl.contains(&at) {
        payload.insert(0, b'\n');
    }
    Ok(vec![SpliceOp {
        start: at,
        end: at,
        replacement: payload,
    }])
}

fn plan_replace_section(
    e: &PlanEdit,
    sec: Option<&SecX>,
    section: &str,
) -> Result<Vec<SpliceOp>, BodyError> {
    let sec = sec.expect("replace_section resolved a section");
    if e.rev.is_empty() {
        // A BLOCK target (`sec.n` is its `^id`) speaks the block voice: the
        // subject is the block the caller addressed, and the remedy names the
        // read that mints the BLOCK's own token — the section's rev can never
        // unlock a block rewrite, and a pre-flight that fires before the
        // committer (putregistry 4b before 4c) is the message the caller
        // actually sees (W-2 acceptance: a refusal must not send its reader
        // to a token no door accepts).
        if sec.n.starts_with('^') {
            return Err(BodyError {
                code: "ECAS".to_string(),
                message: format!(
                    "replace_section on {} requires a fresh rev (a whole-block rewrite is destructive)",
                    go_quote(&sec.n)
                ),
                remedy: format!(
                    "read the block (sections:[{}]) and pass its rev",
                    go_quote(&sec.n)
                ),
                context: vec![
                    ("block".to_string(), sec.n.clone()),
                    ("current_rev".to_string(), sec.rev.clone()),
                ],
            });
        }
        return Err(BodyError {
            code: "ECAS".to_string(),
            message: format!(
                "replace_section on {} requires a fresh rev (whole-section rewrite is destructive)",
                go_quote(section)
            ),
            // The value NAMED here would be `sec.rev` — rev8, message parity
            // only (see `rev8`). Naming it in the slot that demands a CAS sends
            // the caller back with a token every door refuses, so the remedy
            // points at the read that mints the real one (the `plan.rs:295`
            // twin's wording).
            remedy: "read the section and pass its rev".to_string(),
            context: vec![
                ("section".to_string(), section.to_string()),
                ("current_rev".to_string(), sec.rev.clone()),
            ],
        });
    }
    if e.rev != sec.rev {
        return Err(conflict_err(section, &e.rev, sec));
    }
    let payload = if e.body.is_empty() {
        Vec::new()
    } else {
        ensure_trailing_nl(&e.body)
    };
    Ok(vec![SpliceOp {
        start: sec.start,
        end: sec.end,
        replacement: payload,
    }])
}

fn plan_create_section(view: &DocView<'_>, e: &PlanEdit) -> Result<Vec<SpliceOp>, BodyError> {
    if view.resolve_section(&e.target).is_ok() {
        return Err(BodyError {
            code: "E_EXISTS".to_string(),
            message: format!(
                "section {} already exists",
                go_quote(&hpath_display(&e.target))
            ),
            remedy: "target the existing section with append/replace_section, or create under a distinct heading".to_string(),
            context: vec![("section".to_string(), hpath_display(&e.target))],
        });
    }
    let mut b: Vec<u8> = Vec::new();
    if !view.raw.is_empty() && !view.raw.ends_with('\n') {
        b.push(b'\n');
    }
    b.extend_from_slice(b"# ");
    // Preserved byte-exactly, and known wrong: the committer creates the LAST
    // segment under the parent the earlier segments name, so a candidate heading
    // spelled `Notes/Fresh` is a section that will never exist. A pre-existing
    // policy-evaluation defect, deliberately left alone.
    b.extend_from_slice(hpath_display(&e.target).as_bytes());
    b.push(b'\n');
    b.extend_from_slice(&ensure_trailing_nl(&e.body));
    let at = view.raw.len();
    Ok(vec![SpliceOp {
        start: at,
        end: at,
        replacement: b,
    }])
}

fn plan_set_property(view: &DocView<'_>, e: &PlanEdit) -> Result<Vec<SpliceOp>, BodyError> {
    // A frontmatter key is a SINGLE hpath segment: `yaml_safe_key` admits the
    // flat dotted spelling (`task.index.caps`) but no `/`, so a multi-segment
    // address is refused by that charset owner below rather than silently
    // taking a piece of itself.
    let joined = hpath_display(&e.target);
    let (key, val) = (&joined, &e.body);
    // The key passes the ONE charset owner here — the composed line is
    // `{key}: {value}`, so an unvalidated KEY forges frontmatter exactly as an
    // unvalidated VALUE does.
    let safe_key = yaml_safe_key(key).map_err(|InvalidPropertyKey| BodyError {
        code: "E_FAIL_LOUD".to_string(),
        message: format!("invalid frontmatter key {}", go_quote(key)),
        remedy: INVALID_PROPERTY_KEY_REMEDY.to_string(),
        context: vec![("key".to_string(), key.clone())],
    })?;
    // The value passes the ONE quoting owner here — BEFORE the frontmatter
    // probe — so the multi-line refusal keeps its Go-golden ordering.
    let safe = yaml_safe_value(val).map_err(|MultiLineValue| BodyError {
        code: "E_FAIL_LOUD".to_string(),
        message: format!("property value for {} contains a newline", go_quote(key)),
        remedy:
            "frontmatter values are single-line in v1; put multi-line content in a body section"
                .to_string(),
        context: vec![("key".to_string(), key.clone())],
    })?;
    if !view.has_fm {
        return Err(BodyError {
            code: "E_NO_MATCH".to_string(),
            message: format!(
                "the document has no frontmatter block to set {} in",
                go_quote(key)
            ),
            remedy: "add a frontmatter block (--- ... ---) at the top of the file, then retry"
                .to_string(),
            context: vec![("key".to_string(), key.clone())],
        });
    }
    for k in &view.fm {
        if k.key == *key {
            // § A.6.3c: a semantic no-op keeps the stored spelling. The exact
            // span bytes are spliced back, so the document is byte-identical
            // and nothing computed over SOURCE BYTES (§ A.6.2) moves.
            let stored = &view.raw[k.start..k.end];
            if fm_spelling_preserves(stored, val) {
                return Ok(vec![SpliceOp {
                    start: k.start,
                    end: k.end,
                    replacement: stored.as_bytes().to_vec(),
                }]);
            }
            let mut repl = safe.clone().into_bytes();
            // The guard tests the ENCODED value, never the caller's (§ A.6.3b).
            // The two differ exactly where this law bites: the empty string
            // encodes to `""`, so a guard on `val` would emit `note:""` over a
            // stored bare `note:` line — bytes every external YAML parser
            // reads as no property at all, voiding the whole block.
            if !safe.is_empty()
                && k.start == k.end
                && k.start > 0
                && view.raw.as_bytes()[k.start - 1] == b':'
            {
                repl.insert(0, b' ');
            }
            return Ok(vec![SpliceOp {
                start: k.start,
                end: k.end,
                replacement: repl,
            }]);
        }
    }
    // One uniform line shape for a create (§ A.6.3b). The former empty-value
    // case emitted a bare `{key}:\n` — the YAML null § A.6.3 says this plane
    // cannot express, forged out of a caller's empty string. The encoder never
    // returns empty bytes, so the uniform shape needs no special case.
    let line = format!("{safe_key}: {safe}\n");
    Ok(vec![SpliceOp {
        start: view.fm_end,
        end: view.fm_end,
        replacement: line.into_bytes(),
    }])
}

/// A `set_property` key outside the frontmatter charset: the ONE refusal
/// `yaml_safe_key` mints. Each caller renders it in its own vocabulary
/// (`BodyError` here, a wire `bad_request` at the splice face).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPropertyKey;

impl std::fmt::Display for InvalidPropertyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a frontmatter key is dotted segments of [A-Za-z0-9_-]+")
    }
}

/// A frontmatter key that has passed `yaml_safe_key` — the ONLY spelling the
/// `{key}: {value}` composition accepts. The field is private, so this type
/// cannot be minted outside this module: a call site that composes a key
/// without discharging `yaml_safe_key`'s `Result` does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SafeKey<'a>(&'a str);

impl SafeKey<'_> {
    /// The validated key bytes (address use — lookups and edit targets).
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0
    }
}

impl std::fmt::Display for SafeKey<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// The `set_property` KEY owner, S4b's sibling for the other half of the
/// composed `{key}: {value}` line. A key outside the grammar carries `: `, a
/// newline or a `---` fence into frontmatter bytes and forges keys the caller
/// never named, so it is REFUSED, never sanitized. Fallibility is the guard:
/// `SafeKey` has no other constructor.
///
/// The grammar is dotted segments of `[A-Za-z0-9_-]+` — the FLAT DOTTED key
/// law `docs/run-plane.md` mandates for the task grammar and the birth path
/// already writes (`task.index.caps`). The patch face refusing what birth
/// landed made one law into two and pushed callers off the face onto disk
/// edits (dogfood r3 f6). A `.` separates segments and is never a segment
/// byte: `.`, a leading or trailing dot, and `..` name no property and stay
/// refused. Nothing widens — a dot carries no `: `, no newline, no fence.
///
/// # Errors
/// `InvalidPropertyKey` when the key is empty, carries an empty segment, or
/// carries a byte outside `[A-Za-z0-9_-.]`.
pub fn yaml_safe_key(key: &str) -> Result<SafeKey<'_>, InvalidPropertyKey> {
    if key.is_empty()
        || key.split('.').any(|seg| {
            seg.is_empty()
                || !seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return Err(InvalidPropertyKey);
    }
    Ok(SafeKey(key))
}

/// The ONE spelling of the key law every door teaches. It used to be two
/// literals — one in the rebuild committer, one at the wire splice face — so a
/// caller's recovery quality was a function of which door they entered, and a
/// law change had two places to miss.
pub const INVALID_PROPERTY_KEY_REMEDY: &str =
    "a property key is dotted segments of [A-Za-z0-9_-]+ (single line, no spaces or ':')";

/// The uniform `InvalidPropertyKey` refusal sentence, `multi_line_value_refusal`'s
/// sibling for the KEY half — for doors that speak one string (the wire face).
/// The rebuild door carries `message`/`remedy` apart and pairs its own message
/// with `INVALID_PROPERTY_KEY_REMEDY`; both render the same bytes.
#[must_use]
pub fn invalid_property_key_refusal(key: &str) -> String {
    format!(
        "invalid frontmatter key {} — {INVALID_PROPERTY_KEY_REMEDY}",
        go_quote(key)
    )
}

/// A multi-line `set_property` value: the ONE refusal `yaml_safe_value` mints.
/// Each caller renders it in its own vocabulary (`BodyError` here, a wire
/// `bad_request` at the splice face).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiLineValue;

impl std::fmt::Display for MultiLineValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a frontmatter value may not contain a newline")
    }
}

/// The uniform `MultiLineValue` refusal sentence, spoken by EVERY value-plane
/// write door (wire-contract § A.6.3a). One law refused in two dialects is two
/// laws to the callers who meet it — recovery quality became a function of
/// which door the caller entered — so the sentence lives here, beside the
/// refusal it renders, and every door calls it rather than spelling its own.
///
/// A door MAY append its own provenance clause (which caller value carried the
/// newline); the sentence itself is fixed: the key by name, the v1 single-line
/// rule, and the body-section escape.
#[must_use]
pub fn multi_line_value_refusal(key: &str) -> String {
    format!(
        "property value for \"{key}\" contains a newline — frontmatter values are \
         single-line in v1; put multi-line content in a body section"
    )
}

/// The `set_property` value ENCODER — wire-contract § A.6.3, and the inverse of
/// the § A.6.1 decode every read seam serves through.
///
/// **The rule is one sentence:** emit the plain form when the plain form
/// decodes back to exactly the caller's string; otherwise emit a double-quoted
/// scalar. The plane is typed `string`, so any emit that would be read back as
/// something else — a null, a nested collection, a comment, a mapping, or a
/// quoted scalar the caller never wrote — is a value the engine forged.
///
/// [`needs_quoting`] carries the enumerated triggers. ONE spelling stays
/// VERBATIM: a one-level flow list (`[a, b]`), the only way this string plane
/// can author a list. **The typed-scalar carve-out is RETIRED** (2026-08-23,
/// card `all-digit-short-ids-read-as-int`): `true`, `7` and every all-digit
/// agent short id used to emit plain, so a caller string like `19895504` read
/// back as an INTEGER in every foreign parser, and `02146210` read back as
/// 576648 (YAML 1.1 octal, `PyYAML`) — on `owner`, `session`, `agent`, `from`,
/// the join keys of the fleet's whole tree. A value the caller spelled as a
/// string is now written so it reads back as that string; a timestamp still
/// emits plain because `serde_yaml` reads it back as a string already.
///
/// The double-quoted form is the fleet-canonical one (`ccc-cli task claim`
/// writes it), so two writers on one tree stop churning each other's bytes.
/// **This replaces the former single-quoted emit and the former verbatim
/// list-of-list emit** — the latter made `owner: [[b1892b5a]]` out of a flat
/// caller string and then let the I4 nested-frontmatter law refuse it, blaming
/// the caller for the emitter's nesting (dogfood season 1, finding 2).
///
/// SHARED (U8b): the splice plan-lowering quotes `set_property` values through
/// THIS predicate — single-owner discipline, so the `check_write` candidate and
/// the written bytes cannot drift.
///
/// # Errors
/// `MultiLineValue` when the value carries a `\n`/`\r` (D11): the server writes
/// `{key}: {value}`, and a single-line YAML scalar CANNOT carry a raw newline —
/// an escaped-scalar approach leaks, so a multi-line value is refused, never
/// sanitized. Fallibility is the guard: no caller can mint a quoted value
/// without passing it.
pub fn yaml_safe_value(val: &str) -> Result<String, MultiLineValue> {
    if val.contains(['\n', '\r']) {
        return Err(MultiLineValue);
    }
    if needs_quoting(val) {
        return Ok(model::scalar::double_quote(val));
    }
    Ok(val.to_string())
}

/// The value encoder for a door that carries a TYPED list arm
/// (`create(props = {…})`, D6) — [`yaml_safe_value`] minus its one-level
/// flow-list carve-out.
///
/// That carve-out is the string plane's only way to author a list, so it stays
/// where the plane is a string (the patch face). Here it is a LEAK: a caller
/// that hands this door the scalar `"[a, b]"` means the seven-byte string, and
/// the list it wants it spells `PropValue::List`. So a plain emit that would
/// read back as a collection quotes.
///
/// **The former named residual is closed.** `"7"` and `"true"` used to land
/// verbatim here and read back as int / bool; since 2026-08-23 (card
/// `all-digit-short-ids-read-as-int`) [`yaml_safe_value`] quotes them, and this
/// door inherits that. `"2026-08-07"` still lands verbatim — `serde_yaml` reads
/// a timestamp back as a string, so the plain emit already round-trips.
/// **Residual, named:** this door has no numeric or boolean typed arm, so a
/// caller who MEANS the integer 7 can no longer author it through `props`
/// (`PropValue::List` remains the one typed arm). A def-declared `int`/`bool`
/// property (`shape.rs` `SHAPE_INT`/`SHAPE_BOOL`) is therefore unwritable
/// through the value plane and must be born in the record's own body bytes.
///
/// # Errors
/// `MultiLineValue` — the D11 law, unchanged.
pub fn yaml_safe_scalar(val: &str) -> Result<String, MultiLineValue> {
    let encoded = yaml_safe_value(val)?;
    if encoded == val
        && matches!(
            super::fm::classify_scalar_or_flow(val),
            super::fm::FmValue::List(_)
        )
    {
        return Ok(model::scalar::double_quote(val));
    }
    Ok(encoded)
}

/// ONE member of a flow list (`tags: [a, b]`), encoded for the position it
/// lands in — [`yaml_safe_value`]'s sibling for the one shape that plane
/// cannot express as a scalar. A member is quoted whenever the plain emit
/// would be read back as something else IN A FLOW CONTEXT: everything
/// [`yaml_safe_value`] already quotes, plus the flow punctuation (`,` `[` `]`
/// `{` `}`) that would end the member early or nest a collection inside it.
/// The verbatim one-level-flow-list carve-out `yaml_safe_value` grants a
/// scalar does NOT survive here: `[a, b]` as a MEMBER is a nested list, so it
/// quotes.
///
/// # Errors
/// `MultiLineValue` — same D11 law as every other value door: a member
/// carrying `\n`/`\r` is refused, never sanitized.
pub fn yaml_safe_flow_item(val: &str) -> Result<String, MultiLineValue> {
    let encoded = yaml_safe_value(val)?;
    if encoded == val && val.contains([',', '[', ']', '{', '}']) {
        return Ok(model::scalar::double_quote(val));
    }
    Ok(encoded)
}

/// A LIST frontmatter value — `create(props = {"tags": [...]})`'s door-side
/// spelling (D6). One line, flow style (`[a, b]`), because frontmatter values
/// are single-line in v1 and this is the spelling the corpus already carries
/// (`tags: [type/agent]`); an empty list is `[]`. Members go through
/// [`yaml_safe_flow_item`], so a hostile member quotes instead of forging a
/// second member or a nested collection.
///
/// # Errors
/// `MultiLineValue` — a member carries `\n`/`\r`.
pub fn yaml_safe_flow(items: &[String]) -> Result<String, MultiLineValue> {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&yaml_safe_flow_item(item)?);
    }
    out.push(']');
    Ok(out)
}

/// The § A.6.3c preservation predicate — may an UPDATE keep the stored value
/// spelling? True when the stored spelling already decodes (§ A.6.1) to
/// exactly the caller's string AND classifies as neither `Nested` nor `Null` —
/// the two classes this string plane cannot express, which a write must repair
/// to the quoted canonical form rather than leave standing.
///
/// The newline clause keeps D11 uniform: a stored escape spelling (`"a\nb"`)
/// decodes to a text the doors REFUSE to accept, and preservation must not
/// smuggle it past that refusal as a "no-op".
///
/// **Deliberately NOT widened by the 2026-08-23 encoder change** (card
/// `all-digit-short-ids-read-as-int`). The write side now quotes a value a YAML
/// parser would read back as a number, a bool or a 1.1-typed spelling, but this
/// predicate keeps the two exclusions it had. So a stored `owner: 19895504`
/// survives a same-value write-back with its bytes, and the 37 such ids on the
/// live root are repaired only by a write that CHANGES the value. Two reasons,
/// both load-bearing: preservation is what stops two writers oscillating over
/// one line (§ A.6.3c's whole purpose), and repairing on a no-op would turn a
/// 2.8 % population into a rewrite of every record anyone touches. Repairing
/// the class on purpose is a corpus migration, not a write-door behaviour.
///
/// ONE owner, shared by every § A.6.3a door: the value-span splice above keeps
/// the span bytes; the line-composing doors keep the spelling through
/// [`yaml_preserve_or_encode`].
#[must_use]
pub fn fm_spelling_preserves(stored: &str, val: &str) -> bool {
    !val.contains(['\n', '\r'])
        && model::scalar::text(stored) == val
        && !matches!(
            super::fm::classify_scalar_or_flow(stored.trim()),
            super::fm::FmValue::Nested | super::fm::FmValue::Null
        )
}

/// § A.6.3c at a LINE-COMPOSING door: given the stored `{key}: {value}` line
/// (when the key exists), keep the stored value spelling on a semantic no-op;
/// otherwise encode fresh through [`yaml_safe_value`]. The colon remainder is
/// addressed exactly as `fm_index` addresses it — one leading space skipped —
/// so the two readers of a key line cannot drift.
///
/// # Errors
/// `MultiLineValue` when the caller's value carries a `\n`/`\r` (D11) — the
/// refusal is uniform whether or not a stored spelling exists.
pub fn yaml_preserve_or_encode(
    stored_line: Option<&str>,
    val: &str,
) -> Result<String, MultiLineValue> {
    let stored = stored_line.and_then(|line| {
        let colon = line.find(':')?;
        let rest = &line[colon + 1..];
        Some(rest.strip_prefix(' ').unwrap_or(rest))
    });
    if let Some(stored) = stored
        && fm_spelling_preserves(stored, val)
    {
        return Ok(stored.trim().to_string());
    }
    yaml_safe_value(val)
}

/// The § A.6.3 quote triggers, enumerated. Each row names a way the PLAIN emit
/// would be read back as something other than the caller's string.
fn needs_quoting(val: &str) -> bool {
    // The trim is the read seam's first act (§ A.6.1), so surrounding
    // whitespace only survives inside quotes. Empty lands here too: a bare
    // `key: ` is a null, and this plane has no null to mean.
    if val != val.trim() || val.is_empty() {
        return true;
    }
    // A leading quote byte would be read back AS quoting, losing itself.
    if val.starts_with(['\'', '"']) {
        return true;
    }
    // An INTERIOR tab (the trim above only catches the outer ones). Measured
    // 2026-08-23 with `PyYAML`: `k: a<TAB>b` is "while scanning for the next
    // token" — the whole frontmatter block dies, not one key — while
    // `serde_yaml` reads the same bytes happily, so the parser oracle below
    // never flags it. Inside the double-quoted emit the tab is legal and reads
    // back verbatim.
    if val.contains('\t') {
        return true;
    }
    // A comment in value position: the value would read back empty.
    if val.starts_with('#') || val.contains(" #") || val.contains("\t#") {
        return true;
    }
    // A mapping in value position — the pre-A.6 trigger, kept.
    if val.contains(": ") || val.ends_with(':') {
        return true;
    }
    // Whatever the plain emit would PARSE as, asked of the checker's own
    // classifier so the encoder cannot judge by a second rule. `Nested` is the
    // I4 class the emitter must never manufacture; `Null` is the type this
    // plane cannot express.
    if matches!(
        super::fm::classify_scalar_or_flow(val),
        super::fm::FmValue::Nested | super::fm::FmValue::Null
    ) {
        return true;
    }
    // …and then the same question asked of a REAL YAML parser, because the
    // classifier above is the engine's own and is measurably more permissive
    // than YAML (2026-08-23, card 17, `props=` hostile table).
    //
    // Measured with `PyYAML` on `k: <val>`: a value opening `- `, `? `, `,`,
    // `[`-that-is-not-a-flow-list, `*`, `&`, `%`, `@`, `` ` `` or an
    // unterminated `[[a]] and [[b]]` makes bytes NO yaml parser can read —
    // the whole frontmatter block dies, not one key — and `&a`, `!t`, `>`,
    // `|` parse to something the caller never wrote. None of them tripped a
    // trigger above, so every door emitted them plain.
    if !plain_reads_back(val) {
        return true;
    }
    // …and finally the classes a YAML **1.1** resolver types and a 1.2 one does
    // not, because `serde_yaml` is not the only parser that opens these files.
    reads_typed_in_yaml_1_1(val)
}

/// The YAML **1.1** scalar classes that a 1.2 parser leaves as strings — the
/// blind spot of the `serde_yaml` oracle above.
///
/// The fleet reads this frontmatter with `PyYAML` and go-yaml (`yq`, Obsidian) as
/// well as with this engine, and both of those resolve the 1.1 schema. Measured
/// 2026-08-23 (card `all-digit-short-ids-read-as-int`): the agent short id
/// `02146210` is the STRING `"02146210"` to `serde_yaml` — a leading zero is
/// not a 1.2 integer — and the INTEGER 576 648 to `PyYAML`, which reads a
/// leading-zero digit run as octal. Deferring to the 1.2 parser alone would
/// have left the worse half of the defect standing: not a type change, a value
/// change, on the ids the whole tree joins by.
///
/// So the law is the UNION of the two schemas: a value the caller spelled as a
/// string must read back as that string whichever parser opens the file.
fn reads_typed_in_yaml_1_1(val: &str) -> bool {
    // `y`/`yes`/`on` and friends are booleans in 1.1, plain strings in 1.2.
    // (`PyYAML` itself types `yes`/`no`/`on`/`off` but not the one-letter
    // `y`/`n`; the 1.1 SPEC types those too, so they quote rather than depend
    // on which reader is right.)
    if matches!(
        val,
        "y" | "Y"
            | "yes"
            | "Yes"
            | "YES"
            | "n"
            | "N"
            | "no"
            | "No"
            | "NO"
            | "on"
            | "On"
            | "ON"
            | "off"
            | "Off"
            | "OFF"
    ) {
        return true;
    }
    // The two 1.1 resolver TAGS that are not types at all: `<<` resolves to
    // `tag:yaml.org,2002:merge` and `=` to `…:value`, and a plain emit of
    // either makes `PyYAML` refuse the whole block ("could not determine a
    // constructor for the tag") — the block dies, not one key. Measured
    // 2026-08-23; `serde_yaml` reads both back as strings, so the oracle above
    // never flags them. Quoted, `PyYAML` reads both as the caller's string.
    if matches!(val, "<<" | "=") {
        return true;
    }
    let body = val.strip_prefix(['+', '-']).unwrap_or(val);
    // The radix-prefixed integers. `0x1f` and `0o17` are 1.2 integers the
    // oracle above already catches, but the UNDERSCORE forms are 1.1-only:
    // `0x1_f` is 31 and `0b1_010` is 10 to `PyYAML`, plain strings to
    // `serde_yaml`. `0b…` is 1.1-only in every spelling. One arm covers all
    // three radices, underscores included.
    if let Some(rest) = body
        .strip_prefix("0x")
        .or_else(|| body.strip_prefix("0X"))
        .or_else(|| body.strip_prefix("0o"))
        .or_else(|| body.strip_prefix("0O"))
        .or_else(|| body.strip_prefix("0b"))
        .or_else(|| body.strip_prefix("0B"))
        && rest.bytes().any(|b| b.is_ascii_hexdigit())
        && rest.bytes().all(|b| b.is_ascii_hexdigit() || b == b'_')
    {
        return true;
    }
    // A digit run — decimal in both schemas, OCTAL in 1.1 when it leads with a
    // zero, and `1_000` is a number in 1.1 only. The whole class quotes: an id
    // is digits, and which integer it becomes must not depend on the reader.
    let digit_run = |s: &str| {
        s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().all(|b| b.is_ascii_digit() || b == b'_')
    };
    if digit_run(body) {
        return true;
    }
    // `1_000.5` — the 1.1 float spelling `serde_yaml` reads as a string.
    if let Some((int, frac)) = body.split_once('.')
        && (int.contains('_') || frac.contains('_'))
        && (int.is_empty() || digit_run(int))
        && (frac.is_empty() || digit_run(frac))
    {
        return true;
    }
    // Sexagesimal: `12:30` is 750 and `1:02:03` is 3723 to a 1.1 resolver, and
    // the colon with no space after it trips no trigger above. Each group after
    // the first is one base-60 digit — `PyYAML`'s `[0-5]?[0-9]` — so `1:60` and
    // `12:345` are strings to every parser and stay plain.
    let mut groups = body.split(':');
    let Some(first) = groups.next() else {
        return false;
    };
    let mut seen = false;
    for g in groups {
        seen = true;
        let base = g.split_once('.').map_or(g, |(a, _)| a);
        let base60 = matches!(base.len(), 1 | 2)
            && base.bytes().all(|b| b.is_ascii_digit())
            && base.parse::<u8>().is_ok_and(|n| n < 60);
        if !base60 {
            return false;
        }
    }
    seen && digit_run(first)
}

/// Would the PLAIN emit `key: {val}` read back as exactly `val`? Asked of
/// `serde_yaml` — the enumerated triggers above are the fast, teachable ones;
/// this is the one that cannot be out-enumerated.
///
/// ONE carve-out survives: a plain form that parses as a NON-string is legal
/// only when the checker's classifier reads it as a one-level flow list — the
/// single shape this string plane cannot otherwise author (§ A.6.3). Everything
/// else quotes, `serde_yaml`'s verdict deciding.
///
/// The `Bool | Int | Float | Timestamp` arm that used to sit beside `List` was
/// removed 2026-08-23 (card `all-digit-short-ids-read-as-int`): it let a value
/// the caller spelled as a string be emitted as a number or a boolean. 203 of
/// 8 125 distinct 8-hex ids on the live sessions root (2.5 %) are all-digit,
/// and 37 of them already sit bare in frontmatter under `session`, `agent`,
/// `from`, `owner`, `author` — read as integers by every foreign parser.
///
/// This oracle is HALF the law: `serde_yaml` resolves YAML 1.2, and the 1.1
/// classes it leaves as strings are [`reads_typed_in_yaml_1_1`]'s. Measured
/// cost of the two together: 806 of 29 270 distinct plain-spelled values
/// (2.754 %) change spelling on a CHANGED write — 515 ints, 245 floats, 37
/// short ids, 7 bools, 1 sexagesimal, 1 interior tab — all plain→quoted, none
/// quoted→plain, and `PyYAML` reads every changed emit back as the caller's
/// string. The timestamp class does not move: `serde_yaml` answers `String`
/// for it, so the plain emit already round-trips here (`PyYAML` still reads a
/// `datetime` — a named residual, § A.6.3).
fn plain_reads_back(val: &str) -> bool {
    let Ok(parsed) = serde_yaml::from_str::<serde_yaml::Value>(&format!("k: {val}\n")) else {
        return false;
    };
    let Some(read_back) = parsed.get("k") else {
        return false;
    };
    match read_back {
        serde_yaml::Value::String(s) => s == val,
        _ => matches!(
            super::fm::classify_scalar_or_flow(val),
            super::fm::FmValue::List(_)
        ),
    }
}

fn plan_anchored(
    view: &DocView<'_>,
    e: &PlanEdit,
    sec: Option<&SecX>,
    section: &str,
) -> Result<Vec<SpliceOp>, BodyError> {
    let sec = sec.expect("replace resolved a target");
    if e.find.is_empty() {
        return Err(BodyError {
            code: "E_NO_MATCH".to_string(),
            message: format!("{} requires a Find anchor", e.op),
            remedy: format!("set Find to the exact bytes to {}", e.op),
            context: vec![("section".to_string(), section.to_string())],
        });
    }
    let content = &view.raw.as_bytes()[sec.start..sec.end];
    let starts = find_anchors(content, sec.start, &e.find, e.all, section)?;

    if e.rev.is_empty() {
        if e.all {
            return Err(BodyError {
                code: "ECAS".to_string(),
                message: format!("an all-occurrence {} requires a rev", e.op),
                remedy: "read the section and pass its rev to confirm the current content"
                    .to_string(),
                context: vec![
                    ("section".to_string(), section.to_string()),
                    ("current_rev".to_string(), sec.rev.clone()),
                ],
            });
        }
        // omitted rev, single exact anchor → relaxation; the foreign_changes
        // warning is journal-derived and dropped on the conformance path.
    } else if e.rev != sec.rev {
        return Err(conflict_err(section, &e.rev, sec));
    }

    Ok(starts
        .into_iter()
        .map(|s| SpliceOp {
            start: s,
            end: s + e.find.len(),
            replacement: e.body.clone().into_bytes(),
        })
        .collect())
}

fn find_anchors(
    content: &[u8],
    base: usize,
    find: &str,
    all: bool,
    section: &str,
) -> Result<Vec<usize>, BodyError> {
    let fb = find.as_bytes();
    let mut starts = Vec::new();
    let mut off = 0usize;
    while off + fb.len() <= content.len() {
        match content[off..].windows(fb.len()).position(|w| w == fb) {
            Some(i) => {
                starts.push(base + off + i);
                off = off + i + fb.len();
            }
            None => break,
        }
    }
    if starts.is_empty() {
        return Err(BodyError {
            code: "E_NO_MATCH".to_string(),
            message: format!(
                "anchor {} not found in section {}",
                go_quote(find),
                go_quote(section)
            ),
            remedy: "read the section (md read) and copy the exact bytes; Find is byte-exact (no normalization)".to_string(),
            context: vec![
                ("section".to_string(), section.to_string()),
                ("find".to_string(), find.to_string()),
            ],
        });
    }
    if starts.len() > 1 && !all {
        return Err(BodyError {
            code: "E_AMBIGUOUS".to_string(),
            message: format!(
                "anchor {} occurs {} times in section {}",
                go_quote(find),
                starts.len(),
                go_quote(section)
            ),
            remedy: "extend Find to a unique span, or set All to edit every occurrence".to_string(),
            context: vec![
                ("section".to_string(), section.to_string()),
                ("count".to_string(), starts.len().to_string()),
            ],
        });
    }
    Ok(starts)
}

fn conflict_err(section: &str, expected: &str, sec: &SecX) -> BodyError {
    BodyError {
        code: "ECAS".to_string(),
        message: format!(
            "rev conflict on section {}: expected {}, current {}",
            go_quote(section),
            expected,
            sec.rev
        ),
        remedy: format!(
            "the section changed since you read it; re-read {} and recompose against the rev that read publishes",
            go_quote(section)
        ),
        context: vec![
            ("section".to_string(), section.to_string()),
            ("expected_rev".to_string(), expected.to_string()),
            ("current_rev".to_string(), sec.rev.clone()),
        ],
    }
}

// --- splice mechanics ---

fn assert_disjoint(ops: &[SpliceOp]) -> Result<(), BodyError> {
    let mut sorted: Vec<&SpliceOp> = ops.iter().collect();
    sorted.sort_by_key(|o| o.start);
    for w in sorted.windows(2) {
        if w[1].start < w[0].end {
            return Err(BodyError {
                code: "E_WOULD_CORRUPT".to_string(),
                message: "edits overlap on the same bytes".to_string(),
                remedy: "the edits target overlapping ranges; split them or resolve to non-overlapping anchors".to_string(),
                context: Vec::new(),
            });
        }
    }
    Ok(())
}

/// High→low STABLE application: same-offset insertions keep edit order.
fn apply_splices(src: &[u8], ops: &[SpliceOp]) -> Vec<u8> {
    let mut order: Vec<usize> = (0..ops.len()).collect();
    order.sort_by(|&a, &b| ops[b].start.cmp(&ops[a].start).then(b.cmp(&a)));
    let mut out = src.to_vec();
    for i in order {
        let op = &ops[i];
        out.splice(op.start..op.end, op.replacement.iter().copied());
    }
    out
}

/// Go `ensureTrailingNL` — SHARED write-discipline helper (U8b: the splice
/// plan-lowering in `wire-serve` composes the same payload bytes this rebuild
/// composes; one owner, two consumers). Neutral data only — no wire types.
#[must_use]
pub fn ensure_trailing_nl(s: &str) -> Vec<u8> {
    if s.is_empty() || s.ends_with('\n') {
        return s.as_bytes().to_vec();
    }
    let mut v = s.as_bytes().to_vec();
    v.push(b'\n');
    v
}

/// Go `fenceOpenAtEOF` (scan.go): frontmatter-skipped, `CommonMark` fence
/// delimiters (≤3 spaces indent, 3+ backticks/tildes; a backtick opener's
/// info string must not contain a backtick; a closer matches char + length).
fn fence_open_at_eof(src: &[u8]) -> bool {
    let body_start = frontmatter_body_start(src);
    let mut fence_ch = 0u8;
    let mut fence_len = 0usize;
    let mut off = body_start;
    loop {
        let line_start = off;
        let mut line_end = line_start;
        while line_end < src.len() && src[line_end] != b'\n' {
            line_end += 1;
        }
        let text = trim_trail_ws(&src[line_start..line_end]);
        if fence_ch != 0 {
            if let Some((ch, run_len, rest)) = fence_delim(text)
                && ch == fence_ch
                && run_len >= fence_len
                && trim_trail_ws(rest).is_empty()
            {
                fence_ch = 0;
                fence_len = 0;
            }
        } else if let Some((ch, run_len, rest)) = fence_delim(text)
            && (ch != b'`' || !rest.contains(&b'`'))
        {
            fence_ch = ch;
            fence_len = run_len;
        }
        if line_end >= src.len() {
            break;
        }
        off = line_end + 1;
    }
    fence_ch != 0
}

fn frontmatter_body_start(src: &[u8]) -> usize {
    let Ok(s) = std::str::from_utf8(src) else {
        return 0;
    };
    let Some(rest) = s
        .strip_prefix("---")
        .and_then(|r| r.strip_prefix('\r').or(Some(r)))
        .and_then(|r| r.strip_prefix('\n'))
    else {
        return 0;
    };
    let base = s.len() - rest.len();
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        offset += line.len();
        if text == "---" {
            return base + offset;
        }
    }
    0
}

fn trim_trail_ws(b: &[u8]) -> &[u8] {
    let mut end = b.len();
    while end > 0 && matches!(b[end - 1], b' ' | b'\t' | b'\r') {
        end -= 1;
    }
    &b[..end]
}

fn fence_delim(line: &[u8]) -> Option<(u8, usize, &[u8])> {
    let mut i = 0;
    while i < line.len() && i < 3 && line[i] == b' ' {
        i += 1;
    }
    if i >= line.len() {
        return None;
    }
    let ch = line[i];
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let mut run = 0;
    while i + run < line.len() && line[i + run] == ch {
        run += 1;
    }
    if run < 3 {
        return None;
    }
    Some((ch, run, &line[i + run..]))
}

/// Go pkg/body `rev8`: xxhash64 (seed 0) over the bytes, 16-hex zero-padded,
/// first 8. Reproduced dependency-free — MESSAGE parity only, never a CAS the
/// engine enforces.
#[must_use]
pub fn rev8(b: &[u8]) -> String {
    format!("{:016x}", xxh64(b, 0))[..8].to_string()
}

// XXH64 (Yann Collet, BSD) — straight from the spec.
const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

fn xxh64(data: &[u8], seed: u64) -> u64 {
    let len = data.len() as u64;
    let mut h: u64;
    let mut rest = data;
    if data.len() >= 32 {
        let mut v1 = seed.wrapping_add(P1).wrapping_add(P2);
        let mut v2 = seed.wrapping_add(P2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(P1);
        while rest.len() >= 32 {
            v1 = round(v1, u64le(&rest[0..8]));
            v2 = round(v2, u64le(&rest[8..16]));
            v3 = round(v3, u64le(&rest[16..24]));
            v4 = round(v4, u64le(&rest[24..32]));
            rest = &rest[32..];
        }
        h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        h = merge_round(h, v1);
        h = merge_round(h, v2);
        h = merge_round(h, v3);
        h = merge_round(h, v4);
    } else {
        h = seed.wrapping_add(P5);
    }
    h = h.wrapping_add(len);
    while rest.len() >= 8 {
        h ^= round(0, u64le(&rest[0..8]));
        h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        rest = &rest[8..];
    }
    if rest.len() >= 4 {
        h ^= u64::from(u32le(&rest[0..4])).wrapping_mul(P1);
        h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        rest = &rest[4..];
    }
    for &b in rest {
        h ^= u64::from(b).wrapping_mul(P5);
        h = h.rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^= h >> 32;
    h
}

fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2))
        .rotate_left(31)
        .wrapping_mul(P1)
}

fn merge_round(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
}

fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().expect("8 bytes"))
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().expect("4 bytes"))
}
