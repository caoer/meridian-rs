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
///
/// # The field is PRIVATE, and that is the other half of R31
///
/// [`fingerprint_span`] is fallible so that no caller can mint over an empty
/// normalized span. A public constructor would leave that guard discharged but
/// holed: `Fingerprint(some_string)` reintroduces exactly the token the owner
/// refuses to produce, and the type would then mean only *"the digest came from
/// `fingerprint_span`, because nobody currently bypasses it"* — a property of
/// today's call sites rather than of the type. Sealing makes it a property of
/// the type: **the only way to hold a `Fingerprint` is to have minted one, and
/// the only mint refuses the empty span.**
///
/// This is a MAINTAINER-facing invariant, not an attacker-facing door — the
/// engine is not defending against its own crates. It earns its keep at stage
/// 3, where receipt unification and cross-root both touch fingerprints and a
/// future author would otherwise reach for the tuple constructor without ever
/// meeting the rule.
///
/// Read the token with [`Fingerprint::as_str`], take it with
/// [`Fingerprint::into_string`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// The token text, borrowed — for comparison and rendering.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The token text, taken — for a caller that stores it (a lock block, a
    /// receipt, a wire fact). Consumes the fingerprint, so nothing is cloned.
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

/// The one condition under which a fingerprint cannot exist: the span's
/// norm-v2 canonicalization is **empty** (R31).
///
/// `blake3` of empty input is a *universal match* — every empty-normalizing
/// span in every document mints the identical token — so such a token carries
/// zero information about the content it names. That voids the fingerprint's
/// whole contract:
///
/// > **A fingerprint must not be able to match content it does not cover.**
///
/// An empty normalized span is therefore not a thing to hash. It is a typed
/// refusal at mint time and a verdict that can never read green
/// ([`ContentVerdict::EmptySpan`]).
///
/// This is a **unit error on purpose**. There is exactly one way to void the
/// contract, and naming it as a TYPE — rather than a bool a caller may ignore —
/// is what makes every door discharge it or fail to compile (R5: *a boolean
/// helper a caller may ignore is not a guard; a type a caller must discharge
/// is*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptySpan;

/// Mint the `span2` fingerprint of `node` in `doc` (spec §3):
/// `b3(norm2(raw[span]))` under the whole-file anchor-removal law.
/// Convenience over [`fingerprint_with_removals`] — computing removals once
/// per document and reusing them across nodes is the batch path.
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

/// [`fingerprint_with_removals`] over a resolved SPAN rather than a `&Node`
/// handle. `span2` hashes the span's own bytes (spec §3), so the node
/// contributes nothing else — a caller holding a resolved [`crate::Target`]
/// mints without a second tree walk. THE owner of the mint expression; the
/// `&Node` forms delegate here.
///
/// **THE owner of the empty-span invariant, too** (R31). This is the single
/// place norm-v2 bytes meet the hasher — [`syntax::norm_v2_slice`] has exactly
/// one caller, this one — so refusing here closes the CLASS rather than any one
/// ref form that happens to normalize away. Which forms those are is not this
/// function's business and deliberately so: it guards the *property*, so a
/// future ref form, dialect rule, or codec that reduces to nothing is closed the
/// day it lands, with no enumeration to remember to update.
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
    /// The live span normalizes to NOTHING, so no fingerprint exists to compare
    /// the pinned token against ([`EmptySpan`]). **Never green** — and that is
    /// structural, not a policy choice: [`ContentVerdict::Green`] is reachable
    /// only through a recomputed token, and [`fingerprint_span`] cannot produce
    /// one here.
    ///
    /// **This is the LOAD-BEARING arm of R31.** The empty-span class is
    /// unreachable through `mrd pin` (every ref form refuses at mint — see the
    /// mint door in `wire_serve::write`), so the class arrives only in a HAND-
    /// or TOOL-AUTHORED lock block. That makes the verdict side, not the mint,
    /// the place the guard has to bite: before this arm, a stored pin over an
    /// empty-normalizing span read GREEN in every document and no edit anywhere
    /// could ever turn it red.
    ///
    /// A pinned token whose digest happens to BE `blake3("")` needs no special
    /// case: over an empty span it lands here, and over a non-empty span it
    /// reddens as ordinary drift. Either way it is never green — which is why
    /// the guard is a property of the recompute and not a forbidden constant.
    EmptySpan,
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
    // R31: the recompute is fallible, so `Green` below is reachable only for a
    // span that HAS canonical bytes. An empty one never reaches the compare.
    let Ok(actual) = fingerprint_span(doc, span, &syntax::anchor_removals(&doc.raw)) else {
        return ContentVerdict::EmptySpan;
    };
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

// ── The property arm of the fingerprint plane (R4 18a.2) ───────────────────
//
// A `meridian-lock` pin selects EITHER body path segments OR frontmatter
// property keys (R4, `path` XOR `properties`). The body arm is `span2` above.
// This is the property arm, and it exists here rather than in `lock` for two
// reasons: [`Fingerprint`]'s field is sealed (R31), and blake3 is the engine's
// ONE hasher — a second mint site in another crate would hold both properties
// only by convention.

/// The codec token a PROPERTY fingerprint carries — **distinct from
/// [`CODEC_SPAN2`], and that distinctness is the point.**
///
/// A properties digest is not span bytes, so under never-conflate-hash-planes
/// it is a different digest domain and must be self-describing as one. The
/// consequence that earns it: [`verify_content`] compares the codec field and
/// answers [`ContentVerdict::Unverifiable`] for anything it does not implement,
/// so a props token handed to the span verifier **refuses loudly** instead of
/// recomputing span bytes and reporting a confident wrong answer. A shared
/// token would have made that failure silent — the one silent failure in a
/// schema whose every other refusal is loud.
///
/// Ruled 2026-08-03 (advisor `c2e19632`): ZT's typed blocks in `86449b4e` spell
/// a codec only on the PATH arm; for the properties arm he typed *"the
/// fingerprint would be the wire format of such objects"* (17:36) and later
/// ruled the canonical-keyed-map digest. No token was ever ratified, so the
/// governing laws decide and there is nothing here overruled.
pub const CODEC_PROPS1: &str = "props1";

/// Domain separation for the property arm: the first bytes of every canonical
/// property serialization.
///
/// Belt AND braces with [`CODEC_PROPS1`]. The token says which domain a digest
/// claims; this says which domain the HASHER actually ran over. Separating in
/// the bytes as well as in the label means a props digest cannot equal a span
/// digest even if some future caller mislabels one.
pub const PROPS_DOMAIN: &str = "props1\n";

/// The state of one frontmatter property — **three states, and all three
/// fingerprint differently** (R4 18a.2, re-extracted from session `86449b4e`).
///
/// The distinction is not academic: Obsidian's own property editor produces
/// bare `status:` routinely, so a page that never had the key and a page whose
/// key was cleared in the UI are different facts about the page. A fixed-column
/// table collapses the first into the second; a keyed map cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    /// The key is omitted from the map entirely.
    Absent,
    /// The key is present and null — bare `status:`, or an explicit null
    /// spelling ([`NULL_SPELLINGS`]).
    Null,
    /// A present, non-null value in its WIRE FORM verbatim. Nothing is coerced,
    /// so true types survive and `status: ""` stays the empty string — which is
    /// neither absent nor null.
    Scalar(String),
}

/// Every spelling of YAML null, pinned as data rather than as parser behavior.
///
/// This crate hand-parses frontmatter (no-serde crate law), so "bare `status:`
/// is null" is OUR rule to keep true. The pinned list is what the
/// parser-behavior tests assert against, so a future library swap that
/// disagrees fails a test instead of silently re-collapsing a state.
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

/// The canonical keyed-map serialization a property fingerprint hashes.
///
/// - **Keyed map, never a fixed-column table** (R4): each entry names its key,
///   so [`PropValue::Absent`] is a value the map can hold rather than a row
///   that is missing.
/// - **Sorted keys**, so selector order is irrelevant BY CONSTRUCTION rather
///   than by a caller remembering to sort.
/// - **Length-prefixed** key and value, so no key or value byte sequence can
///   forge a delimiter and make two different maps serialize identically.
///
/// An empty `keys` selects ALL keys in the map — R4's `[]` = all, symmetric
/// with the path arm's `[]` = the whole body.
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

/// Mint the property fingerprint of `keys` in `map` (R4's `properties` arm).
///
/// Infallible, unlike [`fingerprint_span`]: [`PROPS_DOMAIN`] means the hashed
/// input is never empty, so the universal-match hazard R31 guards against
/// cannot arise here. A selection that resolves to no keys is a real, distinct
/// statement about the page rather than an uninformative token.
///
/// Duplicate keys in the selector are the CALLER's refusal (`lock` refuses them
/// per R4, never dedupes) — this function is given a validated selection.
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

    /// The U-SPEC cross-link: the production mint reproduces the pinned golden
    /// token of the fixtures' `X0` doc (`norm_v2_fixtures.rs`; spec §2.1's
    /// worked example). One literal, two suites — the hash domain cannot fork.
    #[test]
    fn x0_token_matches_spec_golden() {
        let d = doc("# A\nintro\n\n# B\nbody\n");
        assert_eq!(
            fingerprint(&d, &d.root).expect("X0 has content").0,
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

    fn fm(raw: &str) -> crate::YamlMap {
        fn walk(n: &Node) -> Option<crate::YamlMap> {
            if let NodeKind::Frontmatter { map } = &n.kind {
                return Some(map.clone());
            }
            n.children.iter().find_map(walk)
        }
        walk(&doc(raw).root).expect("the fixture has frontmatter")
    }

    /// **The whole reason the property arm gets its own codec** (ruled
    /// 2026-08-03): a props token handed to the SPAN verifier must refuse
    /// loudly, never recompute span bytes and answer confidently about a digest
    /// that never covered them.
    #[test]
    fn a_props_token_is_unverifiable_to_the_span_verifier() {
        let d = doc("---\nstatus: open\n---\n\n# A\nbody\n");
        let token = properties_fingerprint(&fm("---\nstatus: open\n---\n\nbody\n"), &[]);

        assert!(
            token.as_str().starts_with("fp1.props1.b3."),
            "the token names its own domain: {token}"
        );
        // It is a well-formed token — the refusal below is about the CODEC, not
        // about the grammar (without this, `Unverifiable` proves nothing).
        let parts = parse_fingerprint(token.as_str()).expect("a props token is grammatical");
        assert_eq!(parts.codec, CODEC_PROPS1);

        match verify_content(&d, &d.root, token.as_str()) {
            ContentVerdict::Unverifiable { codec, .. } => assert_eq!(codec, CODEC_PROPS1),
            other => panic!("a props token must not be verified as span bytes: {other:?}"),
        }
        // Had the arms shared `span2`, this same call would have returned Red
        // with a confident, meaningless `actual`.
        assert_ne!(CODEC_PROPS1, CODEC_SPAN2);
    }

    /// The canonical keyed map is length-prefixed, so no key or value can forge
    /// a delimiter: two DIFFERENT maps must not serialize identically.
    #[test]
    fn canonical_property_bytes_cannot_be_forged() {
        // `a` = "b=Sx" vs a key literally spelled to imitate the encoding.
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
