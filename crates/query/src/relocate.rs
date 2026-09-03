//! The move plan (`docs/move.md` §4–§6): the corpus-wide, span-exact
//! reference-rewrite plan for a file or directory move. A plan, never an
//! application — the door (`wire_serve::relocate`) lands it.
//!
//! *Would break* is decided by the address owner itself: every reference is
//! re-resolved through [`CorpusIndex::resolve_linkpath`] against the post-move
//! index (every moved path re-keyed, the referring page at its own post-move
//! path), and only a reference whose answer changed is rewritten. The new
//! spelling is minted at the class the author wrote (full path / shortest
//! unique suffix / bare name), a bare name the move would leave between two
//! files of one basename is the ambiguity refusal, and a file under an
//! immutable prefix is reported instead of rewritten.

use std::collections::BTreeMap;

use model::{ByteSpan, CorpusIndex, Docs, Document, Node, NodeKind};

/// What the caller asks to move — the door has already resolved the into-form
/// and refused the path-level cases (`move.md` §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveSpec {
    /// OLD, workspace-relative, no trailing slash.
    pub old: String,
    /// NEW, workspace-relative, the full destination.
    pub new: String,
    /// OLD is a directory: every corpus member under it moves.
    pub is_dir: bool,
    /// Prefixes never written (`--immutable`), no trailing slash.
    pub immutable: Vec<String>,
    /// The names this root answers to — its declared name and its bound
    /// alias — so a rooted spelling (`name:path`) naming THIS root is
    /// rewritten and one naming another root is left alone.
    pub root_names: Vec<String>,
}

/// The reference classes a move rewrites (`move.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    Wikilink,
    Embed,
    /// A `[[…]]` inside the frontmatter block.
    Frontmatter,
    /// A plain `root:path` string inside the frontmatter block naming this root.
    Rooted,
    /// A `meridian-lock` row's `object:`.
    LockRow,
}

impl RefKind {
    /// The stable output word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            RefKind::Wikilink => "wikilink",
            RefKind::Embed => "embed",
            RefKind::Frontmatter => "frontmatter",
            RefKind::Rooted => "rooted",
            RefKind::LockRow => "lock",
        }
    }
}

/// One byte-span replacement against a page's pre-image: the target slot's
/// bytes (never the fragment or alias around them) and what replaces them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub span: ByteSpan,
    pub text: String,
    pub kind: RefKind,
    /// The spelling as written.
    pub old: String,
    /// The spelling the door mints.
    pub new: String,
}

/// The page's `meridian-lock` block re-rendered with its stale `object:` rows
/// repointed — one span, the whole fence, and the rows that changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockRewrite {
    pub span: ByteSpan,
    pub text: String,
    /// `(old object, new object)` per rewritten row.
    pub rows: Vec<(String, String)>,
}

/// Every edit one page takes: link-slot rewrites plus, at most, one lock
/// re-render. Spans are disjoint by construction — a link node is never inside
/// a fence, and the lock block is a fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRewrite {
    pub path: String,
    pub links: Vec<Rewrite>,
    pub lock: Option<LockRewrite>,
}

impl FileRewrite {
    /// How many link rewrites of `kind` this page takes.
    #[must_use]
    pub fn count(&self, kind: RefKind) -> usize {
        self.links.iter().filter(|r| r.kind == kind).count()
    }

    /// How many lock rows this page repoints.
    #[must_use]
    pub fn lock_rows(&self) -> usize {
        self.lock.as_ref().map_or(0, |l| l.rows.len())
    }

    /// The page's new bytes: every edit applied to `raw`, the pre-image the
    /// spans index.
    #[must_use]
    pub fn apply(&self, raw: &str) -> String {
        let mut edits: Vec<(&ByteSpan, &str)> = self
            .links
            .iter()
            .map(|r| (&r.span, r.text.as_str()))
            .collect();
        if let Some(lock) = &self.lock {
            edits.push((&lock.span, lock.text.as_str()));
        }
        edits.sort_by_key(|(span, _)| span.start);
        let mut out = String::with_capacity(raw.len());
        let mut at = 0;
        for (span, text) in edits {
            out.push_str(&raw[at..span.start]);
            out.push_str(text);
            at = span.end;
        }
        out.push_str(&raw[at..]);
        out
    }
}

/// A reference in a file under an immutable prefix that would break — reported,
/// never rewritten (`move.md` §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub path: String,
    /// 1-based line of the reference.
    pub line: usize,
    pub kind: RefKind,
    pub old: String,
    /// The spelling the door would have minted.
    pub new: String,
}

/// A bare spelling the move would leave resolving between two files of one
/// name (`move.md` §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguity {
    pub source: String,
    pub linkpath: String,
    pub candidates: Vec<String>,
}

/// The link census: how many ambient references resolve and how many dangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkCensus {
    pub resolved: usize,
    pub dangling: usize,
}

impl LinkCensus {
    fn tally(&mut self, resolved: bool) {
        if resolved {
            self.resolved += 1;
        } else {
            self.dangling += 1;
        }
    }
}

/// The whole plan (`move.md` §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePlan {
    /// Every corpus member that moves, `(from, to)`.
    pub renames: Vec<(String, String)>,
    pub rewrites: Vec<FileRewrite>,
    pub immutable: Vec<Skip>,
    pub ambiguous: Vec<Ambiguity>,
    /// Pages whose `meridian-lock` block does not parse: their rows could not
    /// be read, so none were repointed.
    pub lock_unreadable: Vec<String>,
    pub before: LinkCensus,
    /// The census the plan predicts, simulated over the post-move index.
    pub after: LinkCensus,
}

impl MovePlan {
    /// Link-slot rewrites across every page.
    #[must_use]
    pub fn links_rewritten(&self) -> usize {
        self.rewrites.iter().map(|f| f.links.len()).sum()
    }

    /// Lock rows repointed across every page.
    #[must_use]
    pub fn lock_rows_rewritten(&self) -> usize {
        self.rewrites.iter().map(FileRewrite::lock_rows).sum()
    }
}

/// Is `path` under `prefix` — equal to it, or inside it as a directory?
#[must_use]
pub fn under_prefix(prefix: &str, path: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Plan the move: nothing is applied.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn plan(index: &CorpusIndex, docs: &Docs, spec: &MoveSpec) -> MovePlan {
    let mapping = mapping_of(docs, spec);
    let mut after = CorpusIndex::new();
    for (path, doc) in docs {
        after.insert(&mapped(&mapping, path), doc);
    }
    let ctx = Ctx {
        index,
        after: &after,
        docs,
        mapping: &mapping,
        spec,
    };

    let mut plan = MovePlan {
        renames: mapping
            .iter()
            .map(|(a, b)| (a.clone(), b.clone()))
            .collect(),
        rewrites: Vec::new(),
        immutable: Vec::new(),
        ambiguous: Vec::new(),
        lock_unreadable: Vec::new(),
        before: LinkCensus::default(),
        after: LinkCensus::default(),
    };

    for (path, doc) in docs {
        let frozen = spec.immutable.iter().any(|p| under_prefix(p, path));
        let src_after = mapped(&mapping, path);
        let mut file = FileRewrite {
            path: path.clone(),
            links: Vec::new(),
            lock: None,
        };

        for occ in occurrences(doc) {
            let ambient = !addr::head_carries_root_separator(&occ.target);
            if ambient {
                plan.before
                    .tally(index.resolve_linkpath(&occ.target, path).is_some());
            }
            let spelled_after = match ctx.decide(path, &occ.target) {
                Decision::Keep => occ.target.clone(),
                Decision::Rewrite(new) => {
                    if frozen {
                        plan.immutable.push(Skip {
                            path: path.clone(),
                            line: line_of(&doc.raw, occ.slot.start),
                            kind: occ.kind,
                            old: occ.target.clone(),
                            new,
                        });
                        occ.target.clone()
                    } else {
                        file.links.push(Rewrite {
                            span: occ.slot.clone(),
                            text: new.clone(),
                            kind: occ.kind,
                            old: occ.target.clone(),
                            new: new.clone(),
                        });
                        new
                    }
                }
                Decision::Ambiguous(candidates) => {
                    if frozen {
                        plan.immutable.push(Skip {
                            path: path.clone(),
                            line: line_of(&doc.raw, occ.slot.start),
                            kind: occ.kind,
                            old: occ.target.clone(),
                            new: ctx.full_form(path, &occ.target),
                        });
                    } else {
                        plan.ambiguous.push(Ambiguity {
                            source: path.clone(),
                            linkpath: occ.target.clone(),
                            candidates,
                        });
                    }
                    occ.target.clone()
                }
            };
            if ambient {
                plan.after
                    .tally(after.resolve_linkpath(&spelled_after, &src_after).is_some());
            }
        }

        // Class 3 — frontmatter rooted strings naming this root.
        if let Some(fm) = frontmatter_span(doc) {
            for occ in frontmatter_rooted(&doc.raw, fm, &spec.root_names) {
                let Some(new) = map_string_path(spec, &occ.path) else {
                    continue;
                };
                if frozen {
                    plan.immutable.push(Skip {
                        path: path.clone(),
                        line: line_of(&doc.raw, occ.slot.start),
                        kind: RefKind::Rooted,
                        old: occ.path.clone(),
                        new,
                    });
                } else {
                    file.links.push(Rewrite {
                        span: occ.slot.clone(),
                        text: new.clone(),
                        kind: RefKind::Rooted,
                        old: occ.path.clone(),
                        new,
                    });
                }
            }
        }

        // Class 4 — lock rows.
        match lock::find(doc) {
            Ok(Some(found)) => {
                let mut updated = found.lock.clone();
                let mut rows = Vec::new();
                for pin in &mut updated.pins {
                    let Some(new_object) = ctx.repoint_object(path, &pin.object) else {
                        continue;
                    };
                    rows.push((pin.object.clone(), new_object.clone()));
                    pin.object = new_object;
                }
                if !rows.is_empty() {
                    if frozen {
                        let line = line_of(&doc.raw, found.span.start);
                        for (old, new) in rows {
                            plan.immutable.push(Skip {
                                path: path.clone(),
                                line,
                                kind: RefKind::LockRow,
                                old,
                                new,
                            });
                        }
                    } else {
                        file.lock = Some(LockRewrite {
                            span: found.span.clone(),
                            text: lock::render(&updated),
                            rows,
                        });
                    }
                }
            }
            Ok(None) => {}
            Err(_) => plan.lock_unreadable.push(path.clone()),
        }

        if !file.links.is_empty() || file.lock.is_some() {
            plan.rewrites.push(file);
        }
    }
    plan
}

/// The census over a corpus as it stands: every ambient body and frontmatter
/// link, resolved or dangling. The read-back half of a receipt.
#[must_use]
pub fn link_census(index: &CorpusIndex, docs: &Docs) -> LinkCensus {
    let mut census = LinkCensus::default();
    for (path, doc) in docs {
        for occ in occurrences(doc) {
            if addr::head_carries_root_separator(&occ.target) {
                continue;
            }
            census.tally(index.resolve_linkpath(&occ.target, path).is_some());
        }
    }
    census
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    index: &'a CorpusIndex,
    after: &'a CorpusIndex,
    docs: &'a Docs,
    mapping: &'a BTreeMap<String, String>,
    spec: &'a MoveSpec,
}

enum Decision {
    Keep,
    Rewrite(String),
    Ambiguous(Vec<String>),
}

impl Ctx<'_> {
    fn names_this_root(&self, name: &addr::MountName) -> bool {
        self.spec.root_names.iter().any(|n| n == name.as_str())
    }

    /// Where `spelling` lands today, through the three-rule owner's order: a
    /// corpus key, a corpus key plus `.md`, then stage 1.
    fn resolve_now(&self, spelling: &str, from: &str) -> Option<String> {
        if self.docs.contains_key(spelling) {
            return Some(spelling.to_owned());
        }
        let with_md = format!("{spelling}.md");
        if self.docs.contains_key(&with_md) {
            return Some(with_md);
        }
        self.index.resolve_linkpath(spelling, from)
    }

    fn decide(&self, src: &str, target: &str) -> Decision {
        if addr::head_carries_root_separator(target) {
            return self.decide_rooted(src, target);
        }
        let Some(pre) = self.index.resolve_linkpath(target, src) else {
            return Decision::Keep;
        };
        let expected = mapped(self.mapping, &pre);
        let src_after = mapped(self.mapping, src);
        let (key, has_md) = split_md(target);
        let expected_key = strip_md(&expected);

        if key.contains('/') {
            if self.after.resolve_linkpath(target, &src_after).as_deref() == Some(expected.as_str())
            {
                return Decision::Keep;
            }
            if key.eq_ignore_ascii_case(strip_md(&pre)) {
                return Decision::Rewrite(with_md(expected_key, has_md));
            }
            let segments: Vec<&str> = expected_key.split('/').collect();
            for n in 2..=segments.len() {
                let suffix = segments[segments.len() - n..].join("/");
                if self.unique(&suffix, &expected) {
                    return Decision::Rewrite(with_md(&suffix, has_md));
                }
            }
            return Decision::Rewrite(with_md(expected_key, has_md));
        }

        // A bare name (`move.md` §5). A basename the move changed is rewritten
        // to the new name when that name is unique — a rename onto a basename
        // another page carries is the ambiguity refusal. An unchanged basename
        // keeps its bucket (a move never adds a file to one), so the only
        // thing the move can change is the pick: an answer that flips to a
        // twin refuses, never a silent retarget.
        let old_base = basename(strip_md(&pre));
        let new_base = basename(expected_key);
        if !old_base.eq_ignore_ascii_case(new_base) {
            let candidates = self.after.linkpath_candidates(new_base);
            if candidates.as_slice() == [expected.clone()] {
                return Decision::Rewrite(with_md(new_base, has_md));
            }
            return Decision::Ambiguous(candidates);
        }
        if self.after.resolve_linkpath(target, &src_after).as_deref() == Some(expected.as_str()) {
            return Decision::Keep;
        }
        Decision::Ambiguous(self.after.linkpath_candidates(target))
    }

    fn decide_rooted(&self, src: &str, target: &str) -> Decision {
        let Ok(parsed) = addr::Addr::parse(target) else {
            return Decision::Keep;
        };
        let Some(root) = parsed.root() else {
            return Decision::Keep;
        };
        if !self.names_this_root(root) {
            return Decision::Keep;
        }
        let Some(colon) = target.find(':') else {
            return Decision::Keep;
        };
        let path = &target[colon + 1..];
        let Some(dest) = self.resolve_now(path, src) else {
            return Decision::Keep;
        };
        let Some(new_dest) = self.mapping.get(&dest) else {
            return Decision::Keep;
        };
        let (_, has_md) = split_md(path);
        Decision::Rewrite(format!(
            "{}{}",
            &target[..=colon],
            with_md(strip_md(new_dest), has_md)
        ))
    }

    /// Is `spelling`'s post-move candidate set exactly `expected`?
    fn unique(&self, spelling: &str, expected: &str) -> bool {
        let candidates = self.after.linkpath_candidates(spelling);
        candidates.len() == 1 && candidates[0] == expected
    }

    /// The full-path form a reference would take after the move — the advisory
    /// spelling an immutable skip reports for a bare name that would flip.
    fn full_form(&self, src: &str, target: &str) -> String {
        let (_, has_md) = split_md(target);
        match self.index.resolve_linkpath(target, src) {
            Some(pre) => with_md(strip_md(&mapped(self.mapping, &pre)), has_md),
            None => target.to_owned(),
        }
    }

    /// A lock row's new `object:`, or `None` when the row names no moved page.
    fn repoint_object(&self, src: &str, object: &str) -> Option<String> {
        let (prefix, path) = if addr::head_carries_root_separator(object) {
            let parsed = addr::Addr::parse(object).ok()?;
            if !self.names_this_root(parsed.root()?) {
                return None;
            }
            let colon = object.find(':')?;
            (&object[..=colon], &object[colon + 1..])
        } else {
            ("", object)
        };
        let dest = self.resolve_now(path, src)?;
        let new_dest = self.mapping.get(&dest)?;
        Some(format!("{prefix}{}", strip_md(new_dest)))
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Every corpus member that moves: `old → new` for a file, `old/… → new/…`
/// for a directory.
fn mapping_of(docs: &Docs, spec: &MoveSpec) -> BTreeMap<String, String> {
    let mut mapping = BTreeMap::new();
    if spec.is_dir {
        let prefix = format!("{}/", spec.old);
        for path in docs.keys() {
            if let Some(rest) = path.strip_prefix(&prefix) {
                mapping.insert(path.clone(), format!("{}/{rest}", spec.new));
            }
        }
    } else if docs.contains_key(&spec.old) {
        mapping.insert(spec.old.clone(), spec.new.clone());
    }
    mapping
}

fn mapped(mapping: &BTreeMap<String, String>, path: &str) -> String {
    mapping
        .get(path)
        .cloned()
        .unwrap_or_else(|| path.to_owned())
}

/// A plain path string (a rooted string's path half) mapped across the move:
/// the directory prefix or the file itself, with or without `.md`. `None`
/// when it names nothing under OLD.
fn map_string_path(spec: &MoveSpec, path: &str) -> Option<String> {
    if spec.is_dir {
        if path == spec.old {
            return Some(spec.new.clone());
        }
        if let Some(rest) = path.strip_prefix(&format!("{}/", spec.old)) {
            return Some(format!("{}/{rest}", spec.new));
        }
        return None;
    }
    if path == spec.old {
        return Some(spec.new.clone());
    }
    let (key, has_md) = split_md(path);
    if !has_md && format!("{key}.md") == spec.old {
        return Some(strip_md(&spec.new).to_owned());
    }
    None
}

/// `(spelling without one trailing .md, whether it carried one)`.
fn split_md(spelling: &str) -> (&str, bool) {
    if spelling.len() >= 3 && spelling[spelling.len() - 3..].eq_ignore_ascii_case(".md") {
        (&spelling[..spelling.len() - 3], true)
    } else {
        (spelling, false)
    }
}

fn strip_md(path: &str) -> &str {
    split_md(path).0
}

fn with_md(spelling: &str, has_md: bool) -> String {
    if has_md {
        format!("{spelling}.md")
    } else {
        spelling.to_owned()
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn line_of(raw: &str, offset: usize) -> usize {
    raw[..offset.min(raw.len())].matches('\n').count() + 1
}

// ---------------------------------------------------------------------------
// Occurrences — where a reference's target slot sits in the page bytes
// ---------------------------------------------------------------------------

struct Occ {
    /// The target slot: the bytes after the opening `[[`, up to the fragment,
    /// the alias, or the closing `]]`.
    slot: ByteSpan,
    target: String,
    kind: RefKind,
}

/// Every wikilink and embed the parse yields, plus every `[[…]]` inside the
/// frontmatter block, slot order.
fn occurrences(doc: &Document) -> Vec<Occ> {
    let mut out = Vec::new();
    body_links(&doc.root, &doc.raw, &mut out);
    if let Some(fm) = frontmatter_span(doc) {
        frontmatter_links(&doc.raw, fm, &mut out);
    }
    out.sort_by_key(|o| o.slot.start);
    out
}

fn body_links(node: &Node, raw: &str, out: &mut Vec<Occ>) {
    match &node.kind {
        NodeKind::Wikilink { target, .. } => {
            push_slot(raw, &node.span, target, RefKind::Wikilink, out);
        }
        NodeKind::Embed { target, .. } => push_slot(raw, &node.span, target, RefKind::Embed, out),
        _ => {}
    }
    for child in &node.children {
        body_links(child, raw, out);
    }
}

/// The slot is located structurally — the first `[[` inside the node's own
/// span — and verified to carry the parsed target byte for byte; a node whose
/// bytes do not (a parse the lexer normalized) is left alone rather than
/// rewritten at a guessed offset.
fn push_slot(raw: &str, span: &ByteSpan, target: &str, kind: RefKind, out: &mut Vec<Occ>) {
    let Some(head) = raw.get(span.clone()) else {
        return;
    };
    let Some(open) = head.find("[[") else {
        return;
    };
    let start = span.start + open + 2;
    if raw.get(start..start + target.len()) == Some(target) {
        out.push(Occ {
            slot: start..start + target.len(),
            target: target.to_owned(),
            kind,
        });
    }
}

fn frontmatter_span(doc: &Document) -> Option<ByteSpan> {
    doc.root
        .children
        .iter()
        .find(|c| matches!(c.kind, NodeKind::Frontmatter { .. }))
        .map(|n| n.span.clone())
}

/// The wikilink grammar scanned over the frontmatter bytes: `[[target#frag|alias]]`,
/// no newline inside. The body parse keeps frontmatter link-free by law, so
/// this scan is the one owner of frontmatter link positions.
fn frontmatter_links(raw: &str, fm: ByteSpan, out: &mut Vec<Occ>) {
    let Some(text) = raw.get(fm.clone()) else {
        return;
    };
    let mut at = 0;
    while let Some(open) = text[at..].find("[[") {
        let start = at + open + 2;
        let Some(close) = text[start..].find("]]") else {
            break;
        };
        let inner = &text[start..start + close];
        if !inner.contains('\n') && !inner.starts_with('[') {
            let len = inner.find(['#', '|']).unwrap_or(inner.len());
            let target = &inner[..len];
            if !target.is_empty() {
                out.push(Occ {
                    slot: fm.start + start..fm.start + start + len,
                    target: target.to_owned(),
                    kind: RefKind::Frontmatter,
                });
            }
        }
        at = start + close + 2;
    }
}

struct RootedOcc {
    /// The path half of the token, after the `root:` head and before any `#`.
    slot: ByteSpan,
    path: String,
}

/// A value-token boundary inside frontmatter: whitespace, a quote, a bracket,
/// a comma, or a backtick.
fn is_delim(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '[' | ']' | ',' | '`')
}

/// Plain `root:path` tokens inside the frontmatter block whose root names
/// this root — a value token bounded by [`is_delim`], parsed through the one
/// address constructor. A token inside a `[[…]]` is a frontmatter wikilink,
/// owned by [`frontmatter_links`].
fn frontmatter_rooted(raw: &str, fm: ByteSpan, root_names: &[String]) -> Vec<RootedOcc> {
    let mut out = Vec::new();
    let Some(text) = raw.get(fm.clone()) else {
        return out;
    };
    let mut at = 0;
    while at < text.len() {
        let Some(first) = text[at..].find(|c: char| !is_delim(c)) else {
            break;
        };
        let start = at + first;
        let len = text[start..].find(is_delim).unwrap_or(text.len() - start);
        let token = &text[start..start + len];
        at = start + len;
        if text[..start].ends_with("[[") || !addr::head_carries_root_separator(token) {
            continue;
        }
        let Some(colon) = token.find(':') else {
            continue;
        };
        let Ok(parsed) = addr::Addr::parse(token) else {
            continue;
        };
        let Some(root) = parsed.root() else {
            continue;
        };
        if !root_names.iter().any(|n| n == root.as_str()) {
            continue;
        }
        // The path slot is what `addr` parsed, located in the token verbatim —
        // the grammar owns the split (`address-grammar.md` §4.1).
        let end = colon + 1 + parsed.path().len();
        if parsed.path().is_empty() || token.get(colon + 1..end) != Some(parsed.path()) {
            continue;
        }
        out.push(RootedOcc {
            slot: fm.start + start + colon + 1..fm.start + start + end,
            path: token[colon + 1..end].to_owned(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> std::sync::Arc<Document> {
        std::sync::Arc::new(model::build(raw.to_owned(), syntax::parse(raw)))
    }

    fn corpus(pages: &[(&str, &str)]) -> (CorpusIndex, Docs) {
        let mut docs = Docs::new();
        let mut index = CorpusIndex::new();
        for (path, raw) in pages {
            let d = doc(raw);
            index.insert(path, &d);
            docs.insert((*path).to_owned(), d);
        }
        (index, docs)
    }

    fn spec(old: &str, new: &str, is_dir: bool) -> MoveSpec {
        MoveSpec {
            old: old.to_owned(),
            new: new.to_owned(),
            is_dir,
            immutable: Vec::new(),
            root_names: vec!["wiki".to_owned()],
        }
    }

    fn rewrites_of<'a>(plan: &'a MovePlan, path: &str) -> &'a FileRewrite {
        plan.rewrites
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} is rewritten: {plan:#?}"))
    }

    /// A bare link whose basename does not change still resolves after the
    /// move and is byte-untouched; the plan is link-neutral.
    #[test]
    fn a_bare_link_to_a_relocated_page_is_untouched() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            ("notes/fan.md", "see [[x]] and [[x#Top|alias]]\n"),
        ]);
        let plan = plan(&index, &docs, &spec("a/x.md", "b/x.md", false));
        assert!(plan.rewrites.is_empty(), "{plan:#?}");
        assert_eq!(
            plan.renames,
            vec![("a/x.md".to_owned(), "b/x.md".to_owned())]
        );
        assert_eq!(plan.before, plan.after);
        assert_eq!(plan.after.dangling, 0);
    }

    /// Renaming the basename rewrites bare links to the new name, keeping the
    /// fragment and alias bytes outside the slot and a `.md` the author wrote.
    #[test]
    fn a_basename_change_rewrites_bare_links_slot_only() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            (
                "notes/fan.md",
                "see [[x]] and ![[x#Top|alias]] and [[X.md]]\n",
            ),
        ]);
        let plan = plan(&index, &docs, &spec("a/x.md", "a/y.md", false));
        let file = rewrites_of(&plan, "notes/fan.md");
        let raw = &docs["notes/fan.md"].raw;
        assert_eq!(
            file.apply(raw),
            "see [[y]] and ![[y#Top|alias]] and [[y.md]]\n"
        );
        assert_eq!(file.count(RefKind::Wikilink), 2);
        assert_eq!(file.count(RefKind::Embed), 1);
        assert_eq!(plan.after.dangling, 0);
    }

    /// A partial path becomes the shortest unique suffix; a full path stays
    /// full; a bare link stays bare; a partial path that still resolves is
    /// untouched.
    #[test]
    fn a_directory_move_rewrites_at_the_class_the_author_wrote() {
        let (index, docs) = corpus(&[
            ("domains/knowledge/duckdb/DUCKDB.md", "# DuckDB\n"),
            (
                "domains/knowledge/duckdb/tips.md",
                "# Tips\n\nback to [[DUCKDB]]\n",
            ),
            (
                "synthesis/db.md",
                "[[duckdb/tips]] · [[knowledge/duckdb/DUCKDB]] · [[domains/knowledge/duckdb/tips]] · [[tips]]\n",
            ),
        ]);
        let plan = plan(
            &index,
            &docs,
            &spec("domains/knowledge/duckdb", "domains/data/duckdb", true),
        );
        assert_eq!(plan.renames.len(), 2);
        let file = rewrites_of(&plan, "synthesis/db.md");
        assert_eq!(
            file.apply(&docs["synthesis/db.md"].raw),
            "[[duckdb/tips]] · [[duckdb/DUCKDB]] · [[domains/data/duckdb/tips]] · [[tips]]\n"
        );
        assert!(
            plan.rewrites
                .iter()
                .all(|f| f.path != "domains/knowledge/duckdb/tips.md"),
            "a bare link between two moved pages still resolves: {plan:#?}"
        );
        assert_eq!(plan.after.dangling, 0);
    }

    /// A partial path whose shortest suffix is taken by a twin takes the next
    /// longer one.
    #[test]
    fn a_partial_path_takes_the_shortest_suffix_that_is_unique() {
        let (index, docs) = corpus(&[
            ("domains/knowledge/duckdb/tips.md", "# Tips\n"),
            ("domains/legacy/duckdb/tips.md", "# Old tips\n"),
            ("synthesis/db.md", "[[knowledge/duckdb/tips]]\n"),
        ]);
        let plan = plan(
            &index,
            &docs,
            &spec("domains/knowledge/duckdb", "domains/data/duckdb", true),
        );
        let file = rewrites_of(&plan, "synthesis/db.md");
        assert_eq!(
            file.apply(&docs["synthesis/db.md"].raw),
            "[[data/duckdb/tips]]\n"
        );
    }

    /// A bare name the move would leave between two files of one basename
    /// refuses with the pair named, and nothing is planned for that page —
    /// even though the resolver's tie-break would have picked the moved file.
    #[test]
    fn a_basename_collision_is_the_ambiguity_refusal() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            ("b/y.md", "# Y\n"),
            ("notes/fan.md", "see [[x]]\n"),
        ]);
        let plan = plan(&index, &docs, &spec("a/x.md", "a/y.md", false));
        assert_eq!(plan.ambiguous.len(), 1, "{plan:#?}");
        let amb = &plan.ambiguous[0];
        assert_eq!(amb.source, "notes/fan.md");
        assert_eq!(amb.linkpath, "x");
        assert_eq!(
            amb.candidates,
            vec!["a/y.md".to_owned(), "b/y.md".to_owned()]
        );
        assert!(plan.rewrites.is_empty());
    }

    /// A move that flips a bare link's pick to the twin is the same refusal:
    /// the source's own directory decided the pick, and the page left it.
    #[test]
    fn a_move_that_flips_a_bare_pick_to_the_twin_refuses_too() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            ("far/x.md", "# Other X\n"),
            ("far/fan.md", "see [[x]]\n"),
        ]);
        let plan = plan(&index, &docs, &spec("far/x.md", "zz/x.md", false));
        assert_eq!(plan.ambiguous.len(), 1, "{plan:#?}");
        assert_eq!(plan.ambiguous[0].source, "far/fan.md");
        assert_eq!(
            plan.ambiguous[0].candidates,
            vec!["a/x.md".to_owned(), "zz/x.md".to_owned()]
        );
    }

    /// Moved beside a twin the tie-break still ranks below it, the bare link
    /// keeps its answer — a standing collision is not this door's finding.
    #[test]
    fn a_move_that_keeps_a_bare_pick_is_left_alone() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            ("far/x.md", "# Other X\n"),
            ("a/fan.md", "see [[x]]\n"),
        ]);
        let plan = plan(&index, &docs, &spec("a/x.md", "b/x.md", false));
        assert!(plan.ambiguous.is_empty(), "{plan:#?}");
        assert!(plan.rewrites.is_empty(), "{plan:#?}");
    }

    /// A collision the move neither creates nor moves into is not this door's
    /// finding: the link keeps resolving where it did.
    #[test]
    fn a_standing_collision_elsewhere_is_left_alone() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            ("t/readme.md", "# R\n"),
            ("u/readme.md", "# R\n"),
            ("t/fan.md", "see [[readme]] and [[x]]\n"),
        ]);
        let plan = plan(&index, &docs, &spec("a/x.md", "b/x.md", false));
        assert!(plan.ambiguous.is_empty(), "{plan:#?}");
        assert!(plan.rewrites.is_empty(), "{plan:#?}");
    }

    /// Frontmatter wikilinks — scalar and list — and a rooted string naming
    /// this root are rewritten; one naming another root is not.
    #[test]
    fn frontmatter_links_and_rooted_strings_are_rewritten() {
        let (index, docs) = corpus(&[
            ("people/zt.md", "# ZT\n"),
            (
                "notes/card.md",
                "---\nowner: \"[[zt]]\"\nseat: [[people/zt|ZT]]\nlist:\n  - \"[[zt#Bio]]\"\nsource: \"wiki:people/zt.md#Bio\"\nother: \"elsewhere:people/zt.md\"\n---\n# Card\n",
            ),
        ]);
        let plan = plan(
            &index,
            &docs,
            &spec("people/zt.md", "domains/people/zt-user.md", false),
        );
        let file = rewrites_of(&plan, "notes/card.md");
        assert_eq!(
            file.apply(&docs["notes/card.md"].raw),
            "---\nowner: \"[[zt-user]]\"\nseat: [[domains/people/zt-user|ZT]]\nlist:\n  - \"[[zt-user#Bio]]\"\nsource: \"wiki:domains/people/zt-user.md#Bio\"\nother: \"elsewhere:people/zt.md\"\n---\n# Card\n"
        );
        assert_eq!(file.count(RefKind::Frontmatter), 3);
        assert_eq!(file.count(RefKind::Rooted), 1);
    }

    /// A lock row's `object:` is repointed and nothing else in the row moves;
    /// a row naming another page stays.
    #[test]
    fn a_lock_object_row_is_repointed_and_the_rest_of_the_row_stays() {
        let mut lock = lock::Lock::new();
        lock.upsert_pin(lock::PinEntry::new(
            "a/x",
            "9ae3f1deadbeef",
            lock::Selector::Path(vec!["Top".to_owned()]),
            "fp1.span2.b3.a8222f5a",
        ));
        lock.upsert_pin(lock::PinEntry::new(
            "other",
            "0000000000000000",
            lock::Selector::Path(Vec::new()),
            "fp1.span2.b3.00000000",
        ));
        let pinner = format!("# Pinner\n\n{}\n", lock::render(&lock));
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n\n## Top\n"),
            ("other.md", "# Other\n"),
            ("notes/pinner.md", pinner.as_str()),
        ]);
        let plan = plan(&index, &docs, &spec("a/x.md", "b/z.md", false));
        let file = rewrites_of(&plan, "notes/pinner.md");
        let lock_edit = file.lock.as_ref().expect("the lock is re-rendered");
        assert_eq!(lock_edit.rows, vec![("a/x".to_owned(), "b/z".to_owned())]);
        let applied = file.apply(&docs["notes/pinner.md"].raw);
        assert!(applied.contains("object: \"[[b/z]]\""), "{applied}");
        assert!(applied.contains("hash: \"9ae3f1deadbeef\""), "{applied}");
        assert!(applied.contains("object: \"[[other]]\""), "{applied}");
        assert_eq!(plan.lock_rows_rewritten(), 1);
    }

    /// A file under an immutable prefix is never rewritten: its breaking
    /// references are reported with their line and both spellings.
    #[test]
    fn an_immutable_prefix_reports_instead_of_rewriting() {
        let (index, docs) = corpus(&[
            ("a/x.md", "# X\n"),
            ("sources/git/rec.md", "# Rec\n\nline three [[x]]\n"),
            ("notes/fan.md", "see [[x]]\n"),
        ]);
        let mut s = spec("a/x.md", "a/y.md", false);
        s.immutable = vec!["sources".to_owned()];
        let plan = plan(&index, &docs, &s);
        assert_eq!(plan.immutable.len(), 1, "{plan:#?}");
        let skip = &plan.immutable[0];
        assert_eq!(skip.path, "sources/git/rec.md");
        assert_eq!(skip.line, 3);
        assert_eq!(skip.kind, RefKind::Wikilink);
        assert_eq!((skip.old.as_str(), skip.new.as_str()), ("x", "y"));
        assert!(plan.rewrites.iter().all(|f| f.path == "notes/fan.md"));
        assert_eq!(
            plan.after.dangling, 1,
            "the frozen link is predicted to dangle"
        );
    }

    /// A dangling link is nothing to protect: untouched, counted dangling on
    /// both sides.
    #[test]
    fn a_dangling_link_is_left_alone() {
        let (index, docs) = corpus(&[("a/x.md", "# X\n"), ("notes/fan.md", "see [[ghost]]\n")]);
        let plan = plan(&index, &docs, &spec("a/x.md", "b/x.md", false));
        assert!(plan.rewrites.is_empty());
        assert_eq!(plan.before.dangling, 1);
        assert_eq!(plan.after.dangling, 1);
    }
}
