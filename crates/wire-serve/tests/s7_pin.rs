//! S7: `mrd pin` mints a real `meridian-lock` pin — read-mint gate (D16),
//! rev-neutral slug promotion (D15), content+lock in one `commit_batch` (D7).
//! Drives `write::splice` on a real workspace; gate tests are single-session
//! in-process (`ReadMintStore` across read then pin).

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PutAt, Recovery, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// Pinning page (no lock yet — first pin births one at EOF).
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

/// Pinned page; `Leader's Guideline` → D15 slug `leaders-guideline`.
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

/// A pinnable workspace — **git-initialised**, because R4 gives every pin row a
/// `hash` and admits no row without one. A bare directory is no longer a
/// workspace a pin can land in; that case is its own test
/// ([`without_git_the_pin_refuses_because_r4_admits_no_hashless_row`], which
/// builds its fixture with [`bare_workspace`] directly).
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let (dir, root) = bare_workspace();
    git_init(dir.path());
    (dir, root)
}

/// The same fixture WITHOUT a repo — only the no-git refusal wants this.
fn bare_workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// Pin-only splice (lock block is the write).
fn pin_args(selector: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("plan.md".into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: selector.into(),
            vibe: None,
        }),
    }
}

/// Pin fact off a splice response.
fn pin_fact(body: &ResponseBody) -> wire::PinFact {
    let ResponseBody::Splice { pin, .. } = body else {
        panic!("splice body");
    };
    pin.as_deref()
        .cloned()
        .expect("a pin request answers with a pin fact")
}

fn read_page(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// Live fingerprint of a lock `ref`, same path as the verify plane.
fn live_fingerprint(root: &fs::WorkspaceRoot, declared_ref: &str) -> String {
    let (rel, _) = declared_ref.split_once('#').expect("a ref names a section");
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    let r#ref = match model::selector::Selector::parse(declared_ref) {
        model::selector::Selector::Heading(segs) => model::Ref::Hpath(
            segs.iter()
                .map(|h| model::HpathSeg {
                    h: h.clone(),
                    n: None,
                })
                .collect(),
        ),
        model::selector::Selector::Block(id) => model::Ref::anchor(id).expect("block id"),
        other => panic!("unpinnable selector class: {other:?}"),
    };
    let target = model::resolve(&doc, &r#ref).expect("the lock ref resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
        .expect("the fixture target has content")
        .into_string()
}

// GATE 1 + 3: real pin lands; bare CLI (actor absent) is trusted

/// Bare CLI pin (no actor, D16): lock block + slug promotion + green fingerprint.
#[test]
fn a_bare_cli_pin_mints_a_real_lock_block_and_promotes_the_slug() {
    let (_dir, root) = workspace();

    let out =
        splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None).expect("pin commits");
    let fact = pin_fact(&out.body);

    assert_eq!(
        fact.selector, "Guide/Leader's-Guideline",
        "canonical selector"
    );
    assert_eq!(
        fact.declared_ref, "guide.md#Guide/Leader's Guideline",
        "the lock ref is the RAW heading chain — the spelling model::resolve takes"
    );
    assert_eq!(
        fact.anchor, "leaders-guideline",
        "D15 slug, apostrophe dropped"
    );
    assert!(fact.promoted, "the target had no id, so this pin wrote one");
    let blob = fact.blob.clone().expect("a real repo answers a blob oid");
    assert_eq!(blob.len(), 40, "a git blob oid: {blob}");

    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline"),
        "a freshly minted pin must verify green immediately"
    );

    let pinner = read_page(&root, "plan.md");
    let expected_block = format!(
        "```meridian-lock\n\
         version: 2\n\
         pins:\n\
         \x20 - object: \"[[guide]]\"\n\
         \x20   hash: \"{blob}\"\n\
         \x20   path: [\"Guide\", \"Leader's Guideline\"]\n\
         \x20   fingerprint: \"{}\"\n\
         ```\n",
        fact.fingerprint
    );
    assert!(
        pinner.ends_with(&expected_block),
        "the lock births at EOF in canonical bytes.\n--- got ---\n{pinner}\n--- want tail ---\n{expected_block}"
    );
    assert_eq!(
        pinner,
        format!("{PINNER}\n{expected_block}"),
        "placement law: one blank line before the block, one terminator after, \
         and the page's own bytes untouched"
    );

    assert_eq!(
        read_page(&root, "guide.md"),
        TARGET.replace(
            "## Leader's Guideline\n",
            "## Leader's Guideline\n^leaders-guideline\n"
        ),
        "promotion writes exactly one own-line slug marker, leaving the heading \
         text (and therefore the section's address) untouched"
    );

    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(found.lock.pins.len(), 1);
    assert_eq!(
        found.lock.pins[0].hash, blob,
        "R4: the blob oid rides the pin row it was minted for, not a shared table"
    );
    assert_eq!(
        found.lock.pins[0].object, "guide",
        "the object is the wiki link's INNER text — the target path minus `.md`"
    );
}

/// **The path array is built from `hpath_raw` SEGMENTS, never by splitting a
/// joined string** (R1.6 — arrays for machines, no string address forms).
///
/// The discriminator is `sanitize_heading`, which is MANY-TO-ONE: it rewrites
/// every space to `-`. The fixture heading is `Leader's Guideline` (a SPACE),
/// and the two candidate derivations disagree on exactly that byte:
///
/// - **From `hpath_raw` segments** → `["Guide", "Leader's Guideline"]`. The
///   space survives, because the raw pre-image is what was carried.
/// - **From the joined string** — `fact.selector` (`Guide/Leader's-Guideline`)
///   or `declared_ref`, split on `/` → `["Guide", "Leader's-Guideline"]`. A
///   HYPHEN. That address resolves to nothing, and the pin would read
///   red-dangling forever.
///
/// So a hyphen in the second element is proof the joined string was the input.
/// The assert is on the space, and it cannot pass by accident: no sanitized
/// spelling of this heading contains one.
#[test]
fn the_path_array_is_built_from_raw_segments_not_by_splitting_a_joined_string() {
    let (_dir, root) = workspace();
    let out =
        splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None).expect("pin commits");
    let fact = pin_fact(&out.body);
    assert_eq!(
        fact.selector, "Guide/Leader's-Guideline",
        "the human address is SANITIZED — the space is already gone here, which \
         is what makes it useless as a source for the array"
    );

    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(
        found.lock.pins[0].selector,
        lock::Selector::Path(vec!["Guide".into(), "Leader's Guideline".into()]),
        "raw segments: the SPACE survives. Splitting either joined spelling on \
         `/` would have put a hyphen here"
    );

    // The canonical bytes say it too — the array is the stored form, so no
    // reader downstream is handed a delimiter to re-split either.
    assert!(
        read_page(&root, "plan.md").contains("path: [\"Guide\", \"Leader's Guideline\"]"),
        "the stored form is the array, carrying the raw text verbatim"
    );
}

/// A heading whose RAW text begins with `^` is REFUSED, not written.
///
/// R4 spells an anchor pin as a path array whose sole element is a `^id`, so an
/// element like `^looks-like-an-anchor` sitting among heading segments would
/// make the row's GRAIN unreadable — block or section, with nothing in the bytes
/// to tell them apart. Mixed arrays appear nowhere in the ratified trace, so the
/// engine refuses rather than assigning one a meaning.
#[test]
fn a_heading_that_begins_with_a_caret_refuses_rather_than_minting_an_ambiguous_array() {
    let (dir, root) = workspace();
    // A SPACE after the caret, deliberately: `^Alpha-Beta` would parse as a real
    // block anchor on the heading line and refuse one rung earlier (slug
    // collision), which would test the wrong thing. `^Alpha Beta` is plain
    // heading text — the case that actually reaches the array builder.
    std::fs::write(
        dir.path().join("guide.md"),
        "# Guide\n\n## ^Alpha Beta\n\nbody text.\n",
    )
    .expect("a heading whose raw text opens with a caret");

    let err = splice(&root, 0, &pin_args("Guide/^Alpha-Beta"), &[], None)
        .expect_err("an ambiguous path array must refuse");

    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.clone().unwrap_or_default();
    assert!(
        message.contains("begins with `^`") && message.contains("Nothing was written"),
        "the refusal names the ambiguity and says nothing landed: {message}"
    );
    assert_eq!(
        read_page(&root, "plan.md"),
        PINNER,
        "a refused pin leaves the pinning page byte-untouched"
    );
}

/// **KEY-SET PIN over the serialized `PinFact` (all-hands #1).**
///
/// `Option` + `skip_serializing_if` is not a version gate: it skips on the
/// VALUE being none, never on the SESSION being v2. So what a field means for
/// the wire is decided by whether anything POPULATES it — and U8 changed
/// exactly that for `blob`.
///
/// Before U8, a pin outside git minted `blob: None` and the key serialized
/// AWAY, so `PinFact`'s key set varied with the environment. Under R4 the hash
/// is mandatory: a pin either carries one or refuses, so `blob` is now ALWAYS
/// present wherever a `PinFact` exists at all. That is a key-set change, and it
/// is the class value-pinning sweeps are blind to — they pin worked values
/// (spans, revs, roots), not the presence of a key.
///
/// This test is the detector for that class on this body. `PinFact` rides
/// `splice.pin`, which is v3-only at decode, so nothing here should reach a v2
/// frame — this pins the key set so a later change to that reachability is
/// caught HERE, loudly, instead of shipping green.
///
/// **What this test alone does NOT prove.** Its workspace is git-initialised, so
/// it cannot witness the absent-`blob` case directly. The invariant "a `PinFact`
/// exists ⟹ `blob` is present" is established by this test TOGETHER with
/// [`without_git_the_pin_refuses_because_r4_admits_no_hashless_row`], which
/// covers the other branch: no git, no `PinFact` at all. Neither half is
/// sufficient alone, and a future edit that softened the refusal back to honest
/// degradation would break that second test, not this one.
#[test]
fn the_pin_fact_key_set_is_pinned_and_blob_is_now_always_present() {
    let (_dir, root) = workspace();
    let out =
        splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None).expect("pin commits");
    let fact = pin_fact(&out.body);

    let serde_json::Value::Object(map) = serde_json::to_value(&fact).expect("PinFact serializes")
    else {
        panic!("a PinFact serializes to an object");
    };
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "anchor",
            "blob",
            "declared_ref",
            "fingerprint",
            "promoted",
            "selector",
            "target",
        ],
        "the EXACT key set — a new key here is a wire change, and an ABSENT \
         `blob` means something started minting hashless pins again"
    );
    assert!(
        map["blob"].as_str().is_some_and(|b| b.len() == 40),
        "and the key carries a real oid, not null: {:?}",
        map["blob"]
    );
}

/// GATE 4a: promotion is rev-neutral (D14 honesty — cannot move pinned fingerprint).
#[test]
fn promotion_is_rev_neutral_for_the_pinned_fingerprint() {
    let (_dir, root) = workspace();
    let before = live_fingerprint(&root, "guide.md#Guide/Leader's Guideline");
    let sibling_before = live_fingerprint(&root, "guide.md#Guide/Other");

    let out =
        splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None).expect("pin commits");
    let fact = pin_fact(&out.body);
    assert!(fact.promoted);

    let after = live_fingerprint(&root, "guide.md#Guide/Leader's Guideline");
    assert_eq!(
        before, after,
        "norm-v2 removes the marker plus its one leading space, so the promoted \
         section hashes identically"
    );
    assert_eq!(
        fact.fingerprint, before,
        "and the minted token is that same value"
    );
    assert_eq!(
        sibling_before,
        live_fingerprint(&root, "guide.md#Guide/Other"),
        "a sibling section is untouched"
    );
}

/// GATE 4b: re-pin is idempotent (same slug, no second marker).
#[test]
fn a_re_pin_reuses_the_same_slug_and_promotes_nothing() {
    let (_dir, root) = workspace();
    let first = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
            .unwrap()
            .body,
    );
    let target_after_first = read_page(&root, "guide.md");

    let second = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
            .unwrap()
            .body,
    );

    assert_eq!(second.anchor, first.anchor, "same slug, recomputed");
    assert!(!second.promoted, "the second pin writes no marker");
    assert_eq!(
        read_page(&root, "guide.md"),
        target_after_first,
        "the target is byte-unchanged by the re-pin"
    );
    assert_eq!(second.fingerprint, first.fingerprint);
    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(found.lock.pins.len(), 1, "a re-pin updates, never appends");
}

/// GATE 5a: refusal leaves no orphan (promotion ordered after every refusal rung).
#[test]
fn a_corrupt_lock_refuses_the_pin_and_leaves_no_orphan_behind() {
    let (_dir, root) = workspace();
    // Corrupt lock: find refuses, so pin cannot compose its lock edit.
    std::fs::write(
        root.0.join("plan.md"),
        format!("{PINNER}\n```meridian-lock\nversion: 1\ngarbage\n```\n"),
    )
    .expect("corrupt pinner");
    let pinner_before = read_page(&root, "plan.md");
    let fp_before = live_fingerprint(&root, "guide.md#Guide/Leader's Guideline");

    let err = splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
        .expect_err("a corrupt lock refuses the pin");
    assert_eq!(err.code, ErrorCode::BadRequest);

    assert_eq!(
        read_page(&root, "guide.md"),
        TARGET,
        "the TARGET is byte-unchanged: no marker, no orphan to heal"
    );
    assert_eq!(
        read_page(&root, "plan.md"),
        pinner_before,
        "and the corrupt pinning page is left exactly as found"
    );
    assert_eq!(
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline"),
        fp_before,
        "so nothing could have drifted"
    );

    std::fs::write(root.0.join("plan.md"), PINNER).expect("repair");
    let fact = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
            .unwrap()
            .body,
    );
    assert!(
        fact.promoted,
        "the marker is written NOW, on the run that commits"
    );
    assert_eq!(fact.fingerprint, fp_before);
}

/// GATE 5b: crash orphan is benign; re-pin reuses it.
#[test]
fn a_crash_orphan_is_benign_and_a_re_pin_reuses_it() {
    let (_dir, root) = workspace();
    let fp_before = live_fingerprint(&root, "guide.md#Guide/Leader's Guideline");
    // State left by crash between the two renames: marker on disk, no claim.
    let orphaned = TARGET.replace(
        "## Leader's Guideline\n",
        "## Leader's Guideline\n^leaders-guideline\n",
    );
    std::fs::write(root.0.join("guide.md"), &orphaned).expect("stage the orphan");
    assert_eq!(
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline"),
        fp_before,
        "the orphan is fingerprint-neutral — no false drift while it is unclaimed"
    );

    let healed = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
            .unwrap()
            .body,
    );
    assert_eq!(healed.anchor, "leaders-guideline", "the orphan is REUSED");
    assert!(!healed.promoted, "so no second marker is written");
    assert_eq!(
        read_page(&root, "guide.md"),
        orphaned,
        "healing writes nothing to the target at all"
    );
    assert_eq!(healed.fingerprint, fp_before);
}

// GATE 2: read-mint gate, single-session in-process

/// Production-arm read into a session store (H1 shape).
fn session_read(
    root: &fs::WorkspaceRoot,
    store: &receipt::read_mint::ReadMintStore,
    actor: &str,
    rel: &str,
    selector: &str,
) {
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    let params = wire_serve::read::ReadParams {
        mode: Some("sections".into()),
        sections: Some(vec![selector.to_owned()]),
        actor: Some(actor.to_owned()),
        ..Default::default()
    };
    wire_serve::read::composed_read(
        &doc,
        &WPath(rel.into()),
        &wire::Root("r0".into()),
        &params,
        Some(store),
        &wire_serve::read::NO_DECORATIONS,
    )
    .expect("the read serves");
}

fn agent_pin_args(actor: &str, selector: &str) -> SpliceArgs {
    SpliceArgs {
        actor: Some(actor.to_owned()),
        ..pin_args(selector)
    }
}

/// Unread pin refuses; covering read then admits the same request.
#[test]
fn the_gate_refuses_an_unread_pin_and_admits_it_after_a_covering_read() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    let args = agent_pin_args("agent-7", "Guide/Leader's-Guideline");

    let err = splice(&root, 0, &args, &[], Some(&store)).expect_err("un-read pin refuses");
    assert_eq!(err.code, ErrorCode::ReadMintRequired);
    assert_eq!(err.recovery, Recovery::Fix, "the caller reads, then pins");
    assert_eq!(err.path, Some(WPath("guide.md".into())));
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("never in your context")),
        "the refusal teaches: {:?}",
        err.message
    );
    assert_eq!(read_page(&root, "guide.md"), TARGET, "target untouched");
    assert_eq!(read_page(&root, "plan.md"), PINNER, "pinner untouched");

    session_read(
        &root,
        &store,
        "agent-7",
        "guide.md",
        "Guide/Leader's-Guideline",
    );
    assert_eq!(store.len(), 1, "the read minted one receipt");
    let out = splice(&root, 0, &args, &[], Some(&store)).expect("the read-backed pin commits");
    assert_eq!(
        pin_fact(&out.body).fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline")
    );
}

/// Gate is three-part (actor+path+selector); foreign/sibling receipts do not cover.
#[test]
fn another_actors_read_and_a_sibling_sections_read_both_fail_the_gate() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();

    session_read(
        &root,
        &store,
        "somebody-else",
        "guide.md",
        "Guide/Leader's-Guideline",
    );
    session_read(&root, &store, "agent-7", "guide.md", "Guide/Other");

    let err = splice(
        &root,
        0,
        &agent_pin_args("agent-7", "Guide/Leader's-Guideline"),
        &[],
        Some(&store),
    )
    .expect_err("neither receipt covers this selector");
    assert_eq!(err.code, ErrorCode::ReadMintRequired);
    assert_eq!(read_page(&root, "guide.md"), TARGET, "nothing was written");
}

/// Sidecar (no session ledger) refuses actor pins and names that reason.
#[test]
fn a_host_with_no_session_refuses_an_actor_pin_and_says_why() {
    let (_dir, root) = workspace();
    let err = splice(
        &root,
        0,
        &agent_pin_args("agent-7", "Guide/Leader's-Guideline"),
        &[],
        None,
    )
    .expect_err("no ledger, no answer");
    assert_eq!(err.code, ErrorCode::ReadMintRequired);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("no read-receipt ledger") && m.contains("sidecar")),
        "{:?}",
        err.message
    );
}

/// GATE 7: receipt is "was it read", not "is it current" → `write_conflict` on drift.
#[test]
fn a_rev_change_between_read_and_pin_is_a_write_conflict_not_a_silent_pin() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    session_read(
        &root,
        &store,
        "agent-7",
        "guide.md",
        "Guide/Leader's-Guideline",
    );

    std::fs::write(
        root.0.join("guide.md"),
        TARGET.replace("review before you close.", "review AFTER you close."),
    )
    .expect("foreign edit");

    let err = splice(
        &root,
        0,
        &agent_pin_args("agent-7", "Guide/Leader's-Guideline"),
        &[],
        Some(&store),
    )
    .expect_err("the stale receipt refuses");
    assert_eq!(err.code, ErrorCode::WriteConflict);
    assert!(
        err.expected.is_some() && err.actual.is_some(),
        "the refusal carries both revs"
    );
    assert!(
        !read_page(&root, "guide.md").contains("^leaders-guideline"),
        "and it refused before the promotion — the gate is ordered first"
    );

    session_read(
        &root,
        &store,
        "agent-7",
        "guide.md",
        "Guide/Leader's-Guideline",
    );
    splice(
        &root,
        0,
        &agent_pin_args("agent-7", "Guide/Leader's-Guideline"),
        &[],
        Some(&store),
    )
    .expect("the re-read authorizes the pin");
}

// Plan §6 edge cases

/// `^id` pin reuses the existing handle; no promotion; block-grain fingerprint.
#[test]
fn an_anchor_selector_is_pinned_at_block_grain_with_no_promotion() {
    let (_dir, root) = workspace();
    // Read face (and pin path) address list-item anchors only — Go parity.
    std::fs::write(
        root.0.join("guide.md"),
        "# Guide\n\n- the claim sentence. ^claim\n",
    )
    .expect("anchored target");

    let fact = pin_fact(
        &splice(&root, 0, &pin_args("^claim"), &[], None)
            .unwrap()
            .body,
    );
    assert_eq!(fact.selector, "^claim");
    assert_eq!(fact.declared_ref, "guide.md#^claim");
    assert_eq!(fact.anchor, "claim", "the existing id IS the handle");
    assert!(!fact.promoted);
    assert_eq!(
        read_page(&root, "guide.md"),
        "# Guide\n\n- the claim sentence. ^claim\n",
        "an anchor pin writes nothing to the target"
    );
    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "guide.md#^claim"),
        "and the block-grain token is what the verify plane recomputes"
    );

    // R4's anchor form: the PATH arm with the `^id` as its SOLE element. There
    // is no third selector arm, and the array is never widened to the host
    // section — that would silently turn a block claim into a section claim.
    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(
        found.lock.pins[0].selector,
        lock::Selector::Path(vec!["^claim".into()]),
    );
}

/// Missing page/selector → `pin_target_missing` (not a permanently-red claim).
#[test]
fn a_missing_target_or_selector_refuses_pin_target_missing() {
    let (_dir, root) = workspace();

    let mut ghost = pin_args("Guide/Leader's-Guideline");
    ghost.pin.as_mut().expect("pin").target = WPath("nope.md".into());
    let err = splice(&root, 0, &ghost, &[], None).expect_err("no such page");
    assert_eq!(err.code, ErrorCode::PinTargetMissing);
    assert_eq!(err.recovery, Recovery::Fix);

    let err = splice(&root, 0, &pin_args("Guide/No-Such-Section"), &[], None)
        .expect_err("no such selector");
    assert_eq!(err.code, ErrorCode::PinTargetMissing);
    // The teaching is unchanged in intent; only its spelling moved off the
    // internal mode name onto a command the reader can run (issue-05).
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("mrd read") && m.contains("Nothing was written")),
        "the refusal teaches how to find the real selectors, and says nothing landed: {:?}",
        err.message
    );
    assert_eq!(read_page(&root, "plan.md"), PINNER, "nothing written");
}

/// `--vibe` writes the blob eagerly; a normal pin only computes the oid.
#[test]
fn vibe_writes_the_eager_blob_where_a_normal_pin_only_computes_it() {
    let (dir, root) = workspace();
    let repo = git::Repo::at(dir.path().to_path_buf());

    let normal = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Other"), &[], None)
            .unwrap()
            .body,
    );
    let oid = normal.blob.clone().expect("oid computed");
    assert!(
        !repo.object_exists(&oid).expect("git answers"),
        "a normal pin must not write the object store"
    );

    let mut vibe = pin_args("Guide/Other");
    vibe.pin.as_mut().expect("pin").vibe = Some(true);
    let eager = pin_fact(&splice(&root, 0, &vibe, &[], None).unwrap().body);
    let eager_oid = eager.blob.clone().expect("oid written");
    assert!(
        repo.object_exists(&eager_oid).expect("git answers"),
        "--vibe writes the blob eagerly: {eager_oid}"
    );
}

/// Outside git: the pin REFUSES. Under v1 the claim plane landed and the
/// `objects:` entry was simply absent — two planes, one optional. R4 folded the
/// hash INTO the claim ("if hash is missing, we lost the explicit target
/// meaning"), so the same condition now has no legal row to write. Still no
/// fabricated sha (D5); the honesty just moved from omission to refusal.
#[test]
fn without_git_the_pin_refuses_because_r4_admits_no_hashless_row() {
    let (_dir, root) = bare_workspace();
    let err = splice(&root, 0, &pin_args("Guide/Other"), &[], None)
        .expect_err("no repo, no oid, no legal R4 pin");

    assert_eq!(err.code, ErrorCode::IoError);
    let cause = err.cause.clone().unwrap_or_default();
    assert!(
        cause.contains("blob oid") && cause.contains("Nothing was written"),
        "the refusal names the missing hash and says nothing landed: {cause}"
    );
    assert_eq!(
        read_page(&root, "plan.md"),
        PINNER,
        "a refused pin leaves the pinning page byte-untouched"
    );
}

/// Dry pin rehearses and writes nothing (lock nor promotion).
#[test]
fn a_dry_pin_writes_neither_the_lock_nor_the_promotion() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Guide/Leader's-Guideline");
    args.dry = true;

    let out = splice(&root, 0, &args, &[], None).expect("dry rehearses");
    let fact = pin_fact(&out.body);
    assert!(fact.promoted, "it reports what a real run would write");
    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline")
    );
    assert!(out.committed.is_none(), "no delta");
    assert_eq!(read_page(&root, "plan.md"), PINNER, "no lock block");
    assert_eq!(read_page(&root, "guide.md"), TARGET, "no promotion");
}

/// Second pin unions into the existing lock (one block, both claims).
#[test]
fn a_second_pin_unions_into_the_existing_lock_block() {
    let (_dir, root) = workspace();
    let first = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
            .unwrap()
            .body,
    );
    let second = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Other"), &[], None)
            .unwrap()
            .body,
    );

    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(found.lock.pins.len(), 2, "both claims are held");
    assert_eq!(
        found.lock.pins[0].selector,
        lock::Selector::Path(vec!["Guide".into(), "Leader's Guideline".into()]),
    );
    assert_eq!(found.lock.pins[0].fingerprint, first.fingerprint);
    assert_eq!(
        found.lock.pins[1].selector,
        lock::Selector::Path(vec!["Guide".into(), "Other".into()]),
    );
    assert_eq!(found.lock.pins[1].fingerprint, second.fingerprint);
    assert_eq!(
        doc.raw.matches("```meridian-lock").count(),
        1,
        "the sole writer mints exactly one block"
    );
}

/// Pin + caller edits land in one sealed batch; armed facts stay 1:1 with request.
#[test]
fn a_pin_rides_alongside_caller_edits_in_one_batch() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Guide/Leader's-Guideline");
    args.edits = vec![Edit {
        target: wire::SecRef::FmKey {
            fm_key: "title".into(),
        },
        edit: EditShape::Put {
            at: PutAt::Upsert,
            text: "Plan v2".into(),
        },
        if_node_rev: None,
    }];

    let out = splice(&root, 0, &args, &[], None).expect("content + lock commit together");
    let ResponseBody::Splice { armed, .. } = &out.body else {
        panic!("splice body");
    };
    assert_eq!(armed.edits.len(), 1, "armed edits are 1:1 with the request");

    let page = read_page(&root, "plan.md");
    assert!(page.contains("title: Plan v2"), "the caller's edit landed");
    assert!(page.contains("```meridian-lock"), "and so did the lock");
    let frame = out.committed.expect("one delta");
    assert_eq!(
        frame.delta.files.len(),
        1,
        "ONE file moved: content and lock are the same rename"
    );
    assert_eq!(frame.delta.files[0].path, WPath("plan.md".into()));
}

/// Self-pin of a non-lock-holding section: re-read pre-image between promotion and lock.
#[test]
fn a_page_can_pin_its_own_section() {
    let (_dir, root) = workspace();
    // Pin first of two sections so EOF lock does not land inside the pin.
    std::fs::write(
        root.0.join("plan.md"),
        "---\ntitle: Plan\n---\n\n# Premise\n\nthe premise.\n\n# Plan\n\ndraws from it.\n",
    )
    .expect("two-section pinner");
    let mut args = pin_args("Premise");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let fact = pin_fact(
        &splice(&root, 0, &args, &[], None)
            .expect("self-pin commits")
            .body,
    );
    assert_eq!(fact.declared_ref, "plan.md#Premise");
    assert_eq!(fact.anchor, "premise");

    let page = read_page(&root, "plan.md");
    assert!(
        page.contains("# Premise\n^premise\n"),
        "the promotion landed on its own line: {page}"
    );
    assert!(page.ends_with("```\n"), "and the lock block at EOF: {page}");
    // Lock sits in the page but outside the pinned section — green immediately.
    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "plan.md#Premise"),
        "a self-pin of a non-containing section verifies green immediately"
    );
}

/// Self-pin of the section the lock lands in refuses (permanently red otherwise).
#[test]
fn a_self_pin_of_the_section_holding_the_lock_refuses() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Plan");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let err = splice(&root, 0, &args, &[], None).expect_err("unverifiable by construction");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("lock-is-content")),
        "the refusal names the reason: {:?}",
        err.message
    );
    assert!(
        !read_page(&root, "plan.md").contains("```meridian-lock"),
        "and no lock was written"
    );
}

/// Taken slug refuses rather than minting a duplicate id.
#[test]
fn a_taken_slug_refuses_rather_than_minting_a_duplicate_id() {
    let (_dir, root) = workspace();
    std::fs::write(
        root.0.join("guide.md"),
        "# Guide\n\nsomething else entirely ^leaders-guideline\n\n## Leader's Guideline\n\nbody.\n",
    )
    .expect("pre-taken slug");

    let err = splice(&root, 0, &pin_args("Guide/Leader's-Guideline"), &[], None)
        .expect_err("the slug is taken");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("already taken")),
        "{:?}",
        err.message
    );
}

/// Trailing-whitespace heading still promotes rev-neutrally (marker is own line).
#[test]
fn a_trailing_whitespace_heading_promotes_rev_neutrally() {
    let (_dir, root) = workspace();
    std::fs::write(
        root.0.join("guide.md"),
        "# Guide\n\n## Padded Title  \n\nbody here.\n",
    )
    .expect("padded heading");
    let before = live_fingerprint(&root, "guide.md#Guide/Padded Title");

    let fact = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Padded-Title"), &[], None)
            .unwrap()
            .body,
    );
    assert!(fact.promoted);
    assert_eq!(
        read_page(&root, "guide.md"),
        "# Guide\n\n## Padded Title  \n^padded-title\n\nbody here.\n",
        "the padded heading line is untouched — the marker takes its own line"
    );
    assert_eq!(
        live_fingerprint(&root, "guide.md#Guide/Padded Title"),
        before,
        "and the section hashes identically"
    );
    assert_eq!(fact.fingerprint, before);
}

/// Stale `if_root` refuses before promotion.
#[test]
fn a_stale_world_guard_refuses_before_the_promotion() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Guide/Leader's-Guideline");
    args.if_root = Some(wire::Root("b3:deadbeef".into()));

    let err = splice(&root, 0, &args, &[], None).expect_err("stale plan refuses");
    assert_eq!(err.code, ErrorCode::RootMismatch);
    assert_eq!(read_page(&root, "guide.md"), TARGET, "no promotion");
    assert_eq!(read_page(&root, "plan.md"), PINNER, "no lock");
}

/// Fresh world guard survives the pin's own root advance (guard on client-pinned root).
#[test]
fn a_fresh_world_guard_survives_the_pins_own_root_advance() {
    let (_dir, root) = workspace();
    let live = wire_serve::ambient_root(&root).expect("ambient");
    let mut args = pin_args("Guide/Leader's-Guideline");
    args.if_root = Some(live.clone());

    let out = splice(&root, 0, &args, &[], None).expect("the guarded pin commits");
    let ResponseBody::Splice {
        root_before,
        root_after,
        ..
    } = &out.body
    else {
        panic!("splice body");
    };
    assert_ne!(
        root_before, &live,
        "the reported root_before is post-promotion — the promotion is a real write"
    );
    assert!(root_after.is_some());
    assert!(read_page(&root, "plan.md").contains("```meridian-lock"));
}

/// `git init` with a deterministic identity for fixture plumbing.
fn git_init(dir: &std::path::Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "s7@example.invalid"],
        vec!["config", "user.name", "s7"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
}
