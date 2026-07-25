//! S7: `mrd pin` mints a real `meridian-lock` pin — the read-mint gate (D16),
//! rev-neutral slug promotion (D15), and content+lock in ONE `commit_batch`
//! (D7).
//!
//! The pin rides `Op::Splice` as a sibling field, so `args.path` is the PINNING
//! page and the lock block is a content edit on it. Every test here drives the
//! production choke-point (`wire_serve::write::splice`) against a real
//! on-disk workspace, and the gate tests are SINGLE-SESSION IN-PROCESS: one
//! `ReadMintStore` held across a read call and then a pin call, which is the
//! only shape that models a daemon session (two CLI processes could not).

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PutAt, Recovery, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// The pinning page — the drawing end. It has no lock block, so the first pin
/// BIRTHS one at EOF.
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

/// The pinned page. `Leader's Guideline` exercises the D15 slug (apostrophe
/// dropped, not separating): the derived id is `leaders-guideline`.
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// A pin-only splice: no caller edits, the lock block IS the write.
fn pin_args(selector: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
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

/// The pin fact off a splice response.
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

/// The live fingerprint of a lock `ref`, computed exactly the way the VERIFY
/// plane computes it: parse the ref through the normative selector grammar,
/// resolve it against the live document, hash that span. Every drift assertion
/// in this file goes through here — a pin whose minted token does not equal this
/// value is a pin that reads red the moment it lands.
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
        .0
}

// ---------------------------------------------------------------------------
// GATE 1 + 3: a real pin lands, and the bare CLI (actor absent) is trusted
// ---------------------------------------------------------------------------

/// The whole verb, end to end: a pin with NO actor (the local-operator-trusted
/// CLI door, D16) births a canonical lock block carrying `pins:` AND `objects:`,
/// promotes the slug anchor into the target, and mints a fingerprint the verify
/// plane recomputes identically.
#[test]
fn a_bare_cli_pin_mints_a_real_lock_block_and_promotes_the_slug() {
    let (dir, root) = workspace();
    // A repo, so the `objects:` plane has a blob to name.
    git_init(dir.path());

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

    // The pinned digest is EXACTLY what the verify plane recomputes.
    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline"),
        "a freshly minted pin must verify green immediately"
    );

    // The lock block, on disk, in canonical form.
    let pinner = read_page(&root, "plan.md");
    let expected_block = format!(
        "```meridian-lock\n\
         version: 1\n\
         objects:\n\
         \x20 \"guide.md\": \"{blob}\"\n\
         pins:\n\
         \x20 - ref: \"guide.md#Guide/Leader's Guideline\"\n\
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

    // The promotion landed on its OWN LINE under the heading, and nowhere else.
    assert_eq!(
        read_page(&root, "guide.md"),
        TARGET.replace(
            "## Leader's Guideline\n",
            "## Leader's Guideline\n^leaders-guideline\n"
        ),
        "promotion writes exactly one own-line slug marker, leaving the heading \
         text (and therefore the section's address) untouched"
    );

    // And it round-trips through the strict reader.
    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(found.lock.pins.len(), 1);
    assert_eq!(found.lock.objects, vec![("guide.md".into(), blob)]);
}

/// GATE 4a: the promotion is **rev-neutral** — the pinned section's fingerprint
/// is byte-identical before and after the slug marker lands. This is the honesty
/// claim behind D14 (promoting into a target this actor may not own is permitted
/// BECAUSE it cannot move that target's fingerprint): if it moved, every OTHER
/// page pinning the same section would redden on somebody else's pin.
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

/// GATE 4b: promotion is **idempotent** — a re-pin recomputes the SAME slug,
/// sees it already present, and promotes nothing. That is what keeps a benign
/// orphan from accumulating (a counter or random id would leave a new marker per
/// pin, each one a fingerprint-neutral but permanent wart).
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
    // The lock still holds exactly ONE pin: `upsert_pin` unions in place.
    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(found.lock.pins.len(), 1, "a re-pin updates, never appends");
}

/// GATE 5a: a REFUSAL leaves no orphan. This test used to assert the opposite,
/// and the assertion was the defect (review finding 12): it simulated the G3
/// crash residual with a deterministic REFUSAL — a corrupt lock block on the
/// pinning page — and then asserted the marker survived it. A refusal is not a
/// crash. A crash is survivable and heals; that refusal repeated identically on
/// every re-pin, so the bytes it left in a page the request does not even name
/// could never heal, and the promotion is now ordered after every refusal rung.
///
/// The vehicle is worth keeping precisely because it refuses at a DIFFERENT rung
/// from the ref-grammar repro in `s2fix_promotion` (`lock_engine_edit`, inside
/// the batch composition rather than the prologue): between them they show it is
/// the ORDERING that holds, not one patched rung.
#[test]
fn a_corrupt_lock_refuses_the_pin_and_leaves_no_orphan_behind() {
    let (_dir, root) = workspace();
    // A hand-mangled lock block: `lock::find` refuses it, so the pin cannot
    // compose its lock edit at all.
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

    // Repair the page by hand (the #8 §3 remedy the refusal names) and the same
    // pin commits — the refusal was about the lock state, not the pin.
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

/// GATE 5b: the benign orphan still heals. The pin's two inodes are two renames
/// (residual G3, unchanged and still accepted), and the promotion's rename is
/// ordered first — so the one survivable failure mode is a CRASH between them:
/// "anchor written, lock not". A refusal can no longer produce that state
/// ([`a_corrupt_lock_refuses_the_pin_and_leaves_no_orphan_behind`]), so the
/// aftermath is staged the way a crash leaves it — the marker on disk, no claim
/// — and the claim under test is what a later pin does with it.
#[test]
fn a_crash_orphan_is_benign_and_a_re_pin_reuses_it() {
    let (_dir, root) = workspace();
    let fp_before = live_fingerprint(&root, "guide.md#Guide/Leader's Guideline");
    // The state a crash between the two renames leaves behind: exactly what
    // `promote_anchor` writes, and nothing else.
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

// ---------------------------------------------------------------------------
// GATE 2: the read-mint gate, single-session in-process
// ---------------------------------------------------------------------------

/// One engine session: a `ReadMintStore` held across a read call and then a pin
/// call — the H1 shape (the ledger is NOT the engine's, so a write cannot
/// evaporate it). Serving the read through the production arm is the point: the
/// receipt under test is the one a real read mints.
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

/// The agent path refuses an UNREAD pin — "you cannot attest content that was
/// never in your context" — and the same request succeeds once a covering read
/// has minted its receipt. Both halves in ONE session, ONE store.
#[test]
fn the_gate_refuses_an_unread_pin_and_admits_it_after_a_covering_read() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    let args = agent_pin_args("agent-7", "Guide/Leader's-Guideline");

    // Nothing read yet.
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
    // And it refused BEFORE any byte moved — no promotion, no lock.
    assert_eq!(read_page(&root, "guide.md"), TARGET, "target untouched");
    assert_eq!(read_page(&root, "plan.md"), PINNER, "pinner untouched");

    // Read the exact selector, then pin: same request, now admitted.
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

/// The gate is keyed on all three parts of the receipt. A FOREIGN actor's read
/// does not authorize mine, and a SIBLING section's read does not authorize a
/// pin into a section nobody saw (D6's grain — the whole reason the receipt is
/// selector-grained rather than doc-level).
#[test]
fn another_actors_read_and_a_sibling_sections_read_both_fail_the_gate() {
    let (_dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();

    // The right selector, read by the WRONG actor.
    session_read(
        &root,
        &store,
        "somebody-else",
        "guide.md",
        "Guide/Leader's-Guideline",
    );
    // The right actor, reading a SIBLING selector.
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

/// A host with NO session layer (the per-request sidecar) cannot know what an
/// actor read, so a pin carrying an actor refuses — and the message NAMES that
/// reason, so the caller reads it as the architecture rather than a bug.
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

/// GATE 7: the rev-recheck. A receipt answers "was it read", never "is it
/// current" — so a target edited between the read and the pin yields
/// `write_conflict`, never a silent pin over bytes the actor never saw.
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

    // A foreign writer changes the very section the receipt covers.
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

    // A re-read re-mints at the live rev; the pin then lands.
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

// ---------------------------------------------------------------------------
// The rest of plan §6's edge cases
// ---------------------------------------------------------------------------

/// An `^id` selector needs no promotion: the stable handle already exists, so
/// the pin reuses it verbatim and the pin's grain is that block's own span.
#[test]
fn an_anchor_selector_is_pinned_at_block_grain_with_no_promotion() {
    let (_dir, root) = workspace();
    // The read face projects anchor rows from LIST ITEMS only (Go parity:
    // paragraph/task/callout/fence anchors are not addressable there), and the
    // pin path inherits that addressability rather than growing its own.
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
}

/// A missing page and a missing selector both refuse `pin_target_missing`
/// rather than minting a claim the drift plane could only render red(dangling).
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
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("mode toc")),
        "the refusal teaches how to find the real selectors: {:?}",
        err.message
    );
    assert_eq!(read_page(&root, "plan.md"), PINNER, "nothing written");
}

/// `--vibe` writes the blob EAGERLY into the object store, so the pinned content
/// is retrievable before any commit references it (a normal pin only computes
/// the oid). Proven by asking git whether the object exists.
#[test]
fn vibe_writes_the_eager_blob_where_a_normal_pin_only_computes_it() {
    let (dir, root) = workspace();
    git_init(dir.path());
    let repo = git::Repo::at(dir.path().to_path_buf());

    // Normal pin: the oid is named, the object is NOT in the store.
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

    // Vibe pin of the same target (its bytes moved under the promotion, so ask
    // for the fresh oid) — now the object EXISTS.
    let mut vibe = pin_args("Guide/Other");
    vibe.pin.as_mut().expect("pin").vibe = Some(true);
    let eager = pin_fact(&splice(&root, 0, &vibe, &[], None).unwrap().body);
    let eager_oid = eager.blob.clone().expect("oid written");
    assert!(
        repo.object_exists(&eager_oid).expect("git answers"),
        "--vibe writes the blob eagerly: {eager_oid}"
    );
}

/// Outside a git repository the claim plane still lands and the retrieval plane
/// is simply ABSENT — honest degradation (D5), never a fabricated sha.
#[test]
fn without_git_the_pin_lands_with_no_objects_entry() {
    let (_dir, root) = workspace();
    let fact = pin_fact(
        &splice(&root, 0, &pin_args("Guide/Other"), &[], None)
            .unwrap()
            .body,
    );
    assert!(fact.blob.is_none(), "no repo, no oid");

    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(found.lock.pins.len(), 1, "the claim plane landed");
    assert!(
        found.lock.objects.is_empty(),
        "the retrieval plane is absent"
    );
}

/// A dry pin rehearses everything and writes NOTHING — not the lock, and not the
/// promotion (zero disk effects means zero), while still reporting the plan.
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

/// A second pin on the same page UNIONS into the existing lock: the sibling
/// claim keeps its position and its fingerprint, and the page carries exactly
/// one block.
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
    assert_eq!(found.lock.pins[0].declared_ref, first.declared_ref);
    assert_eq!(found.lock.pins[0].fingerprint, first.fingerprint);
    assert_eq!(found.lock.pins[1].declared_ref, second.declared_ref);
    assert_eq!(
        doc.raw.matches("```meridian-lock").count(),
        1,
        "the sole writer mints exactly one block"
    );
}

/// A pin can ride ALONGSIDE caller edits on the pinning page: both the content
/// edit and the lock block land in the SAME sealed batch, so the page moves
/// once. The armed facts stay 1:1 with the REQUEST edits — the engine-minted
/// lock edit is reported through the pin fact, never as a phantom armed row.
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

/// A page pinning ITS OWN section: the promotion lands on the same file the lock
/// is about to be written to, so the pre-image must be re-read between them.
/// Getting this wrong would splice the lock against stale spans.
///
/// The pinned section must not be the one the lock block lands in — see
/// [`a_self_pin_of_the_section_holding_the_lock_refuses`] for that door.
#[test]
fn a_page_can_pin_its_own_section() {
    let (_dir, root) = workspace();
    // `# Plan` is the page's last section, so an EOF lock would land inside it;
    // pin the FIRST of two sections instead.
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
    // Self-consistency: the lock block sits inside the PAGE but outside the
    // PINNED section, so the pinned fingerprint is still the live one. This is
    // the assertion that would break if the lock had been spliced against the
    // pre-promotion pre-image.
    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "plan.md#Premise"),
        "a self-pin of a non-containing section verifies green immediately"
    );
}

/// The other half of lock-is-content: pinning the section the lock block itself
/// lands in could never be green, on this write or any later one, so it is
/// refused at mint time rather than written as a permanently-red claim.
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

/// A slug that is already taken by ANOTHER node refuses loudly instead of
/// minting a duplicate id (which would make the handle ambiguous forever).
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

/// A heading line ending in whitespace still promotes rev-neutrally, because the
/// marker never touches that line: norm-v2's R2 removal takes the whole marker
/// line including its terminator, so the padding is irrelevant. (A tail marker
/// would have had to reason about it — and would have edited the heading text.)
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

/// The world guard still means what it says on the pin path: a stale `if_root`
/// refuses BEFORE the promotion, so a rejected plan never leaves a marker
/// behind.
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

/// A FRESH world guard passes even though the pin's own promotion advances the
/// corpus root mid-splice — the guard is honored against the root the client
/// pinned, and the batch re-guards on the current one.
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

/// `git init` with a deterministic identity, so the fixture repo answers
/// plumbing calls on any machine.
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
