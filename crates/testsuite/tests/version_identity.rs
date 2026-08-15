//! Blocker B-02, executable: the two-dimensional version identity
//! (merkle-spec §4.2.5) and the mechanical retirement vocabulary
//! (wire-contract §5.7/§8.2/§12.3). Three errors, three facts, never
//! flattened — and nothing refuses retired before a cutover boundary.
//!
//! Every serving identity here that carries a law-2 boundary SIMULATES the
//! post-cutover world (plan-rulings-final R1): the cutover itself is step 7
//! machinery and no live door is rewired by this card.

use model::{
    HASH_LAW_FLAT, HASH_LAW_RADIX, MerkleRoot, RootTokenVerdict, RootVersion, ServingVersion,
    merkle_root, parse_root,
};
use wire::{ErrorBody, ErrorCode, Recovery};

const HEX_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEX_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn fixture_corpus() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("notes/plan.md", b"# Plan\n\nbody\n".as_slice()),
        (
            "receipts/2026-07-18.md",
            b"# Receipts\n\n- one\n".as_slice(),
        ),
    ]
}

/// §5.7: retired ≠ mismatch ≠ unknown-future — one typed test per family,
/// against one simulated post-cutover serving identity.
#[test]
fn three_errors_three_facts_never_flattened() {
    // Cutover landed with the domain at v1: boundary = first law-2 position.
    let serving = ServingVersion::law2_cutover(1);
    assert_eq!(serving.current.ladder_position(), 2, "law2(1) ⇒ b3b:");
    assert_eq!(serving.law_boundary, 2);

    // Fact 1 — the LAW moved: a held token from the KNOWN RETIRED law-1
    // family refuses `fingerprint_version_retired`, never mismatch.
    for retired in [format!("b3:{HEX_A}"), format!("b3a:{HEX_A}")] {
        assert_eq!(
            serving.classify(&retired),
            RootTokenVerdict::Retired,
            "{retired} is a retired-family token"
        );
    }

    // Fact 2 — the premise moved: a current-family token with an unequal
    // digest is the ordinary scoped mismatch, reached by token compare.
    let held = format!("b3b:{HEX_A}");
    assert_eq!(serving.classify(&held), RootTokenVerdict::Current);
    let live = MerkleRoot(format!("b3b:{HEX_B}"));
    assert_ne!(held, live.0, "unequal digest ⇒ fingerprint_mismatch");

    // Fact 3 — the token is past my law: an UNKNOWN FUTURE family refuses
    // the distinct unsupported-version code.
    for future in [format!("b3c:{HEX_A}"), format!("b3zzz:{HEX_A}")] {
        assert_eq!(
            serving.classify(&future),
            RootTokenVerdict::UnsupportedFuture,
            "{future} is past the serving law"
        );
    }

    // The three facts carry three distinct wire codes, one recovery class.
    let retired = serde_json::to_value(ErrorCode::FingerprintVersionRetired).unwrap();
    let unsupported = serde_json::to_value(ErrorCode::FingerprintVersionUnsupported).unwrap();
    let mismatch = serde_json::to_value(ErrorCode::RootMismatch).unwrap();
    assert_eq!(retired, serde_json::json!("fingerprint_version_retired"));
    assert_eq!(
        unsupported,
        serde_json::json!("fingerprint_version_unsupported")
    );
    assert_ne!(retired, mismatch);
    assert_ne!(unsupported, mismatch);
    assert_ne!(retired, unsupported);
    assert_eq!(
        ErrorCode::FingerprintVersionRetired.recovery(),
        Recovery::Resync
    );
    assert_eq!(
        ErrorCode::FingerprintVersionUnsupported.recovery(),
        Recovery::Resync
    );
}

/// The retired refusal carries the Appendix C teaching text (§8.2
/// register-law family), byte-for-byte with `<scope>` filled — and an
/// old-law token fixture actually classifies Retired first.
#[test]
fn retired_fixture_teaches_appendix_c_text() {
    // A REAL old-law token: the production law-1 mint over the fixture
    // corpus at domain v0.
    let old_token = merkle_root(&fixture_corpus(), 0);
    assert!(old_token.0.starts_with("b3:"));

    // Simulated post-cutover serving (R1: simulate, never activate).
    let serving = ServingVersion::law2_cutover(0);
    assert_eq!(
        serving.classify(&old_token.0),
        RootTokenVerdict::Retired,
        "a held old-law token is the retired family, never a mismatch"
    );

    let refusal = ErrorBody::version_retired("notes");
    assert_eq!(refusal.code, ErrorCode::FingerprintVersionRetired);
    assert_eq!(refusal.recovery, Recovery::Resync);
    assert_eq!(
        refusal.message.as_deref(),
        Some(
            "this token was minted under a retired hash law. The premise did \
             not move — the law did. Re-mint at the same scope \
             (fingerprint{scope: \"notes\"}) and re-plan once."
        )
    );

    // The distinct unsupported teaching (drafted at §8.2 — no Appendix C
    // source), also mechanical.
    let refusal = ErrorBody::version_unsupported("notes");
    assert_eq!(refusal.code, ErrorCode::FingerprintVersionUnsupported);
    assert_eq!(refusal.recovery, Recovery::Resync);
    assert_eq!(
        refusal.message.as_deref(),
        Some(
            "this token was minted under a hash law this engine does not \
             know — the token is newer than the law being served. Re-mint at \
             the same scope (fingerprint{scope: \"notes\"}) to proceed under \
             the serving law; to keep the newer tokens, upgrade the engine, \
             not the token."
        )
    );
}

/// k3's migration law: epoch-free tokens survive RESTART — they die only at
/// a hash-law change. The mint and the classification carry no instance or
/// epoch dimension, so a restart cannot invalidate a held token.
#[test]
fn epoch_free_tokens_survive_restart_die_only_at_hash_law_change() {
    let corpus = fixture_corpus();
    let held = merkle_root(&corpus, 0);

    // RESTART: an independent re-derivation from the same bytes — fresh
    // fold state, no carried memory — mints the identical token, and the
    // held token still classifies Current.
    let after_restart = merkle_root(&fixture_corpus(), 0);
    assert_eq!(held, after_restart, "no instance salt anywhere in the mint");
    let serving = ServingVersion::law1(0);
    assert_eq!(serving.classify(&held.0), RootTokenVerdict::Current);

    // The hash-law change is the ONE event that kills it: the same held
    // token under the post-cutover identity is the retired family.
    let post_cutover = ServingVersion::law2_cutover(0);
    assert_eq!(post_cutover.classify(&held.0), RootTokenVerdict::Retired);

    // R1, mechanical: under EVERY pre-cutover (law-1) serving identity the
    // boundary is 0, so NO position can classify Retired — retirement
    // begins only at the cutover's no-return boundary.
    for domain in 0..5 {
        let serving = ServingVersion::law1(domain);
        for position in 0..8 {
            let token = RootVersion::law1(position).token([7u8; 32]);
            assert_ne!(
                serving.classify(&token.0),
                RootTokenVerdict::Retired,
                "law-1 serving at domain {domain} refused position {position} retired"
            );
        }
    }
}

/// §12.3 ladder conformance across BOTH dimensions: the worked-table
/// prefixes hold for law 1, the cutover advances the same ladder by one,
/// parse inverts mint, the same surviving hex never compares equal across
/// identities, and a held position decodes to its two-dimensional identity.
#[test]
fn ladder_conformance_across_both_dimensions() {
    // Law-1 column: the §12.3 worked table's own prefixes.
    assert_eq!(RootVersion::law1(0).prefix(), "b3:");
    assert_eq!(RootVersion::law1(1).prefix(), "b3a:");
    assert_eq!(RootVersion::law1(2).prefix(), "b3b:");
    // Law-2 column: the cutover rides the same ladder — one advance.
    assert_eq!(RootVersion::law2(1).prefix(), "b3b:");
    assert_eq!(RootVersion::law2(2).prefix(), "b3c:");
    // Deep ladder positions keep the bijective base-26 law.
    assert_eq!(RootVersion::law1(26).prefix(), "b3z:");
    assert_eq!(RootVersion::law1(27).prefix(), "b3aa:");

    // Parse inverts mint across the (law, domain) grid.
    let digest = [3u8; 32];
    for version in [
        RootVersion::law1(0),
        RootVersion::law1(1),
        RootVersion::law1(27),
        RootVersion::law2(1),
        RootVersion::law2(2),
    ] {
        let token = version.token(digest);
        let parsed = parse_root(&token.0).expect("a minted token parses");
        assert_eq!(parsed.position, version.ladder_position());
        assert_eq!(
            parsed.digest,
            blake3::Hash::from_bytes(digest).to_hex().as_str()
        );
    }

    // §4.2.5: a leaf value SURVIVES the cutover but its token RE-SPELLS —
    // the same 64 hex under two identities never compares equal.
    let law1_token = RootVersion::law1(1).token(digest);
    let law2_token = RootVersion::law2(1).token(digest);
    assert_ne!(law1_token, law2_token);
    assert_eq!(
        law1_token.0.split_once(':').unwrap().1,
        law2_token.0.split_once(':').unwrap().1,
        "same surviving hex"
    );

    // Decode: a held position names its (law, domain) pair, each dimension
    // independently comparable. History: d0 → d1 → cutover → domain bump.
    let serving = ServingVersion {
        current: RootVersion::law2(2),
        law_boundary: ServingVersion::law2_cutover(1).law_boundary,
    };
    assert_eq!(serving.decode(0), Some(RootVersion::law1(0)));
    assert_eq!(serving.decode(1), Some(RootVersion::law1(1)));
    assert_eq!(serving.decode(2), Some(RootVersion::law2(1)));
    assert_eq!(serving.decode(3), Some(RootVersion::law2(2)));
    assert_eq!(
        serving.decode(4),
        None,
        "future families have no identity here"
    );
    let held = serving.decode(2).unwrap();
    assert_eq!(
        held.hash_law, serving.current.hash_law,
        "law dimension: equal"
    );
    assert_ne!(
        held.domain, serving.current.domain,
        "domain dimension: moved"
    );
    assert_eq!(
        serving.classify(&RootVersion::law2(1).token(digest).0),
        RootTokenVerdict::DomainMoved,
        "same law + older domain is the mismatch fact, never a version refusal"
    );
    assert_eq!(HASH_LAW_FLAT, 1);
    assert_eq!(HASH_LAW_RADIX, 2);

    // The malformed family: none of these are Root-family tokens.
    let serving = ServingVersion::law1(0);
    for not_a_token in [
        "absent",
        "fp1.span2.b3.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "780d2fb4cf68f60f",
        "bf:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "b3:AAAA",
        "b3:aaaa",
        "b3A:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "",
    ] {
        assert!(parse_root(not_a_token).is_none(), "{not_a_token:?}");
        assert_eq!(serving.classify(not_a_token), RootTokenVerdict::Malformed);
    }
}
