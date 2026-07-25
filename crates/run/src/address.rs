//! Task addressing: frontmatter `task.<name>` bindings → the anchored fenced
//! code block, via the strict §2.1 mint plane. See the crate docs for the
//! grammar. Every fault is a DISTINCT typed error — the CLI's exit-2 surface
//! depends on telling "no such task" from "binding present, block gone".

use std::path::Path;

use model::{ByteSpan, Document, Node, NodeKind, NodeRev, Ref, ResolveError, YamlMap};

use crate::fence::{self, FenceError, TaskBlock};

/// The frontmatter prefix a task binding key carries: `task.<name>`.
pub const TASK_PREFIX: &str = "task.";

/// Reserved binding sub-key suffixes — `task.<name>.<suffix>` keys carry the
/// block's declarations, never a binding of their own.
pub const RESERVED_SUFFIXES: [&str; 3] = ["caps", "args", "env"];

/// One task binding as declared: the task `name` (the fm key minus the
/// `task.` prefix) and the block `anchor` id its value references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBinding {
    /// The task name (`fix-drift` for key `task.fix-drift`).
    pub name: String,
    /// The referenced block id (`fix-1` for value `[[#^fix-1]]`).
    pub anchor: String,
}

/// A fully resolved task: its binding plus the classified fenced code block
/// the anchor keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTask {
    /// The binding that addressed the block.
    pub binding: TaskBinding,
    /// The classified block: language + spans + source.
    pub block: TaskBlock,
    /// The addressed code block's `node_rev` — the procedure-hash carried into
    /// the run receipt (attestation: WHICH code ran, not just the task NAME).
    pub task_rev: String,
}

/// Why addressing refused. Page-class variants are environment faults (exit 2);
/// the rest are authoring/addressing faults (also exit 2) — all pre-eval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    /// The page file does not exist under the workspace root.
    PageNotFound { path: String },
    /// The page exists but was refused (non-UTF-8 content — never lossy-decoded).
    PageInvalid { path: String, reason: String },
    /// I/O failure reading the page.
    PageIo { path: String, reason: String },
    /// TASK was named but the page declares no `task.<name>` key. Carries the
    /// declared task names so the caller can list them.
    NoTask {
        name: String,
        available: Vec<String>,
    },
    /// TASK was omitted and the page declares no task bindings at all.
    NoTasks,
    /// TASK was omitted and the page declares more than one binding — the
    /// caller lists them and exits 2, it never guesses.
    ManyTasks { available: Vec<String> },
    /// A binding value is not a same-file block linktext (`#^id`).
    InvalidBinding {
        name: String,
        value: String,
        reason: String,
    },
    /// The task NAME is outside the one identifier charset (fix9). The name is
    /// not decoration: it is stamped verbatim into every run receipt (`task`,
    /// and `actor` as `run:<name>`), so a name carrying markdown is a name that
    /// forges the record of its own run.
    InvalidTaskName { name: String },
    /// A binding value references another file (`other.md#^id`) — cross-file
    /// task refs are an S1 NON-GOAL (plan decision #11), refused, not deferred
    /// silently.
    CrossFileRef { name: String, value: String },
    /// The binding's fm key exists but its anchor resolves to nothing — DISTINCT
    /// from [`AddressError::NoTask`]: the declaration is present, the block is
    /// gone (a dangling binding is an authoring fault the author must see).
    DanglingBinding { name: String, anchor: String },
    /// The anchor id appears more than once in the page — the mint plane never
    /// silently picks (`ambiguous_ref`).
    AmbiguousAnchor {
        name: String,
        anchor: String,
        count: usize,
    },
    /// The anchor resolves, but it does not key a fenced code block.
    NotACodeBlock { name: String, anchor: String },
    /// The block was found but its fence refused classification.
    Fence { name: String, error: FenceError },
}

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressError::PageNotFound { path } => write!(f, "page not found: {path}"),
            AddressError::PageInvalid { path, reason } => {
                write!(f, "page {path} refused: {reason}")
            }
            AddressError::PageIo { path, reason } => write!(f, "cannot read page {path}: {reason}"),
            AddressError::NoTask { name, available } => {
                write!(f, "no task '{name}' on this page")?;
                if !available.is_empty() {
                    write!(f, " (declared: {})", available.join(", "))?;
                }
                Ok(())
            }
            AddressError::NoTasks => write!(f, "this page declares no tasks"),
            AddressError::ManyTasks { available } => write!(
                f,
                "this page declares {} tasks — name one: {}",
                available.len(),
                available.join(", ")
            ),
            AddressError::InvalidBinding {
                name,
                value,
                reason,
            } => write!(
                f,
                "task '{name}' binding '{value}' is not a same-file block ref: {reason}"
            ),
            AddressError::InvalidTaskName { name } => write!(
                f,
                "task name '{name}' is outside the one identifier charset [A-Za-z0-9-] (§2.4, decision 011) — \
                 a task name is stamped into every run receipt as `task` and as the actor `run:{name}`, \
                 so bytes that can render as markdown would forge that record; rename the binding key \
                 `{TASK_PREFIX}{name}` to letters, digits and dashes (`{TASK_PREFIX}fix-drift`)"
            ),
            AddressError::CrossFileRef { name, value } => write!(
                f,
                "task '{name}' binding '{value}' references another file — cross-file task refs are out of scope (S1 non-goal)"
            ),
            AddressError::DanglingBinding { name, anchor } => write!(
                f,
                "task '{name}' is declared but its block '^{anchor}' does not exist on this page (dangling binding)"
            ),
            AddressError::AmbiguousAnchor {
                name,
                anchor,
                count,
            } => write!(
                f,
                "task '{name}' anchor '^{anchor}' appears {count} times on this page (ambiguous)"
            ),
            AddressError::NotACodeBlock { name, anchor } => write!(
                f,
                "task '{name}' anchor '^{anchor}' does not key a fenced code block"
            ),
            AddressError::Fence { name, error } => write!(f, "task '{name}': {error}"),
        }
    }
}

impl std::error::Error for AddressError {}

/// Load one workspace page into a parsed document, splitting the environment
/// faults the rescue map names: missing file, non-UTF-8 refusal, other I/O.
///
/// # Errors
/// [`AddressError::PageNotFound`] / [`AddressError::PageInvalid`] /
/// [`AddressError::PageIo`].
pub fn load_page(root: &fs::WorkspaceRoot, rel_path: &Path) -> Result<Document, AddressError> {
    let path = rel_path.display().to_string();
    fs::load(root, rel_path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AddressError::PageNotFound { path },
        std::io::ErrorKind::InvalidData => AddressError::PageInvalid {
            path,
            reason: e.to_string(),
        },
        _ => AddressError::PageIo {
            path,
            reason: e.to_string(),
        },
    })
}

/// The page's frontmatter map, if any.
pub(crate) fn frontmatter(doc: &Document) -> Option<&YamlMap> {
    fn find(node: &Node) -> Option<&YamlMap> {
        if let NodeKind::Frontmatter { map } = &node.kind {
            return Some(map);
        }
        node.children.iter().find_map(find)
    }
    find(&doc.root)
}

/// Strip one pair of matching surrounding quotes (frontmatter values keep
/// their quote characters; the key side is unquoted by `model`).
fn unquote(value: &str) -> &str {
    let v = value.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// Parse a binding value as the same-file block linktext: `#^id`, with
/// optional `[[…]]` brackets and `|alias` sugar (the §2.2 walk-plane spelling,
/// sugar stripped before resolution). Anything else is a typed refusal.
fn parse_binding_value(name: &str, raw_value: &str) -> Result<String, AddressError> {
    let mut v = unquote(raw_value);
    if let Some(inner) = v.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        v = inner;
    }
    // Alias sugar: `#^id|label` → `#^id`.
    if let Some((linktext, _alias)) = v.split_once('|') {
        v = linktext;
    }
    let v = v.trim();
    let err = |reason: &str| AddressError::InvalidBinding {
        name: name.to_owned(),
        value: raw_value.to_owned(),
        reason: reason.to_owned(),
    };
    let Some((target, block)) = v.split_once("#^") else {
        return Err(err("expected a block ref of the form [[#^id]]"));
    };
    if !target.is_empty() {
        return Err(AddressError::CrossFileRef {
            name: name.to_owned(),
            value: raw_value.to_owned(),
        });
    }
    if !syntax::is_block_id(block) {
        return Err(err("block id is outside the [A-Za-z0-9-] charset"));
    }
    Ok(block.to_owned())
}

/// Every task binding the page declares, in document order, each value
/// validated. A malformed binding ANYWHERE refuses loudly at load — one broken
/// declaration must not silently vanish from `--list`.
///
/// # Errors
/// [`AddressError::InvalidBinding`] / [`AddressError::CrossFileRef`] for the
/// first malformed binding.
pub fn bindings(doc: &Document) -> Result<Vec<TaskBinding>, AddressError> {
    let Some(map) = frontmatter(doc) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (key, value) in &map.0 {
        let Some(rest) = key.strip_prefix(TASK_PREFIX) else {
            continue;
        };
        // `task.<name>.caps` / `.args` / `.env` are declarations, not bindings.
        if let Some((_, suffix)) = rest.rsplit_once('.')
            && RESERVED_SUFFIXES.contains(&suffix)
        {
            continue;
        }
        if rest.is_empty() {
            continue;
        }
        // THE NAME BOUNDARY (fix9). `rest` is whatever an author typed after
        // `task.` — arbitrary frontmatter-key bytes — and it is stamped verbatim
        // into every run receipt line (`"task": …`, `"actor": "run:…"`). A name
        // like `[[guide#^goal@green.b3af12cd|G]]` therefore lands an `@fp` claim
        // token in a claim-link position on a plane no candidate strip sees: the
        // receipt rides beside `.edits`, in a different FILE. The token is only
        // the narrowest instance — the general fault is that a task name can
        // render as markdown at all.
        //
        // So the name takes the SAME charset guard as the binding's value below
        // (`is_block_id`, ruling 011's one charset). Both halves of a binding —
        // the name and the anchor it references — are identifiers, and the
        // refusal makes the hostile bytes unrepresentable rather than removable.
        if !syntax::is_block_id(rest) {
            return Err(AddressError::InvalidTaskName {
                name: rest.to_owned(),
            });
        }
        let anchor = parse_binding_value(rest, value)?;
        out.push(TaskBinding {
            name: rest.to_owned(),
            anchor,
        });
    }
    Ok(out)
}

/// Resolve `task` on the page: named → that binding; omitted → the page's ONLY
/// binding (one runs; many is a loud list-and-refuse, never a guess).
///
/// # Errors
/// Every [`AddressError`] variant except the page-load class.
pub fn resolve_task(doc: &Document, task: Option<&str>) -> Result<ResolvedTask, AddressError> {
    let all = bindings(doc)?;
    let names = || all.iter().map(|b| b.name.clone()).collect::<Vec<_>>();
    let binding = match task {
        Some(name) => all
            .iter()
            .find(|b| b.name == name)
            .cloned()
            .ok_or_else(|| AddressError::NoTask {
                name: name.to_owned(),
                available: names(),
            })?,
        None => match all.as_slice() {
            [] => return Err(AddressError::NoTasks),
            [only] => only.clone(),
            _ => return Err(AddressError::ManyTasks { available: names() }),
        },
    };
    resolve_binding(doc, &binding)
}

/// Resolve one binding's anchor to its fenced code block and classify it.
fn resolve_binding(doc: &Document, binding: &TaskBinding) -> Result<ResolvedTask, AddressError> {
    let r#ref = Ref::anchor(binding.anchor.clone()).map_err(|_| AddressError::InvalidBinding {
        name: binding.name.clone(),
        value: binding.anchor.clone(),
        reason: "block id is outside the [A-Za-z0-9-] charset".to_owned(),
    })?;
    let target = model::resolve(doc, &r#ref).map_err(|e| match e {
        ResolveError::NotFound => AddressError::DanglingBinding {
            name: binding.name.clone(),
            anchor: binding.anchor.clone(),
        },
        ResolveError::Ambiguous(candidates) => AddressError::AmbiguousAnchor {
            name: binding.name.clone(),
            anchor: binding.anchor.clone(),
            count: candidates.len(),
        },
    })?;
    let (code_span, task_rev) =
        host_code_block(doc, &target.span).ok_or_else(|| AddressError::NotACodeBlock {
            name: binding.name.clone(),
            anchor: binding.anchor.clone(),
        })?;
    let block = fence::classify(doc, &code_span).map_err(|error| AddressError::Fence {
        name: binding.name.clone(),
        error,
    })?;
    Ok(ResolvedTask {
        binding: binding.clone(),
        block,
        task_rev: task_rev.0,
    })
}

/// The fenced code block an anchor keys. An anchor cannot sit inside a fence
/// (fence content is masked at parse), so a block-keying anchor is the
/// Obsidian own-line form directly BELOW the fence: the nearest `CodeBlock`
/// whose span ends before the anchor's line with only blank bytes between.
fn host_code_block(doc: &Document, anchor_span: &ByteSpan) -> Option<(ByteSpan, NodeRev)> {
    fn collect<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
        if matches!(node.kind, NodeKind::CodeBlock { .. }) {
            out.push(node);
        }
        for c in &node.children {
            collect(c, out);
        }
    }
    let mut blocks: Vec<&Node> = Vec::new();
    collect(&doc.root, &mut blocks);
    blocks
        .into_iter()
        .filter(|b| {
            b.span.end <= anchor_span.start
                && doc.raw[b.span.end..anchor_span.start]
                    .bytes()
                    .all(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        })
        .max_by_key(|b| b.span.end)
        .map(|b| (b.span.clone(), b.node_rev.clone()))
}
