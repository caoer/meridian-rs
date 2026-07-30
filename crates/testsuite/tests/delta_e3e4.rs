//! D3-DELTA gate 1: the E3/E4 Delta fixtures byte-exact vs COMPUTED values —
//! `seq`, roots, `file_rev`s, and node entries all derived by the engine, never
//! transcribed (contract §7.1 worked frames are the assertion target).
//!
//! Fixture derivation discipline (Advisor-gated Flag A): S1/S2 states derive
//! from the frozen §4.4 worked splice requests applied to the committed S0
//! bytes — E3: `"ship by August"→"ship by September"` on plan + the §6.3 E3
//! receipt line appended; E4: `put:end "- new item\n"` on plan Q4 + the E4
//! receipt line. The derived S2 plan is cross-checked byte-identical to the
//! committed `wsfix/s2` fixture. If ANY derived value disagrees with a
//! printed §7.1 value this suite fails — the STOP condition; bytes are never
//! hand-tuned.

use serde_json::{Value, json};

/// §6.3 E3 receipt line, frozen bytes (also pinned in `crates/receipt`).
const E3_LINE: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";
/// §6.3 E4 receipt line, frozen bytes.
const E4_LINE: &str = "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z root_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043";

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The corpus root over the two §12-domain files (computed, version 0).
fn root_of(plan: &str, receipts: &str) -> wire::Root {
    let files: Vec<(&str, &[u8])> = vec![
        ("notes/plan.md", plan.as_bytes()),
        ("receipts/2026-07-18.md", receipts.as_bytes()),
    ];
    wire::Root(model::merkle_root(&files, 0).0)
}

/// One computed Delta frame between two corpus states (file order as the
/// worked frames print it: plan, then receipts).
fn computed_delta(
    seq: u64,
    plan: (&str, &str),
    receipts: (&str, &str),
    now: &str,
) -> wire::DeltaFrame {
    let files = [
        ("notes/plan.md", plan),
        ("receipts/2026-07-18.md", receipts),
    ]
    .into_iter()
    .map(|(path, (before, after))| {
        let fd =
            model::delta::file_delta(Some(&doc(before)), Some(&doc(after))).expect("state changed");
        wire_map::project_file_delta(path, &fd)
    })
    .collect();
    wire::DeltaFrame {
        delta: wire::Delta {
            seq,
            root_before: root_of(plan.0, receipts.0),
            root_after: root_of(plan.1, receipts.1),
            actor: Some("agent:b0864fb2".into()),
            now: Some(now.into()),
            files,
        },
        effects: vec![],
    }
}

/// The §7.1 printed E3 frame, verbatim.
fn e3_printed() -> Value {
    json!({"delta":{
     "seq":1,
     "root_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
     "root_after":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
     "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
     "files":[
      {"path":"notes/plan.md","change":"modified",
       "file_rev_before":"e3c4acaceb75b907","file_rev_after":"a9794a262e67ed02",
       "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q3"}],"change":"edited",
                 "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
                 "span_after":[49,75]}]},
      {"path":"receipts/2026-07-18.md","change":"modified",
       "file_rev_before":"920a40c4ee23d37c","file_rev_after":"2731acfa39bbb92c",
       "nodes":[{"anchor":"r-000042","change":"added",
                 "node_rev_after":"639a2dca46f6fcc8","span_after":[26,248]}]}]}})
}

/// The §7.1 printed E4 frame, verbatim.
fn e4_printed() -> Value {
    json!({"delta":{
     "seq":2,
     "root_before":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
     "root_after":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
     "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
     "files":[
      {"path":"notes/plan.md","change":"modified",
       "file_rev_before":"a9794a262e67ed02","file_rev_after":"5f27a2814b517680",
       "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q4"}],"change":"edited",
                 "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
                 "span_after":[75,150]}]},
      {"path":"receipts/2026-07-18.md","change":"modified",
       "file_rev_before":"2731acfa39bbb92c","file_rev_after":"9167b12b0eb13be6",
       "nodes":[{"anchor":"r-000043","change":"added",
                 "node_rev_after":"c912d4578883f288","span_after":[249,473]}]}]}})
}

struct States {
    plan: [String; 3],
    receipts: [String; 3],
}

/// Derive S0→S1→S2 from committed S0 bytes + the frozen §4.4 worked edits.
fn states() -> States {
    let plan_s0 = wsfix("s0/notes/plan.md");
    let receipts_s0 = wsfix("s0/receipts/2026-07-18.md");
    // E3: match edit inside Goals>Q3 + receipt append (§4.4/§6.3)
    let plan_s1 = plan_s0.replace("ship by August", "ship by September");
    assert_ne!(plan_s0, plan_s1, "E3 old text must occur");
    let receipts_s1 = format!("{receipts_s0}{E3_LINE}\n");
    // E4: put at:end on Goals>Q4 (span-end byte = EOF at S1) + receipt append
    let plan_s2 = format!("{plan_s1}- new item\n");
    let receipts_s2 = format!("{receipts_s1}{E4_LINE}\n");
    States {
        plan: [plan_s0, plan_s1, plan_s2],
        receipts: [receipts_s0, receipts_s1, receipts_s2],
    }
}

/// The derivation cross-check: the §4.4-derived S2 plan is byte-identical to
/// the COMMITTED s2 fixture, and the derived byte counts match §0.3.
#[test]
fn derived_states_match_committed_and_frozen_counts() {
    let s = states();
    assert_eq!(s.plan[1].len(), 139, "S1 plan bytes (§0.3)");
    assert_eq!(s.plan[2].len(), 150, "S2 plan bytes (§0.3)");
    assert_eq!(s.receipts[1].len(), 249, "S1 receipts bytes (§0.3)");
    assert_eq!(s.receipts[2].len(), 474, "S2 receipts bytes (§0.3)");
    assert_eq!(
        s.plan[2],
        wsfix("s2/notes/plan.md"),
        "derived S2 plan ≡ committed fixture"
    );
}

/// Gate 1: computed E3 and E4 Delta frames — roots via the real corpus fold,
/// revs/spans/entries via the real build+delta computation — equal the
/// printed §7.1 frames value-for-value, and read back typed.
#[test]
fn computed_e3_e4_deltas_match_frozen_frames() {
    let s = states();
    let e3 = computed_delta(
        1,
        (&s.plan[0], &s.plan[1]),
        (&s.receipts[0], &s.receipts[1]),
        "2026-07-18T20:31:04Z",
    );
    assert_eq!(serde_json::to_value(&e3).unwrap(), e3_printed(), "E3");
    let e4 = computed_delta(
        2,
        (&s.plan[1], &s.plan[2]),
        (&s.receipts[1], &s.receipts[2]),
        "2026-07-18T20:33:41Z",
    );
    assert_eq!(serde_json::to_value(&e4).unwrap(), e4_printed(), "E4");
}
