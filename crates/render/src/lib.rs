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

use std::collections::BTreeMap;

use wire_map::facts::ReadFact;
use wire_map::gotext::fields_count;

pub mod walk;

/// One claim-link address **as written in the body**: the link's target
/// spelling plus its block id, verbatim off the parsed node.
///
/// The address is the key, not a resolution: this crate never resolves a
/// linkpath, never reads a lock, and never computes a fingerprint. The CALLER
/// resolves — it is the one holding the corpus and the pin — and hands back a
/// map keyed by exactly the spellings its own document uses. That is what keeps
/// the rejected `render → lock → fingerprint` coupling out of here (D10), and
/// it is also why a decoration can never point at an address the body does not
/// literally contain.
///
/// **D12:** `target` is opaque to this crate. A later `root:` prefix rides
/// inside it untouched — the slot is reserved by not being parsed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClaimLink {
    /// The link's target spelling, verbatim (`guide`, `sub/guide.md`, …).
    pub target: String,
    /// The link's block id, verbatim — the promoted `^slug` a pin's target
    /// carries. The slug is a HANDLE, never the pin's identity: the pin's
    /// fingerprint covers the section span its `ref` resolves to, and an
    /// anchor node's span is only its host line.
    pub block: String,
}

/// The claim-link decorations for one render: address → the shaped token text
/// the walker splices into that link's block-ref slot (`@green.b3af12cd`,
/// minted by `syntax::fp_token` — the same predicate the strip-on-put side
/// recognizes, so decorate and strip cannot drift apart).
pub type Decorations = BTreeMap<ClaimLink, String>;

/// The empty decoration map — the one spelling of "nothing to decorate".
///
/// A host with no corpus (the per-request sidecar, the bare CLI) passes THIS
/// rather than an `Option`, so an absent map and an empty map cannot mean two
/// different things — the same discipline s1c applied to the anchor plane.
pub static NO_DECORATIONS: Decorations = Decorations::new();

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
    /// Stage-2 S10: the claim-link decorations for this render (D10). It rides
    /// the HEADER, so it is an input to the render as a whole and not to one
    /// mode — a toc render simply has no link to decorate. Empty
    /// ([`NO_DECORATIONS`]) is the inert input and the M1 behavior exactly.
    pub decorations: &'a Decorations,
}

/// One resolved sections-mode row: the selector that hit plus its fact.
#[derive(Debug, Clone)]
pub struct SectionRow<'a> {
    pub sel: &'a wire::ReadSel,
    pub fact: &'a ReadFact,
}

/// **The human address, derived HERE (U14).** A read fact carries its address
/// as raw segments; this is the joined, sanitized spelling a person reads —
/// `Notes/Slash-Title-Here`, or `^id` on a block-anchor row.
///
/// It is the same projection the read face used to CARRY as `ReadFact.hpath`,
/// moved to the one plane that wants it. Decision 14: arrays for machines,
/// text for humans — so the lossy many-to-one spelling is minted at the render
/// door, where being lossy costs nothing, and no machine surface publishes it
/// as if it were an address.
///
/// Because it is derived from the same segments and by the same
/// [`wire_map::gotext::sanitize_heading`] owner, the rendered bytes are
/// unchanged by the promotion.
#[must_use]
pub fn address_text(fact: &ReadFact) -> String {
    match &fact.anchor {
        Some(id) => format!("^{id}"),
        None => fact
            .hpath
            .iter()
            .map(|s| wire_map::gotext::sanitize_heading(&s.h))
            .collect::<Vec<_>>()
            .join("/"),
    }
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
    /// The render-face production configuration (U4b): blocks THE ENGINE
    /// EMITS are elided from rendered section content. The raw read/cat face
    /// never routes through render and stays verbatim (byte pin #4).
    ///
    /// Elision is **per-language, not per-namespace** (U36). A `meridian-lock`
    /// is machine-written and elided; a `meridian-mount` is content the user
    /// authored, so it renders. The name is kept for its call sites; the law it
    /// selects is [`ElideBy::EngineEmitted`].
    #[must_use]
    pub fn with_meridian_elision() -> Self {
        TextRenderer {
            elide_lang: Some(ElideBy::EngineEmitted),
        }
    }
}

/// The elision law: drop fenced blocks whose bytes THE ENGINE EMITTED.
///
/// The predicate is [`lock::is_engine_emitted`] — derived in `lock` from the
/// registered canonical writers. Render owns no list and never grew one; it
/// also does not decide the law, only that elision is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElideBy {
    /// Elide the blocks the engine writes: languages with a registered
    /// canonical writer, decided by [`lock::is_engine_emitted`].
    ///
    /// Deliberately **not** the `meridian-*` namespace: reservation answers
    /// "is this ours?", readership answers "should a reader see it?", and a
    /// human-authored `meridian-mount` block answers those two differently.
    EngineEmitted,
}

impl ElideBy {
    fn matches(self, lang: &str) -> bool {
        match self {
            ElideBy::EngineEmitted => lock::is_engine_emitted(lang),
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
                    let content =
                        walk::emit_section(doc, row.fact, self.elide_lang, header.decorations)?;
                    sections.push(RenderedSection {
                        hpath: address_text(row.fact),
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
            row.n,
            indent,
            address_text(row),
            row.words,
            row.sec_rev
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

    /// A selector from its human spelling, through the ONE ingress door.
    fn sel(s: &str) -> wire::ReadSel {
        wire::ReadSel::parse(s)
    }

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
        let rows = toc_rows(&facts, &[]);
        let words_total: u64 = facts.iter().map(|f| f.words).sum();
        let header = Header {
            display_path: "$S/x.md",
            file_rev: &doc.root.node_rev.0,
            words_total,
            decorations: &NO_DECORATIONS,
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
        let fact = resolve_selector(&facts, &sel("Notes")).expect("resolves");
        let rows = [SectionRow {
            sel: &sel("Notes"),
            fact,
        }];
        let header = Header {
            display_path: "$S/x.md",
            file_rev: &doc.root.node_rev.0,
            words_total: 0,
            decorations: &NO_DECORATIONS,
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
        let fact = resolve_selector(&facts, &sel("H")).expect("resolves");
        let rows = [SectionRow {
            sel: &sel("H"),
            fact,
        }];
        let header = Header {
            display_path: "p",
            file_rev: "r",
            words_total: 0,
            decorations: &NO_DECORATIONS,
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

    /// The S10 claim-link hook, same law as the elision hook: an EMPTY
    /// decoration map emits the raw slice byte-identically (the U0 goldens
    /// depend on it), and an inhabited one decorates exactly the addresses it
    /// names — never a neighbour that merely shares a slug or a target.
    #[test]
    fn decoration_hook_inert_when_empty_and_exact_when_inhabited() {
        let raw = "# H\n\nsee [[guide#^goal|Goal]] and [[other#^goal|Other]] and \
                   [[guide#^misc]] and [[Page#Q@Home]]\n";
        let (doc, facts) = doc_and_facts(raw);
        let fact = resolve_selector(&facts, &sel("H")).expect("resolves");
        let rows = [SectionRow {
            sel: &sel("H"),
            fact,
        }];

        let inert = TextRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: Header {
                        display_path: "p",
                        file_rev: "r",
                        words_total: 0,
                        decorations: &NO_DECORATIONS,
                    },
                    rows: &rows,
                    notice: None,
                },
            )
            .expect("renders");
        let cs = fact.content_span.as_ref().expect("content span");
        assert_eq!(
            inert.sections[0].content.as_bytes(),
            &raw.as_bytes()[usize::try_from(cs.0).unwrap()..usize::try_from(cs.1).unwrap()],
            "an empty map is byte-identity"
        );

        let mut decorations = Decorations::new();
        decorations.insert(
            ClaimLink {
                target: "guide".into(),
                block: "goal".into(),
            },
            "@green.b3af12cd".into(),
        );
        let out = TextRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: Header {
                        display_path: "p",
                        file_rev: "r",
                        words_total: 0,
                        decorations: &decorations,
                    },
                    rows: &rows,
                    notice: None,
                },
            )
            .expect("renders");
        let got = &out.sections[0].content;
        assert!(
            got.contains("[[guide#^goal@green.b3af12cd|Goal]]"),
            "the named address decorates: {got}"
        );
        assert!(
            got.contains("[[other#^goal|Other]]"),
            "a same-SLUG link to another target does not: {got}"
        );
        assert!(
            got.contains("[[guide#^misc]]"),
            "a same-TARGET link to another block does not: {got}"
        );
        assert!(
            got.contains("[[Page#Q@Home]]"),
            "and a heading fragment's `@` is not a slot at all: {got}"
        );
        assert_eq!(
            syntax::strip_fp(got),
            inert.sections[0].content,
            "the decoration is exactly what the put-side strip removes — \
             decorate and strip are inverses, which is what makes the round \
             trip disk-clean"
        );
    }

    /// F1: the token rides the ADDRESS slot, located structurally — not the last
    /// byte-match of `#^<block>` anywhere in the link. A label that quotes its own
    /// block ref (an ordinary sentence, no adversarial author) used to absorb the
    /// mint: the strip then could not see it, and the token reached disk.
    ///
    /// The claim is asserted as the PROPERTY — `syntax::fp_removals` over the
    /// rendered bytes finds the token, and the strip returns them to the inert
    /// render. A decoration the strip cannot remove fails here by construction.
    #[test]
    fn decoration_rides_the_address_slot_when_the_label_repeats_the_ref() {
        let raw = "# H\n\nsee [[guide#^goal|guide#^goal in full]] and \
                   ![[guide#^goal|#^goal]]\n";
        let (doc, facts) = doc_and_facts(raw);
        let fact = resolve_selector(&facts, &sel("H")).expect("resolves");
        let rows = [SectionRow {
            sel: &sel("H"),
            fact,
        }];
        let header = |decorations| Header {
            display_path: "p",
            file_rev: "r",
            words_total: 0,
            decorations,
        };
        let render = |decorations| {
            TextRenderer::default()
                .render(
                    &doc,
                    &RenderJob::Sections {
                        header: header(decorations),
                        rows: &rows,
                        notice: None,
                    },
                )
                .expect("renders")
        };

        let inert = render(&NO_DECORATIONS);
        let mut decorations = Decorations::new();
        decorations.insert(
            ClaimLink {
                target: "guide".into(),
                block: "goal".into(),
            },
            "@green.b3af12cd".into(),
        );
        let got = render(&decorations).sections[0].content.clone();

        assert_eq!(
            syntax::fp_removals(&got).len(),
            2,
            "one token per claim link, in the block-ref slot the strip owns: {got}"
        );
        assert_eq!(
            syntax::strip_fp(&got),
            inert.sections[0].content,
            "so the round trip is disk-clean: a token the strip cannot reach is a \
             token that reaches disk"
        );
        assert!(
            got.contains("[[guide#^goal@green.b3af12cd|guide#^goal in full]]"),
            "the label is authored text and stays verbatim: {got}"
        );
    }
}
