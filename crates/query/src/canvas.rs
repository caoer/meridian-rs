//! The JSON Canvas carrier (`move.md` §4 class 5): where a `.canvas` file's
//! rewritable strings sit in its RAW bytes.
//!
//! A canvas holds two reference slots per node — `file`, the vault-relative
//! path a file node points at, and `text`, a fragment of markdown that may
//! carry wikilinks. The move door rewrites both, and it must leave every other
//! byte of the JSON exactly as the author's editor wrote it: key order,
//! indentation, whitespace, escape style. So nothing here re-serializes. The
//! scan walks the document structurally and reports each slot as a byte span
//! into the original text plus the decoded value, with an offset table that
//! maps a position inside the decoded value back to the raw bytes it came
//! from. A rewrite is then the same span-exact splice the markdown classes
//! take.
//!
//! It is a LOCATOR, not a validator: it refuses the shapes it cannot address
//! (not one JSON object, a `nodes` that is not an array) and is otherwise
//! deliberately incurious about whether a canvas obeys the rest of the spec.

use std::fmt::Write as _;

use model::ByteSpan;

/// How deep a canvas may nest before the scan gives up. A canvas is a flat
/// list of nodes; anything past this is not one, and a bounded refusal beats a
/// blown stack on a hostile file.
const MAX_DEPTH: usize = 64;

/// One JSON string the door may rewrite, located in the canvas's raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// The string literal's CONTENT span — inside the quotes, escapes as
    /// written.
    pub raw: ByteSpan,
    /// The decoded value: what the string MEANS, escapes resolved.
    pub value: String,
    /// Decoded byte *i* → the raw offset it was decoded from. One entry longer
    /// than `value`, the last being the closing quote's offset, so any decoded
    /// range maps to a raw range by two lookups.
    offsets: Vec<usize>,
}

impl Slot {
    /// The raw span holding `value[start..end]`.
    ///
    /// # Panics
    /// If the range is not within the decoded value.
    #[must_use]
    pub fn raw_span(&self, start: usize, end: usize) -> ByteSpan {
        self.offsets[start]..self.offsets[end]
    }
}

/// Every rewritable slot of one canvas, in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slots {
    /// `nodes[].file` — a vault-relative path.
    pub files: Vec<Slot>,
    /// `nodes[].text` — a fragment of markdown.
    pub texts: Vec<Slot>,
}

/// Locate the rewritable slots of a JSON Canvas.
///
/// `None` when `raw` is not one JSON object this scan can walk, or carries a
/// `nodes` that is not an array. The caller reports such a file and leaves it
/// alone; there is no half-read canvas.
///
/// A canvas with no `nodes` key is not a refusal — Obsidian writes a bare `{}`
/// for a canvas the author has just created, and a file with nothing to
/// rewrite is not a file the door failed to read.
#[must_use]
pub fn slots(raw: &str) -> Option<Slots> {
    let mut scan = Scan::new(raw);
    let mut out = Slots::default();
    scan.ws();
    if scan.peek()? != b'{' {
        return None;
    }
    scan.object(Ctx::Root, &mut out)?;
    scan.ws();
    (scan.at == raw.len()).then_some(out)
}

/// Where the scan stands, which is the whole of its interest in the shape: the
/// top-level object (whose `nodes` key opens the array), an element of that
/// array (whose `file` and `text` keys are slots), or anywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Root,
    Node,
    Other,
}

struct Scan<'a> {
    raw: &'a str,
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl<'a> Scan<'a> {
    fn new(raw: &'a str) -> Self {
        Scan {
            raw,
            bytes: raw.as_bytes(),
            at: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        let hit = self.peek() == Some(byte);
        if hit {
            self.at += 1;
        }
        hit
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        self.eat(byte).then_some(())
    }

    /// One object. `ctx` says which of its keys are slots.
    fn object(&mut self, ctx: Ctx, out: &mut Slots) -> Option<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        self.expect(b'{')?;
        self.ws();
        if !self.eat(b'}') {
            loop {
                self.ws();
                let key = self.string()?;
                self.ws();
                self.expect(b':')?;
                self.ws();
                match (ctx, key.value.as_str()) {
                    // `nodes` is the one key whose shape the scan insists on:
                    // anything but an array there is a file it cannot address.
                    (Ctx::Root, "nodes") => {
                        if self.peek()? != b'[' {
                            return None;
                        }
                        self.array(Ctx::Node, out)?;
                    }
                    (Ctx::Node, "file") => self.slot(out, false)?,
                    (Ctx::Node, "text") => self.slot(out, true)?,
                    _ => self.value(out)?,
                }
                self.ws();
                if self.eat(b',') {
                    continue;
                }
                self.expect(b'}')?;
                break;
            }
        }
        self.depth -= 1;
        Some(())
    }

    /// A slot key's value: kept when it is a string, skipped when the canvas
    /// wrote something else there (a foreign format's shape is its own).
    fn slot(&mut self, out: &mut Slots, is_text: bool) -> Option<()> {
        if self.peek()? != b'"' {
            return self.value(out);
        }
        let slot = self.string()?;
        if is_text {
            out.texts.push(slot);
        } else {
            out.files.push(slot);
        }
        Some(())
    }

    fn array(&mut self, elements: Ctx, out: &mut Slots) -> Option<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        self.expect(b'[')?;
        self.ws();
        if !self.eat(b']') {
            loop {
                self.ws();
                if self.peek()? == b'{' {
                    self.object(elements, out)?;
                } else {
                    self.value(out)?;
                }
                self.ws();
                if self.eat(b',') {
                    continue;
                }
                self.expect(b']')?;
                break;
            }
        }
        self.depth -= 1;
        Some(())
    }

    /// Any value, walked for its structure alone.
    fn value(&mut self, out: &mut Slots) -> Option<()> {
        self.ws();
        match self.peek()? {
            b'"' => self.string().map(|_| ()),
            b'{' => self.object(Ctx::Other, out),
            b'[' => self.array(Ctx::Other, out),
            b't' => self.literal("true"),
            b'f' => self.literal("false"),
            b'n' => self.literal("null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn literal(&mut self, word: &str) -> Option<()> {
        if self.raw[self.at..].starts_with(word) {
            self.at += word.len();
            return Some(());
        }
        None
    }

    /// A number, consumed by its byte set. The scan locates slots; it does not
    /// adjudicate whether `1.2.3` is a number, because no slot's position
    /// depends on the answer.
    fn number(&mut self) -> Option<()> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        (self.at > start).then_some(())
    }

    /// One string literal, decoded, with each decoded byte's raw origin.
    fn string(&mut self) -> Option<Slot> {
        self.expect(b'"')?;
        let start = self.at;
        let mut value = String::new();
        let mut offsets: Vec<usize> = Vec::new();
        loop {
            let from = self.at;
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    break;
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self.peek()?;
                    self.at += 1;
                    let ch = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return None,
                    };
                    push_char(&mut value, &mut offsets, ch, from, false);
                }
                // A raw control byte is not a JSON string.
                byte if byte < 0x20 => return None,
                _ => {
                    let ch = self.raw[self.at..].chars().next()?;
                    self.at += ch.len_utf8();
                    push_char(&mut value, &mut offsets, ch, from, true);
                }
            }
        }
        let close = self.at - 1;
        offsets.push(close);
        Some(Slot {
            raw: start..close,
            value,
            offsets,
        })
    }

    /// A `\u` escape, the leading `\u` already consumed — a surrogate pair
    /// decodes to its one character, a lone surrogate is not a string this
    /// scan will address.
    fn unicode_escape(&mut self) -> Option<char> {
        let high = self.hex4()?;
        if (0xD800..0xDC00).contains(&high) {
            self.expect(b'\\')?;
            self.expect(b'u')?;
            let low = self.hex4()?;
            if !(0xDC00..0xE000).contains(&low) {
                return None;
            }
            return char::from_u32(0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00));
        }
        char::from_u32(high)
    }

    fn hex4(&mut self) -> Option<u32> {
        let text = self.raw.get(self.at..self.at + 4)?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.at += 4;
        u32::from_str_radix(text, 16).ok()
    }
}

/// Append `ch` and one offset per byte it occupies. `verbatim` — the character
/// stood unescaped in the source, so its bytes ARE the raw bytes and each maps
/// to its own offset; an escaped character has one source position, and every
/// byte of it maps there.
fn push_char(value: &mut String, offsets: &mut Vec<usize>, ch: char, from: usize, verbatim: bool) {
    let start = value.len();
    value.push(ch);
    for i in start..value.len() {
        offsets.push(if verbatim { from + (i - start) } else { from });
    }
}

/// `text` as the CONTENT of a JSON string literal: the two structural bytes and
/// the control range escaped, every other byte verbatim — so an already-plain
/// path lands byte-identical to how a canvas editor would write it.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: &str = "{\n\t\"nodes\":[\n\t\t{\"id\":\"a\",\"type\":\"file\",\"file\":\"a/x.md\",\"subpath\":\"#Top\",\"x\":-120},\n\t\t{\"id\":\"b\",\"type\":\"text\",\"text\":\"see [[x]]\\nand more\",\"x\":4}\n\t],\n\t\"edges\":[{\"id\":\"e\",\"fromNode\":\"a\",\"toNode\":\"b\"}]\n}";

    #[test]
    fn the_scan_finds_file_and_text_slots_and_nothing_else() {
        let slots = slots(CANVAS).expect("a canvas");
        assert_eq!(slots.files.len(), 1);
        assert_eq!(slots.texts.len(), 1);
        assert_eq!(slots.files[0].value, "a/x.md");
        assert_eq!(slots.texts[0].value, "see [[x]]\nand more");
        assert_eq!(&CANVAS[slots.files[0].raw.clone()], "a/x.md");
    }

    /// The offset table maps a position inside the DECODED text back through
    /// the escapes to the raw bytes — the whole point of not re-serializing.
    #[test]
    fn a_decoded_range_maps_back_across_an_escape() {
        let slots = slots(CANVAS).expect("a canvas");
        let text = &slots.texts[0];
        let start = text.value.find("[[").expect("a link") + 2;
        let end = text.value.find("]]").expect("a link");
        let span = text.raw_span(start, end);
        assert_eq!(&CANVAS[span], "x");
        // The `\n` is two raw bytes and one decoded byte: a position after it
        // still lands on its own source byte.
        let tail = text.value.find("more").expect("the tail");
        let span = text.raw_span(tail, tail + 4);
        assert_eq!(&CANVAS[span], "more");
    }

    /// What the scan cannot address it refuses; what it can address and finds
    /// empty it accepts. `{}` is the file Obsidian writes for a new canvas —
    /// reporting it every run would be a false alarm the operator learns to
    /// ignore.
    #[test]
    fn only_the_shapes_the_scan_cannot_address_are_refused() {
        assert!(slots("{\"nodes\":[}").is_none(), "not JSON");
        assert!(slots("[]").is_none(), "not an object");
        assert!(slots("{\"nodes\":[]} trailing").is_none(), "trailing bytes");
        assert!(slots("{\"nodes\":3}").is_none(), "nodes is not an array");
        assert!(slots("{\"nodes\":[]}").is_some(), "an empty canvas is one");
        assert_eq!(slots("{}"), Some(Slots::default()), "a new canvas is one");
        assert_eq!(
            slots("{\"edges\":[]}"),
            Some(Slots::default()),
            "a canvas of edges alone has nothing to rewrite, and is not unread"
        );
    }

    /// A `file` key outside a node — in an edge, or nested in some other
    /// object — is not a slot: position is decided structurally, never by the
    /// key name alone.
    #[test]
    fn a_file_key_outside_a_node_is_not_a_slot() {
        let raw = "{\"nodes\":[],\"edges\":[{\"file\":\"a/x.md\"}],\"meta\":{\"file\":\"b.md\"}}";
        let slots = slots(raw).expect("a canvas");
        assert!(slots.files.is_empty(), "{slots:#?}");
    }

    #[test]
    fn escapes_survive_a_round_trip_through_the_decoder() {
        let raw =
            "{\"nodes\":[{\"type\":\"text\",\"text\":\"q \\\" b \\\\ u \\u00e9 \\ud83d\\ude00\"}]}";
        let slots = slots(raw).expect("a canvas");
        assert_eq!(slots.texts[0].value, "q \" b \\ u é 😀");
    }

    #[test]
    fn escape_touches_only_what_json_requires() {
        assert_eq!(
            escape("domains/data/duckdb/tips.md"),
            "domains/data/duckdb/tips.md"
        );
        assert_eq!(escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(escape("é"), "é");
    }

    #[test]
    fn nesting_past_the_depth_bound_is_refused_not_a_stack_overflow() {
        let deep = format!(
            "{{\"nodes\":[],\"x\":{}{}}}",
            "[".repeat(200),
            "]".repeat(200)
        );
        assert!(slots(&deep).is_none());
    }
}
