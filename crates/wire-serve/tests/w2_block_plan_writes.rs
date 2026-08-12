//! W-2: `^id` block anchors — readable, therefore writeable (D-B face gate a,
//! wire-contract §4.2 "an anchor becomes a write target by the same one-hop
//! path as a section"). The plan lane (`splice.plan_edits`, the daemon face's
//! only write vocabulary) resolves anchor-shaped targets against the SAME fact
//! table the read face resolves against — door symmetry (A.3): what the face
//! read serves, the face put writes; what the read door refuses (absent id,
//! host outside the anchor law), the write door refuses in the same class.
//!
//! The op semantics at a block target:
//! - `match` — find/replace inside the block-leaf bytes (all:false anchored,
//!   all:true RMW), the block twin of the section arm.
//! - `replace_section` — the payload is the block's CONTENT; the `^id` marker
//!   is the ADDRESS, preserved by construction (the exact mirror of the
//!   section arm, whose `Put{content}` preserves the heading by construction).
//!   A payload echoing the marker line-final is the caller repeating the
//!   address and passes through unchanged (the containment spec's case-4
//!   normalization family).
//! - `append` — keeps its designed refusal (its own arm string): a line grows
//!   through `match`/`replace_section`; a NEW line belongs to the enclosing
//!   section.
//!
//! Everything below the lowering is the native lane's existing law: span
//! escapes and marker loss refuse via the kernel's `transition_unrepresentable`
//! identity family, structure damage via the reparse gate, staleness via CAS.

use std::path::PathBuf;

use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, PlanEdit, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(p, body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn plan_args(path: &str, plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path.into()),
        actor: Some("agent:alice".into()),
        now: Some("2026-08-12T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits,
        pin: None,
    }
}

fn native_args(path: &str, edits: Vec<Edit>) -> SpliceArgs {
    SpliceArgs {
        edits,
        plan_edits: Vec::new(),
        ..plan_args(path, Vec::new())
    }
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// The anchor's CAS token, exactly as a sections read of `^id` serves it: the
/// node_rev over the block-leaf span.
fn anchor_rev(raw: &str, id: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::resolve(&doc, &model::Ref::anchor(id.to_owned()).expect("id"))
        .expect("anchor resolves")
        .node_rev
        .0
}

fn read_back(dir: &tempfile::TempDir, rel: &str) -> String {
    std::fs::read_to_string(dir.path().join(rel)).expect("read back")
}

/// `- plain item ^blk-1` is a PLAIN list item — inside the face's anchor law,
/// so its id is toc-listed (readable) and therefore must be writeable.
/// `- [ ] task item ^tsk-1` is TASK-hosted — outside the law on BOTH doors.
const DOC: &str = "# Tasks\n\n- plain item ^blk-1\n- second row\n\n# Notes\n\npara\n";
const DOC_WITH_TASK: &str =
    "# Tasks\n\n- plain item ^blk-1\n- [ ] task item ^tsk-1\n\n# Notes\n\npara\n";

/// The exact miss teaching the plan lane serves today for an anchor-shaped
/// target it cannot resolve — KEPT verbatim by the fix: for an id the fact
/// table does not carry, this is still the honest answer, and the daemon's
/// absent-anchor control pins the class.
fn miss_message(id: &str) -> String {
    format!(
        "no section addressed by \"^{id}\". No edit was applied; the batch is refused \
         whole. Fix: the section map does not list `^` anchors — find the id inline in \
         the section's content, or via CLI `--json` in its `anchors[]`."
    )
}

/// match(all:false) at a toc-listed anchor edits the block in place; the armed
/// row echoes the ANCHOR target with a real rev transition.
#[test]
fn plan_match_at_anchor_edits_the_block() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let rev = anchor_rev(DOC, "blk-1");
    let out = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^blk-1")],
                old: "plain item".into(),
                new: "edited item".into(),
                all: false,
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect("a toc-listed anchor is writeable by its id (W-2)");

    assert_eq!(
        read_back(&dir, "card.md"),
        "# Tasks\n\n- edited item ^blk-1\n- second row\n\n# Notes\n\npara\n",
        "the block edited in place; marker and neighbors untouched"
    );
    let ResponseBody::Splice { armed, .. } = out.body else {
        panic!("splice answers the splice shape");
    };
    let rows = armed.edits;
    assert_eq!(rows.len(), 1, "one armed row for one edit");
    assert_eq!(
        rows[0].target,
        SecRef::Anchor {
            anchor: "blk-1".into()
        },
        "the armed fact names the address the caller wrote through"
    );
    assert_ne!(
        rows[0].node_rev_before, rows[0].node_rev_after,
        "a real transition, both ends carried"
    );
}

/// match(all:true) is the RMW arm over the block-leaf bytes.
#[test]
fn plan_match_all_at_anchor_rmws_the_block() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let out = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^blk-1")],
                old: "item".into(),
                new: "row".into(),
                all: true,
                rev: None,
            }],
        ),
        &[],
        None,
    );
    out.expect("all:true RMW lands at a block target");
    assert_eq!(
        read_back(&dir, "card.md"),
        "# Tasks\n\n- plain row ^blk-1\n- second row\n\n# Notes\n\npara\n",
    );
}

/// replace_section at a block: the payload is the CONTENT, the marker is the
/// ADDRESS — preserved by construction, exactly as the section arm preserves
/// the heading.
#[test]
fn plan_replace_section_at_anchor_replaces_content_and_keeps_the_marker() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let rev = anchor_rev(DOC, "blk-1");
    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("^blk-1")],
                body: "- done item\n".into(),
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect("whole-block rewrite lands");
    assert_eq!(
        read_back(&dir, "card.md"),
        "# Tasks\n\n- done item ^blk-1\n- second row\n\n# Notes\n\npara\n",
        "content replaced; the ^id marker survives the write it addressed"
    );
}

/// A payload already carrying the marker line-final is the caller repeating
/// the address — passes through, never doubled.
#[test]
fn plan_replace_section_at_anchor_passes_the_marker_echo_through() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let rev = anchor_rev(DOC, "blk-1");
    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("^blk-1")],
                body: "- done item ^blk-1".into(),
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect("the echo form lands");
    assert_eq!(
        read_back(&dir, "card.md"),
        "# Tasks\n\n- done item ^blk-1\n- second row\n\n# Notes\n\npara\n",
        "one marker, not two"
    );
}

/// A whole-block rewrite is destructive — same rev demand as the section arm,
/// in the block's own voice.
#[test]
fn plan_replace_section_at_anchor_requires_rev() {
    let (_dir, root) = ws(&[("card.md", DOC)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("^blk-1")],
                body: "- x".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("rev-less whole-block rewrite refuses");
    assert_eq!(
        err.message.as_deref(),
        Some(
            "replace_section on \"^blk-1\" requires a fresh rev (a whole-block rewrite is \
             destructive) — read the block (sections:[\"^blk-1\"]) and pass its rev"
        )
    );
}

/// An empty body would leave a bare `^id` marker hosting nothing — no clean
/// meaning exists, so it refuses and names the honest alternative.
#[test]
fn plan_replace_section_at_anchor_with_empty_body_refuses() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let seed = read_back(&dir, "card.md");
    let rev = anchor_rev(DOC, "blk-1");
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("^blk-1")],
                body: String::new(),
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect_err("emptying a block refuses");
    assert_eq!(
        err.message.as_deref(),
        Some(
            "replace_section on \"^blk-1\" with an empty body would leave a bare `^` marker \
             hosting nothing. No edit was applied; the batch is refused whole. To remove the \
             block, write through the containing section (its heading path)."
        )
    );
    assert_eq!(read_back(&dir, "card.md"), seed, "refusal moved no bytes");
}

/// A stale rev is the CAS ladder's answer, not a target-class miss.
#[test]
fn plan_replace_section_at_anchor_with_stale_rev_conflicts() {
    let (_dir, root) = ws(&[("card.md", DOC)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("^blk-1")],
                body: "- x".into(),
                rev: Some("0123456789abcdef".into()),
            }],
        ),
        &[],
        None,
    )
    .expect_err("a stale block rev refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch, "the CAS class: {err:?}");
}

/// An id the fact table does not carry keeps today's miss teaching verbatim —
/// the daemon's absent-anchor control pins this class.
#[test]
fn plan_match_at_missing_anchor_keeps_the_miss_teaching() {
    let (_dir, root) = ws(&[("card.md", DOC)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^missing")],
                old: "a".into(),
                new: "b".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("an absent id refuses");
    assert_eq!(err.message.as_deref(), Some(miss_message("missing").as_str()));
}

/// A task-hosted id is outside the face's anchor law on BOTH doors: the read
/// face does not list it (the `basic` golden pins `^task1` unresolved), so the
/// write door must not resolve it either — door symmetry, the same class as a
/// miss.
#[test]
fn plan_match_at_task_hosted_anchor_stays_outside_the_face_law() {
    let (dir, root) = ws(&[("card.md", DOC_WITH_TASK)]);
    let seed = read_back(&dir, "card.md");
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^tsk-1")],
                old: "task item".into(),
                new: "x".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("a host outside the anchor law does not resolve on the write door");
    assert_eq!(err.message.as_deref(), Some(miss_message("tsk-1").as_str()));
    assert_eq!(read_back(&dir, "card.md"), seed);
}

/// The append arm keeps its own designed refusal — unchanged by W-2.
#[test]
fn plan_append_at_anchor_keeps_its_designed_refusal() {
    let (_dir, root) = ws(&[("card.md", DOC)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Append {
                hpath: vec![seg("^blk-1")],
                body: "x".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("append at a block keeps refusing");
    assert_eq!(
        err.message.as_deref(),
        Some(
            r#"append to a block anchor "^blk-1" is not supported — append targets a section (the containing heading path)"#
        )
    );
}

/// A duplicated id refuses in the anchor-plane ambiguity voice (A.3 door
/// symmetry over duplicate block ids) — never a silent pick, never a miss.
#[test]
fn plan_match_at_duplicate_anchor_refuses_ambiguous() {
    let dup = "# Tasks\n\n- one ^dup\n- two ^dup\n\n# Notes\n\npara\n";
    let (_dir, root) = ws(&[("card.md", dup)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^dup")],
                old: "one".into(),
                new: "x".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("a duplicated id refuses loudly");
    assert_eq!(err.code, ErrorCode::AmbiguousRef, "{err:?}");
    assert_eq!(
        err.message.as_deref(),
        Some(model::selector::render_anchor_ambiguity("^dup", 2).as_str())
    );
}

/// The plan lane lowers onto the SAME native edit a direct caller builds:
/// byte-identical result, armed rows 1:1 — the u8b equivalence, block grain.
#[test]
fn plan_match_at_anchor_equals_the_native_anchor_edit() {
    let (da, ra) = ws(&[("card.md", DOC)]);
    let out_a = splice(
        &ra,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^blk-1")],
                old: "plain".into(),
                new: "fancy".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("plan lane commits");

    let (db, rb) = ws(&[("card.md", DOC)]);
    let out_b = splice(
        &rb,
        None,
        &native_args(
            "card.md",
            vec![Edit {
                target: SecRef::Anchor {
                    anchor: "blk-1".into(),
                },
                edit: EditShape::Match {
                    old: "plain".into(),
                    new: "fancy".into(),
                },
                if_node_rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("native lane commits");

    assert_eq!(
        read_back(&da, "card.md"),
        read_back(&db, "card.md"),
        "one lowering, one result"
    );
    let (ResponseBody::Splice { armed: aa, .. }, ResponseBody::Splice { armed: ab, .. }) =
        (out_a.body, out_b.body)
    else {
        panic!("both answer the splice shape");
    };
    assert_eq!(aa.edits, ab.edits, "armed rows 1:1");
}

// --- ZT-directed fixtures (card fix-w2-block-write § ZT-directed test fixtures) ---

/// Fixture A — the probe scratch's own shape (`inbox/_unstaged/
/// mrd-mcp-probe-scratch.md` `^probe-anchor`): a toc-listed plain-list anchor
/// that read served and put refused in both caller lanes. Post-fix the plan
/// lane writes it — match inside the block, and whole-block replace with the
/// marker preserved. (The two DAEMON lanes — `at:"^id"` and `ref#^id` —
/// converge to this one plan form; their convergence is pinned daemon-side.)
#[test]
fn fixture_a_probe_anchor_writes_and_survives() {
    const PROBE: &str = "# mrd MCP probe scratch\n\n## Anchors\n\n- anchored list item for block-id addressing ^probe-anchor\n\n## Level tests\n\nold body\n";
    let (dir, root) = ws(&[("probe.md", PROBE)]);
    let rev = anchor_rev(PROBE, "probe-anchor");
    splice(
        &root,
        None,
        &plan_args(
            "probe.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^probe-anchor")],
                old: "anchored list item".into(),
                new: "REWRITTEN list item".into(),
                all: false,
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect("Fixture A: the toc-listed probe anchor accepts a match write");
    let after_match = read_back(&dir, "probe.md");
    assert!(
        after_match.contains("- REWRITTEN list item for block-id addressing ^probe-anchor\n"),
        "match landed inside the block, marker intact:\n{after_match}"
    );

    let rev2 = anchor_rev(&after_match, "probe-anchor");
    splice(
        &root,
        None,
        &plan_args(
            "probe.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("^probe-anchor")],
                body: "- fresh probe row".into(),
                rev: Some(rev2),
            }],
        ),
        &[],
        None,
    )
    .expect("Fixture A: whole-block replace lands");
    assert_eq!(
        read_back(&dir, "probe.md"),
        "# mrd MCP probe scratch\n\n## Anchors\n\n- fresh probe row ^probe-anchor\n\n## Level tests\n\nold body\n",
        "content replaced, address preserved, neighbors untouched"
    );
}

/// Fixture B — `health/runtime.md` `^check`: a code-fence-hosted anchor (the
/// `^check` line trails the closing fence), NOT toc-listed — outside the read
/// face's anchor law, so the write door must not resolve it either (a write
/// door wider than the read door is W-2's own asymmetry, inverted). Its
/// STABLE address through the face is the documented section+find lane: the
/// containing section's heading path plus a content needle — pinned here
/// landing a version-pin edit INSIDE the fence, fence and marker untouched.
/// (The anchor itself stays a walk/run-plane address — `[[page#^check]]`,
/// `md run` — which this face law does not touch. Pin refresh itself is
/// ZT-deferred; only addressability rides this card.)
#[test]
fn fixture_b_fence_hosted_check_anchor_has_the_section_find_lane() {
    const RUNTIME: &str = "# Wiki Runtime\n\n## Tasks\n\n```bash\nchk md \"md version\" \"build: 0099f641\" \"build meridian\"\nchk node \"node --version\" \"v24.16.0\" \"osf rebuild\"\nexit $fail\n```\n\n^check\n";
    let (dir, root) = ws(&[("runtime.md", RUNTIME)]);

    // Half 1 — door symmetry: the fence-hosted id is unlisted on the read
    // face, so the write door misses with the standing teaching.
    let err = splice(
        &root,
        None,
        &plan_args(
            "runtime.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("^check")],
                old: "0099f641".into(),
                new: "aaaaaaaa".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("Fixture B: a host-excluded anchor does not resolve on the write door");
    assert_eq!(err.message.as_deref(), Some(miss_message("check").as_str()));
    assert_eq!(read_back(&dir, "runtime.md"), RUNTIME, "refusal moved no bytes");

    // Half 2 — the stable lane: containing section + content find. The pin
    // edit lands inside the fence; the fence, its other lines, and the
    // trailing ^check marker are byte-untouched.
    splice(
        &root,
        None,
        &plan_args(
            "runtime.md",
            vec![PlanEdit::Match {
                hpath: vec![seg("Wiki Runtime"), seg("Tasks")],
                old: "build: 0099f641".into(),
                new: "build: feedc0de".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("Fixture B: section+find is the stable write lane for a fence-hosted anchor");
    assert_eq!(
        read_back(&dir, "runtime.md"),
        "# Wiki Runtime\n\n## Tasks\n\n```bash\nchk md \"md version\" \"build: feedc0de\" \"build meridian\"\nchk node \"node --version\" \"v24.16.0\" \"osf rebuild\"\nexit $fail\n```\n\n^check\n",
        "one pin moved; fence and ^check marker byte-identical"
    );
}
