//! Fence-language detection — the dispatch axis (verdict ruling 1, plan
//! decision #13). A task block is a fenced code block whose info string names
//! one of exactly two languages: `starlark` (hermetic kernel eval) or `bash`
//! (exec, unsandboxed). Anything else refuses loudly at load — there is no
//! third language and no silent fallback.

use model::{ByteSpan, Document, NodeKind};

/// The two task languages the runner dispatches on. A closed enum — adding a
/// language is a design event, not a parser tweak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskLanguage {
    /// Hermetic Starlark — evaluated in the sealed rules kernel.
    Starlark,
    /// Bash — executed in the invocation cwd (U16); NO governed-tree effect
    /// channel, and a stray write refuses (U6b).
    Bash,
}

impl TaskLanguage {
    /// The fence info-string token this language is declared as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TaskLanguage::Starlark => "starlark",
            TaskLanguage::Bash => "bash",
        }
    }

    /// The guarantee class this language's dispatch path carries (verdict
    /// ruling 4, decision #16) — labeled PER BLOCK, and the claim ships
    /// scoped, never unqualified.
    #[must_use]
    pub fn guarantee_class(self) -> GuaranteeClass {
        match self {
            TaskLanguage::Starlark => GuaranteeClass::Hermetic,
            TaskLanguage::Bash => GuaranteeClass::Unsandboxed,
        }
    }
}

/// What the sandbox can PROVE about a block's execution (decision #16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuaranteeClass {
    /// Proof by construction: the sealed Starlark kernel — zero I/O, closed
    /// builtins, metered.
    Hermetic,
    /// Detection, not prevention: what the exec-window bracket ITSELF provides
    /// — an ungoverned write inside the window is named, never blocked. The
    /// subject is the bracket ([`crate::snapshot::ExecBracket`]), not a task:
    /// the window closes and a `nohup`, launchd plist or cron line writes with
    /// no observer, so this is never a task's class.
    Detected,
    /// No guarantee: a bash task is an unsandboxed shell with undeclared
    /// effects (`docs/laws.md` § Amendment — capabilities do not apply to
    /// bash). The engine states what it observed, never what the block may do.
    Unsandboxed,
}

impl GuaranteeClass {
    /// The report label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GuaranteeClass::Hermetic => "hermetic",
            GuaranteeClass::Detected => "detected",
            GuaranteeClass::Unsandboxed => "unsandboxed",
        }
    }
}

/// One classified task block: its language, the fence node's span in the page,
/// and the inner source (fence lines stripped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBlock {
    /// The dispatch language.
    pub lang: TaskLanguage,
    /// The fence node's byte span in the page (fence lines included).
    pub span: ByteSpan,
    /// The block's source: the bytes between the fence lines, verbatim.
    pub source: String,
}

/// Why fence classification refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceError {
    /// The info string names neither `starlark` nor `bash` (or is empty).
    UnknownLanguage { lang: String },
    /// The fence never closes — a broken block is never executed.
    Unterminated,
    /// The span is not a fenced code block (defensive: addressing hands this
    /// module only code-block spans).
    NotAFence,
}

impl std::fmt::Display for FenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FenceError::UnknownLanguage { lang } if lang.is_empty() => {
                write!(
                    f,
                    "task fence declares no language (expected starlark or bash)"
                )
            }
            FenceError::UnknownLanguage { lang } => write!(
                f,
                "unknown task language '{lang}' (expected starlark or bash)"
            ),
            FenceError::Unterminated => write!(f, "task fence is unterminated"),
            FenceError::NotAFence => write!(f, "target is not a fenced code block"),
        }
    }
}

impl std::error::Error for FenceError {}

/// Classify the code block at `span`: read its language off the model node's
/// info string (first whitespace-separated token) and extract the inner source.
///
/// # Errors
/// [`FenceError::UnknownLanguage`] / [`FenceError::Unterminated`] /
/// [`FenceError::NotAFence`].
pub fn classify(doc: &Document, span: &ByteSpan) -> Result<TaskBlock, FenceError> {
    let node = find_code_block(doc, span).ok_or(FenceError::NotAFence)?;
    let NodeKind::CodeBlock { lang, unterminated } = &node.kind else {
        return Err(FenceError::NotAFence);
    };
    if *unterminated {
        return Err(FenceError::Unterminated);
    }
    let token = lang.split_whitespace().next().unwrap_or_default();
    let language = match token {
        "starlark" => TaskLanguage::Starlark,
        "bash" => TaskLanguage::Bash,
        other => {
            return Err(FenceError::UnknownLanguage {
                lang: other.to_owned(),
            });
        }
    };
    Ok(TaskBlock {
        lang: language,
        span: span.clone(),
        source: inner_source(&doc.raw[span.clone()]),
    })
}

/// The `CodeBlock` node with exactly this span.
fn find_code_block<'a>(doc: &'a Document, span: &ByteSpan) -> Option<&'a model::Node> {
    fn find<'a>(node: &'a model::Node, span: &ByteSpan) -> Option<&'a model::Node> {
        if node.span == *span && matches!(node.kind, NodeKind::CodeBlock { .. }) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, span))
    }
    find(&doc.root, span)
}

/// The bytes between the fence lines: drop the opening fence line and the
/// closing fence line, keep everything between verbatim.
fn inner_source(fence_text: &str) -> String {
    let after_open = fence_text.find('\n').map_or("", |p| &fence_text[p + 1..]);
    match after_open.rfind('\n') {
        Some(p) => after_open[..p].to_owned(),
        // Single-line fence body (open + close on adjacent lines) → empty.
        None => String::new(),
    }
}
