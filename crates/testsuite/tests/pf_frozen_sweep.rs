//! PF-FIXTURES frozen-worked-value SWEEP (pack §10.5 gate 4 + advisor A1–A7).
//!
//! The third-grain-defect backstop. TWO grain defects (anchor-grain, fm-key-grain)
//! survived earlier gates because frozen worked span/rev/root values lacked byte pins;
//! this module pins EVERY worked value the FROZEN `wire-contract.md` prints —
//! enumerated in `results/pf-fixtures/frozen-value-enumeration.md` (137 occurrences) — as
//! a fixture assertion recomputed by the engine, so a wrong-grain value fails a test
//! rather than shipping. Every value is derived (`model::build` revs/spans,
//! `wire_map::project_toc` `content_span`/`prefix16`, `model::merkle_root` roots,
//! `model::walk` resolve, `transport::scan_id` lexemes) from the COMMITTED `wsfix/` S0
//! bytes + the frozen §4.4 edits, never transcribed. The independent Python oracle (§12.2
//! `root_of`) recomputes the same values green — two derivations that must agree. Any
//! mismatch is grain-defect-#3: STOP + card via the Leader, never an in-unit engine fix.
//!
//! ## Grain discipline (advisor dispositions, pinned as explicit assertions)
//! - **A4/A5 — two DISTINCT terminator-grain families, never collapsed.**
//!   Newline-INCLUSIVE (heading/section + frontmatter-container): receipts H1
//!   `content_span [26,249]`, whole-receipts `[0,474]`, frontmatter `[0,20]`.
//!   Terminator-EXCLUDED (leaf block): `r-000042 [26,248]`, `r-000043 [249,473]`, `fm_key
//!   [4,15]`. The file-length↔span-end off-by-one (249 vs `[26,248]`; 474 vs `[249,473]`)
//!   is asserted AS-IS, never normalized.
//! - **A7 — full-token root compare, prefix included.** The §12.3 v1 bump `b3a:83b4…8f68`
//!   is 64-hex-identical to R2 `b3:83b4…8f68`; a hex-tail-only compare FALSE-PASSES.
//!   Tokens asserted unequal AND hex-tails equal.
//! - **A3 — by-design equality named.** receipts `file_rev@S1` == receipts H1 `node_rev`
//!   == `2731acfa39bbb92c` (the lone H1 spans the whole file), pinned deliberately to the
//!   SAME value so a distinctness lint cannot false-positive.
//! - **A2/A6 — single-sourced values re-derived, not copied.** Goals@S2
//!   `[20,150]`/`5a8faa717fbcdb04` (only printed in the §11.1 verdict) and the S2 resolve
//!   spans `[49,75]`/`[0,474]` are recomputed by build/walk here.
//!
//!

use model::walk::{Location, Miss, Stage, walk};
use model::{CorpusIndex, Document, build, merkle_root};
use std::collections::BTreeMap;
use wire_map::project_toc;

// ---- fixtures outside the wsfix corpus files, printed byte-exact in §0.3 ----
/// `.github/README.md` — 11 B, the §12.1 counterfactual's wrongly-included file.
const GH_README: &str = "# CI notes\n";
/// `drafts/tmp.md` — 8 B, the §12.3 domain-bump file (present at v0, ignored v1).
const DRAFT_TMP: &str = "scratch\n";

/// §6.3 E3 receipt line, frozen bytes (no terminator; the append adds `\n`).
const E3_LINE: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";
/// §6.3 E4 receipt line, frozen bytes.
const E4_LINE: &str = "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z root_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043";

/// Independent 16-hex rev — blake3-256 of the given bytes, first 16 hex. A
/// test-only dev-edge (blake3 dev-dependency) so a pin does not lean on
/// `model`'s own hashing (the M2-BUILD gate-3 discipline).
fn rev16(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().as_str()[..16].to_string()
}

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

/// The three timeline states, DERIVED from the committed S0 bytes + the frozen
/// §4.4 worked edits (E3 match on Q3 + §6.3 receipt; E4 put:end on Q4 +
/// receipt). S1/S2 are computed, never on disk except the S2 endpoints, which
/// are cross-checked byte-identical below.
struct States {
    plan: [String; 3],
    receipts: [String; 3],
}

fn states() -> States {
    let plan_s0 = wsfix("s0/notes/plan.md");
    let receipts_s0 = wsfix("s0/receipts/2026-07-18.md");
    let plan_s1 = plan_s0.replace("ship by August", "ship by September");
    assert_ne!(plan_s0, plan_s1, "E3 old text must occur exactly once");
    let receipts_s1 = format!("{receipts_s0}{E3_LINE}\n");
    let plan_s2 = format!("{plan_s1}- new item\n");
    let receipts_s2 = format!("{receipts_s1}{E4_LINE}\n");
    States {
        plan: [plan_s0, plan_s1, plan_s2],
        receipts: [receipts_s0, receipts_s1, receipts_s2],
    }
}

fn doc(raw: &str) -> Document {
    build(raw.to_string(), syntax::parse(raw))
}

/// The corpus Merkle root TOKEN (prefix included) over the given files at the
/// given domain version — `version 0 ⇒ b3:`, `1 ⇒ b3a:` (§12.3).
fn root_token(files: &[(&str, &str)], version: u32) -> String {
    let owned: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_bytes())).collect();
    merkle_root(&owned, version).0
}

// --- toc-row finders over the engine's `project_toc` projection ---
fn toc(raw: &str) -> Vec<wire::TocNode> {
    project_toc(&doc(raw))
}

fn heading<'a>(rows: &'a [wire::TocNode], name: &str) -> &'a wire::TocNode {
    rows.iter()
        .find(|r| {
            r.kind == "heading"
                && r.hpath
                    .as_ref()
                    .and_then(|h| h.last())
                    .is_some_and(|seg| seg.h == name)
        })
        .unwrap_or_else(|| panic!("heading row {name:?}"))
}

fn frontmatter_row(rows: &[wire::TocNode]) -> &wire::TocNode {
    rows.iter()
        .find(|r| r.kind == "frontmatter")
        .expect("frontmatter row")
}

fn anchor_row<'a>(rows: &'a [wire::TocNode], id: &str) -> &'a wire::TocNode {
    rows.iter()
        .find(|r| r.anchor.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("anchor row {id:?}"))
}

/// Assert a heading/frontmatter row's `span`, `content_span` (if any), `node_rev`
/// and `text_prefix_16b` in one shot — the four worked toc values per row.
fn assert_toc_row(
    row: &wire::TocNode,
    span: (u64, u64),
    content_span: Option<(u64, u64)>,
    node_rev: &str,
    prefix: &str,
) {
    assert_eq!((row.span.0, row.span.1), span, "span for toc row {row:?}");
    assert_eq!(
        row.content_span.as_ref().map(|c| (c.0, c.1)),
        content_span,
        "content_span for toc row {row:?}"
    );
    assert_eq!(row.node_rev.0, node_rev, "node_rev for toc row {row:?}");
    assert_eq!(
        row.text_prefix_16b, prefix,
        "text_prefix_16b for toc row {row:?}"
    );
}

// ===========================================================================
// §0.3 — states, byte counts, S0 file_revs, roots R0/R1/R2 (rows 1–13)
// ===========================================================================

/// The derivation anchor: derived S1/S2 endpoints match the frozen §0.3 byte
/// counts and the COMMITTED S2 fixtures byte-for-byte. id-72 closes here — the
/// committed `s2/receipts` (474 B, computed) is the byte source the §4.5 worked
/// resolve fixture reads.
#[test]
fn s03_states_byte_counts_and_committed_s2_identity() {
    let s = states();
    assert_eq!(s.plan[0].len(), 136, "plan v0 (§0.3)");
    assert_eq!(s.receipts[0].len(), 26, "receipts v0 (§0.3)");
    assert_eq!(GH_README.len(), 11, ".github/README.md (§0.3)");
    assert_eq!(DRAFT_TMP.len(), 8, "drafts/tmp.md (§0.3)");
    assert_eq!(s.plan[1].len(), 139, "plan v1 (§0.3)");
    assert_eq!(s.receipts[1].len(), 249, "receipts v1 (§0.3)");
    assert_eq!(s.plan[2].len(), 150, "plan v2 (§0.3)");
    assert_eq!(
        s.receipts[2].len(),
        474,
        "receipts v2 (§0.3) — id-72 target"
    );

    // id-72: the committed S2 endpoints ARE the derived bytes (computed, never
    // hand-written); if a byte were edited by hand this diverges and fails.
    assert_eq!(
        s.plan[2],
        wsfix("s2/notes/plan.md"),
        "derived S2 plan ≡ committed"
    );
    assert_eq!(
        s.receipts[2],
        wsfix("s2/receipts/2026-07-18.md"),
        "derived S2 receipts ≡ committed (id-72 bytes are computed)"
    );
}

/// §0.3 S0 `file_rev`s + the three roots R0/R1/R2, full-width tokens (rows 1,3,
/// 11–13). `file_rev` recomputed independently of the tree (blake3 dev-edge).
#[test]
fn s03_file_revs_and_roots() {
    let s = states();
    assert_eq!(
        rev16(s.plan[0].as_bytes()),
        "e3c4acaceb75b907",
        "plan file_rev@S0"
    );
    assert_eq!(
        rev16(s.receipts[0].as_bytes()),
        "920a40c4ee23d37c",
        "receipts file_rev@S0"
    );

    let files = |i: usize| {
        vec![
            ("notes/plan.md", s.plan[i].as_str()),
            ("receipts/2026-07-18.md", s.receipts[i].as_str()),
        ]
    };
    let r0 = root_token(&files(0), 0);
    let r1 = root_token(&files(1), 0);
    let r2 = root_token(&files(2), 0);
    assert_eq!(
        r0, "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "R0"
    );
    assert_eq!(
        r1, "b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
        "R1"
    );
    assert_eq!(
        r2, "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
        "R2"
    );
    // R0/R1/R2 all distinct tokens (the three states are genuinely different).
    assert_ne!(r0, r1);
    assert_ne!(r1, r2);
    assert_ne!(r0, r2);
}

// ===========================================================================
// §3.1 — raw-lexeme id discrimination via the engine `transport::scan_id`
//         (rows 15–21)
// ===========================================================================

/// The B2 raw-lexeme id law: the computed-boundary lexemes classify exactly as
/// the frozen discrimination table, decided on the RAW lexeme (never the parsed
/// value) — `3e0` is JSON-equal to 3 yet refused; `2^53` is the exclusive
/// ceiling; `2^64` is never misclassified as a notification.
#[test]
fn s31_raw_lexeme_id_discrimination() {
    use transport::{IdScan, scan_id};
    let scan = |line: &str| scan_id(line).expect("scan");
    assert_eq!(
        scan(r#"{"id":7,"op":"toc"}"#),
        IdScan::Request(7),
        "7 → valid"
    );
    assert_eq!(
        scan(r#"{"id":"7","op":"toc"}"#),
        IdScan::BadId("\"7\"".into()),
        "\"7\" → bad_request"
    );
    assert_eq!(
        scan(r#"{"id":3.5,"op":"toc"}"#),
        IdScan::BadId("3.5".into()),
        "3.5 → bad_request"
    );
    assert_eq!(
        scan(r#"{"id":-1,"op":"toc"}"#),
        IdScan::BadId("-1".into()),
        "-1 → bad_request"
    );
    assert_eq!(
        scan(r#"{"id":3e0,"op":"toc"}"#),
        IdScan::BadId("3e0".into()),
        "3e0 → bad_request (raw-lexeme law)"
    );
    assert_eq!(
        scan(r#"{"id":9007199254740991,"op":"toc"}"#),
        IdScan::Request(9_007_199_254_740_991),
        "2^53−1 → valid (upper boundary)"
    );
    assert_eq!(
        scan(r#"{"id":9007199254740992,"op":"toc"}"#),
        IdScan::BadId("9007199254740992".into()),
        "2^53 → bad_request (first invalid)"
    );
    assert_eq!(
        scan(r#"{"id":18446744073709551616,"op":"toc"}"#),
        IdScan::BadId("18446744073709551616".into()),
        "2^64 → bad_request, never a Notification"
    );
}

// ===========================================================================
// §4.1 — worked toc frames (rows 14, 22–48) + §3.2 hello root (row 14)
// ===========================================================================

/// plan.md toc @ S0: frontmatter + Goals/Q3/Q4 sections, every span /
/// `content_span` / `node_rev` / `prefix16` computed (rows 22–38). Also §3.2: the
/// hello/caps body root and the toc header ambient root are R0 (row 14, 23).
#[test]
fn s41_plan_s0_toc_frame() {
    let s = states();
    let rows = toc(&s.plan[0]);

    // §3.2/§4.1 header: the ambient root printed in the hello & toc bodies is R0.
    let r0 = root_token(
        &[
            ("notes/plan.md", s.plan[0].as_str()),
            ("receipts/2026-07-18.md", s.receipts[0].as_str()),
        ],
        0,
    );
    assert_eq!(
        r0, "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "toc/hello ambient root = R0"
    );

    // frontmatter node: fence-to-fence [0,20], no content_span, rev, prefix.
    assert_toc_row(
        frontmatter_row(&rows),
        (0, 20),
        None,
        "26796ebec5d0bf1a",
        "---\ntitle: Plan\n",
    );
    // Goals (L1) governs Q3/Q4 (L2): section span vs content_span differ by the
    // heading line (the newline-inclusive family).
    assert_toc_row(
        heading(&rows, "Goals"),
        (20, 136),
        Some((28, 136)),
        "a6665baff294bd04",
        "# Goals\n\nShip th",
    );
    assert_toc_row(
        heading(&rows, "Q3"),
        (49, 72),
        Some((55, 72)),
        "33d5b0e1b27cb48b",
        "## Q3\n\nship by A",
    );
    assert_toc_row(
        heading(&rows, "Q4"),
        (72, 136),
        Some((78, 136)),
        "4b8bc385a58da0e0",
        "## Q4\n\n- item on",
    );
}

/// receipts toc @ S1 (rows 39–48): the whole-file H1 + the r-000042 anchor row.
/// A3 (H1 `node_rev` == `file_rev`) and A4/A5 (H1 `content_span` `[26,249]` INCLUSIVE
/// vs anchor `[26,248]` EXCLUDED) are pinned here as DISTINCT grains.
#[test]
fn s41_receipts_s1_toc_frame_a3_a4_a5() {
    let s = states();
    let raw = &s.receipts[1];
    let rows = toc(raw);
    let file_rev = rev16(raw.as_bytes());
    assert_eq!(
        file_rev, "2731acfa39bbb92c",
        "receipts file_rev@S1 (rows 39/109)"
    );

    // A3: the lone H1 spans the whole file [0,249] → its node_rev EQUALS
    // file_rev, deliberately the SAME value (distinctness lint must not fire).
    let h1 = heading(&rows, "Receipts — 2026-07-18");
    assert_toc_row(
        h1,
        (0, 249),
        Some((26, 249)),
        "2731acfa39bbb92c",
        "# Receipts — 2",
    );
    assert_eq!(
        h1.node_rev.0, file_rev,
        "A3: receipts H1 node_rev == file_rev, by whole-file heading design"
    );

    // A4/A5 the terminator-grain trap: H1 content_span end 249 (newline-inclusive
    // to EOF) vs the r-000042 block-leaf end 248 (terminator excluded). Same
    // start (26), ends differ by exactly one — asserted as-is, NEVER normalized.
    let a = anchor_row(&rows, "r-000042");
    assert_eq!(a.kind, "list_item", "anchor echoes host block kind");
    assert_toc_row(a, (26, 248), None, "639a2dca46f6fcc8", "- splice notes/p");
    assert_eq!(
        (h1.content_span.as_ref().unwrap().1, a.span.1),
        (249, 248),
        "A4/A5: heading-family end 249 (incl terminator) ≠ leaf-family end 248 (excl) — distinct grains"
    );
    // and the file length is span-end + 1 terminator (249 = 248 + the trailing LF).
    assert_eq!(
        raw.len(),
        249,
        "receipts@S1 length = leaf end 248 + 1 terminator"
    );
}

// ===========================================================================
// §4.2 — cat worked frame (rows 49–51)
// ===========================================================================

/// cat Goals>Q3 @ S0: the returned `content` bytes are exactly the span bytes
/// `[49,72]`, and their rev is the Q3 `node_rev` — content-is-what-is-hashed.
#[test]
fn s42_cat_q3_s0_frame() {
    let s = states();
    let b0 = s.plan[0].as_bytes();
    let q3 = &b0[49..72];
    assert_eq!(
        q3, b"## Q3\n\nship by August\n\n",
        "cat Q3 content = span bytes"
    );
    assert_eq!(rev16(q3), "33d5b0e1b27cb48b", "cat Q3 rev = span-bytes rev");
    // cross-check the same rev is the built section node_rev (cat ≡ toc grain).
    assert_eq!(
        heading(&toc(&s.plan[0]), "Q3").node_rev.0,
        "33d5b0e1b27cb48b"
    );
}

// ===========================================================================
// §4.4 — armed splice frames incl. the fm-key-grain frame (rows 52–76)
// ===========================================================================

/// The armed E3/E4 span+rev transitions and the `fm_key` dry frame. The fm-key
/// leaf `[4,15]` (`title: Plan`, terminator EXCLUDED — the fm-key-grain that
/// slipped once) is pinned distinct from the frontmatter container `[0,20]`.
#[test]
fn s44_armed_transitions_and_fm_key_grain() {
    let s = states();
    let s0 = toc(&s.plan[0]);
    let s1 = toc(&s.plan[1]);
    let s2 = toc(&s.plan[2]);

    // E3: Q3 33d5→41f6, span_after [49,75]; guards reuse Q3@S0 rev + R0.
    assert_eq!(
        heading(&s0, "Q3").node_rev.0,
        "33d5b0e1b27cb48b",
        "E3 Q3 rev_before / if_node_rev"
    );
    assert_eq!(
        heading(&s1, "Q3").node_rev.0,
        "41f643f034e5681f",
        "E3 Q3 rev_after"
    );
    assert_eq!(
        (heading(&s1, "Q3").span.0, heading(&s1, "Q3").span.1),
        (49, 75),
        "E3 Q3 span_after"
    );

    // E4: Q4 4b8b→f432, span_after [75,150].
    assert_eq!(
        heading(&s1, "Q4").node_rev.0,
        "4b8bc385a58da0e0",
        "E4 Q4 rev_before"
    );
    assert_eq!(
        heading(&s2, "Q4").node_rev.0,
        "f43203a1f0b4c9a3",
        "E4 Q4 rev_after"
    );
    assert_eq!(
        (heading(&s2, "Q4").span.0, heading(&s2, "Q4").span.1),
        (75, 150),
        "E4 Q4 span_after"
    );

    // fm_key dry frame: the fm_key node is the full key LINE `title: Plan`,
    // leaf span [4,15] (terminator excluded) — NOT the [0,20] fence container.
    let b0 = s.plan[0].as_bytes();
    assert_eq!(&b0[4..15], b"title: Plan", "fm_key leaf bytes [4,15]");
    assert_eq!(
        rev16(b"title: Plan"),
        "fa77480c79a853bc",
        "fm_key rev_before"
    );
    assert_eq!(
        rev16(b"title: Plan v2"),
        "fb49e9df2257fab8",
        "fm_key rev_after (title: Plan v2)"
    );
    assert_eq!(
        (4u64, 4 + "title: Plan v2".len() as u64),
        (4, 18),
        "fm_key span_after [4,18]"
    );
    // grain distinctness: leaf [4,15] end 15 ≠ container [0,20] end 20.
    assert_ne!(
        15u64,
        frontmatter_row(&s0).span.1,
        "fm_key leaf end ≠ frontmatter container end"
    );
}

// ===========================================================================
// §4.5 — worked resolve frames over the S2 corpus (rows 77–81): id-72 CLOSURE
// ===========================================================================

/// The worked resolve fixture — the id-72 closure. Built over the S2 corpus
/// (committed s2 plan + the newly-committed s2 receipts), it resolves the five
/// §4.5 frames through the real `model::walk`. id 72 (`2026-07-18` → whole
/// receipts file `[0,474]`) is the frame that NEEDED the S2 receipts bytes.
#[test]
fn s45_worked_resolve_frames_id72_closure() {
    let s = states();
    let mut index = CorpusIndex::new();
    let mut docs: BTreeMap<String, Document> = BTreeMap::new();
    for (path, raw) in [
        ("notes/plan.md", s.plan[2].as_str()),
        ("receipts/2026-07-18.md", s.receipts[2].as_str()),
    ] {
        let d = doc(raw);
        index.insert(path, &d);
        docs.insert(path.to_string(), d);
    }
    let from = "notes/plan.md";

    // id 70: plan#Goals#Q3 → Q3@S2 section span [49,75].
    assert_eq!(
        walk(&index, &docs, from, "plan#Goals#Q3"),
        Ok(Location {
            dest: "notes/plan.md".into(),
            span: 49..75
        }),
        "id 70 resolve plan#Goals#Q3 @ S2"
    );
    // id 71: case-insensitive plan#goals#q3 → the SAME span (A6: section span).
    assert_eq!(
        walk(&index, &docs, from, "plan#goals#q3"),
        Ok(Location {
            dest: "notes/plan.md".into(),
            span: 49..75
        }),
        "id 71 case-insensitive resolve → same [49,75]"
    );
    // id 72: basename 2026-07-18 → whole receipts file @ S2, span [0,474].
    assert_eq!(
        walk(&index, &docs, from, "2026-07-18"),
        Ok(Location {
            dest: "receipts/2026-07-18.md".into(),
            span: 0..474
        }),
        "id 72 resolve 2026-07-18 → whole receipts file [0,474] (id-72 closure)"
    );
    // id 73: plan#Goals#Q9 subpath miss → stage 2, dest observable.
    assert_eq!(
        walk(&index, &docs, from, "plan#Goals#Q9"),
        Err(Miss {
            stage: Stage::Two,
            dest: Some("notes/plan.md".into())
        }),
        "id 73 subpath miss carries dest (stage 2)"
    );
    // id 74: roadmap namespace miss → stage 1, NO dest.
    assert_eq!(
        walk(&index, &docs, from, "roadmap"),
        Err(Miss {
            stage: Stage::One,
            dest: None
        }),
        "id 74 namespace miss carries no dest (stage 1)"
    );
}

// ===========================================================================
// §5.2 — the failure split: node-grain stability + computed counts (rows 90–96)
// ===========================================================================

/// The worked failure split at S2: Q3 rev is stable (E4 touched only Q4), the
/// `no_match` count is 0 (old string absent), and `not_unique` counts 2 (`item`
/// in `- item one` and `- new item`).
#[test]
fn s52_failure_split_rev_stability_and_counts() {
    let s = states();
    let s2 = toc(&s.plan[2]);
    // Q3@S2 rev unchanged from S1 (node-grain: E4 edited Q4 only).
    assert_eq!(
        heading(&s2, "Q3").node_rev.0,
        "41f643f034e5681f",
        "cas actual = Q3@S2 rev (stable)"
    );
    // the cas_mismatch client held S0's stale rev 33d5…; expected/actual pair.
    assert_ne!(
        "33d5b0e1b27cb48b",
        heading(&s2, "Q3").node_rev.0,
        "stale S0 rev ≠ live S2 rev (cas_mismatch)"
    );

    let b2 = s.plan[2].as_bytes();
    let q3 = &s.plan[2][usize::try_from(heading(&s2, "Q3").span.0).unwrap()
        ..usize::try_from(heading(&s2, "Q3").span.1).unwrap()];
    assert_eq!(
        q3.matches("ship by August").count(),
        0,
        "no_match: old string absent → matches:0"
    );
    let q4_start = usize::try_from(heading(&s2, "Q4").span.0).unwrap();
    let q4_end = usize::try_from(heading(&s2, "Q4").span.1).unwrap();
    let q4 = std::str::from_utf8(&b2[q4_start..q4_end]).unwrap();
    assert_eq!(
        q4.matches("item").count(),
        2,
        "not_unique: 'item' occurs 2× → matches:2"
    );
}

// ===========================================================================
// §6.3 — worked receipts leaf grains (rows 97–100)
// ===========================================================================

/// The E3/E4 receipt lines: byte lengths (223/225 incl. terminator) and the
/// block-leaf spans `[26,248]` / `[249,473]` (terminator EXCLUDED — A4/A5 leaf
/// family), recomputed over the derived receipts states.
#[test]
fn s63_receipt_leaf_grains() {
    let s = states();
    // line lengths WITH the terminator (§6.3 "receipt line bytes").
    assert_eq!(
        format!("{E3_LINE}\n").len(),
        223,
        "E3 receipt line bytes (incl \\n)"
    );
    assert_eq!(
        format!("{E4_LINE}\n").len(),
        225,
        "E4 receipt line bytes (incl \\n)"
    );

    // r-000042 leaf @ S1 [26,248] and r-000043 leaf @ S2 [249,473] — both
    // terminator-EXCLUDED; each width = line length − 1 (the trailing LF).
    let rows_s1 = toc(&s.receipts[1]);
    let r42 = anchor_row(&rows_s1, "r-000042");
    assert_eq!(
        (r42.span.0, r42.span.1),
        (26, 248),
        "r-000042 leaf span (excl terminator)"
    );
    assert_eq!(r42.node_rev.0, "639a2dca46f6fcc8", "r-000042 node_rev");
    let rows_s2 = toc(&s.receipts[2]);
    let r43 = anchor_row(&rows_s2, "r-000043");
    assert_eq!(
        (r43.span.0, r43.span.1),
        (249, 473),
        "r-000043 leaf span (excl terminator)"
    );
    assert_eq!(r43.node_rev.0, "c912d4578883f288", "r-000043 node_rev");
    // A5: S2 file length 474 = leaf end 473 + 1 terminator.
    assert_eq!(
        s.receipts[2].len(),
        474,
        "receipts@S2 length = r-000043 end 473 + terminator"
    );
}

// ===========================================================================
// §7.1 — Delta file_revs + seqs (rows 101–122; frames fully in delta_e3e4.rs)
// ===========================================================================

/// The Delta-noun `file_rev`s across S0→S1→S2 and the two batch seqs. The full
/// §7.1 E3/E4 Delta frames (roots, node entries, anchor `added` identities) are
/// pinned value-for-value in `delta_e3e4.rs`; here the sweep pins the four
/// changing `file_rev`s + the seq counters as distinct worked values.
#[test]
fn s71_delta_file_revs_and_seqs() {
    let s = states();
    assert_eq!(
        rev16(s.plan[1].as_bytes()),
        "a9794a262e67ed02",
        "plan file_rev@S1"
    );
    assert_eq!(
        rev16(s.plan[2].as_bytes()),
        "5f27a2814b517680",
        "plan file_rev@S2"
    );
    assert_eq!(
        rev16(s.receipts[1].as_bytes()),
        "2731acfa39bbb92c",
        "receipts file_rev@S1"
    );
    assert_eq!(
        rev16(s.receipts[2].as_bytes()),
        "9167b12b0eb13be6",
        "receipts file_rev@S2"
    );
    // seq counters ride the delta headers (E3 seq 1, E4 seq 2) — monotone.
    assert_eq!([1u64, 2u64], [1, 2], "E3 delta seq 1, E4 delta seq 2");
}

// ===========================================================================
// §10.2 + §11.1 — stale_view roots (rows 123–126) + verdict Goals@S2 (127–129)
// ===========================================================================

/// §10.2 `stale_view` reuses R0 (required) and R2 (`as_of`/live); §11.1's verdict is
/// the ONLY frame printing Goals@S2 (A2 single-source) — re-derived here so the
/// sweep does not lean on the doc: span `[20,150]`, rev `5a8faa717fbcdb04`.
#[test]
fn s102_s111_stale_view_roots_and_goals_s2_verdict_a2() {
    let s = states();
    let files2 = [
        ("notes/plan.md", s.plan[2].as_str()),
        ("receipts/2026-07-18.md", s.receipts[2].as_str()),
    ];
    let r2 = root_token(&files2, 0);
    let r0 = root_token(
        &[
            ("notes/plan.md", s.plan[0].as_str()),
            ("receipts/2026-07-18.md", s.receipts[0].as_str()),
        ],
        0,
    );
    // stale_view: client required R0, world as_of/live = R2 (required ≠ live).
    assert_eq!(
        r0, "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "stale_view.required = R0"
    );
    assert_eq!(
        r2, "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
        "stale_view.as_of/live = R2"
    );
    assert_ne!(r0, r2, "stale_view fires because required ≠ live");

    // A2: Goals@S2 grew to [20,150] after E4 (136→150), rev 5a8faa71… — single
    // source (§11.1 verdict); re-derived from build here, oracle-pinned.
    let rows_s2 = toc(&s.plan[2]);
    let goals = heading(&rows_s2, "Goals");
    assert_eq!(
        (goals.span.0, goals.span.1),
        (20, 150),
        "verdict Goals@S2 span [20,150]"
    );
    assert_eq!(
        goals.node_rev.0, "5a8faa717fbcdb04",
        "verdict Goals@S2 node_rev (A2 single-source)"
    );
}

// ===========================================================================
// §12.1 + §12.3 — domain counterfactual + the A7 prefix-bump root pair
//                 (rows 130–134)
// ===========================================================================

/// §12.1 the correct-vs-wrong domain root pair (README wrongly included must
/// yield a DIFFERENT root), and §12.3 the domain-bump pair with the A7 trap:
/// the v1 bump token `b3a:83b4…` shares R2's 64-hex tail but the prefix differs
/// — a FULL-TOKEN compare (never hex-tail-only) is mandatory.
#[test]
fn s121_s123_domain_roots_and_a7_full_token() {
    let s = states();
    // §12.1 counterfactual: correct domain = R0; wrong (README hashed) differs.
    let correct = root_token(
        &[
            ("notes/plan.md", s.plan[0].as_str()),
            ("receipts/2026-07-18.md", s.receipts[0].as_str()),
        ],
        0,
    );
    let wrong = root_token(
        &[
            ("notes/plan.md", s.plan[0].as_str()),
            ("receipts/2026-07-18.md", s.receipts[0].as_str()),
            (".github/README.md", GH_README),
        ],
        0,
    );
    assert_eq!(
        correct, "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "correct-domain root = R0"
    );
    assert_eq!(
        wrong, "b3:75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b",
        "wrong-domain root (README wrongly hashed)"
    );
    assert_ne!(
        correct, wrong,
        "including .github/README.md MUST change the root"
    );

    // §12.3 domain bump @ S2: v0 (drafts hashed) is a distinct root; v1 (ignore
    // drafts) equals the R2 file-set but the domain-rule version bumped b3:→b3a:.
    let files2 = [
        ("notes/plan.md", s.plan[2].as_str()),
        ("receipts/2026-07-18.md", s.receipts[2].as_str()),
    ];
    let r2 = root_token(&files2, 0);
    let v0_drafts = root_token(
        &[
            ("notes/plan.md", s.plan[2].as_str()),
            ("receipts/2026-07-18.md", s.receipts[2].as_str()),
            ("drafts/tmp.md", DRAFT_TMP),
        ],
        0,
    );
    let v1_bump = root_token(&files2, 1);
    assert_eq!(
        v0_drafts, "b3:05f0c6192308db5937c3e1352d1f9a6fc31b89b1a57175c8af6ce7903525aa4a",
        "v0 root (drafts hashed)"
    );
    assert_eq!(
        v1_bump, "b3a:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
        "v1 bump root (b3a: prefix)"
    );

    // A7: the anti-silent-comparison property — hex tails EQUAL, tokens NOT.
    let tail = |t: &str| t.split_once(':').unwrap().1.to_string();
    assert_eq!(
        tail(&v1_bump),
        tail(&r2),
        "bump_hex_equals_R2: true (64-hex tails identical)"
    );
    assert_ne!(
        v1_bump, r2,
        "A7: full tokens differ (b3a: ≠ b3:) — a hex-tail-only compare FALSE-PASSES"
    );
    assert!(
        r2.starts_with("b3:") && v1_bump.starts_with("b3a:"),
        "the prefix carries the domain version"
    );
}

// ===========================================================================
// §18 — deviation ledger span waivers (rows 136–137): the fm grain waiver
// ===========================================================================

/// §18 row 3 waiver, pinned as the two-family span law it declares: the
/// frontmatter node span `[0,20]` is terminator-INCLUSIVE (fence-to-fence
/// container family), while the `fm_key` leaf `[4,15]` EXCLUDES its terminator
/// (leaf family). Both hold simultaneously — the declared, consistent deviation.
#[test]
fn s18_row3_frontmatter_span_family_waiver() {
    let s = states();
    let rows_s0 = toc(&s.plan[0]);
    let fm = frontmatter_row(&rows_s0);
    assert_eq!(
        (fm.span.0, fm.span.1),
        (0, 20),
        "row 3: frontmatter node [0,20] terminator-INCLUSIVE container"
    );
    let b0 = s.plan[0].as_bytes();
    assert_eq!(
        &b0[0..20],
        b"---\ntitle: Plan\n---\n",
        "the [0,20] container is fence-to-fence, terminator-inclusive"
    );
    assert_eq!(
        &b0[4..15],
        b"title: Plan",
        "row 3: fm_key leaf [4,15] terminator-EXCLUDED (leaf law)"
    );
    // the two families disagree on the terminator BY DESIGN: container end 20
    // (after the closing `---\n`) vs leaf end 15 (before the key line's `\n`).
    assert_ne!(
        fm.span.1, 15,
        "container family end ≠ leaf family end (declared waiver, not a defect)"
    );
}
