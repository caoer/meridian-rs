//! M2-RESOLVE — the walk plane through `model::walk`, and the two-plane splits.
//!
//! Gate 2: the `obsidian-compat@1.12.7` app-oracle pack (`resolution.expected.json`)
//! is reproduced probe-for-probe by `walk.rs` — every value comes from the live
//! oracle, none hand-written. Gate 3: the duplicate-anchor split — the mint plane
//! refuses `ambiguous_ref` (loud) where the walk plane silently picks (last-wins,
//! frozen §2.1). Gate 4: the re-homed CHARSET-GUARD walk-vs-mint split — a
//! `_`-bearing id is `bad_request` on the mint plane and `ref_not_found` (app
//! silent-drop) on the walk. Plus the gate-3 adversarial harness `p2-walk-probes`
//! run against `walk.rs` over the walkvault corpus.

use model::walk::{walk, Location, Miss, Stage};
use model::{build, BadAnchorId, CorpusIndex, Document, Ref, ResolveError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;

/// Build the walkvault corpus (walk.md + blocks.md) into documents + a stage-1
/// name index — byte-identical to the GT pack and the adversarial harness.
fn walkvault() -> (CorpusIndex, BTreeMap<String, Document>) {
    let dir = testsuite::gt_pack_dir().join("obsidian-compat/walkvault");
    let mut index = CorpusIndex::new();
    let mut docs = BTreeMap::new();
    for name in ["walk.md", "blocks.md"] {
        let raw = fs::read_to_string(dir.join(name))
            .unwrap_or_else(|e| panic!("read {name}: {e}"));
        let doc = build(raw.clone(), syntax::parse(&raw));
        index.insert(name, &doc);
        docs.insert(name.to_string(), doc);
    }
    (index, docs)
}

fn oracle_json() -> Value {
    let path = testsuite::gt_pack_dir().join("obsidian-compat/resolution.expected.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read oracle: {e}"));
    serde_json::from_str(&raw).expect("oracle json")
}

fn usize_at(v: &Value) -> usize {
    usize::try_from(v.as_u64().expect("non-negative integer")).expect("fits usize")
}

/// Transcode an Obsidian `Loc.offset` (UTF-16 code units) to a UTF-8 byte index
/// into `raw`. An offset at/past the file's UTF-16 length clamps to `raw.len()`
/// — which is exactly the byte size the oracle records for an EOF-ending span
/// (`dest.stat.size`), so this one map handles both endpoint conventions.
///
/// FINDING (reported to the Leader): `resolution.expected.json` labels its spans
/// "UTF-8 byte offsets (corpus is ASCII)", but walkvault contains `—` (U+2014)
/// and `→` (U+2192), so the app's UTF-16 offsets diverge from UTF-8 by 2 bytes
/// per non-ASCII BMP char. `walk.rs` emits UTF-8 byte spans (contract §1, the
/// engine convention); this transcode is the honest bridge — the assertion still
/// proves walk.rs resolves each ref to the SAME node the live app did.
fn u16_to_byte(raw: &str, u16_off: usize) -> usize {
    let mut u16 = 0;
    for (byte_idx, ch) in raw.char_indices() {
        if u16 >= u16_off {
            return byte_idx;
        }
        u16 += ch.len_utf16();
    }
    raw.len()
}

/// The oracle's recorded answer for each probe, as a `walk::walk` result —
/// parsed from `resolution.expected.json` (never hand-written) and transcoded
/// from the app's UTF-16 offsets to the engine's UTF-8 byte spans.
fn oracle_answers(
    oracle: &Value,
    docs: &BTreeMap<String, Document>,
) -> BTreeMap<String, Result<Location, Miss>> {
    let mut out = BTreeMap::new();
    for p in oracle["probes"].as_array().expect("probes array") {
        let id = p["id"].as_str().expect("probe id").to_string();
        let ans = &p["answer"];
        let result = if ans["ok"].as_bool() == Some(true) {
            let dest = ans["dest"].as_str().expect("dest").to_string();
            let raw = &docs[&dest].raw;
            Ok(Location {
                span: u16_to_byte(raw, usize_at(&ans["span"][0]))
                    ..u16_to_byte(raw, usize_at(&ans["span"][1])),
                dest,
            })
        } else {
            let err = &ans["error"];
            let stage = if err["stage"].as_u64() == Some(1) {
                Stage::One
            } else {
                Stage::Two
            };
            Err(Miss {
                stage,
                dest: err["dest"].as_str().map(str::to_string),
            })
        };
        out.insert(id, result);
    }
    out
}

/// GATE 2 — the app-oracle pack is green through `walk.rs`: every probe (incl.
/// case-insensitivity, generation-skipping, first-match-silent, and the `_`
/// probe WX-6) resolves exactly as the live Obsidian oracle recorded.
#[test]
fn obsidian_compat_pack_green_through_walk() {
    let (index, docs) = walkvault();
    let oracle = oracle_json();
    let answers = oracle_answers(&oracle, &docs);
    assert_eq!(answers.len(), 8, "the 8-probe obsidian-compat set");
    for p in oracle["probes"].as_array().expect("probes") {
        let id = p["id"].as_str().expect("id");
        let from = p["from"].as_str().expect("from");
        let refstr = p["ref"].as_str().expect("ref");
        let got = walk(&index, &docs, from, refstr);
        assert_eq!(&got, &answers[id], "obsidian-compat probe {id}");
    }
}

/// GATE 3 — the split is observable: the mint plane refuses a duplicate anchor
/// `ambiguous_ref` (loud) while the walk plane silently resolves it on the same
/// input (last-wins per frozen §2.1 + harness WX-5).
#[test]
fn duplicate_anchor_split_mint_loud_walk_silent() {
    let (index, docs) = walkvault();
    let blocks = &docs["blocks.md"];
    let ResolveError::Ambiguous(cands) =
        model::resolve(blocks, &Ref::anchor("dup1").unwrap()).expect_err("mint refuses loud")
    else {
        panic!("mint plane must refuse a duplicate anchor with Ambiguous (ambiguous_ref)");
    };
    assert_eq!(cands.len(), 2, "two ^dup1 blocks in blocks.md");

    let loc = walk(&index, &docs, "blocks.md", "blocks#^dup1")
        .expect("walk plane resolves the duplicate silently");
    assert_eq!(loc.dest, "blocks.md");
    let last = cands.iter().max_by_key(|t| t.span.start).expect("a candidate");
    assert_eq!(loc.span, last.span, "walk silently picks the LAST ^dup1 (app-exact)");
}

/// GATE 4 — the re-homed CHARSET-GUARD walk-vs-mint split: a `_`-bearing id is
/// `bad_request` on the mint plane (out-of-grammar, loud) and `ref_not_found`
/// stage 2 on the walk plane (the app never indexes it — silent drop). Both
/// conform (decision 013 consequence 3). Matches obsidian-compat WX-6.
#[test]
fn charset_walk_vs_mint_split_underscore() {
    let (index, docs) = walkvault();
    assert_eq!(
        Ref::anchor("under-probe_x"),
        Err(BadAnchorId { id: "under-probe_x".to_string() }),
        "mint plane refuses the `_` id → bad_request"
    );
    let got = walk(&index, &docs, "walk.md", "blocks#^under-probe_x");
    assert_eq!(
        got,
        Err(Miss { stage: Stage::Two, dest: Some("blocks.md".to_string()) }),
        "walk plane follows the app: ref_not_found stage 2, dest observable"
    );
}

/// HARNESS — the gate-3 adversarial `p2-walk-probes` run against `walk.rs` over
/// the walkvault corpus. GT-pinned probes (WL-1..6b, WX-6) must match the live
/// oracle; the extra oracle predictions runnable over the corpus (WX-1/2/4/5)
/// are asserted from their laws. Probes referencing fixtures not in the corpus
/// (WX-3 `sub/walk.md`, WX-7 `alias-target.md`) and the degenerate parse-edge
/// (WX-8, pack-pinned with no in-corpus answer) are documented skips.
#[test]
fn harness_p2_walk_probes_green_over_walkvault() {
    let (index, docs) = walkvault();
    let gt = oracle_answers(&oracle_json(), &docs);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data/harness/p2-walk-probes.json");
    let probes: Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|e| panic!("read p2: {e}")))
            .expect("p2 json");

    let mut checked = 0;
    for p in probes["probes"].as_array().expect("probes") {
        let id = p["id"].as_str().expect("id");
        if matches!(id, "WX-3" | "WX-7" | "WX-8") {
            continue; // fixture not in the walkvault corpus / pack-pinned parse edge
        }
        let from = p["from"].as_str().expect("from");
        let refstr = p["ref"].as_str().expect("ref");
        let got = walk(&index, &docs, from, refstr);

        if let Some(want) = gt.get(id) {
            assert_eq!(&got, want, "harness {id} vs live obsidian-compat oracle");
        } else {
            match id {
                // case-fold + duplicate composed → the SAME node as WL-1's first Beta.
                "WX-1" => assert_eq!(got, gt["WL-1"], "WX-1 folds to WL-1's first Beta"),
                // case-insensitive slash-bearing literal segment → WL-6a's `## a/b`.
                "WX-2" => assert_eq!(got, gt["WL-6a"], "WX-2 folds to WL-6a's literal a/b"),
                // stage-1 miss: no dest (unresolved first-class).
                "WX-4" => assert_eq!(
                    got,
                    Err(Miss { stage: Stage::One, dest: None }),
                    "WX-4 stage-1 miss carries no dest"
                ),
                // duplicate block id: walk follows the app — last-wins, silent.
                "WX-5" => {
                    let loc = got.expect("WX-5 resolves silently");
                    assert_eq!(loc.dest, "blocks.md");
                    let ResolveError::Ambiguous(cands) =
                        model::resolve(&docs["blocks.md"], &Ref::anchor("dup1").unwrap())
                            .expect_err("mint dup is ambiguous")
                    else {
                        panic!("mint dup1 must be Ambiguous");
                    };
                    let last = cands.iter().max_by_key(|t| t.span.start).expect("candidate");
                    assert_eq!(loc.span, last.span, "WX-5 walk picks the LAST ^dup1");
                }
                other => panic!("unhandled runnable harness probe {other}"),
            }
        }
        checked += 1;
    }
    assert!(checked >= 11, "ran the walkvault-answerable harness probes (got {checked})");
}
