//! The named model→wire projection seam (review C1 ruling, 4/4 lenses):
//! tree-flatten + `text_prefix_16b` + node ordering as a **tested library
//! function, never bin code**.
//!
//! # Charter
//! **Owns:** the projection from `model`'s governed tree (non-serializable, by
//! law) to `wire`'s flat node list (contract §5.2): flattening, kind mapping,
//! the frozen prefix law, and the frozen total node order.
//!
//! **Never does:** I/O, framing, parsing, business logic, body formatting.
//!
//! # Law 3, as amended (review C1)
//! Original wording: "only the bin sees wire and model together." Amended:
//! **"only the named wire-map seam and the bin see both."** This crate IS that
//! seam — the one place projection behavior lives and is tested; `sidecar`
//! stays wiring-only. Growing projection pressure lands here, never in a bin.
//!
//! # Rungs
//! Rung 1: `project` (toc/extract node lists). Rung 2+: projection of resolve
//! targets and splice verdicts joins additively.

/// Project a parsed document onto the wire node list: flatten the governed
/// tree to the contract's flat kinds, compute `text_prefix_16b` from the raw
/// bytes, attach `node_rev`s, and emit in the frozen total order
/// (span.start asc, span.end desc, kind-ordinal asc — contract §5.2).
#[must_use]
pub fn project(doc: &model::Document) -> Vec<wire::Node> {
    let _ = doc;
    todo!("rung 1: tree-flatten + kind map + prefix + order")
}

/// The frozen prefix law (contract §5.2), implemented — the seam proof, real
/// code with the contract's worked examples as tests.
///
/// Window: `raw[start : min(start+16, len)]` — clamped at EOF, never padded.
/// Decode: longest valid-UTF-8 head emitted as-is; each remaining byte as the
/// four-char ASCII sequence `\xhh` (lowercase hex). Only a single multibyte
/// character truncated by the window end ever gets escaped.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the unwrap re-reads only bytes from_utf8 already validated"
)]
pub fn prefix_16b(raw: &[u8], start: usize) -> String {
    use std::fmt::Write;

    let end = start.saturating_add(16).min(raw.len());
    let window = &raw[start.min(raw.len())..end];
    match std::str::from_utf8(window) {
        Ok(s) => s.to_owned(),
        Err(e) => {
            let k = e.valid_up_to();
            let mut out = String::with_capacity(k + (window.len() - k) * 4);
            out.push_str(std::str::from_utf8(&window[..k]).unwrap());
            for b in &window[k..] {
                let _ = write!(out, "\\x{b:02x}");
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::prefix_16b;

    /// Contract §5.2 worked example: 2-byte split. `é` = 0xc3 0xa9 at bytes
    /// 15–17; the window cuts after its first byte.
    #[test]
    fn split_multibyte_2byte() {
        let raw = "# 1234567890123é tail".as_bytes();
        assert_eq!(prefix_16b(raw, 0), "# 1234567890123\\xc3");
    }

    /// Contract §5.2 worked example: 4-byte split. `𝄞` = 0xf0 0x9d 0x84 0x9e
    /// at bytes 14–18; the window cuts after its second byte.
    #[test]
    fn split_multibyte_4byte() {
        let raw = "# abcdefghijkl𝄞 x".as_bytes();
        assert_eq!(prefix_16b(raw, 0), "# abcdefghijkl\\xf0\\x9d");
    }

    /// Contract §5.3 example: a node starting fewer than 16 bytes before EOF
    /// has a short prefix — clamped, never padded ("## Beta\n", 8 bytes).
    #[test]
    fn clamped_at_eof() {
        let raw = "---\ntitle: demo\n---\n\n# Alpha\n\n## Beta\n".as_bytes();
        assert_eq!(prefix_16b(raw, 30), "## Beta\n");
    }
}
