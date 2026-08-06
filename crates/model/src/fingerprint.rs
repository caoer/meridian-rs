//! Fingerprint plane — attestation content identity as a self-describing
//! CID-token (`docs/fingerprint-norm-spec.md`).
//!
//! # Three hash planes, never conflated (spec §1)
//! One hash family (BLAKE3-256), three domains:
//!
//! - **`node_rev`** ([`crate::NodeRev`]) — raw span bytes, 16 hex, CAS race
//!   detector.
//! - **Workspace merkle** ([`crate::merkle_root`]) — raw file bytes → 32-byte
//!   guard cursor (`b3:`).
//! - **fingerprint** — `fp1.span2.b3.<64hex>`: BLAKE3 over **norm-v2**
//!   span bytes ([`syntax::anchor_removals`]). The rev a pin holds.
//!
//! Anchor promotion (#6 §2): writing ` ^block-id` moves `node_rev` and the
//! workspace root and never moves the fingerprint (no false drift).
//!
//! # No hash-time graph walk (spec §3, §6)
//! `span2` hashes the span's own bytes — an `![[embed]]` contributes link
//! bytes only. Cross-document transitivity is lock-is-content (#8 §5): a
//! page's fingerprint covers its `meridian-lock`, which holds targets'
//! fingerprints; drift propagates at pin-update time.
//!
//! # `RevClass` — one hasher, two classes
//! - [`RevClass::Content`] — fingerprint token (blake3 over norm-v2; sole
//!   engine hasher).
//! - [`RevClass::Object`] — git object id at `commit:location`, verified by
//!   equality; the engine never computes it.

use crate::{ByteSpan, Document, Node};

/// Token-grammar version field (spec §2.1).
pub const FP_VERSION: &str = "fp1";
/// The M1-live codec: node span bytes at norm-v2 (spec §2.2).
pub const CODEC_SPAN2: &str = "span2";
/// The live hash fn: BLAKE3-256, digest = exactly 64 lowercase hex (§2.3).
pub const HASHFN_B3: &str = "b3";

/// Fingerprint token — `version.codec.hashfn.digest` (e.g. `fp1.span2.b3.<64hex>`).
/// Full-length in `meridian-lock` / receipts (#4 §5); renderers abbreviate the
/// digest (`@40b167ed`, #6 §4).
///
/// **Field private (R31):** the only constructor path is mint, which refuses an
/// empty normalized span. Read via [`Fingerprint::as_str`] /
/// [`Fingerprint::into_string`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// The token text, borrowed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The token text, taken — for a caller that stores it.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A grammar-parsed token (spec §2.4). Parse is codec-agnostic, so tokens
/// minted by newer codecs/hash-fns still parse; whether this build can verify
/// one is [`verify_content`]'s question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintParts {
    pub version: String,
    pub codec: String,
    pub hashfn: String,
    pub digest: String,
}

/// Empty norm-v2 span — fingerprint cannot exist (R31).
///
/// `blake3("")` is a universal match — the identical token for every empty
/// span — so it covers nothing. Typed refusal at mint; the verdict is never
/// green ([`ContentVerdict::EmptySpan`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptySpan;

/// Mint the `span2` fingerprint of `node` in `doc` (spec §3):
/// `b3(norm2(raw[span]))` under the whole-file anchor-removal law. For a batch,
/// use [`fingerprint_with_removals`] and compute the removals once per document.
///
/// # Errors
/// [`EmptySpan`] when the node's normalized span is empty — see that type.
pub fn fingerprint(doc: &Document, node: &Node) -> Result<Fingerprint, EmptySpan> {
    fingerprint_with_removals(doc, node, &syntax::anchor_removals(&doc.raw))
}

/// [`fingerprint`] with the document's removal ranges precomputed
/// ([`syntax::anchor_removals`] — file coordinates, whole-file
/// identification per spec §4.1/§4.3).
///
/// # Errors
/// [`EmptySpan`] — see [`fingerprint_span`].
pub fn fingerprint_with_removals(
    doc: &Document,
    node: &Node,
    removals: &[ByteSpan],
) -> Result<Fingerprint, EmptySpan> {
    fingerprint_span(doc, &node.span, removals)
}

/// Mint over a resolved span (spec §3). Sole owner of the mint expression and
/// the empty-span refusal (R31) — the only place norm-v2 bytes meet the hasher.
/// The `&Node` forms delegate here.
///
/// # Errors
/// [`EmptySpan`] when `norm_v2_slice` yields no bytes.
pub fn fingerprint_span(
    doc: &Document,
    span: &ByteSpan,
    removals: &[ByteSpan],
) -> Result<Fingerprint, EmptySpan> {
    let canonical = syntax::norm_v2_slice(&doc.raw, span, removals);
    if canonical.is_empty() {
        return Err(EmptySpan);
    }
    Ok(Fingerprint(format!(
        "{FP_VERSION}.{CODEC_SPAN2}.{HASHFN_B3}.{}",
        blake3::hash(&canonical).to_hex()
    )))
}

/// Grammar-only parse (spec §2.4). `None` = malformed — not a fingerprint
/// token at all, which includes bare 16-hex `node_rev`s and the `b3:`+64hex
/// workspace-merkle form. Digest length is validated only when the hashfn is
/// known (`b3` → 64).
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

/// A content-class verification outcome (spec §2.4). `Unverifiable`
/// (recognized token, codec/hash-fn this build does not implement — grey
/// `unverifiable-fingerprint`) is distinct from `Malformed` (not a token) and
/// from `Red` (verified, drifted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentVerdict {
    /// Recomputed fingerprint equals the pinned token.
    Green,
    /// Recomputed and compared — the content drifted; `actual` is the current
    /// fingerprint (the re-pin candidate).
    Red { actual: Fingerprint },
    /// Parses, but some member of the self-describing triple is not
    /// implemented here — grey, never green, never red. All three members are
    /// carried verbatim so a render names which one is unknown
    /// ([`ContentVerdict::unknown_members`]).
    Unverifiable {
        version: String,
        codec: String,
        hashfn: String,
    },
    /// Not a fingerprint token.
    Malformed,
    /// Live span normalizes empty ([`EmptySpan`]) — no fingerprint to compare.
    /// Never green (R31): `Green` requires a recomputed token, and
    /// [`fingerprint_span`] cannot produce one. Mint refuses empty spans, so
    /// this arm bites hand-/tool-authored pins.
    EmptySpan,
}

impl ContentVerdict {
    /// The triple members this build does not implement, in token order
    /// (`version` / `codec` / `hashfn`) — empty for every verdict but
    /// [`ContentVerdict::Unverifiable`].
    ///
    /// The single owner of "what this build implements" — a grey render asks
    /// here instead of re-comparing against [`FP_VERSION`] / [`CODEC_SPAN2`] /
    /// [`HASHFN_B3`], so the answer cannot drift from [`verify_content`]'s
    /// dispatch.
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
/// verifiable as long as their codec is implemented; unknown prefixes are grey.
#[must_use]
pub fn verify_content(doc: &Document, node: &Node, pinned: &str) -> ContentVerdict {
    verify_content_span(doc, &node.span, pinned)
}

/// [`verify_content`] over a resolved span rather than a `&Node` handle, for a
/// caller holding a resolved [`crate::Target`]. Owns the verdict dispatch;
/// [`verify_content`] delegates here.
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
    // R31: the recompute is fallible, so `Green` is reachable only for a span
    // with canonical bytes; an empty one never reaches the compare.
    let Ok(actual) = fingerprint_span(doc, span, &syntax::anchor_removals(&doc.raw)) else {
        return ContentVerdict::EmptySpan;
    };
    if actual.0 == pinned {
        ContentVerdict::Green
    } else {
        ContentVerdict::Red { actual }
    }
}

/// The class of a pinned rev (design 2 §2.3). Choosing the class is the
/// caller's; computing and verifying both classes is core, under one engine
/// hasher (blake3, for the content class only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevClass {
    /// A fingerprint token — the engine's blake3 over norm-v2 span bytes. The
    /// default.
    Content,
    /// The git object id at `commit:location`, carried verbatim and verified by
    /// equality; the engine never computes it.
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
/// against the one git observed now (design 2 §2.3). The engine performs no
/// hashing here — git owns the content-addressing.
#[must_use]
pub fn verify_object(pinned: &str, observed_git_oid: &str) -> bool {
    pinned == observed_git_oid
}

// Property arm (R4 18a.2): a lock pin is path XOR properties; the body arm is
// `span2` above. Lives here so the sealed [`Fingerprint`] mint and the sole
// blake3 hasher stay one site.

/// Property-fingerprint codec — distinct from [`CODEC_SPAN2`], because
/// properties are a different digest domain. [`verify_content`] matches codec
/// and returns [`ContentVerdict::Unverifiable`] for unknowns, so a props token
/// handed to the span verifier refuses instead of silently recomputing.
pub const CODEC_PROPS1: &str = "props1";

/// Domain-separation prefix for property canonical bytes, so a props digest
/// cannot equal a span digest under mislabeling.
pub const PROPS_DOMAIN: &str = "props1\n";

/// Frontmatter property state — three states, three fingerprints (R4 18a.2).
/// Absent (key missing) ≠ null (bare `status:` / null spellings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    /// The key is omitted from the map entirely.
    Absent,
    /// The key is present and null — bare `status:`, or an explicit null
    /// spelling ([`NULL_SPELLINGS`]).
    Null,
    /// A present, non-null value in its wire form verbatim. Nothing is coerced,
    /// so `status: ""` stays the empty string — neither absent nor null.
    Scalar(String),
}

/// YAML-null spellings, pinned as data (no-serde hand-parse). Tests assert
/// against this list so a parser swap cannot silently re-collapse states.
pub const NULL_SPELLINGS: [&str; 5] = ["", "~", "null", "Null", "NULL"];

/// Classify `key` in `map` into its [`PropValue`] state.
#[must_use]
pub fn classify_property(map: &crate::YamlMap, key: &str) -> PropValue {
    match map.0.iter().find(|(k, _)| k == key) {
        None => PropValue::Absent,
        Some((_, raw)) => {
            let trimmed = raw.trim();
            if NULL_SPELLINGS.contains(&trimmed) {
                PropValue::Null
            } else {
                PropValue::Scalar(raw.clone())
            }
        }
    }
}

/// Canonical keyed-map bytes a property fingerprint hashes.
///
/// - Keyed map (R4): [`PropValue::Absent`] is an explicit value, not a missing row.
/// - Sorted keys — selector order irrelevant by construction.
/// - Length-prefixed key/value — no delimiter forgery across maps.
///
/// Empty `keys` ⇒ all keys (R4 `[]` = all; path arm same).
#[must_use]
pub fn canonical_property_bytes(map: &crate::YamlMap, keys: &[String]) -> String {
    let mut selected: Vec<&str> = if keys.is_empty() {
        map.keys().collect()
    } else {
        keys.iter().map(String::as_str).collect()
    };
    selected.sort_unstable();

    let mut out = String::from(PROPS_DOMAIN);
    for key in selected {
        out.push_str(&key.len().to_string());
        out.push(':');
        out.push_str(key);
        match classify_property(map, key) {
            PropValue::Absent => out.push_str("=A"),
            PropValue::Null => out.push_str("=N"),
            PropValue::Scalar(value) => {
                out.push_str("=S");
                out.push_str(&value.len().to_string());
                out.push(':');
                out.push_str(&value);
            }
        }
        out.push('\n');
    }
    out
}

/// Property fingerprint of `keys` in `map` (R4 `properties` arm).
///
/// Infallible: [`PROPS_DOMAIN`] keeps the hash input non-empty, so the R31
/// hazard cannot arise. Assumes a validated selection — the caller must refuse
/// duplicate selector keys (R4).
#[must_use]
pub fn properties_fingerprint(map: &crate::YamlMap, keys: &[String]) -> Fingerprint {
    Fingerprint(format!(
        "{FP_VERSION}.{CODEC_PROPS1}.{HASHFN_B3}.{}",
        blake3::hash(canonical_property_bytes(map, keys).as_bytes()).to_hex()
    ))
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

    /// The production mint reproduces the pinned golden token of the fixtures'
    /// `X0` doc (`norm_v2_fixtures.rs`; spec §2.1's worked example).
    #[test]
    fn x0_token_matches_spec_golden() {
        let d = doc("# A\nintro\n\n# B\nbody\n");
        assert_eq!(
            fingerprint(&d, &d.root).expect("X0 has content").0,
            "fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152"
        );
    }

    /// Promotion neutrality (spec §1/§5): the fingerprint holds at document and
    /// section grain; `node_rev` moves at both.
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

        // A real edit still reddens.
        let d2 = doc("# A\nintro edited\n\n# B\nbody\n");
        assert_ne!(fingerprint(&d0, &d0.root), fingerprint(&d2, &d2.root));
    }

    /// `span2` performs no hash-time embed expansion (spec §3/§6). Editing the
    /// embedded document moves the embedded doc's own fingerprint and the
    /// workspace merkle — never the embedding doc's fingerprint.
    #[test]
    fn embeds_do_not_expand() {
        let embedding = "# Doc\n\nintro\n\n![[embedded]]\n";
        let embedded_v1 = "# Embedded\n\nsection body v1\n";
        let embedded_v2 = "# Embedded\n\nsection body v2 — edited\n";

        let host = doc(embedding);
        let (e1, e2) = (doc(embedded_v1), doc(embedded_v2));

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
        // …while the embedding doc's fingerprint (same bytes) is untouched.
        assert_eq!(host_fp, fingerprint(&host, &host.root));
    }

    /// Verification is four-way (spec §2.4): green / red / unverifiable
    /// (unknown codec or hashfn — grey, distinct from malformed) / malformed
    /// (incl. both superseded spellings).
    #[test]
    fn verify_content_verdicts() {
        let d1 = doc("# A\nbody v1\n");
        let pin = fingerprint(&d1, &d1.root).expect("d1 has content").0;
        assert_eq!(verify_content(&d1, &d1.root, &pin), ContentVerdict::Green);

        let d2 = doc("# A\nbody v2\n");
        let ContentVerdict::Red { actual } = verify_content(&d2, &d2.root, &pin) else {
            panic!("drifted content must verify Red");
        };
        assert_eq!(actual, fingerprint(&d2, &d2.root).expect("d2 has content"));

        let hex64 = "0".repeat(64);
        assert_eq!(
            verify_content(&d1, &d1.root, &format!("fp1.zzz9.b3.{hex64}")),
            ContentVerdict::Unverifiable {
                version: "fp1".to_string(),
                codec: "zzz9".to_string(),
                hashfn: "b3".to_string()
            }
        );
        // An unknown version parses too: grey, never malformed.
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

        // Bare node_rev and b3:-prefixed root are not tokens.
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

    /// The `Unverifiable` arm names the unknown triple member.
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

        // A verifiable verdict names nothing.
        let pin = fingerprint(&d, &d.root).expect("d has content").0;
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

    /// An object-class pin verifies against the git object id, produced by
    /// `git hash-object` itself; the engine only compares equality.
    #[test]
    fn object_class_verifies_against_git_object_id() {
        let content = b"section body v1\n";
        let oid = git_hash_object(content);
        // A real git blob oid is 40 lowercase hex (sha1), not a fingerprint token.
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

        assert_eq!(RevClass::Object.as_str(), "object");
        assert_eq!(RevClass::Content.as_str(), "content");
    }

    /// The canonical git object id of a blob: `git hash-object --stdin` over
    /// `content`. Test-only ground truth.
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

    fn fm(raw: &str) -> crate::YamlMap {
        fn walk(n: &Node) -> Option<crate::YamlMap> {
            if let NodeKind::Frontmatter { map } = &n.kind {
                return Some(map.clone());
            }
            n.children.iter().find_map(walk)
        }
        walk(&doc(raw).root).expect("the fixture has frontmatter")
    }

    /// A props token handed to the span verifier must refuse, never recompute
    /// span bytes against a digest that never covered them.
    #[test]
    fn a_props_token_is_unverifiable_to_the_span_verifier() {
        let d = doc("---\nstatus: open\n---\n\n# A\nbody\n");
        let token = properties_fingerprint(&fm("---\nstatus: open\n---\n\nbody\n"), &[]);

        assert!(
            token.as_str().starts_with("fp1.props1.b3."),
            "the token names its own domain: {token}"
        );
        // Well-formed, so the refusal below is about the codec, not the grammar.
        let parts = parse_fingerprint(token.as_str()).expect("a props token is grammatical");
        assert_eq!(parts.codec, CODEC_PROPS1);

        match verify_content(&d, &d.root, token.as_str()) {
            ContentVerdict::Unverifiable { codec, .. } => assert_eq!(codec, CODEC_PROPS1),
            other => panic!("a props token must not be verified as span bytes: {other:?}"),
        }
        assert_ne!(CODEC_PROPS1, CODEC_SPAN2);
    }

    /// The canonical keyed map is length-prefixed, so no key or value can forge
    /// a delimiter: two different maps must not serialize identically.
    #[test]
    fn canonical_property_bytes_cannot_be_forged() {
        // A value spelled to imitate the encoding vs the map it imitates.
        let one = fm("---\na: \"b=S1:c\"\n---\n\nbody\n");
        let two = fm("---\na: b\nc: d\n---\n\nbody\n");
        assert_ne!(
            canonical_property_bytes(&one, &[]),
            canonical_property_bytes(&two, &[])
        );
        // Domain separation is the first thing in the bytes.
        assert!(canonical_property_bytes(&one, &[]).starts_with(PROPS_DOMAIN));
    }
}
