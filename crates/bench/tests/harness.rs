//! Harness integrity: determinism, inventory soundness, claims join logic,
//! report rendering, cache reuse. These gate the *measuring instrument* — if
//! the instrument drifts, every number it produces is noise.

use std::collections::BTreeMap;

use bench::claims::{Claim, Direction, ThresholdSource, VerdictKind, join};
use bench::corpus;
use bench::generator::generate_file;
use bench::inventory::Inventory;
use bench::profile::{
    ConstructRate, KNOWN_CONSTRUCTS, Pathology, Profile, Recipe, SizeDist, Unicode,
};

fn dense_profile() -> Profile {
    let constructs = KNOWN_CONSTRUCTS
        .iter()
        .map(|id| {
            (
                (*id).to_owned(),
                ConstructRate {
                    file_rate: 1.0,
                    per_file: 3.0,
                },
            )
        })
        .collect();
    Profile {
        name: "test-dense".into(),
        provenance: "unit test".into(),
        files: 5,
        size: SizeDist {
            median_bytes: 1500,
            sigma: 0.5,
            max_bytes: 32_768,
        },
        constructs,
        pathology: Pathology {
            unterminated_fence_rate: 0.0,
        },
        unicode: Unicode { cjk_rate: 0.2 },
    }
}

#[test]
fn generation_is_deterministic() {
    let recipe = Recipe::new(dense_profile(), 42, None);
    let tmp = std::env::temp_dir().join(format!("bench-det-{}", std::process::id()));
    let a = corpus::generate_into(&tmp.join("a"), &recipe).unwrap();
    let b = corpus::generate_into(&tmp.join("b"), &recipe).unwrap();
    assert_eq!(
        a.corpus_hash, b.corpus_hash,
        "same recipe must regenerate byte-identically"
    );
    assert_eq!(a.total_bytes, b.total_bytes);
    let fa = corpus::load_files(&tmp.join("a")).unwrap();
    let fb = corpus::load_files(&tmp.join("b")).unwrap();
    assert_eq!(fa, fb);
    // Different seed ⇒ different corpus.
    let other = Recipe::new(dense_profile(), 43, None);
    let c = corpus::generate_into(&tmp.join("c"), &other).unwrap();
    assert_ne!(a.corpus_hash, c.corpus_hash);
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn inventory_sidecars_sum_to_corpus_inventory() {
    let recipe = Recipe::new(dense_profile(), 7, None);
    let tmp = std::env::temp_dir().join(format!("bench-inv-{}", std::process::id()));
    corpus::generate_into(&tmp, &recipe).unwrap();
    let corpus_inv = corpus::load_inventory(&tmp).unwrap();
    // Sum the per-file sidecars.
    let mut summed = Inventory::new();
    for (rel, _) in corpus::load_files(&tmp).unwrap() {
        let sidecar = tmp.join(format!("{rel}.inventory.json"));
        let text = std::fs::read_to_string(&sidecar).unwrap();
        summed.merge(&serde_json::from_str(&text).unwrap());
    }
    assert_eq!(summed, corpus_inv);
    // file_rate 1.0 ⇒ every construct present at least once per file.
    for id in KNOWN_CONSTRUCTS {
        assert!(
            corpus_inv.count(id) >= u64::from(recipe.files),
            "construct {id}: {} < {} files",
            corpus_inv.count(id),
            recipe.files
        );
    }
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn unterminated_fence_is_final_and_counted() {
    let mut profile = dense_profile();
    profile.pathology.unterminated_fence_rate = 1.0;
    let recipe = Recipe::new(profile, 3, Some(4));
    for index in 0..recipe.files {
        let gf = generate_file(&recipe, index);
        assert!(gf.inventory.count("fence_unterminated") >= 1);
        let delims = gf
            .text
            .lines()
            .filter(|l| l.trim_start().starts_with("```"))
            .count();
        assert_eq!(
            delims % 2,
            1,
            "open fence must leave an odd delimiter count"
        );
    }
}

#[test]
fn corpus_cache_reuses_without_regenerating() {
    let recipe = Recipe::new(dense_profile(), 11, Some(3));
    let dir = corpus::ensure(&recipe).unwrap();
    let first = corpus::read_manifest(&dir).unwrap();
    let dir2 = corpus::ensure(&recipe).unwrap();
    let second = corpus::read_manifest(&dir2).unwrap();
    assert_eq!(dir, dir2);
    assert_eq!(
        first.created_unix_secs, second.created_unix_secs,
        "second ensure() must hit the cache, not regenerate"
    );
}

fn claim(id: &str, direction: Direction, threshold: Option<f64>, source: ThresholdSource) -> Claim {
    Claim {
        id: id.into(),
        title: id.into(),
        metric: "x".into(),
        direction,
        baseline: None,
        baseline_provenance: None,
        threshold,
        threshold_source: source,
        harness: "test".into(),
        recipe: None,
        note: None,
    }
}

#[test]
fn verdict_join_covers_all_states() {
    let claims = vec![
        claim(
            "hi.pass",
            Direction::Higher,
            Some(10.0),
            ThresholdSource::Literal,
        ),
        claim(
            "hi.fail",
            Direction::Higher,
            Some(10.0),
            ThresholdSource::Literal,
        ),
        claim(
            "lo.pass",
            Direction::Lower,
            Some(10.0),
            ThresholdSource::Literal,
        ),
        claim(
            "lo.fail",
            Direction::Lower,
            Some(10.0),
            ThresholdSource::Literal,
        ),
        claim("measured", Direction::Lower, None, ThresholdSource::Literal),
        claim(
            "ruleset.gates.nothing",
            Direction::Lower,
            Some(1.0),
            ThresholdSource::Ruleset,
        ),
        claim(
            "untested",
            Direction::Lower,
            Some(10.0),
            ThresholdSource::Literal,
        ),
    ];
    let measured: BTreeMap<String, f64> = [
        ("hi.pass", 12.0),
        ("hi.fail", 8.0),
        ("lo.pass", 8.0),
        ("lo.fail", 12.0),
        ("measured", 5.0),
        ("ruleset.gates.nothing", 99.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let verdicts: BTreeMap<String, VerdictKind> = join(&claims, &measured)
        .into_iter()
        .map(|v| (v.claim_id, v.verdict))
        .collect();
    assert_eq!(verdicts["hi.pass"], VerdictKind::Pass);
    assert_eq!(verdicts["hi.fail"], VerdictKind::Fail);
    assert_eq!(verdicts["lo.pass"], VerdictKind::Pass);
    assert_eq!(verdicts["lo.fail"], VerdictKind::Fail);
    assert_eq!(verdicts["measured"], VerdictKind::Measured);
    assert_eq!(
        verdicts["ruleset.gates.nothing"],
        VerdictKind::Measured,
        "ruleset thresholds must not gate until rung 6 resolves them"
    );
    assert_eq!(verdicts["untested"], VerdictKind::Untested);
}

#[test]
fn bundled_claims_load_and_render() {
    let claims = bench::claims::load_bundled().expect("claims.toml is valid");
    assert!(claims.iter().any(|c| c.id == "parse.throughput.corpus"));
    let report = bench::report::build(&claims).unwrap();
    let md = bench::report::render_markdown(&report, &claims);
    assert!(
        md.contains("UNTESTED"),
        "day-1 report must show UNTESTED claims"
    );
    assert!(md.contains("parse.throughput.corpus"));
}

#[test]
fn validate_rejects_size_bounds_that_would_panic_sampling() {
    // max_bytes < 64 passes the old median/max check but makes sample_size call
    // `clamp(64, max_bytes)` with min > max, which panics. validate must reject.
    let mut profile = dense_profile();
    profile.size = SizeDist {
        median_bytes: 32,
        sigma: 0.0,
        max_bytes: 32,
    };
    assert!(
        profile.validate().is_err(),
        "max_bytes < 64 must be rejected (clamp would panic)"
    );
}

#[test]
fn validate_rejects_nonfinite_intensity_that_would_hang() {
    // A non-finite per_file reaches poisson's Knuth loop, whose `p < NaN` exit
    // is never true — an infinite loop. validate must reject before generation.
    let mut profile = dense_profile();
    profile.constructs.insert(
        "heading".to_owned(),
        ConstructRate {
            file_rate: 1.0,
            per_file: f64::NAN,
        },
    );
    assert!(
        profile.validate().is_err(),
        "non-finite per_file must be rejected (poisson would hang)"
    );
}

#[test]
fn bundled_profiles_load() {
    for name in ["vault-2026", "monster-10mb", "fence-bomb"] {
        let profile = Profile::resolve(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(profile.name, name);
    }
}
