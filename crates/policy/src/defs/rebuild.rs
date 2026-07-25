//! The conformance candidate rebuild — Go `body.ApplyForConformance` +
//! `planEditResolved` ported byte-exactly: put-plan-vocabulary edits over the
//! PREV document produce the CANDIDATE document (or the rebuild refusal) the
//! I4 verdict judges. This is the write plane's plan/splice/reparse pipeline
//! MINUS flock/I3/CAS/journal/write:
//!
//! - per-edit rev is NEUTRALIZED to the current section rev when the target
//!   resolves (CAS is the host write engine's, in ITS OWN hash domain) — but a
//!   destructive `replace_section` still REQUIRES a rev (substrate guard), and
//!   the ECAS teaching embeds the section's Go-domain rev (xxhash64/8-hex over
//!   the content span — reproduced here for byte parity, used for MESSAGES
//!   only, never as a CAS the engine enforces);
//! - append newline discipline (`ensureTrailingNL` + the leading-`\n` rule
//!   with in-batch `pendingNL`), EOF `create_section` placement, single-line
//!   property injection, high→low disjoint splicing, and the reparse +
//!   `fenceOpenAtEOF` delta (`E_WOULD_CORRUPT`) mirror Splice by construction;
//! - NO journal dedupe (the journal is host-side): a dedupe-skippable append
//!   lands in the candidate — byte-identical text mints byte-identical
//!   findings, which the (rule|message) delta suppresses, so the verdict is
//!   unchanged (fixture-pinned);
//! - anchored-op `foreign_changes` relaxation warnings are journal-derived and
//!   DROPPED here — `ApplyForConformance` never consumes warnings.
//!
//! Every refusal string is verbatim from `internal/body/{map,splice}.go`.

use super::BodyError;
use super::go_fmt::go_quote;
// The heading address law has ONE owner (stage-2 S0): the frozen Go-text
// predicate in `model`, which `wire-map`'s projection re-exports. The local
// copy here trimmed with Rust's ambient `.trim()` — a set that agrees with Go
// today but is not pinned to it.
use model::gotext::sanitize_heading;

/// One put-plan edit (Go `body.Edit` as `plansToBodyEdits` builds it: the
/// daemon face's vocabulary; `Old` is never set on this face).
#[derive(Debug, Clone, Default)]
pub struct PlanEdit {
    /// replace | append | `replace_section` | `create_section` | `set_property`
    pub op: String,
    /// heading path, `^id` / `#^id` block, new section name, or fm key
    pub target: String,
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
    hpath: String,
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
        let mut sections = Vec::new();
        let mut ord: Vec<usize> = Vec::new();
        let mut odepth: Vec<u8> = Vec::new();
        let mut hstack: Vec<(u8, String)> = Vec::new();
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
            // sanitized hpath chain
            while hstack.last().is_some_and(|(d, _)| *d >= sec.depth) {
                hstack.pop();
            }
            hstack.push((sec.depth, sanitize_heading(&sec.title)));
            sections.push(SecX {
                n: ord
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("."),
                hpath: hstack
                    .iter()
                    .map(|(_, s)| s.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
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

    /// Go `resolveSection`: hpath matches a section's qualified `HPath` or its
    /// Title. Zero = `E_NO_MATCH`; >1 = `E_AMBIGUOUS` with candidate ordinals.
    fn resolve_section(&self, hpath: &str) -> Result<&SecX, BodyError> {
        let hits: Vec<&SecX> = self
            .sections
            .iter()
            .filter(|s| s.hpath == hpath || s.title == hpath)
            .collect();
        match hits.len() {
            0 => Err(BodyError {
                code: "E_NO_MATCH".to_string(),
                message: format!("no section addressed by {}", go_quote(hpath)),
                remedy: "run `md toc` to list the document's section paths".to_string(),
                context: vec![("hpath".to_string(), hpath.to_string())],
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
                        go_quote(hpath),
                        n
                    ),
                    remedy: format!(
                        "qualify with the full heading path or an ordinal; candidates: {}",
                        cands.join(", ")
                    ),
                    context: vec![
                        ("hpath".to_string(), hpath.to_string()),
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
                    hpath: format!("^{id}"),
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
            return Ok((None, e.target.clone()));
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

fn block_ref(target: &str) -> Option<&str> {
    if let Some(id) = target.strip_prefix("#^") {
        return Some(id);
    }
    target.strip_prefix('^')
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
                go_quote(&e.target)
            ),
            remedy: "append targets a section (pass the containing heading path); to change the block's line use replace with a Find anchor".to_string(),
            context: vec![
                ("block".to_string(), e.target.clone()),
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
        return Err(BodyError {
            code: "ECAS".to_string(),
            message: format!(
                "replace_section on {} requires a fresh rev (whole-section rewrite is destructive)",
                go_quote(section)
            ),
            remedy: format!(
                "read the section (md read) and pass its rev; current rev is {}",
                sec.rev
            ),
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
            message: format!("section {} already exists", go_quote(&e.target)),
            remedy: "target the existing section with append/replace_section, or create under a distinct heading".to_string(),
            context: vec![("section".to_string(), e.target.clone())],
        });
    }
    let mut b: Vec<u8> = Vec::new();
    if !view.raw.is_empty() && !view.raw.ends_with('\n') {
        b.push(b'\n');
    }
    b.extend_from_slice(b"# ");
    b.extend_from_slice(e.target.as_bytes());
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
    let (key, val) = (&e.target, &e.body);
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(BodyError {
            code: "E_FAIL_LOUD".to_string(),
            message: format!("invalid frontmatter key {}", go_quote(key)),
            remedy: "a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')".to_string(),
            context: vec![("key".to_string(), key.clone())],
        });
    }
    if val.contains(['\n', '\r']) {
        return Err(BodyError {
            code: "E_FAIL_LOUD".to_string(),
            message: format!("property value for {} contains a newline", go_quote(key)),
            remedy:
                "frontmatter values are single-line in v1; put multi-line content in a body section"
                    .to_string(),
            context: vec![("key".to_string(), key.clone())],
        });
    }
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
    let safe = yaml_safe_value(val);
    for k in &view.fm {
        if k.key == *key {
            let mut repl = safe.clone().into_bytes();
            if !val.is_empty()
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
    let line = if val.is_empty() {
        format!("{key}:\n")
    } else {
        format!("{key}: {safe}\n")
    };
    Ok(vec![SpliceOp {
        start: view.fm_end,
        end: view.fm_end,
        replacement: line.into_bytes(),
    }])
}

/// Go `yamlSafeValue` (E2 finding 1): quote IFF the `k: <val>` probe ERRORS
/// or decodes a MAP; everything that parses non-map passes VERBATIM — typed
/// scalars, flow lists, and crucially list-of-list (`[[a, b]]`), which lands
/// raw so the I4 nested-frontmatter law (not a shape check) judges it. The
/// error class is approximated by its two live shapes: an unquoted mid-value
/// `": "` (mapping in value position) and an unterminated leading quote.
/// SHARED (U8b): the splice plan-lowering quotes `set_property` values through
/// THIS predicate — single-owner discipline, so the `check_write` candidate and
/// the written bytes cannot drift.
#[must_use]
pub fn yaml_safe_value(val: &str) -> String {
    if val.is_empty() {
        return String::new();
    }
    let map_shaped = val.starts_with('{');
    let unquoted = !val.starts_with(['\'', '"']);
    let colon_space = unquoted && val.contains(": ");
    let unterminated_quote = (val.starts_with('\'') && (val.len() < 2 || !val.ends_with('\'')))
        || (val.starts_with('"') && (val.len() < 2 || !val.ends_with('"')));
    if map_shaped || colon_space || unterminated_quote {
        return format!("'{}'", val.replace('\'', "''"));
    }
    val.to_string()
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
                remedy: format!(
                    "read the section (md read) and pass its rev ({}) to confirm the current content",
                    sec.rev
                ),
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
            "the section changed since you read it; re-read it (md read {}) and recompose against rev {}",
            go_quote(section),
            sec.rev
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
