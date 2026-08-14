//! Dogfood 2026-08-08 refusal-teaching gates (card `p2-dogfood-refusal-teaching`).
//!
//! Three refusals that answered with a bare code (or a self-defeating Fix)
//! during the dogfood run must teach at the exemplar bar the codebase sets for
//! itself: *what happened, what was not done, the fix*. Pins properties, not
//! bytes (the `refusal_contract` precedent). Codes and recovery classes stay
//! frozen — teaching text only.
//!
//! - P2-a: write-door `bad_path` on an absolute spelling names the
//!   workspace-relative confinement rule and, when the path lies inside this
//!   workspace, the relative respelling.
//! - P3-e: the fm_key-miss Fix states the § A.6.3a value-plane semantics —
//!   upsert's `text` is the VALUE alone — so following it cannot double the key.
//! - opus P3-2: `file_not_found` on the degrade/write plane names the miss and
//!   a servable Fix instead of echoing the path as a bare token.

use wire::{Edit, EditShape, ErrorCode, Path as WPath, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const PAGE: &str = "---\ntitle: Race\n---\n# Log\n\nseed line\n";

/// Splice args whose one edit targets `target` with a Match (content-free —
/// every probe here refuses before any edit applies).
fn args_for(path: String, target: SecRef) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target,
            edit: EditShape::Match {
                old: "seed line".into(),
                new: "grown line".into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    }
}

fn fm_key(key: &str) -> SecRef {
    SecRef::FmKey {
        fm_key: key.to_owned(),
    }
}

/// P2-a. The dogfood shape verbatim: a file the read door happily serves by
/// absolute path is refused by the write door — and the refusal must name the
/// confinement rule and the workspace-relative respelling, not just echo the
/// path.
#[test]
fn bad_path_on_an_inside_absolute_spelling_teaches_rule_and_respelling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let canonical = dir.path().canonicalize().expect("canonicalize");
    std::fs::write(canonical.join("f.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(canonical.clone());

    let absolute = canonical.join("f.md").to_str().expect("utf8").to_owned();
    let err = splice(
        &root,
        None,
        &args_for(absolute.clone(), fm_key("title")),
        &[],
        None,
    )
    .expect_err("an absolute spelling refuses at the write door");

    // Frozen surface: the code stays `bad_path`, the offending path stays echoed.
    assert_eq!(err.code, ErrorCode::BadPath);
    assert_eq!(
        err.path.as_ref().map(|p| p.0.as_str()),
        Some(absolute.as_str())
    );

    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");
    // 1. The rule: write doors admit workspace-relative spellings only.
    assert!(
        m.contains("workspace-relative"),
        "names the confinement rule: {m}"
    );
    // 2. The remedy: the concrete relative respelling, distinguishable from
    //    the absolute echo (backtick-quoted).
    assert!(
        m.contains("respell") && m.contains("`f.md`"),
        "names the relative respelling: {m}"
    );
    // 3. Partial state disclosed.
    assert!(
        m.contains("Nothing was written"),
        "discloses partial state: {m}"
    );
}

/// P2-a control: an absolute path OUTSIDE this workspace still teaches the
/// rule but proposes no respelling — there is none to propose.
#[test]
fn bad_path_outside_the_root_teaches_the_rule_without_a_respelling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));

    let err = splice(
        &root,
        None,
        &args_for("/etc/hosts.md".into(), fm_key("title")),
        &[],
        None,
    )
    .expect_err("an absolute spelling refuses at the write door");

    assert_eq!(err.code, ErrorCode::BadPath);
    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");
    assert!(
        m.contains("workspace-relative"),
        "names the confinement rule: {m}"
    );
    assert!(
        !m.contains("respell it as"),
        "proposes no respelling for a path outside the root: {m}"
    );
}

/// P3-e. The fm_key-miss Fix teaches `at: upsert` — and must state that upsert
/// is a value-plane door (§ A.6.3a): its `text` is the VALUE alone, the engine
/// composes `key: value`. The dogfood run followed the current Fix with
/// line-thinking and wrote `title: title: born`.
#[test]
fn fm_key_miss_fix_states_value_only_upsert_semantics() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("f.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));

    let err = splice(
        &root,
        None,
        &args_for("f.md".into(), fm_key("verdict")),
        &[],
        None,
    )
    .expect_err("a Match on an absent fm key refuses ref_not_found");

    assert_eq!(err.code, ErrorCode::RefNotFound);
    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");
    // The Fix keeps teaching the door…
    assert!(
        m.contains("at: upsert"),
        "still teaches the upsert door: {m}"
    );
    // …and now states its value-plane grain: the text is the VALUE alone,
    // the engine composes the `key: value` line.
    assert!(
        m.contains("VALUE alone"),
        "states the value-only semantics: {m}"
    );
    assert!(
        m.contains("`verdict: "),
        "shows the composed line for the asked key: {m}"
    );
}

/// opus P3-2, degrade/write plane (`load_doc`): the most common miss of all
/// uses the established refusal shape — names the miss, discloses partial
/// state, and gives a servable Fix — instead of a bare `file_not_found: path`.
#[test]
fn file_not_found_on_the_load_plane_names_the_miss_and_a_fix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));

    let err = wire_serve::load_doc(&root, &WPath("missing.md".into()))
        .expect_err("a missing file refuses");

    // Frozen surface: code and echoed path.
    assert_eq!(err.code, ErrorCode::FileNotFound);
    assert_eq!(err.path.as_ref().map(|p| p.0.as_str()), Some("missing.md"));

    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");
    assert!(m.contains("missing.md"), "names the missing file: {m}");
    assert!(m.contains("workspace root"), "names where it looked: {m}");
    // The register moved (r5 F2): one demanded Fix became fitted suggestions
    // by entry — the fix clause now rides as "Fixes — whichever fits".
    assert!(m.contains("Fixes —"), "carries a fix clause: {m}");
    // The write half of the trap: a write to a missing path never births it —
    // birth is its own door.
    assert!(m.contains("birth"), "points writes at the birth door: {m}");
}

/// One law, one sentence, at BOTH value-plane write doors (wire-contract
/// § A.6.3a, dogfood s7). The upsert door used to say only that the value must
/// be single-line — no key, no remedy — so recovery quality was a function of
/// which door the caller entered.
#[test]
fn both_value_plane_doors_spell_the_multi_line_refusal_the_same_way() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("f.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));

    let mut args = args_for("f.md".into(), fm_key("ml"));
    args.edits[0].edit = EditShape::Put {
        at: wire::PutAt::Upsert,
        text: "a\nb".to_owned(),
    };
    let err = splice(&root, None, &args, &[], None)
        .expect_err("a newline in a frontmatter value is refused, never sanitized");

    assert_eq!(err.code, ErrorCode::BadRequest);
    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");
    // The key by name, the v1 rule, and the executable escape — the words the
    // `set_property` door already spoke.
    assert!(m.contains("\"ml\""), "names the offending key: {m}");
    assert!(m.contains("single-line in v1"), "states the rule: {m}");
    assert!(
        m.contains("put multi-line content in a body section"),
        "teaches the escape: {m}"
    );
}
