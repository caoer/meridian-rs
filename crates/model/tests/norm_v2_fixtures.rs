//! U-SPEC golden fixtures for `docs/norm-v2-spec.md` (fingerprint hash domain;
//! decision 2026-07-24-fingerprint-cid-representation).
//!
//! `reference` = spec-verbatim norm-v2 (§4) + token (§2). Production
//! (`syntax::{anchor_removals,norm_v2_slice,norm_v2}` + `model::fingerprint`)
//! asserted against reference and golden — three spellings of one law. Golden
//! data is the contract (move = new codec prefix, not an edit).

use model::{Node, NodeKind, build};

/// Spec-verbatim reference implementation (norm-v2-spec §2, §4).
mod reference {
    use std::ops::Range;

    /// §4.1 — anchor identification: the syntax lexer over the WHOLE file is
    /// the one normative grammar; norm-v2 consumes the marker spans verbatim.
    pub(crate) fn anchor_markers(input: &str) -> Vec<Range<usize>> {
        syntax::parse(input)
            .into_iter()
            .filter(|n| matches!(n.kind, syntax::DialectKind::Anchor { .. }))
            .map(|n| n.span)
            .collect()
    }

    /// §4.2 — removal ranges (file coordinates) for every marker: R1 tail
    /// anchors, R2 own-line anchors, R2b own-line at end-of-input; overlaps
    /// merge by union.
    pub(crate) fn removals(file: &[u8], markers: &[Range<usize>]) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for m in markers {
            let line_start = file[..m.start]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |p| p + 1);
            let own_line = file[line_start..m.start]
                .iter()
                .all(|&b| b == b' ' || b == b'\t');
            if own_line {
                if let Some(p) = file[m.end..].iter().position(|&b| b == b'\n') {
                    // R2: the whole line through its terminator (a CRLF's `\r`
                    // sits inside the removed line).
                    out.push(line_start..m.end + p + 1);
                } else {
                    // R2b: last line, no terminator — take the PRECEDING
                    // line's terminator instead, so terminator-exclusive
                    // slices stay neutral under own-line promotion at EOF.
                    let mut t = line_start;
                    if t >= 1 && file[t - 1] == b'\n' {
                        t -= 1;
                        if t >= 1 && file[t - 1] == b'\r' {
                            t -= 1;
                        }
                    }
                    out.push(t..file.len());
                }
            } else {
                // R1: the marker plus exactly ONE preceding space/tab — the
                // exact inverse of what promotion inserts (#6 §2). The lexer
                // guarantees the preceding byte is a space or tab here.
                out.push(m.start - 1..m.end);
            }
        }
        merge(out)
    }

    fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
        ranges.sort_by_key(|r| r.start);
        let mut out: Vec<Range<usize>> = Vec::new();
        for r in ranges {
            match out.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => out.push(r),
            }
        }
        out
    }

    /// §4.3 — apply file-level removals to a node span: emit the span's bytes
    /// minus every removal∩span. Slicing never re-classifies anchors.
    pub(crate) fn norm_v2_slice(
        file: &[u8],
        span: &Range<usize>,
        removals: &[Range<usize>],
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(span.len());
        let mut pos = span.start;
        for r in removals {
            let s = r.start.max(span.start);
            let e = r.end.min(span.end);
            if s >= e {
                continue;
            }
            if pos < s {
                out.extend_from_slice(&file[pos..s]);
            }
            pos = pos.max(e);
        }
        if pos < span.end {
            out.extend_from_slice(&file[pos..span.end]);
        }
        out
    }

    /// Whole-input canonicalization: the document grain of §4.3.
    pub(crate) fn norm_v2(input: &str) -> Vec<u8> {
        let markers = anchor_markers(input);
        let removals = removals(input.as_bytes(), &markers);
        norm_v2_slice(input.as_bytes(), &(0..input.len()), &removals)
    }

    /// §2.1/§2.2 — mint the M1-live fingerprint token over canonical bytes:
    /// `fp1.span2.b3.<64hex>`.
    pub(crate) fn fingerprint(canonical: &[u8]) -> String {
        format!("fp1.span2.b3.{}", blake3::hash(canonical).to_hex())
    }

    /// §2.4 parse — grammar-only, codec-agnostic. `None` = malformed. Digest
    /// LENGTH is validated only when the hashfn is known (`b3` → 64).
    pub(crate) fn parse_token(s: &str) -> Option<(String, String, String, String)> {
        let mut it = s.split('.');
        let (Some(version), Some(codec), Some(hashfn), Some(digest), None) =
            (it.next(), it.next(), it.next(), it.next(), it.next())
        else {
            return None;
        };
        let field_ok = |f: &str| {
            !f.is_empty()
                && f.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        };
        let hex_ok =
            |f: &str| !f.is_empty() && f.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !(field_ok(version) && field_ok(codec) && field_ok(hashfn) && hex_ok(digest)) {
            return None;
        }
        if hashfn == "b3" && digest.len() != 64 {
            return None;
        }
        Some((
            version.to_string(),
            codec.to_string(),
            hashfn.to_string(),
            digest.to_string(),
        ))
    }
}

// Golden canonicalization table (spec §7): (name, input, expected canonical)

const CANONICAL_GOLDENS: &[(&str, &str, &str)] = &[
    ("tail_anchor", "para one ^goal\n", "para one\n"),
    (
        "own_line_mid",
        "|a|\n|b|\n^tbl\n\nafter\n",
        "|a|\n|b|\n\nafter\n",
    ),
    ("own_line_eof_terminated", "|a|\n|b|\n^tbl\n", "|a|\n|b|\n"),
    ("own_line_eof_unterminated", "|a|\n|b|\n^tbl", "|a|\n|b|"),
    ("mid_line_caret_kept", "a ^b c\n", "a ^b c\n"),
    (
        "fenced_code_kept",
        "```\ncode ^notanchor\n```\n",
        "```\ncode ^notanchor\n```\n",
    ),
    (
        "crlf_tail",
        "line one ^crlf-anchor\r\nplain\r\n",
        "line one\r\nplain\r\n",
    ),
    ("unicode_id_kept", "uni ^ünïcode\n", "uni ^ünïcode\n"),
    ("underscore_id_kept", "dropped ^b_1\n", "dropped ^b_1\n"),
    (
        "heading_anchor",
        "## Target ^h-anchor\nbody\n",
        "## Target\nbody\n",
    ),
    ("two_spaces_before_anchor", "text  ^id\n", "text \n"),
    ("tab_before_anchor", "text\t^id\n", "text\n"),
    ("trailing_ws_after_id", "text ^id  \n", "text  \n"),
    (
        "frontmatter_caret",
        "---\ntitle: x ^fm\n---\nbody\n",
        "---\ntitle: x\n---\nbody\n",
    ),
    ("anchor_only_doc", "^solo\n", ""),
    ("empty_doc", "", ""),
    (
        "inline_code_then_tail_anchor",
        "see `x` ^real\n",
        "see `x`\n",
    ),
    ("consecutive_own_line_eof", "^a\n^b", ""),
];

/// The §5 neutrality pair: X0 = pre-promotion, X1 = X0 with one tail anchor
/// promoted onto the intro paragraph. canonical(X1) == canonical(X0) == X0.
const X0: &str = "# A\nintro\n\n# B\nbody\n";
const X1: &str = "# A\nintro ^goal\n\n# B\nbody\n";

/// Pinned digest/token literals (spec §7 golden tokens; regenerate via
/// `print_goldens`). `GOLD_X0_DIGEST` is the digest of `X0`'s canonical bytes
/// — and therefore of `X1`'s.
const GOLD_X0_DIGEST: &str = "40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152";
const GOLD_PARA_DIGEST: &str = "1f9ac8f88eebd39e9d6a8466f104317be67d923867a804b85a678ae47de63eb2";

fn walk_find<'a>(n: &'a Node, pred: &impl Fn(&Node) -> bool) -> Option<&'a Node> {
    if pred(n) {
        return Some(n);
    }
    n.children.iter().find_map(|c| walk_find(c, pred))
}

fn section<'a>(root: &'a Node, name: &str) -> &'a Node {
    walk_find(
        root,
        &|n| matches!(&n.kind, NodeKind::Section { heading_text, .. } if heading_text == name),
    )
    .unwrap_or_else(|| panic!("no section {name}"))
}

fn doc(raw: &str) -> model::Document {
    build(raw.to_string(), syntax::parse(raw))
}

#[test]
fn canonical_goldens_hold() {
    for (name, input, expected) in CANONICAL_GOLDENS {
        let got = reference::norm_v2(input);
        assert_eq!(
            got.as_slice(),
            expected.as_bytes(),
            "fixture `{name}`: canonical bytes diverge\n got: {:?}\nwant: {expected:?}",
            String::from_utf8_lossy(&got),
        );
        assert_eq!(
            syntax::norm_v2(input).as_slice(),
            expected.as_bytes(),
            "fixture `{name}`: PRODUCTION norm_v2 diverges from the golden"
        );
    }
}

/// §5.1 — promotion never moves the fingerprint, at document grain.
/// §5.2 — the same promotion MUST move `node_rev` (CAS sees real bytes).
#[test]
fn neutrality_document_grain() {
    assert_eq!(reference::norm_v2(X0).as_slice(), X0.as_bytes());
    assert_eq!(reference::norm_v2(X1).as_slice(), X0.as_bytes());

    let fp0 = reference::fingerprint(&reference::norm_v2(X0));
    let fp1 = reference::fingerprint(&reference::norm_v2(X1));
    assert_eq!(fp0, fp1, "promotion caused false drift at document grain");
    assert_eq!(fp0, format!("fp1.span2.b3.{GOLD_X0_DIGEST}"));

    let (d0, d1) = (doc(X0), doc(X1));
    assert_ne!(
        d0.root.node_rev, d1.root.node_rev,
        "CAS must see the promotion byte-change at document grain"
    );

    // PRODUCTION mint agrees with reference and golden.
    // `.expect` is the non-vacuity guard, not ceremony: the mint returns a
    // Result now, and two `Err`s compare EQUAL — an undischarged neutrality
    // assert would pass on two documents that both fingerprint nothing.
    let pf0 = model::fingerprint::fingerprint(&d0, &d0.root).expect("X0 root has content");
    let pf1 = model::fingerprint::fingerprint(&d1, &d1.root).expect("X1 root has content");
    assert_eq!(pf0, pf1, "production: false drift at document grain");
    assert_eq!(pf0.as_str(), fp0, "production token != reference token");
}

/// §5 at section grain: the promoted paragraph lives in section A; section A's
/// fingerprint holds, its `node_rev` moves, and untouched section B is inert on
/// BOTH planes.
#[test]
fn neutrality_section_grain() {
    let (d0, d1) = (doc(X0), doc(X1));
    let (a0, a1) = (section(&d0.root, "A"), section(&d1.root, "A"));

    let rem0 = reference::removals(X0.as_bytes(), &reference::anchor_markers(X0));
    let rem1 = reference::removals(X1.as_bytes(), &reference::anchor_markers(X1));
    let c0 = reference::norm_v2_slice(X0.as_bytes(), &a0.span, &rem0);
    let c1 = reference::norm_v2_slice(X1.as_bytes(), &a1.span, &rem1);
    assert_eq!(c0, c1, "promotion caused false drift at section grain");
    assert_eq!(reference::fingerprint(&c0), reference::fingerprint(&c1));
    assert_ne!(a0.node_rev, a1.node_rev, "CAS must move at section grain");

    // PRODUCTION section-grain mint agrees with reference.
    let (p0, p1) = (
        model::fingerprint::fingerprint(&d0, a0).expect("section A of X0 has content"),
        model::fingerprint::fingerprint(&d1, a1).expect("section A of X1 has content"),
    );
    assert_eq!(p0, p1, "production: false drift at section grain");
    assert_eq!(p0.as_str(), reference::fingerprint(&c0));

    let (b0, b1) = (section(&d0.root, "B"), section(&d1.root, "B"));
    assert_eq!(b0.node_rev, b1.node_rev, "untouched section B moved");
}

/// §4.2 R2b at section + document grain: own-line promotion at EOF of an
/// unterminated file is neutral because the preceding terminator is consumed.
#[test]
fn neutrality_own_line_at_eof() {
    let pre = "# A\n|x|\n|---|\n|1|";
    let post = "# A\n|x|\n|---|\n|1|\n^tbl";
    assert_eq!(reference::norm_v2(post).as_slice(), pre.as_bytes());

    let (dpre, dpost) = (doc(pre), doc(post));
    let (apre, apost) = (section(&dpre.root, "A"), section(&dpost.root, "A"));
    let rpre = reference::removals(pre.as_bytes(), &reference::anchor_markers(pre));
    let rpost = reference::removals(post.as_bytes(), &reference::anchor_markers(post));
    let cpre = reference::norm_v2_slice(pre.as_bytes(), &apre.span, &rpre);
    let cpost = reference::norm_v2_slice(post.as_bytes(), &apost.span, &rpost);
    assert_eq!(
        reference::fingerprint(&cpre),
        reference::fingerprint(&cpost),
        "R2b: own-line promotion at EOF caused false drift"
    );
    assert_ne!(apre.node_rev, apost.node_rev);

    // PRODUCTION R2b agreement at section grain.
    assert_eq!(
        model::fingerprint::fingerprint(&dpre, apre).expect("pre-promotion A has content"),
        model::fingerprint::fingerprint(&dpost, apost).expect("post-promotion A has content"),
        "production: R2b false drift"
    );
}

/// §4.3 grain consistency — the model re-spans an Anchor node to its HOST
/// block line; norm-v2 of that host span yields the pre-promotion block bytes.
#[test]
fn anchor_host_block_grain() {
    let d = doc(X1);
    let anchor = walk_find(&d.root, &|n| matches!(n.kind, NodeKind::Anchor { .. }))
        .expect("promoted doc has an anchor node");
    let rem = reference::removals(X1.as_bytes(), &reference::anchor_markers(X1));
    let canon = reference::norm_v2_slice(X1.as_bytes(), &anchor.span, &rem);
    assert_eq!(canon.as_slice(), b"intro");
    assert_eq!(
        reference::fingerprint(&canon),
        format!("fp1.span2.b3.{GOLD_PARA_DIGEST}")
    );
}

// Token grammar (spec §2)

#[test]
fn token_mint_and_roundtrip() {
    let tok = reference::fingerprint(b"intro");
    let (version, codec, hashfn, digest) =
        reference::parse_token(&tok).expect("minted token parses");
    assert_eq!(version, "fp1");
    assert_eq!(codec, "span2");
    assert_eq!(hashfn, "b3");
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    );

    // PRODUCTION parse agrees field-for-field.
    let parts = model::fingerprint::parse_fingerprint(&tok).expect("production parses");
    assert_eq!(
        (parts.version, parts.codec, parts.hashfn, parts.digest),
        (version, codec, hashfn, digest)
    );
}

#[test]
fn token_malformed_rejected() {
    let hex64 = "0".repeat(64);
    for bad in [
        String::new(),
        "fp1.span2.b3".to_string(),                  // three fields
        format!("fp1.span2.b3.{hex64}.extra"),       // five fields
        format!("fp1..b3.{hex64}"),                  // empty codec
        format!("FP1.span2.b3.{hex64}"),             // uppercase version
        format!("fp1.span2.b3.{}", "0".repeat(63)),  // b3 length wrong
        format!("fp1.span2.b3.{}", "0".repeat(65)),  // b3 length wrong
        format!("fp1.span2.b3.{}G", "0".repeat(63)), // non-hex digest
        format!("fp1.span2.b3.{}", "A".repeat(64)),  // uppercase hex
        format!("fp1.span-2.b3.{hex64}"),            // charset: dash in field
    ] {
        assert!(
            reference::parse_token(&bad).is_none(),
            "malformed token accepted: {bad:?}"
        );
        assert!(
            model::fingerprint::parse_fingerprint(&bad).is_none(),
            "PRODUCTION accepted malformed token: {bad:?}"
        );
    }
}

/// §2.4 — self-describing survives its implementations: an unknown version,
/// codec, or hashfn PARSES (grey `unverifiable` downstream), it is not
/// malformed; digest length is unchecked for an unknown hashfn.
#[test]
fn token_unknown_version_codec_or_hashfn_parses() {
    let hex64 = "0".repeat(64);
    let (version, ..) =
        reference::parse_token(&format!("fp9.span2.b3.{hex64}")).expect("unknown version parses");
    assert_eq!(version, "fp9");
    assert!(model::fingerprint::parse_fingerprint(&format!("fp9.span2.b3.{hex64}")).is_some());
    let (_, codec, ..) =
        reference::parse_token(&format!("fp1.zzz9.b3.{hex64}")).expect("unknown codec parses");
    assert_eq!(codec, "zzz9");
    let (.., hashfn, digest) =
        reference::parse_token("fp1.span2.xx.0123abc").expect("unknown hashfn parses");
    assert_eq!(hashfn, "xx");
    assert_eq!(digest, "0123abc");

    // PRODUCTION: unknown codec/hashfn parse (grey downstream), not malformed.
    assert!(model::fingerprint::parse_fingerprint(&format!("fp1.zzz9.b3.{hex64}")).is_some());
    assert!(model::fingerprint::parse_fingerprint("fp1.span2.xx.0123abc").is_some());
}

/// Regenerator for the pinned literals — `cargo test -p model --test
/// norm_v2_fixtures -- --ignored --nocapture print_goldens`.
#[test]
#[ignore = "golden regenerator, run by hand"]
fn print_goldens() {
    println!(
        "GOLD_X0_DIGEST   = {}",
        blake3::hash(&reference::norm_v2(X0)).to_hex()
    );
    println!("GOLD_PARA_DIGEST = {}", blake3::hash(b"intro").to_hex());
}
