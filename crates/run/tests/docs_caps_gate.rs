//! CI gate: every cap string and root-declaration example in `docs/*.md`
//! parses through the REAL parsers (card docs-cap-strings-parse-gate,
//! 19-20-mrd-statusd-integration). Two escapes motivated it, both P1s the
//! gate must catch forever (regression fixtures below):
//!
//! 1. run-plane.md's convention example carried an inline `# longest pattern
//!    wins` comment that PARSES AS A CAP and bricks the whole table of any
//!    root that copy-pastes it (pre-existing, republished by #150, fixed in
//!    #152);
//! 2. wire-contract § A.8's normative example used `md.patch` — not an
//!    [`effects::EffectKind`], so an implementer building a fixture from it
//!    encodes a value the engine cannot emit (fixed in #152).
//!
//! # What is extracted (the gate's aperture — anything outside it is unseen)
//!
//! - **Declaration fences**: any fenced block whose body carries a
//!   `run.caps.` line is a root-declaration example. Its frontmatter region
//!   (between `---` markers when present, else the whole body) is rebuilt as
//!   a page and fed through the same path a real root takes —
//!   [`caps::conventions_from_declaration`].
//! - **Explicit-grant lines**: every `task.<name>.caps:` line in any fence,
//!   fed through [`CapSet::parse`] like [`caps::explicit_caps`] feeds it.
//! - **Inline cap tokens**: every `` `md.…` `` backtick token outside fences,
//!   fed through [`CapSet::parse`] (the comma/space grammar, so single caps
//!   and lists ride one path). Every `` `run.caps.…` `` token likewise: a
//!   bare key validates its pattern through [`Conventions::new`]; a full
//!   `key: value` entry validates both halves.
//! - **Wire effect kinds**: every `"kind":"md.…"` string inside a fence must
//!   be an [`effects::EffectKind::ALL`] spelling.
//!
//! # Annotation convention
//!
//! An intentionally-refusing teaching example carries
//! `<!-- caps-gate: refuses -->` on its own line-or-the-line-above (for a
//! fence: the line above the opening fence line). Annotated examples are ASSERTED
//! to refuse — an annotated example that parses fails the gate too, so the
//! annotations stay honest. Nothing is silently exempted.
//!
//! Tokens carrying doc meta-syntax (`<`, `>`, `…`) are placeholders, not
//! spellings — skipped by lexical rule, stated here rather than per-site.
//! The exact token `md.*` is likewise skipped: it is the NAMESPACE's family
//! name ("the md.* partition"), used in prose across the docs, never a cap.

mod support;

use std::fmt::Write as _;
use std::path::Path;

use model::Document;
use run::caps::{self, CapSet, CapsError, Conventions};
use support::doc;

/// The declaration parser, one call site. `source` is the file a real root's
/// declaration was read from — the refusal names it (#159). A doc example has
/// no file, so `None`: the gate's own report already names the doc and line.
fn parse_conventions(declaration: &Document) -> Result<Conventions, CapsError> {
    caps::conventions_from_declaration(declaration, None)
}

/// The annotation that marks a teaching example as refusing on purpose.
const REFUSES: &str = "<!-- caps-gate: refuses -->";

/// Doc meta-syntax that marks a token as a placeholder, not a spelling —
/// plus the exact token `md.*`, the namespace's own family name in prose.
fn is_placeholder(token: &str) -> bool {
    token == "md.*" || token.contains('<') || token.contains('>') || token.contains('…')
}

/// One gate finding: where, what was fed, and what the parser said.
#[derive(Debug)]
struct Violation {
    file: String,
    line: usize,
    detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.detail)
    }
}

/// Scan one markdown document; return every violation. Pure over the text,
/// so the regression fixtures run the SAME extractor as the live docs.
fn scan_markdown(file: &str, text: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    // ── fence pass ──────────────────────────────────────────────────────
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let ticks = trimmed.chars().take_while(|&c| c == '`').count();
        if ticks >= 3 {
            let open = ticks;
            let annotated = i > 0 && lines[i - 1].contains(REFUSES);
            let start = i + 1;
            let mut end = start;
            while end < lines.len() {
                let t = lines[end].trim_start();
                let n = t.chars().take_while(|&c| c == '`').count();
                if n >= open && t[n..].trim().is_empty() {
                    break;
                }
                end += 1;
            }
            check_fence(
                file,
                &lines[start..end],
                start + 1,
                annotated,
                &mut violations,
            );
            i = end + 1;
            continue;
        }
        i += 1;
    }

    // ── inline pass (outside fences) ────────────────────────────────────
    let mut in_fence: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let ticks = t.chars().take_while(|&c| c == '`').count();
        if let Some(open) = in_fence {
            if ticks >= open && t[ticks..].trim().is_empty() {
                in_fence = None;
            }
            continue;
        }
        if ticks >= 3 {
            in_fence = Some(ticks);
            continue;
        }
        let annotated = line.contains(REFUSES) || (idx > 0 && lines[idx - 1].contains(REFUSES));
        for token in backtick_tokens(line) {
            check_inline(file, idx + 1, token, annotated, &mut violations);
        }
    }
    violations
}

/// The content of every properly closed single-backtick span on a line.
fn backtick_tokens(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        tokens.push(&after[..close]);
        rest = &after[close + 1..];
    }
    tokens
}

/// Fence checks: declaration examples, explicit-grant lines, wire kinds.
fn check_fence(
    file: &str,
    body: &[&str],
    first_line: usize,
    annotated: bool,
    violations: &mut Vec<Violation>,
) {
    // Root-declaration example: rebuild the page, feed the real parser.
    if body.iter().any(|l| l.trim_start().starts_with("run.caps.")) {
        let fm = frontmatter_region(body);
        let page = format!("---\n{}\n---\n\n# doc example\n", fm.join("\n"));
        let outcome = parse_conventions(&doc(&page));
        match (annotated, outcome) {
            (false, Err(e)) => violations.push(Violation {
                file: file.to_owned(),
                line: first_line,
                detail: format!("declaration example does not parse: {e}"),
            }),
            (true, Ok(_)) => violations.push(Violation {
                file: file.to_owned(),
                line: first_line,
                detail: "annotated `caps-gate: refuses` but the declaration parses — \
                         fix the example or drop the annotation"
                    .to_owned(),
            }),
            _ => {}
        }
    }
    for (offset, line) in body.iter().enumerate() {
        let t = line.trim();
        // Explicit grant line: `task.<name>.caps: <caps>`.
        if let Some(rest) = t.strip_prefix("task.")
            && let Some((_, value)) = rest.split_once(".caps:")
            && !annotated
            && let Err(e) = CapSet::parse(value)
        {
            violations.push(Violation {
                file: file.to_owned(),
                line: first_line + offset,
                detail: format!("`{t}` does not parse: {e}"),
            });
        }
        // Wire effect kind: `"kind":"md.…"` must be a real descriptor kind.
        for kind in json_md_kinds(line) {
            if !effects::EffectKind::ALL.iter().any(|k| k.as_str() == kind) {
                violations.push(Violation {
                    file: file.to_owned(),
                    line: first_line + offset,
                    detail: format!(
                        "`\"kind\":\"{kind}\"` is not an EffectKind the engine can emit"
                    ),
                });
            }
        }
    }
}

/// The `---`-delimited frontmatter region of a fence body, or the whole body
/// when the example shows bare keys without markers.
fn frontmatter_region<'a>(body: &[&'a str]) -> Vec<&'a str> {
    let marks: Vec<usize> = body
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == "---")
        .map(|(i, _)| i)
        .collect();
    match marks.as_slice() {
        [first, second, ..] => body[first + 1..*second].to_vec(),
        _ => body.to_vec(),
    }
}

/// Every `"kind":"md.…"` value on a line (whitespace-tolerant around `:`).
fn json_md_kinds(line: &str) -> Vec<&str> {
    let mut kinds = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("\"kind\"") {
        rest = &rest[at + "\"kind\"".len()..];
        let after_colon = rest.trim_start().strip_prefix(':').unwrap_or(rest);
        let candidate = after_colon.trim_start();
        if let Some(q) = candidate.strip_prefix('"')
            && let Some(close) = q.find('"')
            && q.starts_with("md.")
        {
            kinds.push(&q[..close]);
        }
    }
    kinds
}

/// Inline checks: `md.…` cap tokens and `run.caps.…` convention entries.
fn check_inline(
    file: &str,
    line: usize,
    token: &str,
    annotated: bool,
    violations: &mut Vec<Violation>,
) {
    let mut push = |detail: String| {
        violations.push(Violation {
            file: file.to_owned(),
            line,
            detail,
        });
    };
    if token.starts_with("md.") {
        if annotated {
            if CapSet::parse(token).is_ok() {
                push(format!(
                    "`{token}` is annotated `caps-gate: refuses` but parses — fix the \
                     example or drop the annotation"
                ));
            }
        } else if !is_placeholder(token)
            && let Err(e) = CapSet::parse(token)
        {
            push(format!("`{token}` does not parse: {e}"));
        }
    } else if let Some(entry) = token.strip_prefix("run.caps.") {
        if annotated || is_placeholder(token) {
            return; // same skip rules; nothing annotated exists today.
        }
        let (pattern, value) = match entry.split_once(':') {
            Some((p, v)) => (p, Some(v)),
            None => (entry, None),
        };
        if let Err(e) = Conventions::new(vec![(pattern.to_owned(), CapSet::none())]) {
            push(format!("`{token}` pattern does not validate: {e}"));
        }
        if let Some(value) = value
            && let Err(e) = CapSet::parse(value)
        {
            push(format!("`{token}` value does not parse: {e}"));
        }
    }
}

/// Every markdown file under the repo's `docs/`.
fn docs() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let mut pages = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("docs/ directory exists") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|e| e == "md") {
            let name = format!("docs/{}", path.file_name().unwrap().to_string_lossy());
            pages.push((name, std::fs::read_to_string(&path).expect("doc reads")));
        }
    }
    assert!(
        !pages.is_empty(),
        "docs/ scan found no markdown — wrong path?"
    );
    pages.sort();
    pages
}

// ── the gate ────────────────────────────────────────────────────────────────

#[test]
fn every_cap_string_and_declaration_example_in_docs_parses() {
    let mut report = String::new();
    let mut count = 0;
    for (name, text) in docs() {
        for v in scan_markdown(&name, &text) {
            count += 1;
            writeln!(report, "  {v}").unwrap();
        }
    }
    assert!(
        count == 0,
        "{count} doc example(s) do not survive the real parsers — a reader \
         copy-pasting them bricks a root or encodes a value the engine cannot \
         emit. Fix the example, or annotate a deliberate refusal with \
         `{REFUSES}` on the line above:\n{report}"
    );
}

// ── regression fixtures: the two historical escapes ─────────────────────────

#[test]
fn the_bricking_inline_comment_escape_is_caught() {
    // Escape 1 (run-plane.md, fixed in #152), verbatim shape: an inline
    // comment on a convention entry parses as a cap and bricks the table.
    let fixture = "\
```yaml
run.caps.fix-*: md.edit
run.caps.fix-note: md.edit:**/tasks/*.md  # longest pattern wins
```
";
    let violations = scan_markdown("fixture.md", fixture);
    assert!(
        violations
            .iter()
            .any(|v| v.detail.contains("declaration example does not parse")),
        "the inline-comment brick must be caught, got: {violations:?}"
    );
}

#[test]
fn the_md_patch_escape_is_caught_inline_and_on_the_wire() {
    // Escape 2 (wire-contract § A.8, fixed in #152): `md.patch` is neither a
    // cap verb nor an EffectKind.
    let fixture = "\
The op rides as `md.patch` on the wire:

```json
{\"kind\":\"md.patch\",\"domain\":\"x\"}
```
";
    let violations = scan_markdown("fixture.md", fixture);
    assert!(
        violations
            .iter()
            .any(|v| v.detail.contains("`md.patch` does not parse")),
        "inline md.patch must be caught, got: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.detail.contains("not an EffectKind")),
        "wire md.patch must be caught, got: {violations:?}"
    );
}

// ── annotation honesty ──────────────────────────────────────────────────────

#[test]
fn an_annotated_example_that_parses_fails_the_gate() {
    let fixture = "\
A healthy cap <!-- caps-gate: refuses -->
annotated as refusing: `md.edit:tasks/*.md`.
";
    let violations = scan_markdown("fixture.md", fixture);
    assert!(
        violations.iter().any(|v| v.detail.contains("but parses")),
        "a stale refuses-annotation must fail, got: {violations:?}"
    );
}

#[test]
fn an_annotated_refusing_example_passes_the_gate() {
    let fixture = "\
The targeted retired spelling <!-- caps-gate: refuses -->
(`md.set_field:status`) refuses with the retirement teaching.
";
    let violations = scan_markdown("fixture.md", fixture);
    assert!(
        violations.is_empty(),
        "annotated refusal is legal, got: {violations:?}"
    );
}

#[test]
fn placeholder_tokens_are_skipped_by_lexical_rule() {
    let fixture = "Scope by session: `md.edit:year=2026/<session>/tasks/*.md`.\n";
    let violations = scan_markdown("fixture.md", fixture);
    assert!(
        violations.is_empty(),
        "placeholders are meta-syntax, got: {violations:?}"
    );
}
