//! The COMPILED-IN render plane (M1 U4a1, decision D-Render-min): the
//! [`Renderer`] trait with ONE production impl ([`ToonRenderer`]) and the
//! node-grain walker ([`walk`]) — no pack API, no `Manifest`, no load gate;
//! fixtures are ordinary golden tests.
//!
//! # Charter
//! **Owns:** the token-efficient projection of a read — the TOON-compact face
//! (U15, DECISION 27 · requirements D1 / decision 14 / R1.6), encoded by
//! [`toon`] and pinned by the reviewed fixtures under
//! `crates/testsuite/data/toon-goldens/` — and the two
//! decoration hook points the marathon decisions reserve (seam now, behavior
//! later):
//! block elision by fenced-language tag (#8) at the walker — BUILT in U4b,
//! opt-in via [`ToonRenderer::with_meridian_elision`] (the render-face
//! production configuration; `default()` stays inert, which is the elision
//! law: render-face only, and never on by accident) —
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

pub mod toon;
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
    /// The ambient corpus fingerprint this read was served at — the `b3b:…`
    /// world-grain token `mrd put --if-fingerprint` takes.
    ///
    /// **Read face v2 (dogfood G8).** The guard wanted a value the human face
    /// never printed, so a terminal user had to drop to `--json` to obtain the
    /// argument the flag they were being offered required. It rides the header
    /// because it is ONE fact about the whole read — a per-row column would
    /// re-type the same 70-odd bytes on every line, which is the very cost
    /// G2 is about.
    pub fingerprint: &'a str,
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
    /// The row's dewey ordinal (`2.1`), or `^id` on a block-anchor row — the
    /// short address that opens this body and re-reads it (read face v2).
    pub n: String,
    /// The row's RAW title, verbatim off the heading.
    pub title: String,
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
/// callers hold a [`ToonRenderer`] directly; the trait names the seam a
/// later pack tier extends.
pub trait Renderer {
    /// Render one job to its text projection.
    ///
    /// # Errors
    /// A typed [`RenderFailed`] (G1) — never a panic.
    fn render(&self, doc: &model::Document, job: &RenderJob<'_>) -> Result<Rendered, RenderFailed>;
}

/// The production renderer: the TOON-compact face (U15), with the block
/// elision predicate as the U4b hook. `default()` is INERT (elide nothing) —
/// the gates construct both configurations and pin the contrast, which is what
/// keeps "elision is opt-in and render-face only" a tested claim rather than a
/// remembered one. The production configuration is
/// [`ToonRenderer::with_meridian_elision`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ToonRenderer {
    /// The block-elision hook (#8 seam): a fenced block whose info-string
    /// matches is dropped from RENDERED section content — never from the
    /// raw read/cat face. `None` = emit everything (the golden-pinned
    /// default).
    pub elide_lang: Option<ElideBy>,
}

impl ToonRenderer {
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
        ToonRenderer {
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

impl Renderer for ToonRenderer {
    fn render(&self, doc: &model::Document, job: &RenderJob<'_>) -> Result<Rendered, RenderFailed> {
        match job {
            RenderJob::Toc { header, rows } => Ok(Rendered {
                text: toc_toon(header, rows),
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
                        n: row.fact.n.clone(),
                        title: row.fact.title.clone(),
                        sec_rev: row.fact.sec_rev.clone(),
                        words: fields_count(&content) as u64,
                        content,
                    });
                }
                Ok(Rendered {
                    text: sections_toon(header, &sections, *notice),
                    sections,
                })
            }
        }
    }
}

/// The read's header facts, as the TOON document's leading scalars. Shared by
/// both modes so the two faces cannot drift into two spellings of one header.
fn header_fields(header: &Header<'_>) -> Vec<(String, toon::Value)> {
    vec![
        ("path".to_string(), toon::Value::str(header.display_path)),
        ("file_rev".to_string(), toon::Value::str(header.file_rev)),
        ("fp".to_string(), toon::Value::str(header.fingerprint)),
        ("words".to_string(), toon::Value::Uint(header.words_total)),
    ]
}

/// The toc face: one TOON document — the header scalars, then the section map
/// as the tabular array the format exists for.
///
/// The dewey `n` is a STRING (`2.1` is an ordinal, not a float) and the
/// encoder quotes it for exactly that reason. `depth` carries what the retired
/// text face carried as leading indentation: a table cannot indent, and
/// dropping the depth would lose the tree shape a toc is read for.
///
/// **Read face v2 — the RELATIVE toc (ZT, 2026-08-04; dogfood G2).** The
/// address column was the FULL sanitized heading path, so every row re-typed
/// its ancestors: on a real 7319-word decision page a 106-char prefix was
/// repeated across 23 rows — 2438 of the map's 5473 bytes, 44%, and 23 of 24
/// rows past 120 columns. The discriminating text sat behind a wall of
/// identical prose and every terminal wrapped it, which defeats the
/// compactness TOON exists for.
///
/// So the column is the row's own RAW `title` and nothing else. The tree it
/// hangs from is not dropped, it is carried by the two columns that already
/// carry it losslessly: `depth` gives the level and `n` gives the dewey
/// position, so a reader reconstructs the path by eye and a machine addresses
/// any row by its `n` verbatim. Raw, not sanitized: `sanitize_heading` is
/// many-to-one, so the sanitized spelling was a projection a reader could not
/// invert back to the heading actually on the page — while `mrd put` takes
/// exactly the raw text this column now prints.
///
/// The structured `toc[]` rows are unchanged and still carry `hpath` as
/// segments (decision 14: arrays for machines, text for humans), so nothing a
/// machine addressed by is lost here.
#[must_use]
pub fn toc_toon(header: &Header<'_>, rows: &[&ReadFact]) -> String {
    let mut fields = header_fields(header);
    let toc: Vec<toon::Value> = rows
        .iter()
        .map(|row| {
            toon::Value::obj([
                ("n", toon::Value::str(&row.n)),
                ("depth", toon::Value::Uint(u64::from(row.depth))),
                ("title", toon::Value::str(&row.title)),
                ("words", toon::Value::Uint(row.words)),
                ("rev", toon::Value::str(&row.sec_rev)),
            ])
        })
        .collect();
    fields.push(("toc".to_string(), toon::Value::Arr(toc)));
    toon::encode(&toon::Value::Obj(fields))
}

/// The marker that opens one section's body in the sections face.
///
/// **Read face v2 (dogfood G2).** It was the full sanitized heading path, so a
/// deep section printed its 199-char address twice — once in the head's row,
/// once as the banner — around an 800-byte body: roughly half the response was
/// address. The marker is now the row's `n` alone.
///
/// `n` is not merely shorter, it is the SELECTOR: a dewey ordinal (or `^id`)
/// is exactly what `--section` takes, so the line that opens a body also says
/// how to ask for that body again. The title, rev, words and byte length stay
/// in the head, where each is stated once.
fn body_marker(n: &str) -> String {
    format!("== {n} ==")
}

/// The sections face: a TOON HEAD carrying the fixed-shape facts, then the
/// section BODIES verbatim, one per row, each opened by its `== n ==` marker.
///
/// **Why the bodies are not TOON.** The format has no block scalar (spec §7.1:
/// a newline inside a string is the `\n` escape, and that is the only
/// mechanism offered). Encoding a markdown section as a quoted TOON scalar
/// would be perfectly machine-legible and unreadable to a person — the exact
/// inverse of R1.6's *arrays for machines, TOON for humans*, on the one face
/// whose whole job is delivering prose to a reader. So the FACTS about the
/// sections are the TOON document and the sections themselves stay prose: the
/// head is a valid TOON document a machine parses to know what follows, and
/// the bodies are the bytes the read exists to serve.
///
/// `notice` rides the head too, so a partial read is a FIELD a reader can look
/// up rather than a sentence to notice at the bottom.
///
/// **The marker is not the boundary.** A `== n ==` line is ordinary text
/// and any section's prose may contain one — a document ABOUT this face
/// certainly does. So the head carries each body's `bytes` (its UTF-8 length),
/// and that is where a body ends: a reader takes the bodies in the head's
/// order, each running exactly its declared length after its marker line. The
/// marker is for the human eye, and content cannot forge a boundary because no
/// boundary is ever found by scanning for one. The alternative — escaping or
/// fencing the prose so the marker becomes unspellable — would edit the bytes
/// the read exists to serve verbatim, which is the A-K1 class.
#[must_use]
pub fn sections_toon(
    header: &Header<'_>,
    sections: &[RenderedSection],
    notice: Option<&str>,
) -> String {
    use std::fmt::Write as _;
    let mut b = sections_head(header, sections, notice);
    for s in sections {
        let _ = write!(b, "\n\n{}\n{}", body_marker(&s.n), s.content);
    }
    b.truncate(b.trim_end_matches('\n').len());
    b
}

/// The sections face's HEAD, alone: the TOON document that declares what
/// bodies follow and how long each one is.
///
/// Split out so the head is one expression with one owner — the gate that
/// asserts the head is valid TOON encodes it through this same function, and a
/// head that only existed inline could be asserted about but never re-derived.
#[must_use]
pub fn sections_head(
    header: &Header<'_>,
    sections: &[RenderedSection],
    notice: Option<&str>,
) -> String {
    let mut fields = header_fields(header);
    let rows: Vec<toon::Value> = sections
        .iter()
        .map(|s| {
            toon::Value::obj([
                ("n", toon::Value::str(&s.n)),
                ("title", toon::Value::str(&s.title)),
                ("rev", toon::Value::str(&s.sec_rev)),
                ("words", toon::Value::Uint(s.words)),
                ("bytes", toon::Value::Uint(s.content.len() as u64)),
            ])
        })
        .collect();
    fields.push(("sections".to_string(), toon::Value::Arr(rows)));
    if let Some(notice) = notice {
        fields.push(("notice".to_string(), toon::Value::str(notice)));
    }
    toon::encode(&toon::Value::Obj(fields))
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

    /// The toc face is ONE TOON document: header scalars, then the section map
    /// as a tabular array. The dewey ordinals arrive quoted because they are
    /// strings — `2.1` bare would decode as a float.
    ///
    /// Read face v2's claim rides here as the deep row: its column is the RAW
    /// leaf title, so a nested heading costs its own name and not its
    /// ancestors' — and `Slash/Title Here` proves the "raw" half, because the
    /// sanitized spelling of that heading (`Slash-Title-Here`) is a different
    /// string that `mrd put` does not take.
    #[test]
    fn the_toc_face_is_one_toon_document() {
        let (doc, facts) = doc_and_facts(RAW);
        let rows = toc_rows(&facts, &[]);
        let words_total: u64 = facts.iter().map(|f| f.words).sum();
        let header = Header {
            display_path: "$S/x.md",
            file_rev: &doc.root.node_rev.0,
            fingerprint: "fp",
            words_total,
            decorations: &NO_DECORATIONS,
        };
        let text = toc_toon(&header, &rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "path: $S/x.md");
        assert_eq!(lines[1], format!("file_rev: {}", doc.root.node_rev.0));
        assert_eq!(lines[2], "fp: fp");
        assert_eq!(lines[3], format!("words: {words_total}"));
        assert_eq!(lines[4], "toc[3]{n,depth,title,words,rev}:");
        assert!(lines[5].starts_with("  \"1\",1,Todo,"), "{}", lines[5]);
        assert!(lines[6].starts_with("  \"2\",1,Notes,"), "{}", lines[6]);
        assert!(
            lines[7].starts_with("  \"2.1\",2,Slash/Title Here,"),
            "the row costs its own RAW title and nothing of its ancestors — \
             the tree rides `depth` and `n`: {}",
            lines[7]
        );
        assert!(
            !text.contains("Notes/Slash"),
            "no row re-types an ancestor path (dogfood G2): {text}"
        );
        assert!(!text.ends_with('\n'), "no trailing newline");
    }

    /// A document with no headings renders `toc: []` — never the retired
    /// `toc[0]:` header, and never a bare missing field.
    #[test]
    fn an_empty_toc_renders_as_an_empty_array() {
        let (doc, facts) = doc_and_facts("just prose, no headings\n");
        let rows = toc_rows(&facts, &[]);
        let header = Header {
            display_path: "p",
            file_rev: &doc.root.node_rev.0,
            fingerprint: "fp",
            words_total: 0,
            decorations: &NO_DECORATIONS,
        };
        assert!(rows.is_empty(), "the fixture really has no heading rows");
        assert!(toc_toon(&header, &rows).ends_with("\ntoc: []"));
    }

    /// The sections face is a TOON HEAD then verbatim bodies (advisor-ruled
    /// 2026-08-04, U15). Two claims, and the first is the one that makes the
    /// hybrid honest: the head is not merely TOON-*shaped*, it IS the document
    /// [`sections_head`] encodes — asserted by re-deriving it, so a head that
    /// drifted from the encoder would redden here.
    #[test]
    fn the_sections_face_is_a_toon_head_then_verbatim_bodies() {
        let (doc, facts) = doc_and_facts(RAW);
        let fact = resolve_selector(&facts, &sel("Notes")).expect("resolves");
        let rows = [SectionRow {
            sel: &sel("Notes"),
            fact,
        }];
        let notice = "unresolved selectors (no rev minted): Ghost";
        let header = Header {
            display_path: "$S/x.md",
            file_rev: &doc.root.node_rev.0,
            fingerprint: "fp",
            words_total: 0,
            decorations: &NO_DECORATIONS,
        };
        let out = ToonRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: header.clone(),
                    rows: &rows,
                    notice: Some(notice),
                },
            )
            .expect("renders");

        let head = sections_head(&header, &out.sections, Some(notice));
        assert!(
            out.text.starts_with(&head),
            "the face opens with exactly the encoded head:\n{head}\n---\n{}",
            out.text
        );
        assert_eq!(
            head.lines().next(),
            Some("path: $S/x.md"),
            "and the head is a TOON document, not a prose header"
        );
        assert!(
            head.contains("sections[1]{n,title,rev,words,bytes}:"),
            "the head declares the bodies AND their lengths: {head}"
        );
        assert!(
            head.contains(&format!("notice: \"{notice}\"")),
            "the notice is a head FIELD, not a trailing sentence: {head}"
        );

        // The body follows its marker, verbatim and subtree-inclusive.
        let body = out.text.split("\n\n== 2 ==\n").nth(1).expect("one body");
        assert_eq!(body, out.sections[0].content.trim_end_matches('\n'));
        assert!(out.sections[0].content.contains("## Slash/Title Here"));
    }

    /// The positive control the advisor's ruling asks for: a body whose PROSE
    /// contains a line spelled exactly like a boundary marker.
    ///
    /// The marker is not the boundary — the head's `bytes` is — so the
    /// lookalike is served verbatim and changes nothing about where the body
    /// ends. This is the claim stated as a test rather than as a comment,
    /// because "content cannot forge a boundary" is precisely the kind of
    /// assertion that rots silently.
    #[test]
    fn prose_that_spells_a_marker_does_not_move_the_boundary() {
        let raw = "# Notes\n\nthe face writes a line like\n\n== 1 ==\n\nand this section is ABOUT that line\n";
        let (doc, facts) = doc_and_facts(raw);
        let fact = resolve_selector(&facts, &sel("Notes")).expect("resolves");
        let rows = [SectionRow {
            sel: &sel("Notes"),
            fact,
        }];
        let header = Header {
            display_path: "p",
            file_rev: "r",
            fingerprint: "fp",
            words_total: 0,
            decorations: &NO_DECORATIONS,
        };
        let out = ToonRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: header.clone(),
                    rows: &rows,
                    notice: None,
                },
            )
            .expect("renders");

        let content = &out.sections[0].content;
        assert!(
            content.contains("\n== 1 ==\n"),
            "the fixture really carries a lookalike marker: {content}"
        );
        // ONE section was read, so the head declares ONE row — the second
        // `== 1 ==` in the face is prose, and the head knows nothing of it.
        let head = sections_head(&header, &out.sections, None);
        assert!(head.contains("sections[1]{"), "{head}");
        assert!(
            head.contains(&format!(",{}", content.len())),
            "the declared length is the body's real byte length: {head}"
        );
        // And the body reached the reader byte-identically, lookalike included.
        assert_eq!(out.text.matches("== 1 ==").count(), 2);
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
            fingerprint: "fp",
            words_total: 0,
            decorations: &NO_DECORATIONS,
        };
        let job = RenderJob::Sections {
            header,
            rows: &rows,
            notice: None,
        };

        let inert = ToonRenderer::default().render(&doc, &job).expect("renders");
        assert!(
            inert.sections[0].content.contains("```meridian-lock"),
            "default elides NOTHING (U0 Go-parity): {}",
            inert.sections[0].content
        );

        let out = ToonRenderer::with_meridian_elision()
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

        let inert = ToonRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: Header {
                        display_path: "p",
                        file_rev: "r",
                        fingerprint: "fp",
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
        let out = ToonRenderer::default()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: Header {
                        display_path: "p",
                        file_rev: "r",
                        fingerprint: "fp",
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
            fingerprint: "fp",
            words_total: 0,
            decorations,
        };
        let render = |decorations| {
            ToonRenderer::default()
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
