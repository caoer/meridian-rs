//! The COMPILED-IN render plane (M1 U4a1, decision D-Render-min): the
//! [`Renderer`] trait with ONE production impl ([`TextRenderer`]) and the
//! node-grain walker ([`walk`]) — no pack API, no `Manifest`, no load gate;
//! fixtures are ordinary golden tests.
//!
//! # Charter
//! **Owns:** the token-efficient TEXT projection of a read — the `readText`
//! byte format ccc-statusd rendered host-side (`read.go:118`), reproduced
//! byte-for-byte against the U0 captured goldens — and the two decoration
//! hook points the marathon decisions reserve (seam now, behavior later):
//! block elision by fenced-language tag (#8) at the walker — BUILT in U4b,
//! opt-in via [`TextRenderer::with_meridian_elision`] (the render-face
//! production configuration; `default()` stays inert for the U0 goldens) —
//! and the `Link/Wikilink` visitor point (#6), NO-OP passthrough in M1
//! (A-K1: stored bytes and the raw read/cat face never change — byte pin
//! #4: `meridian-*` blocks ride the raw face VERBATIM, elision is
//! RENDER-face only).
//!
//! **Never does:** wire shapes (the composed read op is `wire-serve`'s),
//! addressing computation (`wire-map::facts`), parsing (`syntax`), disk
//! (`fs`).
//!
//! # G1 (panic-free law)
//! The walker returns a typed [`RenderFailed`] `{node_kind, node_ref,
//! reason}` — it NEVER panics; the sidecar serve loop is panic-free by law.

use wire_map::facts::ReadFact;
use wire_map::gotext::fields_count;

pub mod walk;

/// Typed render failure (G1): named node kind + a human-addressable node ref
/// + the reason. Recovery class: none (a render bug is a bug, not a retry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFailed {
    /// The node kind being walked when the failure surfaced.
    pub node_kind: String,
    /// A human-addressable spelling of the node (hpath, `^anchor`, or span).
    pub node_ref: String,
    /// What went wrong.
    pub reason: String,
}

impl std::fmt::Display for RenderFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "render_failed: {} at {}: {}",
            self.node_kind, self.node_ref, self.reason
        )
    }
}

impl std::error::Error for RenderFailed {}

/// The header line facts (`readText` line 1): the display path is the HOST
/// face's spelling (the engine never invents paths — the caller passes the
/// string the consumer expects, e.g. an absolute session path).
#[derive(Debug, Clone)]
pub struct Header<'a> {
    pub display_path: &'a str,
    pub file_rev: &'a str,
    pub words_total: u64,
}

/// One resolved sections-mode row: the selector that hit plus its fact.
#[derive(Debug, Clone)]
pub struct SectionRow<'a> {
    pub sel: &'a str,
    pub fact: &'a ReadFact,
}

/// One rendered section: the emitted content plus its recomputed word count
/// (`renderSectionsSidecar` recounts over the RETURNED content, so a block
/// leaf counts its words here even though its fact carries 0).
#[derive(Debug, Clone)]
pub struct RenderedSection {
    pub hpath: String,
    pub sec_rev: String,
    pub words: u64,
    pub content: String,
}

/// A render job: the toc shape table, or selected sections' content.
#[derive(Debug, Clone)]
pub enum RenderJob<'a> {
    /// The shape table over ALREADY-FILTERED rows (`wire_map::facts::
    /// toc_rows` applies the frag scope; refusals are the op layer's).
    Toc {
        header: Header<'a>,
        rows: &'a [&'a ReadFact],
    },
    /// Selected sections, content emitted through the walker (elision hook
    /// point), plus the PARTIAL-read notice when selectors went unresolved.
    Sections {
        header: Header<'a>,
        rows: &'a [SectionRow<'a>],
        notice: Option<&'a str>,
    },
}

/// The rendered result: the text face plus the per-section emissions (the
/// composed read op forwards both).
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub text: String,
    pub sections: Vec<RenderedSection>,
}

/// The render seam — ONE compiled-in impl in M1 (no `dyn`, no registry):
/// callers hold a [`TextRenderer`] directly; the trait names the seam a
/// later pack tier extends.
pub trait Renderer {
    /// Render one job to its text projection.
    ///
    /// # Errors
    /// A typed [`RenderFailed`] (G1) — never a panic.
    fn render(&self, doc: &model::Document, job: &RenderJob<'_>) -> Result<Rendered, RenderFailed>;
}

/// The production text renderer: `readText` byte format, with the block
/// elision predicate as the U4b hook. `default()` is INERT (elide nothing)
/// — the U0 goldens pin Go-parity bytes and the golden gates construct it;
/// the render-face production configuration is
/// [`TextRenderer::with_meridian_elision`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TextRenderer {
    /// The block-elision hook (#8 seam): a fenced block whose info-string
    /// matches is dropped from RENDERED section content — never from the
    /// raw read/cat face. `None` = emit everything (the golden-pinned
    /// default).
    pub elide_lang: Option<ElideBy>,
}

impl TextRenderer {
    /// The render-face production configuration (U4b): engine blocks
    /// (`meridian-*`) are elided from rendered section content. The raw
    /// read/cat face never routes through render and stays verbatim (byte
    /// pin #4).
    #[must_use]
    pub fn with_meridian_elision() -> Self {
        TextRenderer {
            elide_lang: Some(ElideBy::MeridianNamespace),
        }
    }
}

/// The one elision law reserved by decision #8: drop fenced blocks whose
/// info-string is an ENGINE block. The predicate is
/// [`lock::is_meridian_lang`] — the sole owner of the reserved `meridian-*`
/// namespace (#8 §1); render never grows its own list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElideBy {
    /// Elide engine blocks: info-strings in the reserved `meridian-*`
    /// namespace, decided by [`lock::is_meridian_lang`].
    MeridianNamespace,
}

impl ElideBy {
    fn matches(self, lang: &str) -> bool {
        match self {
            ElideBy::MeridianNamespace => lock::is_meridian_lang(lang),
        }
    }
}

impl Renderer for TextRenderer {
    fn render(&self, doc: &model::Document, job: &RenderJob<'_>) -> Result<Rendered, RenderFailed> {
        match job {
            RenderJob::Toc { header, rows } => Ok(Rendered {
                text: toc_text(header, rows),
                sections: Vec::new(),
            }),
            RenderJob::Sections {
                header,
                rows,
                notice,
            } => {
                let mut sections = Vec::with_capacity(rows.len());
                for row in *rows {
                    let content = walk::emit_section(doc, row.fact, self.elide_lang)?;
                    sections.push(RenderedSection {
                        hpath: row.fact.hpath.clone(),
                        sec_rev: row.fact.sec_rev.clone(),
                        words: fields_count(&content) as u64,
                        content,
                    });
                }
                Ok(Rendered {
                    text: sections_text(header, &sections, *notice),
                    sections,
                })
            }
        }
    }
}

/// `readText` toc mode, byte-for-byte (`read.go:120-130`): header line, then
/// `%-6s %s%s  words:%d  rev:%s` rows (dewey padded to 6, two-space indent
/// per depth-1), all trailing newlines trimmed. Marks never render in M1 —
/// the sidecar-backed host face never populated them.
#[must_use]
pub fn toc_text(header: &Header<'_>, rows: &[&ReadFact]) -> String {
    use std::fmt::Write as _;
    let mut b = header_line(header);
    for row in rows {
        let indent = "  ".repeat(row.depth.saturating_sub(1) as usize);
        let _ = writeln!(
            b,
            "{:<6} {}{}  words:{}  rev:{}",
            row.n, indent, row.hpath, row.words, row.sec_rev
        );
    }
    b.truncate(b.trim_end_matches('\n').len());
    b
}

/// `readText` sections mode, byte-for-byte (`read.go:132-138`): header line,
/// one `\n== hpath (rev:.. words:..) ==\ncontent\n` block per section, the
/// NOTICE line when present, all trailing newlines trimmed.
#[must_use]
pub fn sections_text(
    header: &Header<'_>,
    sections: &[RenderedSection],
    notice: Option<&str>,
) -> String {
    use std::fmt::Write as _;
    let mut b = header_line(header);
    for s in sections {
        let _ = write!(
            b,
            "\n== {} (rev:{} words:{}) ==\n{}\n",
            s.hpath, s.sec_rev, s.words, s.content
        );
    }
    if let Some(notice) = notice {
        let _ = write!(b, "\nNOTICE: {notice}\n");
    }
    b.truncate(b.trim_end_matches('\n').len());
    b
}

fn header_line(header: &Header<'_>) -> String {
    format!(
        "{}  file_rev:{}  words:{}\n",
        header.display_path, header.file_rev, header.words_total
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire_map::facts::{read_facts, resolve_selector, toc_rows};

    fn doc_and_facts(raw: &str) -> (model::Document, Vec<ReadFact>) {
        let doc = model::build(raw.to_string(), syntax::parse(raw));
        let facts = read_facts(&wire_map::project_toc(&doc), raw.as_bytes());
        (doc, facts)
    }

    const RAW: &str = "---\ntype: note\n---\n\n# Todo\n\n- [ ] first item\n\n# Notes\n\nseed note line one\n\n## Slash/Title Here\n\ndeep content\n";

    /// The toc text face: `%-6s` dewey padding, depth indent, trailing
    /// newline trimmed — the readText shape on a representative doc.
    #[test]
    fn toc_text_matches_readtext_shape() {
        let (doc, facts) = doc_and_facts(RAW);
        let rows = toc_rows(&facts, "");
        let words_total: u64 = facts.iter().map(|f| f.words).sum();
        let header = Header {
            display_path: "$S/x.md",
            file_rev: &doc.root.node_rev.0,
            words_total,
        };
        let text = toc_text(&header, &rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            format!(
                "$S/x.md  file_rev:{}  words:{words_total}",
                doc.root.node_rev.0
            )
        );
        assert!(lines[1].starts_with("1      Todo  words:"), "{}", lines[1]);
        assert!(lines[2].starts_with("2      Notes  words:"), "{}", lines[2]);
        assert!(
            lines[3].starts_with("2.1      Notes/Slash-Title-Here  words:"),
            "pad-6 then two-space indent: {}",
            lines[3]
        );
        assert!(!text.ends_with('\n'), "TrimRight newline");
    }

    /// The sections text face incl. the NOTICE line and the `\n==` seam.
    #[test]
    fn sections_text_matches_readtext_shape() {
        let (doc, facts) = doc_and_facts(RAW);
        let fact = resolve_selector(&facts, "Notes").expect("resolves");
        let rows = [SectionRow { sel: "Notes", fact }];
        let header = Header {
            display_path: "$S/x.md",
            file_rev: &doc.root.node_rev.0,
            words_total: 0,
        };
        let out = TextRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header,
                    rows: &rows,
                    notice: Some("unresolved selectors (no rev minted): Ghost"),
                },
            )
            .expect("renders");
        assert!(
            out.text
                .contains(&format!("\n== Notes (rev:{} words:", fact.sec_rev)),
            "{}",
            out.text
        );
        assert!(
            out.text
                .ends_with("\nNOTICE: unresolved selectors (no rev minted): Ghost")
        );
        // section content is the RAW face (subtree-inclusive)
        assert!(out.sections[0].content.contains("## Slash/Title Here"));
    }

    /// The U4b hook is INERT by default: a `meridian-*` fenced block renders
    /// verbatim (Go parity, the U0 meridian-block golden), and the named
    /// production configuration elides exactly that block from the RENDERED
    /// face only.
    #[test]
    fn elision_hook_inert_by_default_active_by_predicate() {
        let raw = "# H\n\nbefore\n\n```meridian-lock\nv: 1\n```\n\nafter\n";
        let (doc, facts) = doc_and_facts(raw);
        let fact = resolve_selector(&facts, "H").expect("resolves");
        let rows = [SectionRow { sel: "H", fact }];
        let header = Header {
            display_path: "p",
            file_rev: "r",
            words_total: 0,
        };
        let job = RenderJob::Sections {
            header,
            rows: &rows,
            notice: None,
        };

        let inert = TextRenderer::default().render(&doc, &job).expect("renders");
        assert!(
            inert.sections[0].content.contains("```meridian-lock"),
            "default elides NOTHING (U0 Go-parity): {}",
            inert.sections[0].content
        );

        let out = TextRenderer::with_meridian_elision()
            .render(&doc, &job)
            .expect("renders");
        assert!(
            !out.sections[0].content.contains("meridian-lock"),
            "predicate elides the block: {}",
            out.sections[0].content
        );
        assert!(out.sections[0].content.contains("before"));
        assert!(out.sections[0].content.contains("after"));
    }
}
