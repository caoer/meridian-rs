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
    /// declared task names so the caller can list them, and the asked name's
    /// declaration-only keys (`task.<name>.caps/.args/.env` with no binding) —
    /// the measured near-miss the refusal teaches (dogfood r3 f5).
    NoTask {
        name: String,
        available: Vec<String>,
        declaration_keys: Vec<String>,
    },
    /// TASK was omitted and the page declares no task bindings at all.
    /// `declaration_keys` carries any `task.<name>.<suffix>` keys found — the
    /// same near-miss fact, measured page-wide.
    NoTasks { declaration_keys: Vec<String> },
    /// TASK was omitted and the page declares more than one binding — the
    /// caller lists them and exits 2, it never guesses.
    ManyTasks { available: Vec<String> },
    /// A binding value is not a same-file block linktext (`#^id`).
    InvalidBinding {
        name: String,
        value: String,
        reason: String,
    },
    /// The task name is outside the one identifier charset. Names are stamped
    /// verbatim into run receipts (`task`, `actor` as `run:<name>`), so they
    /// must not carry markdown-renderable bytes.
    InvalidTaskName { name: String },
    /// A binding value references another file (`other.md#^id`) — cross-file
    /// task refs are an S1 NON-GOAL (plan decision #11), refused, not deferred
    /// silently.
    CrossFileRef { name: String, value: String },
    /// The binding's fm key exists but its anchor resolves to nothing — distinct
    /// from [`AddressError::NoTask`]: the declaration is present, the block is gone.
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
            AddressError::NoTask {
                name,
                available,
                declaration_keys,
            } => {
                write!(f, "no task '{name}' on this page")?;
                if !available.is_empty() {
                    write!(f, " (declared: {})", available.join(", "))?;
                }
                if let Some(near_miss) = near_miss_clause(declaration_keys) {
                    write!(f, " — {near_miss}")?;
                }
                Ok(())
            }
            AddressError::NoTasks { declaration_keys } => {
                write!(f, "this page declares no tasks")?;
                match near_miss_clause(declaration_keys) {
                    Some(near_miss) => write!(f, " — {near_miss}"),
                    None => write!(
                        f,
                        " — a task is declared in frontmatter: `task.<name>: \"[[#^block-id]]\"` \
                         binds the name to the fenced code block it runs \
                         (`task.<name>.caps/.args/.env` carry its declarations)"
                    ),
                }
            }
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

/// The declaration-only near-miss clause: `task.<name>.<suffix>` keys were
/// measured on the page with no `task.<name>` binding. `None` when no such
/// keys exist — nothing is invented (the no-unmeasured-remedy law).
fn near_miss_clause(keys: &[String]) -> Option<String> {
    if keys.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    for key in keys {
        if let Some(rest) = key.strip_prefix(TASK_PREFIX)
            && let Some((base, _)) = rest.rsplit_once('.')
            && !names.contains(&base)
        {
            names.push(base);
        }
    }
    let bindings = names
        .iter()
        .map(|n| format!("{TASK_PREFIX}{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let example = names.first().copied().unwrap_or("<name>");
    Some(format!(
        "found {} but no {bindings} binding: the `.caps/.args/.env` keys carry a task's \
         declarations, and the binding that names WHICH block runs is the `{TASK_PREFIX}<name>` \
         key itself — declare `{TASK_PREFIX}{example}: \"[[#^block-id]]\"` naming the fenced \
         code block to run",
        keys.join(", ")
    ))
}

/// The page's declaration-only `task.*` keys: reserved-suffix keys
/// (`task.<name>.caps/.args/.env`) whose `<name>` has no binding key — the
/// near-miss fact the declaration refusals teach. `name` filters to one
/// task's keys; `None` measures page-wide. Document order.
#[must_use]
pub fn declaration_only_keys(doc: &Document, name: Option<&str>) -> Vec<String> {
    let Some(map) = frontmatter(doc) else {
        return Vec::new();
    };
    let bound: Vec<&str> = map
        .0
        .iter()
        .filter_map(|(key, _)| {
            let rest = key.strip_prefix(TASK_PREFIX)?;
            let is_binding = !rest.is_empty()
                && !rest
                    .rsplit_once('.')
                    .is_some_and(|(_, suffix)| RESERVED_SUFFIXES.contains(&suffix));
            is_binding.then_some(rest)
        })
        .collect();
    map.0
        .iter()
        .filter_map(|(key, _)| {
            let rest = key.strip_prefix(TASK_PREFIX)?;
            let (base, suffix) = rest.rsplit_once('.')?;
            if !RESERVED_SUFFIXES.contains(&suffix) || bound.contains(&base) {
                return None;
            }
            if let Some(want) = name
                && base != want
            {
                return None;
            }
            Some(key.clone())
        })
        .collect()
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

/// The § A.6.1 scalar law, through its one owner. The stored form keeps its
/// quote characters (`model`'s flat parse is the hash grain, § A.6.2); a
/// binding VALUE is a string, so it is decoded exactly where it is read.
fn unquote(value: &str) -> String {
    model::scalar::text(value)
}

/// Parse a binding value as the same-file block linktext: `#^id`, with
/// optional `[[…]]` brackets and `|alias` sugar (the §2.2 walk-plane spelling,
/// sugar stripped before resolution). Anything else is a typed refusal.
fn parse_binding_value(name: &str, raw_value: &str) -> Result<String, AddressError> {
    let decoded = unquote(raw_value);
    let mut v = decoded.as_str();
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

/// One row of the page's task table: the declared name, and either its parsed
/// binding or the typed fault its OWN value carries. Value faults are row-scoped
/// — a sibling's broken binding never masks the task a caller addressed, and
/// `--list` prints the fault as the row instead of losing the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredTask {
    /// The task name (the fm key minus the `task.` prefix).
    pub name: String,
    /// The binding, or this row's own value fault.
    pub binding: Result<TaskBinding, AddressError>,
}

/// Every task the page declares, in document order, each value validated into
/// its own row. The one page-eager refusal is the NAME charset guard: a name is
/// stamped verbatim into run receipts, so listing a forging name is itself the
/// harm (ruling 011).
///
/// # Errors
/// [`AddressError::InvalidTaskName`] for the first name outside the charset.
pub fn declared(doc: &Document) -> Result<Vec<DeclaredTask>, AddressError> {
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
        // The name is stamped verbatim into run receipts (`"task"`, `"actor":
        // "run:…"`), where markdown-renderable bytes could forge claim links —
        // so it takes the same charset guard as the anchor value (ruling 011).
        if !syntax::is_block_id(rest) {
            return Err(AddressError::InvalidTaskName {
                name: rest.to_owned(),
            });
        }
        out.push(DeclaredTask {
            name: rest.to_owned(),
            binding: parse_binding_value(rest, value).map(|anchor| TaskBinding {
                name: rest.to_owned(),
                anchor,
            }),
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
    let all = declared(doc)?;
    let names = || all.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
    let row = match task {
        Some(name) => all
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| AddressError::NoTask {
                name: name.to_owned(),
                available: names(),
                declaration_keys: declaration_only_keys(doc, Some(name)),
            })?,
        None => match all.as_slice() {
            [] => {
                return Err(AddressError::NoTasks {
                    declaration_keys: declaration_only_keys(doc, None),
                });
            }
            [only] => only,
            _ => return Err(AddressError::ManyTasks { available: names() }),
        },
    };
    let binding = row.binding.clone()?;
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
/// (fence content is masked at parse); the Obsidian own-line form below the
/// fence resolves to the fence in the MODEL since the F-R4 host widening, so
/// a block-keying anchor's host span IS the `CodeBlock`'s span — this probe
/// no longer re-implements the attachment, it recognizes it.
fn host_code_block(doc: &Document, anchor_span: &ByteSpan) -> Option<(ByteSpan, NodeRev)> {
    fn find(node: &Node, span: &ByteSpan) -> Option<(ByteSpan, NodeRev)> {
        if matches!(node.kind, NodeKind::CodeBlock { .. }) && node.span == *span {
            return Some((node.span.clone(), node.node_rev.clone()));
        }
        node.children.iter().find_map(|c| find(c, span))
    }
    find(&doc.root, anchor_span)
}
