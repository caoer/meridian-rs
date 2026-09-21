//! Disposable document codec (node-rev-merkle-spec §6.9).
//!
//! Explicit private storage vocabulary, never serde on model types. Decode
//! verifies the raw digest and every node's span/rev, bounds lengths by the
//! input, and caps nesting. An unsupported or corrupt entry is a cache miss.
//! Registry owns files and framing; this crate is bytes in, bytes out.
use model::{Document, Node, NodeKind, NodeRev, YamlMap};

pub mod snapshot;

/// Semantic identity of the parser, model, address, codec, and dependencies.
/// It deliberately excludes unrelated daemon code and workspace versions.
pub const GENERATION: &str = env!("MRD_PARSE_GENERATION");
/// A cache limit, never a document-service limit: larger entries parse normally.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

/// Encode one immutable document. Too large or deeply nested means abstain.
#[must_use]
pub fn encode(doc: &Document) -> Option<Vec<u8>> {
    let mut w = Encoder(Vec::new());
    w.string(&doc.raw)?;
    w.node(&doc.root, 0)?;
    Some(w.0)
}

/// Restore only the document whose raw bytes match a verified content digest.
/// Invalid framing, types, spans, revisions, or trailing bytes mean a miss.
#[must_use]
pub fn decode(bytes: &[u8], digest: &[u8; 32]) -> Option<Document> {
    if bytes.len() > MAX_ENTRY_BYTES {
        return None;
    }
    let mut r = Decoder(bytes);
    let raw = r.string()?;
    if model::leaf_digest(raw.as_bytes()) != *digest {
        return None;
    }
    let root = r.node(&raw, 0, &(0..raw.len()))?;
    if !r.0.is_empty() || root.span != (0..raw.len()) || root.hpath.is_some() {
        return None;
    }
    match &root.kind {
        NodeKind::Document { path, line_count }
            if path.is_empty()
                && *line_count == u32::try_from(raw.lines().count()).unwrap_or(u32::MAX) => {}
        _ => return None,
    }
    Some(Document { raw, root })
}

struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, bytes: &[u8]) -> Option<()> {
        if self.0.len().checked_add(bytes.len())? > MAX_ENTRY_BYTES {
            return None;
        }
        self.0.extend_from_slice(bytes);
        Some(())
    }
    fn byte(&mut self, value: u8) -> Option<()> {
        self.bytes(&[value])
    }
    fn number(&mut self, value: usize) -> Option<()> {
        self.bytes(&u64::try_from(value).ok()?.to_le_bytes())
    }
    fn string(&mut self, value: &str) -> Option<()> {
        self.number(value.len())?;
        self.bytes(value.as_bytes())
    }
    fn optional(&mut self, value: Option<&str>) -> Option<()> {
        self.byte(u8::from(value.is_some()))?;
        if let Some(value) = value {
            self.string(value)?;
        }
        Some(())
    }
    fn link(
        &mut self,
        target: &str,
        heading: Option<&str>,
        block: Option<&str>,
        alias: Option<&str>,
    ) -> Option<()> {
        self.string(target)?;
        self.optional(heading)?;
        self.optional(block)?;
        self.optional(alias)
    }

    // Exhaustive matching makes a new model variant a codec compile error.
    #[allow(clippy::too_many_lines)]
    fn kind(&mut self, kind: &NodeKind) -> Option<()> {
        match kind {
            NodeKind::Document { path, line_count } => {
                self.byte(0)?;
                self.string(path)?;
                self.number(*line_count as usize)?;
            }
            NodeKind::Frontmatter { map } => {
                self.byte(1)?;
                self.number(map.0.len())?;
                for (key, value) in &map.0 {
                    self.string(key)?;
                    self.string(value)?;
                }
            }
            NodeKind::Section {
                heading_text,
                level,
            } => {
                self.byte(2)?;
                self.string(heading_text)?;
                self.byte(*level)?;
            }
            NodeKind::Heading { text, level } => {
                self.byte(3)?;
                self.string(text)?;
                self.byte(*level)?;
            }
            NodeKind::Paragraph => self.byte(4)?,
            NodeKind::List => self.byte(5)?,
            NodeKind::ListItem => self.byte(6)?,
            NodeKind::TaskItem { checked, depth } => {
                self.byte(7)?;
                self.byte(u8::from(*checked))?;
                self.number(*depth as usize)?;
            }
            NodeKind::CodeBlock { lang, unterminated } => {
                self.byte(8)?;
                self.string(lang)?;
                self.byte(u8::from(*unterminated))?;
            }
            NodeKind::Callout { r#type, fold } => {
                self.byte(9)?;
                self.string(r#type)?;
                self.string(fold)?;
            }
            NodeKind::Table => self.byte(10)?,
            NodeKind::Wikilink {
                target,
                heading,
                block,
                alias,
            } => {
                self.byte(11)?;
                self.link(
                    target,
                    heading.as_deref(),
                    block.as_deref(),
                    alias.as_deref(),
                )?;
            }
            NodeKind::Link { target } => {
                self.byte(12)?;
                self.string(target)?;
            }
            NodeKind::Embed {
                target,
                heading,
                block,
                alias,
            } => {
                self.byte(13)?;
                self.link(
                    target,
                    heading.as_deref(),
                    block.as_deref(),
                    alias.as_deref(),
                )?;
            }
            NodeKind::Anchor { name } => {
                self.byte(14)?;
                self.string(name)?;
            }
            NodeKind::Tag { name } => {
                self.byte(15)?;
                self.string(name)?;
            }
            NodeKind::InlineCode => self.byte(16)?,
            NodeKind::Comment => self.byte(17)?,
        }
        Some(())
    }
    fn node(&mut self, node: &Node, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.kind(&node.kind)?;
        self.number(node.span.start)?;
        self.number(node.span.end)?;
        self.string(&node.node_rev.0)?;
        self.byte(u8::from(node.hpath.is_some()))?;
        if let Some(path) = &node.hpath {
            self.number(path.len())?;
            for segment in path {
                self.string(segment)?;
            }
        }
        self.number(node.children.len())?;
        for child in &node.children {
            self.node(child, depth + 1)?;
        }
        Some(())
    }
}

struct Decoder<'a>(&'a [u8]);

impl Decoder<'_> {
    fn bytes(&mut self, len: usize) -> Option<&[u8]> {
        let (head, tail) = self.0.split_at_checked(len)?;
        self.0 = tail;
        Some(head)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn flag(&mut self) -> Option<bool> {
        match self.byte()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
    fn number(&mut self) -> Option<usize> {
        usize::try_from(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?)).ok()
    }
    fn count(&mut self) -> Option<usize> {
        let n = self.number()?;
        (n <= self.0.len()).then_some(n)
    }
    fn string(&mut self) -> Option<String> {
        let len = self.count()?;
        std::str::from_utf8(self.bytes(len)?)
            .ok()
            .map(str::to_owned)
    }
    // The outer option is decode failure; the inner is the stored absence.
    #[allow(clippy::option_option)]
    fn optional(&mut self) -> Option<Option<String>> {
        if self.flag()? {
            Some(Some(self.string()?))
        } else {
            Some(None)
        }
    }
    fn level(&mut self) -> Option<u8> {
        let level = self.byte()?;
        (1..=6).contains(&level).then_some(level)
    }
    #[allow(clippy::too_many_lines)]
    fn kind(&mut self) -> Option<NodeKind> {
        Some(match self.byte()? {
            0 => NodeKind::Document {
                path: self.string()?,
                line_count: u32::try_from(self.number()?).ok()?,
            },
            1 => {
                let count = self.count()?;
                let mut pairs = Vec::new();
                for _ in 0..count {
                    pairs.push((self.string()?, self.string()?));
                }
                NodeKind::Frontmatter {
                    map: YamlMap(pairs),
                }
            }
            2 => NodeKind::Section {
                heading_text: self.string()?,
                level: self.level()?,
            },
            3 => NodeKind::Heading {
                text: self.string()?,
                level: self.level()?,
            },
            4 => NodeKind::Paragraph,
            5 => NodeKind::List,
            6 => NodeKind::ListItem,
            7 => NodeKind::TaskItem {
                checked: self.flag()?,
                depth: u32::try_from(self.number()?).ok()?,
            },
            8 => NodeKind::CodeBlock {
                lang: self.string()?,
                unterminated: self.flag()?,
            },
            9 => NodeKind::Callout {
                r#type: self.string()?,
                fold: self.string()?,
            },
            10 => NodeKind::Table,
            11 => NodeKind::Wikilink {
                target: self.string()?,
                heading: self.optional()?,
                block: self.optional()?,
                alias: self.optional()?,
            },
            12 => NodeKind::Link {
                target: self.string()?,
            },
            13 => NodeKind::Embed {
                target: self.string()?,
                heading: self.optional()?,
                block: self.optional()?,
                alias: self.optional()?,
            },
            14 => NodeKind::Anchor {
                name: self.string()?,
            },
            15 => NodeKind::Tag {
                name: self.string()?,
            },
            16 => NodeKind::InlineCode,
            17 => NodeKind::Comment,
            _ => return None,
        })
    }
    fn node(&mut self, raw: &str, depth: usize, parent: &std::ops::Range<usize>) -> Option<Node> {
        if depth > MAX_DEPTH {
            return None;
        }
        let kind = self.kind()?;
        if depth > 0 && matches!(kind, NodeKind::Document { .. }) {
            return None;
        }
        let span = self.number()?..self.number()?;
        if span.start < parent.start || span.end > parent.end {
            return None;
        }
        let node_rev = NodeRev(self.string()?);
        if !model::node_rev_matches(raw, &span, &node_rev) {
            return None;
        }
        let hpath = if self.flag()? {
            let count = self.count()?;
            let mut path = Vec::new();
            for _ in 0..count {
                path.push(self.string()?);
            }
            Some(path)
        } else {
            None
        };
        let count = self.count()?;
        let mut children = Vec::new();
        for _ in 0..count {
            children.push(self.node(raw, depth + 1, &span)?);
        }
        Some(Node {
            kind,
            span,
            node_rev,
            hpath,
            children,
        })
    }
}
