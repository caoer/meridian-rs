//! The **fingerprint plane** — attestation content identity as a
//! self-describing CID-token (`docs/norm-v2-spec.md`; decision
//! 2026-07-24-fingerprint-cid-representation, "#4").
//!
//! # The three hash planes, never conflated (spec §1)
//! One hash family (BLAKE3-256), three domains with distinct jobs:
//!
//! - **`node_rev`** ([`crate::NodeRev`]) — RAW span bytes, 16 hex, the CAS
//!   race detector. Unchanged by this module.
//! - **The workspace tree merkle** ([`crate::merkle_root`]) — RAW file bytes
//!   composed to the 32-byte guard/freshness cursor (`b3:` spelling).
//!   Unchanged by this module.
//! - **fingerprint** (HERE) — `fp1.span2.b3.<64hex>`: BLAKE3 over the node's
//!   span bytes canonicalized by **norm-v2** (anchor-token removal,
//!   [`syntax::anchor_removals`]). The rev a pin holds.
//!
//! The planes split exactly at anchor promotion (#6 §2): pin writing
//! ` ^block-id` into a target MOVES `node_rev` and the workspace root (the
//! bytes really changed — CAS and guard must see it) and NEVER moves the
//! fingerprint (no false drift — the honesty doctrine, #4 §4).
//!
//! # No hash-time graph walk (spec §3, §6)
//! `span2` hashes the span's own bytes: an `![[embed]]` contributes its LINK
//! bytes, never the embedded content. Cross-document transitivity is carried
//! by lock-is-content (#8 §5): a page's fingerprint covers its
//! `meridian-lock` block, which holds its targets' fingerprints — drift
//! propagates at pin-update time, not at hash time. This supersedes the
//! pre-marathon `compose_rev` scheme (hash-of-`node_rev`-hex leaf, hash-time
//! embed expansion, cycle sentinel, dangling refusal, 16-hex truncation) —
//! with no recursion there is nothing to dangle or cycle.
//!
//! # `RevClass` — one hasher, two classes
//! - [`RevClass::Content`] — a fingerprint token, the engine's blake3 over
//!   norm-v2 bytes (the default; the ONE engine hasher).
//! - [`RevClass::Object`] — the git object id at `commit:location`, a
//!   source-2 fact carried verbatim and verified by **equality**; the engine
//!   never computes it (git owns that content-addressing).

use crate::{ByteSpan, Document, Node};

/// Token-grammar version field (spec §2.1).
pub const FP_VERSION: &str = "fp1";
/// The M1-live codec: node span bytes at norm-v2 (spec §2.2).
pub const CODEC_SPAN2: &str = "span2";
/// The live hash fn: BLAKE3-256, digest = exactly 64 lowercase hex (§2.3).
pub const HASHFN_B3: &str = "b3";

/// A full fingerprint token — `version.codec.hashfn.digest`, e.g.
/// `fp1.span2.b3.<64hex>`. Full-length tokens live in `meridian-lock` blocks
/// and receipts (#4 §5); render planes abbreviate the DIGEST field
/// (`@40b167ed`-style, the #6 §4 view grammar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(pub String);

/// A grammar-parsed token (spec §2.4): parse is codec-agnostic, so tokens
/// minted by newer codecs/hash-fns still parse — self-describing survives its
/// implementations. Whether THIS build can verify it is [`verify_content`]'s
/// question, not parse's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintParts {
    pub version: String,
    pub codec: String,
    pub hashfn: String,
    pub digest: String,
}

/// Mint the `span2` fingerprint of `node` in `doc` (spec §3):
/// `b3(norm2(raw[span]))` under the whole-file anchor-removal law.
/// Convenience over [`fingerprint_with_removals`] — computing removals once
/// per document and reusing them across nodes is the batch path.
#[must_use]
pub fn fingerprint(doc: &Document, node: &Node) -> Fingerprint {
    fingerprint_with_removals(doc, node, &syntax::anchor_removals(&doc.raw))
}

/// [`fingerprint`] with the document's removal ranges precomputed
/// ([`syntax::anchor_removals`] — file coordinates, whole-file
/// identification per spec §4.1/§4.3).
#[must_use]
pub fn fingerprint_with_removals(
    doc: &Document,
    node: &Node,
    removals: &[ByteSpan],
) -> Fingerprint {
    fingerprint_span(doc, &node.span, removals)
}

/// [`fingerprint_with_removals`] over a resolved SPAN rather than a `&Node`
/// handle. `span2` hashes the span's own bytes (spec §3), so the node
/// contributes nothing else — a caller holding a resolved [`crate::Target`]
/// mints without a second tree walk. THE owner of the mint expression; the
/// `&Node` forms delegate here.
#[must_use]
pub fn fingerprint_span(doc: &Document, span: &ByteSpan, removals: &[ByteSpan]) -> Fingerprint {
    let canonical = syntax::norm_v2_slice(&doc.raw, span, removals);
    Fingerprint(format!(
        "{FP_VERSION}.{CODEC_SPAN2}.{HASHFN_B3}.{}",
        blake3::hash(&canonical).to_hex()
    ))
}

/// Grammar-only parse (spec §2.4). `None` = malformed — not a fingerprint
/// token at all (this includes the superseded spellings: bare 16-hex
/// `node_rev`s and the `b3:`+64hex workspace-merkle form). Digest LENGTH is
/// validated only when the hashfn is known (`b3` → 64).
#[must_use]
pub fn parse_fingerprint(s: &str) -> Option<FingerprintParts> {
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
    if hashfn == HASHFN_B3 && digest.len() != 64 {
        return None;
    }
    Some(FingerprintParts {
        version: version.to_string(),
        codec: codec.to_string(),
        hashfn: hashfn.to_string(),
        digest: digest.to_string(),
    })
}

/// A content-class verification outcome (spec §2.4). Four-way on purpose:
/// `Unverifiable` (recognized token, codec/hash-fn this build does not
/// implement — renders grey, the `superseded-algo` family) is NOT `Malformed`
/// (not a token) and neither is `Red` (verified, drifted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentVerdict {
    /// Recomputed fingerprint equals the pinned token.
    Green,
    /// Recomputed and compared — the content drifted; `actual` is the current
    /// fingerprint (the re-pin candidate).
    Red { actual: Fingerprint },
    /// Parses, but some member of the self-describing triple is not
    /// implemented here — grey, never green, never red. All THREE members are
    /// carried verbatim so a render names WHICH one is unknown: an
    /// `fp9.span2.b3` token that reported only `(codec, hashfn)` would print a
    /// live-looking pair and hide that the VERSION is the unknown member
    /// ([`ContentVerdict::unknown_members`]).
    Unverifiable {
        version: String,
        codec: String,
        hashfn: String,
    },
    /// Not a fingerprint token.
    Malformed,
}

impl ContentVerdict {
    /// The triple members THIS build does not implement, in token order
    /// (`version` / `codec` / `hashfn`) — empty for every verdict but
    /// [`ContentVerdict::Unverifiable`].
    ///
    /// The single owner of "what this build implements": a grey render asks
    /// HERE rather than re-comparing against [`FP_VERSION`] / [`CODEC_SPAN2`] /
    /// [`HASHFN_B3`] itself, so the answer cannot drift from
    /// [`verify_content`]'s own dispatch.
    #[must_use]
    pub fn unknown_members(&self) -> Vec<&'static str> {
        let ContentVerdict::Unverifiable {
            version,
            codec,
            hashfn,
        } = self
        else {
            return Vec::new();
        };
        [
            ("version", version.as_str(), FP_VERSION),
            ("codec", codec.as_str(), CODEC_SPAN2),
            ("hashfn", hashfn.as_str(), HASHFN_B3),
        ]
        .into_iter()
        .filter(|(_, found, live)| found != live)
        .map(|(member, _, _)| member)
        .collect()
    }
}

/// Verify a **content-class** pin: parse the pinned token, dispatch on its
/// self-described prefix, recompute, compare (spec §2.4). Old tokens stay
/// verifiable as long as their codec is implemented; unknown prefixes are
/// grey — migration lives inside the identifier (#4 §1).
#[must_use]
pub fn verify_content(doc: &Document, node: &Node, pinned: &str) -> ContentVerdict {
    verify_content_span(doc, &node.span, pinned)
}

/// [`verify_content`] over a resolved SPAN rather than a `&Node` handle — the
/// same law, for a caller holding a resolved [`crate::Target`] (the pin-color
/// plane, [`crate::selector::classify_pin`]). THE owner of the verdict
/// dispatch; [`verify_content`] delegates here.
#[must_use]
pub fn verify_content_span(doc: &Document, span: &ByteSpan, pinned: &str) -> ContentVerdict {
    let Some(parts) = parse_fingerprint(pinned) else {
        return ContentVerdict::Malformed;
    };
    if parts.version != FP_VERSION || parts.codec != CODEC_SPAN2 || parts.hashfn != HASHFN_B3 {
        return ContentVerdict::Unverifiable {
            version: parts.version,
            codec: parts.codec,
            hashfn: parts.hashfn,
        };
    }
    let actual = fingerprint_span(doc, span, &syntax::anchor_removals(&doc.raw));
    if actual.0 == pinned {
        ContentVerdict::Green
    } else {
        ContentVerdict::Red { actual }
    }
}

/// The class of a pinned rev (design 2 §2.3, A2). The class DECISION is the
/// caller's (which refs are pointed effects is domain meaning); computing and
/// verifying BOTH classes is core, under ONE engine hasher (blake3, for the
/// content class only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevClass {
    /// A fingerprint token — the engine's blake3 over norm-v2 span bytes. The
    /// default.
    Content,
    /// The git object id at `commit:location` — a source-2 fact carried
    /// verbatim, verified by equality; the engine never computes it.
    Object,
}

impl RevClass {
    /// The wire/lock spelling (`edge.rev_class`): `content` | `object`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RevClass::Content => "content",
            RevClass::Object => "object",
        }
    }
}

/// Verify an **object-class** pin: pure equality of the pinned git object id
/// against the one git observed now (design 2 §2.3). The engine performs NO
/// hashing here — git owns the content-addressing (the one-hasher law: blake3
/// is the engine's only hash family, used for the content class alone). Green
/// iff the ids are byte-equal.
#[must_use]
pub fn verify_object(pinned: &str, observed_git_oid: &str) -> bool {
    pinned == observed_git_oid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeKind, build};

    fn doc(raw: &str) -> Document {
        build(raw.to_string(), syntax::parse(raw))
    }

    fn find_section<'a>(n: &'a Node, name: &str) -> Option<&'a Node> {
        if matches!(&n.kind, NodeKind::Section { heading_text, .. } if heading_text == name) {
            return Some(n);
        }
        n.children.iter().find_map(|c| find_section(c, name))
    }

    /// The U-SPEC cross-link: the production mint reproduces the pinned golden
    /// token of the fixtures' `X0` doc (`norm_v2_fixtures.rs`; spec §2.1's
    /// worked example). One literal, two suites — the hash domain cannot fork.
    #[test]
    fn x0_token_matches_spec_golden() {
        let d = doc("# A\nintro\n\n# B\nbody\n");
        assert_eq!(
            fingerprint(&d, &d.root).0,
            "fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152"
        );
    }

    /// LOAD-BEARING — the promotion-neutrality split (spec §1/§5): one anchor
    /// promotion, two planes, opposite obligations. Fingerprint holds at
    /// document AND section grain; `node_rev` moves at both.
    #[test]
    fn anchor_promotion_neutral_on_fingerprint_visible_to_cas() {
        let d0 = doc("# A\nintro\n\n# B\nbody\n");
        let d1 = doc("# A\nintro ^goal\n\n# B\nbody\n");

        assert_eq!(
            fingerprint(&d0, &d0.root),
            fingerprint(&d1, &d1.root),
            "promotion caused false drift at document grain"
        );
        assert_ne!(
            d0.root.node_rev, d1.root.node_rev,
            "CAS must see the promotion byte-change"
        );

        let (a0, a1) = (
            find_section(&d0.root, "A").expect("A"),
            find_section(&d1.root, "A").expect("A"),
        );
        assert_eq!(
            fingerprint(&d0, a0),
            fingerprint(&d1, a1),
            "promotion caused false drift at section grain"
        );
        assert_ne!(a0.node_rev, a1.node_rev);

        // A REAL edit reddens: fingerprint is normalization, not blindness.
        let d2 = doc("# A\nintro edited\n\n# B\nbody\n");
        assert_ne!(fingerprint(&d0, &d0.root), fingerprint(&d2, &d2.root));
    }

    /// THE NEW LAW (spec §3/§6, supersedes `compose_rev`): `span2` performs NO
    /// hash-time embed expansion. Editing the embedded document moves the
    /// embedded doc's own fingerprint and the workspace merkle — never the
    /// embedding doc's fingerprint. Cross-document drift is the lock plane's
    /// job (lock-is-content, #8 §5), minted at pin time, stage 2.
    #[test]
    fn embeds_do_not_expand() {
        let embedding = "# Doc\n\nintro\n\n![[embedded]]\n";
        let embedded_v1 = "# Embedded\n\nsection body v1\n";
        let embedded_v2 = "# Embedded\n\nsection body v2 — edited\n";

        let host = doc(embedding);
        let (e1, e2) = (doc(embedded_v1), doc(embedded_v2));

        // The embedding doc's fingerprint is a pure function of ITS bytes.
        let host_fp = fingerprint(&host, &host.root);
        assert_eq!(
            host_fp,
            fingerprint(&doc(embedding), &doc(embedding).root),
            "deterministic"
        );

        // The embedded edit moves the embedded doc's fingerprint…
        assert_ne!(fingerprint(&e1, &e1.root), fingerprint(&e2, &e2.root));
        // …and the workspace root (guard plane, raw bytes)…
        let root_v1 = crate::merkle_root(
            &[
                ("embedding.md", embedding.as_bytes()),
                ("embedded.md", embedded_v1.as_bytes()),
            ],
            0,
        );
        let root_v2 = crate::merkle_root(
            &[
                ("embedding.md", embedding.as_bytes()),
                ("embedded.md", embedded_v2.as_bytes()),
            ],
            0,
        );
        assert_ne!(root_v1.0, root_v2.0);
        // …while the embedding doc's fingerprint (same bytes) is untouched —
        // the link is bytes in the span; the linked content is not.
        assert_eq!(host_fp, fingerprint(&host, &host.root));
    }

    /// Verification is four-way (spec §2.4): green / red / unverifiable
    /// (unknown codec or hashfn — grey, distinct from malformed) / malformed
    /// (incl. both superseded spellings).
    #[test]
    fn verify_content_verdicts() {
        let d1 = doc("# A\nbody v1\n");
        let pin = fingerprint(&d1, &d1.root).0;
        assert_eq!(verify_content(&d1, &d1.root, &pin), ContentVerdict::Green);

        let d2 = doc("# A\nbody v2\n");
        let ContentVerdict::Red { actual } = verify_content(&d2, &d2.root, &pin) else {
            panic!("drifted content must verify Red");
        };
        assert_eq!(actual, fingerprint(&d2, &d2.root));

        let hex64 = "0".repeat(64);
        assert_eq!(
            verify_content(&d1, &d1.root, &format!("fp1.zzz9.b3.{hex64}")),
            ContentVerdict::Unverifiable {
                version: "fp1".to_string(),
                codec: "zzz9".to_string(),
                hashfn: "b3".to_string()
            }
        );
        // Unknown VERSION parses too (a future fp2 token is grey here, never
        // malformed — migration lives inside the identifier).
        assert_eq!(
            verify_content(&d1, &d1.root, &format!("fp9.span2.b3.{hex64}")),
            ContentVerdict::Unverifiable {
                version: "fp9".to_string(),
                codec: "span2".to_string(),
                hashfn: "b3".to_string()
            }
        );
        assert_eq!(
            verify_content(&d1, &d1.root, "fp1.span2.xx.0123abc"),
            ContentVerdict::Unverifiable {
                version: "fp1".to_string(),
                codec: "span2".to_string(),
                hashfn: "xx".to_string()
            }
        );

        // Superseded spellings are NOT tokens: bare node_rev, b3:-prefixed root.
        for malformed in [
            "780d2fb4cf68f60f",
            &format!("b3:{hex64}"),
            "",
            "fp1.span2.b3",
        ] {
            assert_eq!(
                verify_content(&d1, &d1.root, malformed),
                ContentVerdict::Malformed,
                "{malformed:?} must be Malformed"
            );
        }
    }

    /// The `Unverifiable` arm NAMES the unknown triple member. Before the
    /// version was carried, an `fp9.span2.b3` grey reported codec=span2 /
    /// hashfn=b3 — both live-looking — and could not say WHICH member this
    /// build does not implement.
    #[test]
    fn unverifiable_names_the_unknown_triple_member() {
        let d = doc("# A\nbody\n");
        let hex64 = "0".repeat(64);
        let cases = [
            (format!("fp9.span2.b3.{hex64}"), vec!["version"]),
            (format!("fp1.zzz9.b3.{hex64}"), vec!["codec"]),
            ("fp1.span2.xx.0123abc".to_string(), vec!["hashfn"]),
            (
                "fp9.zzz9.xx.0123abc".to_string(),
                vec!["version", "codec", "hashfn"],
            ),
        ];
        for (token, expected) in cases {
            let verdict = verify_content(&d, &d.root, &token);
            assert_eq!(
                verdict.unknown_members(),
                expected,
                "{token} must name {expected:?} as unknown"
            );
        }

        // A verifiable verdict names nothing — the list is the grey's alone.
        let pin = fingerprint(&d, &d.root).0;
        assert!(
            verify_content(&d, &d.root, &pin)
                .unknown_members()
                .is_empty()
        );
        assert!(
            verify_content(&d, &d.root, "not-a-token")
                .unknown_members()
                .is_empty()
        );
    }

    /// OBJECT-CLASS REV — an object-class pin verifies against THE git object
    /// id. The ground-truth id is produced by `git hash-object` itself; the
    /// engine only compares equality (no second hasher).
    #[test]
    fn object_class_verifies_against_git_object_id() {
        let content = b"section body v1\n";
        let oid = git_hash_object(content);
        // Sanity: a real git blob oid is 40 lowercase hex (sha1) — not a
        // fingerprint token. The class is genuinely git's, not ours.
        assert_eq!(oid.len(), 40, "git blob oid is 40 hex");
        assert!(oid.chars().all(|c| c.is_ascii_hexdigit()));

        assert!(
            verify_object(&oid, &oid),
            "an object-class pin is green against the same git object id"
        );

        let oid_other = git_hash_object(b"section body v2 edited\n");
        assert_ne!(oid, oid_other, "git ids differ for different content");
        assert!(
            !verify_object(&oid, &oid_other),
            "the object-class pin reddens against a different git object id"
        );

        // The class spelling is stable for the lock/edge surface.
        assert_eq!(RevClass::Object.as_str(), "object");
        assert_eq!(RevClass::Content.as_str(), "content");
    }

    /// The canonical git object id of a blob: `git hash-object --stdin` over
    /// `content`. Test-only ground truth — the engine never computes this.
    fn git_hash_object(content: &[u8]) -> String {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn `git hash-object`");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(content)
            .expect("write content");
        let out = child.wait_with_output().expect("git hash-object output");
        assert!(out.status.success(), "git hash-object failed");
        String::from_utf8(out.stdout)
            .expect("utf8 oid")
            .trim()
            .to_string()
    }
}
