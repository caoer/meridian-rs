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
//!
//! **The domain is the armed SET, not the armed PAYLOADS.** A `PlanEdit` carries
//! no path — the target rides `splice.path` — so hashing the plan rows alone
//! made two sets writing identical edits to DIFFERENT files hash identically.
//! That is exactly the dimension the arm/commit gap turns on: the host gates
//! rows for one file and a commit child that resolved somewhere else produces a
//! matching digest. Each row therefore hashes as a `(path, edit)` pair, and the
//! digest is a function of where the bytes land as well as what they are.
//!
//! **The digest NAMES its domain** ([`DOMAIN_TAG`]), and that is a deployment
//! organ rather than decoration. A host's guard cannot tell a narrow digest from
//! a wide one by looking at it — both are well-formed, and an engine hashing
//! payloads only publishes a perfectly ordinary value. So on a tree running an
//! older engine both children agree, the guard passes, and the class claim the
//! host's header makes degrades silently: a claim true only on a pinned tree.
//! Host/engine skew is measured, not theoretical — a resident daemon ran a stale
//! engine for hours on 2026-08-06. The tag turns that into a refusal a host can
//! reach with a **literal string comparison and no parsing whatsoever**, which
//! is why it is a capability assertion and not a second canonicalization: the
//! courier still copies one opaque string and computes nothing.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ArmedEdit;
use wire::PlanEdit;

/// The digest's domain tag — the literal prefix every value this function
/// produces carries, and the whole of what a host may inspect.
///
/// It names the DOMAIN rather than a version number on purpose. A version
/// constant tells a reader that something changed; this tells them what is
/// covered — the armed set as `(path, edit)` pairs — which is the fact a host's
/// refusal has to explain to whoever hits it.
///
/// An engine predating the path dimension publishes a bare `sha256:…`, so a
/// host asserting this prefix refuses that engine BY NAME instead of guarding
/// against a domain it cannot see. Widening the domain again means a new tag:
/// the value stops matching, and every host still asserting the old one refuses
/// loudly rather than gating the wrong thing quietly.
pub const DOMAIN_TAG: &str = "armed-set-path-edit:";

/// One row of the digest's domain: the content path the edit lands on, and the
/// wire shape that lands there.
///
/// It borrows rather than owns so the value hashed is the value the commit
/// sends — a re-collected copy is a second thing that can drift. The trace
/// facts an [`ArmedEdit`] also carries (`line`, `depth`) are deliberately
/// outside the domain: they describe where in the SOURCE a row was armed, which
/// is not part of what a host authorizes.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ArmedRow<'a> {
    /// The wire shape, carried verbatim into `splice.plan_edits[]`.
    pub edit: &'a PlanEdit,
    /// The content path this edit writes — `splice.path`, which no `PlanEdit`
    /// carries and without which the digest cannot tell two files apart.
    pub path: &'a str,
}

impl<'a> ArmedRow<'a> {
    /// The digest row of one armed edit.
    #[must_use]
    pub fn of(armed: &'a ArmedEdit) -> Self {
        Self {
            edit: &armed.edit,
            path: &armed.path,
        }
    }

    /// The digest rows of a whole armed list, in arm order.
    #[must_use]
    pub fn of_all(armed: &'a [ArmedEdit]) -> Vec<Self> {
        armed.iter().map(Self::of).collect()
    }
}

/// The digest of an armed set: [`DOMAIN_TAG`], then `sha256:`, then 64
/// lowercase hex digits.
///
/// `rows` is the armed set the commit splice would carry — the rows AFTER rev
/// threading, in arm order, each paired with the path it targets. The `edit`
/// halves are byte-for-byte the value of the request's `plan_edits` field and
/// the `path` halves are the request's `path`. Hashing anything else (the
/// pre-threading rows, a re-sorted list, the payloads without their targets)
/// would answer a question nobody asked.
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
/// # Panics
/// Never in practice: [`ArmedRow`] is a two-field struct of a `&str` and a
/// derived-`Serialize` enum of strings, bools, `Option`s and `Vec`s, so it has
/// no map with non-string keys and no non-finite float — the only two shapes
/// `to_value` can reject.
#[must_use]
pub fn armed_digest(rows: &[ArmedRow<'_>]) -> String {
    let canonical = serde_json::to_value(rows)
        .and_then(|value| serde_json::to_string(&value))
        .expect("ArmedRow is a derived-Serialize shape with no map key or float that can fail");
    let mut out = String::with_capacity(DOMAIN_TAG.len() + "sha256:".len() + 64);
    out.push_str(DOMAIN_TAG);
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
    use wire::HpathSeg;

    const CARD: &str = "cards/one.md";
    const OTHER: &str = "cards/two.md";

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

    /// One digest row, written the way a caller reads it: this edit, that file.
    fn row<'a>(path: &'a str, edit: &'a PlanEdit) -> ArmedRow<'a> {
        ArmedRow { edit, path }
    }

    fn canon(rows: &[ArmedRow<'_>]) -> String {
        serde_json::to_string(&serde_json::to_value(rows).expect("value")).expect("string")
    }

    /// The UNTAGGED spelling — what an engine predating the domain tag
    /// published, reproduced locally so the tests can compare against it.
    fn bare_sha256(bytes: &[u8]) -> String {
        let mut out = String::from("sha256:");
        for byte in Sha256::digest(bytes) {
            use std::fmt::Write as _;
            write!(out, "{byte:02x}").expect("string");
        }
        out
    }

    /// The shape of the answer, so a caller can assert on it without re-deriving
    /// the format from this file.
    #[test]
    fn the_digest_is_domain_tagged_prefixed_lowercase_hex() {
        let edit = append("hi");
        let digest = armed_digest(&[row(CARD, &edit)]);
        let hex = digest
            .strip_prefix(DOMAIN_TAG)
            .and_then(|rest| rest.strip_prefix("sha256:"))
            .expect("the domain tag, then the hash prefix");
        assert_eq!(hex.len(), 64, "{digest}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "lowercase hex only: {digest}"
        );
    }

    /// **The tag is a literal a host can assert with no parsing** — the whole
    /// property that lets it be a capability check rather than a second
    /// canonicalization. Pinned as bytes, because a host asserts these bytes and
    /// the two spellings must not drift.
    #[test]
    fn the_domain_tag_is_the_published_literal() {
        assert_eq!(DOMAIN_TAG, "armed-set-path-edit:");
        let edit = append("hi");
        assert!(armed_digest(&[row(CARD, &edit)]).starts_with("armed-set-path-edit:sha256:"));
        assert_eq!(
            armed_digest(&[]),
            format!("{DOMAIN_TAG}{}", bare_sha256(b"[]")),
            "the tag rides every value this function produces, including the empty set — \
             a host that met one untagged answer would have to special-case it"
        );
    }

    /// **The capability layer's REFUSE arm, engine-side.** An engine predating
    /// the path dimension publishes a bare `sha256:…`; a host asserting the tag
    /// refuses it by name. This pins that the two spellings really are
    /// distinguishable by prefix alone — if a future tag were ever empty or
    /// began with `sha256:`, the host's check would silently admit the narrow
    /// engine again, which is the exact skew this tag exists to catch.
    #[test]
    fn an_untagged_digest_is_distinguishable_from_this_engines() {
        let old_engine = bare_sha256(b"[]");
        assert!(
            !old_engine.starts_with(DOMAIN_TAG),
            "the pre-tag spelling must fail a host's literal tag assertion: {old_engine}"
        );
        assert!(
            armed_digest(&[]).starts_with(DOMAIN_TAG),
            "and this engine's must pass it"
        );
    }

    /// The property the whole flag rests on: a different armed set is a
    /// different digest. Body, address, rev and ORDER each move it.
    #[test]
    fn a_different_armed_set_is_a_different_digest() {
        let hi = append("hi");
        let base = armed_digest(&[row(CARD, &hi)]);

        let bye = append("bye");
        assert_ne!(base, armed_digest(&[row(CARD, &bye)]), "body");

        let elsewhere = PlanEdit::Append {
            hpath: vec![seg("Other")],
            body: "hi".to_owned(),
            rev: Some("b3:beef".to_owned()),
        };
        assert_ne!(base, armed_digest(&[row(CARD, &elsewhere)]), "address");

        let restaked = PlanEdit::Append {
            hpath: vec![seg("Notes")],
            body: "hi".to_owned(),
            rev: Some("b3:cafe".to_owned()),
        };
        assert_ne!(
            base,
            armed_digest(&[row(CARD, &restaked)]),
            "rev — the threaded CAS token is part of what was armed"
        );

        let one = append("one");
        let two = append("two");
        assert_ne!(
            armed_digest(&[row(CARD, &one), row(CARD, &two)]),
            armed_digest(&[row(CARD, &two), row(CARD, &one)]),
            "arm order is part of the armed set, not an incidental listing order"
        );
    }

    /// **P3-1, pinned — the test that would have caught it.**
    ///
    /// Two armed sets with BYTE-IDENTICAL edit payloads, aimed at two different
    /// files. Before the domain covered the path this was one digest, so a host
    /// could gate rows for `cards/one.md` and a commit child that resolved to
    /// `cards/two.md` matched the pin and spliced into a file nobody authorized.
    /// That is the dimension R4 (a symlink re-pointed between the legs) and R5
    /// (a pin over the hash domain while the write plane resolves through the
    /// larger addressable set) both turn on.
    ///
    /// The payload equality is asserted, not assumed: without it this test could
    /// pass because the two sets differed in some OTHER field, which is the
    /// vacuous-pass shape the whole feature exists to refuse.
    #[test]
    fn the_same_edits_to_different_paths_are_different_digests() {
        let edit = append("hi");
        let here = [row(CARD, &edit)];
        let there = [row(OTHER, &edit)];

        assert_eq!(
            serde_json::to_value(here[0].edit).expect("value"),
            serde_json::to_value(there[0].edit).expect("value"),
            "the two sets differ in the TARGET and in nothing else — if this fails the \
             assertion below proves nothing"
        );
        assert_ne!(
            armed_digest(&here),
            armed_digest(&there),
            "the digest is a function of the armed SET, so the file an edit lands on \
             moves it; a payload-only digest collides here and re-opens R4 and R5"
        );
    }

    /// **The negative proof of the negative proof.** With the path taken back
    /// out of the domain — which is what a refactor "simplifying" [`ArmedRow`]
    /// away would do — the different-paths case above collides. This reproduces
    /// the pre-fix digest locally and asserts the collision, so the reader can
    /// see the defect rather than take its description on trust.
    #[test]
    fn without_the_path_the_different_paths_case_collides() {
        let edit = append("hi");
        // The pre-fix domain: the payloads alone, exactly `edits_of`'s old value.
        let payloads_only = |rows: &[ArmedRow<'_>]| {
            let edits: Vec<&PlanEdit> = rows.iter().map(|r| r.edit).collect();
            serde_json::to_string(&serde_json::to_value(&edits).expect("value")).expect("string")
        };
        let here = [row(CARD, &edit)];
        let there = [row(OTHER, &edit)];
        assert_eq!(
            payloads_only(&here),
            payloads_only(&there),
            "this is the defect: the old domain cannot tell the two targets apart"
        );
        assert_ne!(canon(&here), canon(&there), "and this is the fix");
    }

    /// **The ADMIT direction.** Extending the domain must not break the ordinary
    /// commit: the same edits against the SAME target still hash equal, so a
    /// host that gated an arm and forwards its digest still commits. A gate that
    /// only proves refusals cannot tell "correctly strict" from "entirely dead".
    #[test]
    fn the_same_edits_to_the_same_path_are_the_same_digest() {
        let armed = |body: &str| {
            let one = PlanEdit::Append {
                hpath: vec![seg("Notes")],
                body: body.to_owned(),
                rev: Some("b3:beef".to_owned()),
            };
            let two = PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "done".to_owned(),
                rev: Some("b3:cafe".to_owned()),
            };
            (one, two)
        };
        let (a1, a2) = armed("hi");
        let (b1, b2) = armed("hi");
        assert_eq!(
            armed_digest(&[row(CARD, &a1), row(CARD, &a2)]),
            armed_digest(&[row(CARD, &b1), row(CARD, &b2)]),
            "two independent evaluations of one armed set agree — the courier property"
        );
    }

    /// Same input, same answer — the property that makes comparing two children
    /// meaningful at all.
    #[test]
    fn the_digest_is_stable_across_calls() {
        let edit = append("hi");
        assert_eq!(
            armed_digest(&[row(CARD, &edit)]),
            armed_digest(&[row(CARD, &edit)])
        );
    }

    /// **Every field of every shape moves the digest**, `HpathSeg.n` first: the
    /// amendment names it as the trap an independent implementation gets wrong,
    /// and until this test no vector carried it at all. `SetProperty`'s `key`
    /// and `value` were likewise unpinned — a canonicalization that dropped
    /// either would have gone unnoticed by every other test in this file.
    #[test]
    fn every_field_moves_the_digest() {
        let numbered = |n: Option<u32>| PlanEdit::Append {
            hpath: vec![HpathSeg {
                h: "Notes".to_owned(),
                n,
            }],
            body: "hi".to_owned(),
            rev: Some("b3:beef".to_owned()),
        };
        let bare = numbered(None);
        let second = numbered(Some(2));
        let third = numbered(Some(3));
        assert_ne!(
            armed_digest(&[row(CARD, &bare)]),
            armed_digest(&[row(CARD, &second)]),
            "HpathSeg.n present vs absent — the disambiguating ordinal of a repeated heading"
        );
        assert_ne!(
            armed_digest(&[row(CARD, &second)]),
            armed_digest(&[row(CARD, &third)]),
            "HpathSeg.n's VALUE — two different headings, and a digest blind to it would \
             let a commit child land in the wrong one"
        );

        let prop = |key: &str, value: &str| PlanEdit::SetProperty {
            key: key.to_owned(),
            value: value.to_owned(),
            rev: Some("b3:beef".to_owned()),
        };
        let owner = prop("owner", "8ab41c02");
        assert_ne!(
            armed_digest(&[row(CARD, &owner)]),
            armed_digest(&[row(CARD, &prop("status", "8ab41c02"))]),
            "SetProperty.key"
        );
        assert_ne!(
            armed_digest(&[row(CARD, &owner)]),
            armed_digest(&[row(CARD, &prop("owner", "deadbeef"))]),
            "SetProperty.value"
        );
    }

    /// The empty set still has a digest, and it is the digest of `[]`. The entry
    /// never commits a zero-armed run, so this value is never compared in
    /// practice — it is pinned so that a future caller which does reach it meets
    /// a defined answer rather than a special case someone has to invent.
    #[test]
    fn the_empty_armed_set_hashes_the_empty_array() {
        assert_eq!(
            armed_digest(&[]),
            format!("{DOMAIN_TAG}{}", bare_sha256(b"[]"))
        );
    }

    /// **The canonicalization itself, pinned against a feature flag.**
    ///
    /// `serde_json::Map` is a `BTreeMap` only while `preserve_order` is off.
    /// Cargo features are additive across a workspace, so any crate anywhere in
    /// the graph enabling it would flip every object in the digest's input to
    /// insertion order and silently redefine the digest. This asserts the
    /// canonical bytes directly, at both levels: `edit` before `path` is
    /// lexicographic and not the row's declaration order either way, and inside
    /// the edit `body` before `hpath` before `rev` is lexicographic rather than
    /// `PlanEdit`'s declaration order (`hpath`, `body`, `rev`).
    #[test]
    fn sorted_keys_are_the_canonical_form() {
        let edit = append("hi");
        assert_eq!(
            canon(&[row(CARD, &edit)]),
            r#"[{"edit":{"append":{"body":"hi","hpath":[{"h":"Notes"}],"rev":"b3:beef"}},"path":"cards/one.md"}]"#,
            "keys must be lexicographic, not declaration order — if this fails, something \
             in the graph enabled serde_json/preserve_order and every armed digest just \
             changed meaning"
        );
    }

    /// **The published test vector**, pinned here so the doc amendment and this
    /// function cannot drift apart. `docs/run-plane.md` prints these exact bytes
    /// and this exact digest; a host verifying independently checks itself
    /// against them before trusting its own implementation.
    ///
    /// The fixture is chosen adversarially rather than for readability. It mixes
    /// both edit shapes; it exercises key ordering that differs from declaration
    /// order at both levels; it carries `HpathSeg.n`, the one field the
    /// amendment names as a trap and which no published vector used to reach;
    /// and it puts `<`, `>`, `&` and a newline in a body — the characters that
    /// make a Go implementation diverge while every ASCII fixture still passes.
    ///
    /// The two rows target DIFFERENT paths, which the armed law does not permit
    /// in one commit. That is deliberate: the vector's job is to pin the
    /// serialization of the domain, and a vector whose rows shared a path would
    /// let an implementation that hashes the path ONCE for the set reproduce it
    /// and then diverge on nothing this file could catch.
    #[test]
    fn the_published_test_vector_holds() {
        let owner = PlanEdit::SetProperty {
            key: "owner".to_owned(),
            value: "8ab41c02".to_owned(),
            rev: Some("7c40e1a8b2f9d356".to_owned()),
        };
        let goals = PlanEdit::Append {
            hpath: vec![HpathSeg {
                h: "Goals".to_owned(),
                n: Some(2),
            }],
            body: "a <b> & c\n".to_owned(),
            rev: Some("a6665baff294bd04".to_owned()),
        };
        let rows = [row(CARD, &owner), row(OTHER, &goals)];
        assert_eq!(
            canon(&rows),
            r#"[{"edit":{"set_property":{"key":"owner","rev":"7c40e1a8b2f9d356","value":"8ab41c02"}},"path":"cards/one.md"},{"edit":{"append":{"body":"a <b> & c\n","hpath":[{"h":"Goals","n":2}],"rev":"a6665baff294bd04"}},"path":"cards/two.md"}]"#
        );
        assert_eq!(
            armed_digest(&rows),
            "armed-set-path-edit:sha256:37c4d09eb84d1e902b887a0b13cc90f67d5888e0bd5ebf9148ac0031ccdcde4a"
        );
    }

    /// The escaping half of the published serialization. A Go host marshalling
    /// the same value escapes `<`, `>` and `&` unless it disables
    /// `SetEscapeHTML`, and escapes U+2028/U+2029 unconditionally; the engine
    /// does neither. These are the exact characters that make an independent
    /// host implementation refuse on ordinary markdown while passing every
    /// ASCII fixture, so the engine's side of that contract is pinned here.
    ///
    /// The path half is checked too: a target is as much a string a host must
    /// reproduce byte-for-byte as a body is, and nothing stops a filename from
    /// carrying either character class.
    #[test]
    fn the_canonical_form_does_not_html_escape_or_escape_separators() {
        let edit = append("a <b> & c\u{2028}d");
        let canonical = canon(&[row("cards/a <b> & c.md", &edit)]);
        assert!(
            canonical.contains("a <b> & c"),
            "raw, unescaped: {canonical}"
        );
        assert!(
            canonical.contains("cards/a <b> & c.md"),
            "the path is raw too: {canonical}"
        );
        assert!(
            canonical.contains('\u{2028}'),
            "U+2028 rides as raw UTF-8, never \\u2028: {canonical:?}"
        );
    }
}
