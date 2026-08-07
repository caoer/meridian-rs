//! The armed-set digest — **the one definition** (`docs/run-plane.md`
//! § Sub-amendment (the armed-set expectation, `--expect-armed`)).
//!
//! The arm/commit split runs the entry twice: the host gates the ARM's rows,
//! then a second child commits. The amendment argues the two children arm
//! identically — recorded-read purity plus an unmoved fingerprint — and then
//! relies on that argument. This module turns the argument into a per-call
//! measurement: the commit child hashes what it armed and refuses before the
//! splice when the value disagrees with what the host was shown.
//!
//! **Why this file exists at all, rather than a hash inlined at the comparison
//! site.** The failure this guards against is not a wrong hash; it is a SECOND
//! hash. Two canonicalizations — one in the engine, one re-derived host-side —
//! agree only by luck, and the two ways they disagree are both terrible: refuse
//! every call, or produce a **vacuous pass** where both sides compute something
//! equally wrong and match. So the digest gets exactly one spelling, in one
//! function, reached by both children, and the host is a courier that copies the
//! string out of the arm's trace rather than a second implementation of it.
//!
//! The serialization is published in the doc amendment so that a host which
//! someday verifies independently lands on the same bytes instead of guessing.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The digest of an armed set: `sha256:` followed by 64 lowercase hex digits.
///
/// `edits` is the array the commit splice would carry — the armed rows AFTER
/// rev threading, in arm order, which is byte-for-byte the value of the
/// request's `plan_edits` field. Hashing anything else (the pre-threading rows,
/// a re-sorted list) would answer a question nobody asked.
///
/// The canonical form is compact JSON with object keys sorted lexicographically
/// by UTF-8 byte order. Both properties come from `serde_json` itself rather
/// than from hand-written emission: `to_value` yields `Value::Object`, which is
/// a `BTreeMap` while the `preserve_order` feature is off, and `to_string` on a
/// `Value` writes compact JSON with RFC 8259-minimal escaping. The feature being
/// off is load-bearing and cargo features are additive across a workspace, so
/// [`sorted_keys_are_the_canonical_form`] pins it — without that test a
/// transitive dependency could enable `preserve_order` and silently redefine
/// every digest this function has ever produced.
///
/// It is generic over the element only so that a caller holding `&[&PlanEdit]`
/// — which is what the commit builds its request field from — can hash the
/// **same value it sends** rather than a re-collected copy. `PlanEdit` and
/// `&PlanEdit` serialize identically, so this widens no contract.
///
/// # Panics
/// Never in practice: `PlanEdit` is a plain derived-`Serialize` enum of
/// strings, bools, `Option`s and `Vec`s, so it has no map with non-string keys
/// and no non-finite float — the only two shapes `to_value` can reject.
#[must_use]
pub fn armed_digest<E: Serialize>(edits: &[E]) -> String {
    let canonical = serde_json::to_value(edits)
        .and_then(|value| serde_json::to_string(&value))
        .expect("PlanEdit is a derived-Serialize shape with no map key or float that can fail");
    let mut out = String::with_capacity("sha256:".len() + 64);
    out.push_str("sha256:");
    for byte in Sha256::digest(canonical.as_bytes()) {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::{HpathSeg, PlanEdit};

    fn seg(h: &str) -> HpathSeg {
        HpathSeg {
            h: h.to_owned(),
            n: None,
        }
    }

    fn append(body: &str) -> PlanEdit {
        PlanEdit::Append {
            hpath: vec![seg("Notes")],
            body: body.to_owned(),
            rev: Some("b3:beef".to_owned()),
        }
    }

    /// The shape of the answer, so a caller can assert on it without re-deriving
    /// the format from this file.
    #[test]
    fn the_digest_is_prefixed_lowercase_hex() {
        let digest = armed_digest(&[append("hi")]);
        let hex = digest.strip_prefix("sha256:").expect("the prefix");
        assert_eq!(hex.len(), 64, "{digest}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "lowercase hex only: {digest}"
        );
    }

    /// The property the whole flag rests on: a different armed set is a
    /// different digest. Body, address, rev and ORDER each move it.
    #[test]
    fn a_different_armed_set_is_a_different_digest() {
        let base = armed_digest(&[append("hi")]);
        assert_ne!(base, armed_digest(&[append("bye")]), "body");
        assert_ne!(
            base,
            armed_digest(&[PlanEdit::Append {
                hpath: vec![seg("Other")],
                body: "hi".to_owned(),
                rev: Some("b3:beef".to_owned()),
            }]),
            "address"
        );
        assert_ne!(
            base,
            armed_digest(&[PlanEdit::Append {
                hpath: vec![seg("Notes")],
                body: "hi".to_owned(),
                rev: Some("b3:cafe".to_owned()),
            }]),
            "rev — the threaded CAS token is part of what was armed"
        );
        let pair = [append("one"), append("two")];
        let flipped = [append("two"), append("one")];
        assert_ne!(
            armed_digest(&pair),
            armed_digest(&flipped),
            "arm order is part of the armed set, not an incidental listing order"
        );
    }

    /// Same input, same answer — the property that makes comparing two children
    /// meaningful at all.
    #[test]
    fn the_digest_is_stable_across_calls() {
        assert_eq!(armed_digest(&[append("hi")]), armed_digest(&[append("hi")]));
    }

    /// The empty set still has a digest, and it is the digest of `[]`. The entry
    /// never commits a zero-armed run, so this value is never compared in
    /// practice — it is pinned so that a future caller which does reach it meets
    /// a defined answer rather than a special case someone has to invent.
    #[test]
    fn the_empty_armed_set_hashes_the_empty_array() {
        let expected = {
            let mut out = String::from("sha256:");
            for byte in Sha256::digest(b"[]") {
                use std::fmt::Write as _;
                write!(out, "{byte:02x}").expect("string");
            }
            out
        };
        let none: [PlanEdit; 0] = [];
        assert_eq!(armed_digest(&none), expected);
    }

    /// **The canonicalization itself, pinned against a feature flag.**
    ///
    /// `serde_json::Map` is a `BTreeMap` only while `preserve_order` is off.
    /// Cargo features are additive across a workspace, so any crate anywhere in
    /// the graph enabling it would flip every object in the digest's input to
    /// insertion order and silently redefine the digest. This asserts the
    /// canonical bytes directly: `body` before `hpath` before `rev` is
    /// lexicographic, not declaration order (which is `hpath`, `body`, `rev`).
    #[test]
    fn sorted_keys_are_the_canonical_form() {
        let canonical =
            serde_json::to_string(&serde_json::to_value([append("hi")]).expect("value"))
                .expect("string");
        assert_eq!(
            canonical, r#"[{"append":{"body":"hi","hpath":[{"h":"Notes"}],"rev":"b3:beef"}}]"#,
            "keys must be lexicographic, not PlanEdit's declaration order — if this \
             fails, something in the graph enabled serde_json/preserve_order and every \
             armed digest just changed meaning"
        );
    }

    /// **The published test vector**, pinned here so the doc amendment and this
    /// function cannot drift apart. `docs/run-plane.md` prints these exact bytes
    /// and this exact digest; a host verifying independently checks itself
    /// against them before trusting its own implementation.
    ///
    /// The fixture is chosen adversarially rather than for readability: it mixes
    /// both edit shapes, exercises key ordering that differs from declaration
    /// order in both of them, and puts `<`, `>`, `&` and a newline in a body —
    /// the characters that make a Go implementation diverge while every ASCII
    /// fixture still passes.
    #[test]
    fn the_published_test_vector_holds() {
        let edits = vec![
            PlanEdit::SetProperty {
                key: "owner".to_owned(),
                value: "8ab41c02".to_owned(),
                rev: Some("7c40e1a8b2f9d356".to_owned()),
            },
            PlanEdit::Append {
                hpath: vec![seg("Goals")],
                body: "a <b> & c\n".to_owned(),
                rev: Some("a6665baff294bd04".to_owned()),
            },
        ];
        let canonical =
            serde_json::to_string(&serde_json::to_value(&edits).expect("value")).expect("string");
        assert_eq!(
            canonical,
            r#"[{"set_property":{"key":"owner","rev":"7c40e1a8b2f9d356","value":"8ab41c02"}},{"append":{"body":"a <b> & c\n","hpath":[{"h":"Goals"}],"rev":"a6665baff294bd04"}}]"#
        );
        assert_eq!(
            armed_digest(&edits),
            "sha256:fdbad8387ab2097175de9cf02217b202177e0aa8dcf80730f1e97434e3d5ac1d"
        );
    }

    /// The escaping half of the published serialization. A Go host marshalling
    /// the same value escapes `<`, `>` and `&` unless it disables
    /// `SetEscapeHTML`, and escapes U+2028/U+2029 unconditionally; the engine
    /// does neither. These are the exact characters that make an independent
    /// host implementation refuse on ordinary markdown while passing every
    /// ASCII fixture, so the engine's side of that contract is pinned here.
    #[test]
    fn the_canonical_form_does_not_html_escape_or_escape_separators() {
        let canonical = serde_json::to_string(
            &serde_json::to_value([append("a <b> & c\u{2028}d")]).expect("value"),
        )
        .expect("string");
        assert!(
            canonical.contains("a <b> & c"),
            "raw, unescaped: {canonical}"
        );
        assert!(
            canonical.contains('\u{2028}'),
            "U+2028 rides as raw UTF-8, never \\u2028: {canonical:?}"
        );
    }
}
