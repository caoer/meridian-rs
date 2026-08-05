//! Def-conformance goldens — byte-exact against the U0 fixture set.
//! `$SESSION` is substituted for the scratch root.
//!
//!
//!
//!
//!
//!
//!
#![allow(clippy::too_many_lines)] // one sequential script fn by design (env is global)

use std::fs;
use std::path::{Path, PathBuf};

use policy::defs::{ConformanceRequest, ConformanceResult, conformance};

// --- the corpus bytes (identical to the Go-side constants) ---

const DEF_PROBE: &str = "---\ntype: def\ndefines: probe\nversion: 1\n---\n\n# Properties\n\n```yaml\ntype:      {shape: line, required: true, default: probe}\nstatus:    {shape: line, required: true, suggest: [open], terminal: [done, retired]}\nclosed_at: {shape: iso, stamp: close}\npriority:  {shape: int}\ntags:      {shape: list(line)}\n```\n^properties\n\n# Sections\n\n## section: Log\n```yaml\nentry: \"- {date}: {note}\"\nlegacy-mark: \"#legacy\"\n```\n\n## section: Answer\n```yaml\nrequired-before-terminal: true\n```\n";

const DEF_PROBE_FAST: &str = "---\ntype: def\ndefines: probe\nversion: 1\n---\n\n# Properties\n\n```yaml\npriority: {shape: line}\n```\n^properties\n";

const DEF_HOME_PROBE: &str = "---\ntype: def\ndefines: probe\nversion: 1\n---\n\n# Properties\n\n```yaml\nseverity: {shape: int}\n```\n^properties\n";

const DEF_BROKEN: &str = "---\ntype: def\ndefines: broken\nversion: 1\n---\n\n# Properties\n\n```yaml\nstatus: {shape: sparkle}\n```\n^properties\n";

const REC_ITEM: &str = "---\ntype: probe\nstatus: open\nclosed_at:\npriority: 3\ntags: [probe/x]\n---\n\n# Log\n\n- 2026-01-01: seeded\n\n# Answer\n\nthe answer body\n\n# Scratch\n\nfreeform notes\n";

const REC_FAST: &str = "---\ntype: probe\npreset: fast\nstatus: open\nclosed_at:\n---\n\n# Log\n\n- 2026-01-01: seeded\n\n# Answer\n\nwill be cleared\n";

const REC_BROKEN: &str = "---\ntype: broken\nstatus: open\n---\n\n# Notes\n\nbroken body\n";

const REC_MYSTERY: &str = "---\ntype: mystery\n---\n\n# Notes\n\nmystery body\n";

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn run(target: &str, prev_raw: &str, next_raw: &str) -> ConformanceResult {
    let prev = doc(prev_raw);
    let next = doc(next_raw);
    conformance(&ConformanceRequest {
        target,
        actor: "w1234567",
        force: false,
        now: "2026-07-24T13:00:00Z",
        prev: &prev,
        next: &next,
        build_doc: &|raw| doc(raw),
    })
}

/// Golden verdict-refusal face: `{message} — {remedy}` ("put: " prefix is the
/// Go face's). `$SESSION` → the scratch root.
fn assert_refusal(res: &ConformanceResult, want: &str, session: &str, case: &str) {
    let want = want.replace("$SESSION", session);
    let got = match &res.refuse {
        Some(e) => format!("{} — {}", e.message, e.remedy),
        None => panic!("{}: expected a refusal, got pass: {:?}", case, res.repairs),
    };
    assert_eq!(got, want, "{case}: refusal bytes diverged from the golden");
}

fn assert_pass(res: &ConformanceResult, case: &str) {
    assert!(
        res.refuse.is_none(),
        "{}: expected pass, got refusal: {:?}",
        case,
        res.refuse.as_ref().map(policy::defs::BodyError::render)
    );
}

/// std-only scratch dir (no tempfile dep), removed on drop.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn defs_conformance_matches_the_u0_goldens() {
    let root = std::env::temp_dir().join(format!("defs-conformance-{}", std::process::id()));
    let _cleanup = Scratch(root.clone());
    for (rel, content) in [
        ("defs/probe.md", DEF_PROBE),
        ("defs/probe-fast.md", DEF_PROBE_FAST),
        ("defs/broken.md", DEF_BROKEN),
        ("records/item.md", REC_ITEM),
        ("records/fast.md", REC_FAST),
        ("records/broken-rec.md", REC_BROKEN),
        ("records/mystery.md", REC_MYSTERY),
        ("home/defs/probe.md", DEF_HOME_PROBE),
    ] {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
    }
    // Hermetic far layer: $UCC_HOME/defs = <root>/home/defs. Edition 2024:
    // env mutation is unsafe (single-threaded here — one test fn by design).
    unsafe { std::env::set_var("UCC_HOME", root.join("home")) };
    let session = root.to_str().unwrap().to_string();
    let item_p = root.join("records/item.md");
    let item = item_p.to_str().unwrap();
    let fast_p = root.join("records/fast.md");
    let fast = fast_p.to_str().unwrap();

    // 1. stratum 1 — int shape refusal (error rung).
    assert_refusal(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace("priority: 3", "priority: abc"),
        ),
        "write violates the probe def: priority: value abc does not satisfy shape int — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/item.md` explains the def) — errors are never forceable",
        &session,
        "shape-bad-int",
    );

    // 2. list shape refusal.
    assert_refusal(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace("tags: [probe/x]", "tags: notalist"),
        ),
        "write violates the probe def: tags: value notalist does not satisfy shape list(line) — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/item.md` explains the def) — errors are never forceable",
        &session,
        "shape-bad-list",
    );

    // 3. iso shape + biconditional compose ("(and 1 more)").
    assert_refusal(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace("closed_at:", "closed_at: notatime"),
        ),
        "write violates the probe def: closed_at: value notatime does not satisfy shape iso (and 1 more) — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/item.md` explains the def) — errors are never forceable",
        &session,
        "shape-bad-iso-plus-biconditional",
    );

    // 4. required-empty.
    assert_refusal(
        &run(item, REC_ITEM, &REC_ITEM.replace("status: open", "status:")),
        "write violates the probe def: status: required by the probe def, missing or empty — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/item.md` explains the def) — errors are never forceable",
        &session,
        "required-empty",
    );

    // 5. biconditional, un-derivable direction.
    assert_refusal(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace("closed_at:", "closed_at: 2026-07-20T10:00:00"),
        ),
        "write violates the probe def: closed_at is set but status \"open\" ∉ terminal [done retired] (status ∈ terminal ⟺ closed_at set) — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/item.md` explains the def) — errors are never forceable",
        &session,
        "biconditional-closed-not-terminal",
    );

    // 6. valid int lands; no repairs.
    let ok = run(
        item,
        REC_ITEM,
        &REC_ITEM.replace("priority: 3", "priority: 7"),
    );
    assert_pass(&ok, "shape-ok-int");
    assert!(ok.repairs.is_empty(), "shape-ok-int: unexpected repairs");

    // 7. entry-grammar miss (warn rung refuses when unforced).
    assert_refusal(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace(
                "- 2026-01-01: seeded\n",
                "- 2026-01-01: seeded\n- freeform without colon\n",
            ),
        ),
        "write violates the probe def: # Log: entry \"- freeform without colon\" does not parse as \"- {date}: {note}\" → would be tagged #legacy — fix the content, or re-submit with force to override — forced warnings render a `forced:` verdict and surface in the def census (R-force)",
        &session,
        "entry-grammar-miss",
    );

    // 8. conforming entry passes.
    assert_pass(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace(
                "- 2026-01-01: seeded\n",
                "- 2026-01-01: seeded\n- 2026-07-21: proper note\n",
            ),
        ),
        "entry-grammar-ok",
    );

    // 9. legacy-marked entry is non-blocking.
    assert_pass(
        &run(
            item,
            REC_ITEM,
            &REC_ITEM.replace(
                "- 2026-01-01: seeded\n",
                "- 2026-01-01: seeded\n- messy junk #legacy\n",
            ),
        ),
        "entry-legacy-nonblocking",
    );

    // 10. terminal transition mints the close-stamp repair (naive now prefix)
    //     and drops the biconditional from the error set.
    let done = run(
        item,
        REC_ITEM,
        &REC_ITEM.replace("status: open", "status: done"),
    );
    assert_pass(&done, "terminal-close-stamp-repair");
    assert_eq!(
        done.repairs,
        vec![policy::defs::Repair {
            key: "closed_at".to_string(),
            value: "2026-07-24T13:00:00".to_string()
        }],
        "terminal-close-stamp-repair: repair set"
    );

    // 11. unknown key warns → refuses (on the closed/terminal variant).
    let item_done = REC_ITEM
        .replace("status: open", "status: done")
        .replace("closed_at:", "closed_at: 2026-07-24T12:00:00");
    assert_refusal(
        &run(
            item,
            &item_done,
            &item_done.replace("type: probe", "type: probe\nvibe: high"),
        ),
        "write violates the probe def: vibe: not declared by the probe def (forward-compat: kept, reported) — fix the content, or re-submit with force to override — forced warnings render a `forced:` verdict and surface in the def census (R-force)",
        &session,
        "unknown-key-warn-refuses",
    );

    // 12. required-before-terminal names the guarded section.
    let fast_cleared = REC_FAST.replace("will be cleared\n", "");
    assert_refusal(
        &run(
            fast,
            &fast_cleared,
            &fast_cleared.replace("status: open", "status: done"),
        ),
        "write violates the probe def: # Answer: must be non-empty before a terminal status (guard: section-non-empty) — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/fast.md` explains the def) — errors are never forceable",
        &session,
        "fast-terminal-guard",
    );

    // 13. preset def is nearer: priority is line-shaped for the fast record.
    assert_pass(
        &run(
            fast,
            REC_FAST,
            &REC_FAST.replace("closed_at:\n", "closed_at:\npriority: abc\n"),
        ),
        "fast-preset-shape",
    );

    // 14. the cascade reaches the $UCC_HOME far layer.
    assert_refusal(
        &run(
            fast,
            REC_FAST,
            &REC_FAST.replace("closed_at:\n", "closed_at:\nseverity: x\n"),
        ),
        "write violates the probe def: severity: value x does not satisfy shape int — the write was refused; the file is unchanged. Fix the violation (`md def check $SESSION/records/fast.md` explains the def) — errors are never forceable",
        &session,
        "fast-cascade-far",
    );

    // 15. a def that exists but fails to load refuses, naming the def path.
    let broken_p = root.join("records/broken-rec.md");
    assert_refusal(
        &run(
            broken_p.to_str().unwrap(),
            REC_BROKEN,
            &REC_BROKEN.replace("status: open", "status: x"),
        ),
        "the broken def failed to load, so this write cannot be validated: def $SESSION/defs/broken.md: INVALID_PARAMS: status: unknown shape \"sparkle\" (closed set: line text iso int bool ref list(line) list(ref) list(iso)) — fix the def file (fail closed), or re-submit with force to override — forced writes render a `forced:` verdict and surface in the def census",
        &session,
        "malformed-def",
    );

    // 16. a kind with no def anywhere passes.
    let mystery_p = root.join("records/mystery.md");
    assert_pass(
        &run(
            mystery_p.to_str().unwrap(),
            REC_MYSTERY,
            &REC_MYSTERY.replace("type: mystery", "type: mystery\nanything: goes"),
        ),
        "nodef-kind",
    );

    // 17. nested frontmatter introduced on a def-less kind refuses (substrate law).
    assert_refusal(
        &run(
            mystery_p.to_str().unwrap(),
            REC_MYSTERY,
            &REC_MYSTERY.replace("type: mystery", "type: mystery\nbroken: {a: b}"),
        ),
        "write violates the record's def: broken: nested frontmatter is an ERROR always, from any writer (substrate law); flatten to a scalar or one-level list — the write was refused; the file is unchanged. Flatten the value (nested frontmatter is an error always, from any writer) — never forceable",
        &session,
        "nested-delta",
    );

    // Guard: the scratch tree existed through the run.
    assert!(Path::new(&session).exists());
}
