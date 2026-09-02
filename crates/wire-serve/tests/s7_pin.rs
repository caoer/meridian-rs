//! S7: `mrd pin` mints a real `meridian-lock` pin — the pin-proof gate
//! (§ A.3 proof law: the request carries the read's own token), rev-neutral
//! slug promotion (D15), content+lock in one `commit_batch` (D7). Drives
//! `write::splice` on a real workspace; gate tests are in-process read-then-pin.

use std::collections::BTreeMap;
use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PutAt, Recovery, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// Pinning page (no lock yet — first pin births one as file preamble).
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

/// Pinned page; `Leader's Guideline` → D15 slug `leaders-guideline`.
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

/// A pinnable workspace — git-initialised: R4 gives every pin row a `hash`
/// and admits no row without one. The no-git case is its own test
/// ([`without_git_the_pin_refuses_because_r4_admits_no_hashless_row`]).
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
        premises: Vec::new(),
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
            selector: wire::ReadSel::parse(selector),
            vibe: None,
            fingerprint: None,
            sec_rev: None,
        }),
        fields: BTreeMap::default(),
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
        model::selector::Selector::Heading(segs) => model::Ref::Hpath(segs),
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

    let out = splice(
        &root,
        None,
        &pin_args("Guide/Leader's Guideline"),
        &[],
        None,
    )
    .expect("pin commits");
    let fact = pin_fact(&out.body);

    assert_eq!(
        fact.selector,
        wire::ReadSel::Hpath {
            hpath: vec![
                wire::HpathSeg {
                    h: "Guide".into(),
                    n: None
                },
                wire::HpathSeg {
                    h: "Leader's Guideline".into(),
                    n: None
                },
            ]
        },
        "the canonical selector is the RAW SEGMENT address (U14) — not a joined \
         string, and not the sanitized spelling that used to be published"
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
    assert_eq!(
        pinner,
        format!("---\ntitle: Plan\n---\n{expected_block}\n# Plan\n\ndraws from the guide.\n"),
        "placement law: the block births as file preamble in canonical bytes — \
         after the frontmatter, one blank line before the body, the page's own \
         bytes untouched"
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

/// The path array is built from `hpath_raw` segments, never by splitting a
/// joined string (R1.6). `sanitize_heading` is many-to-one (space → `-`), so
/// the assert is on the space in `Leader's Guideline`: a hyphen there would
/// prove a joined, sanitized spelling was the input.
#[test]
fn the_path_array_is_built_from_raw_segments_not_by_splitting_a_joined_string() {
    let (_dir, root) = workspace();
    let out = splice(
        &root,
        None,
        &pin_args("Guide/Leader's Guideline"),
        &[],
        None,
    )
    .expect("pin commits");
    let fact = pin_fact(&out.body);
    assert_eq!(
        fact.selector,
        wire::ReadSel::Hpath {
            hpath: vec![
                wire::HpathSeg {
                    h: "Guide".into(),
                    n: None
                },
                wire::HpathSeg {
                    h: "Leader's Guideline".into(),
                    n: None
                },
            ]
        },
        "the canonical selector carries the RAW text, space intact — there is no \
         sanitized spelling anywhere on the pin path for the array to be built from"
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

/// A heading whose RAW text begins with `^` is refused, not written: R4 spells
/// an anchor pin as a path array whose sole element is a `^id`, so such an
/// element among heading segments would make the row's grain unreadable.
#[test]
fn a_heading_that_begins_with_a_caret_refuses_rather_than_minting_an_ambiguous_array() {
    let (dir, root) = workspace();
    // A SPACE after the caret, deliberately: `^Alpha-Beta` would parse as a
    // real block anchor and refuse one rung earlier (slug collision). `^Alpha
    // Beta` is plain heading text — the case that reaches the array builder.
    std::fs::write(
        dir.path().join("guide.md"),
        "# Guide\n\n## ^Alpha Beta\n\nbody text.\n",
    )
    .expect("a heading whose raw text opens with a caret");

    let err = splice(&root, None, &pin_args("Guide/^Alpha Beta"), &[], None)
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

/// Key-set pin over the serialized `PinFact`. `Option` + `skip_serializing_if`
/// skips on the VALUE, never the session, so the wire key set is decided by
/// what populates a field — and R4 made `blob` mandatory: a pin either carries
/// a hash or refuses. Exact-set assert: any key added or removed fails here,
/// loudly. The absent-`blob` branch is covered by
/// [`without_git_the_pin_refuses_because_r4_admits_no_hashless_row`]; neither
/// half is sufficient alone.
#[test]
fn the_pin_fact_key_set_is_pinned_and_blob_is_now_always_present() {
    let (_dir, root) = workspace();
    let out = splice(
        &root,
        None,
        &pin_args("Guide/Leader's Guideline"),
        &[],
        None,
    )
    .expect("pin commits");
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
            "fingerprint",
            "promoted",
            "selector",
            "target",
        ],
        "the EXACT key set — a new key here is a wire change, an ABSENT `blob` \
         means something started minting hashless pins again, and a returning \
         `declared_ref` means ZT decision 14 is being re-opened"
    );
    assert!(
        map["blob"].as_str().is_some_and(|b| b.len() == 40),
        "and the key carries a real oid, not null: {:?}",
        map["blob"]
    );
    assert!(
        map["selector"].is_object(),
        "U14: `selector` is the TAGGED read selector, not a joined string: {:?}",
        map["selector"]
    );
}

/// GATE 4a: promotion is rev-neutral (D14 honesty — cannot move pinned fingerprint).
#[test]
fn promotion_is_rev_neutral_for_the_pinned_fingerprint() {
    let (_dir, root) = workspace();
    let before = live_fingerprint(&root, "guide.md#Guide/Leader's Guideline");
    let sibling_before = live_fingerprint(&root, "guide.md#Guide/Other");

    let out = splice(
        &root,
        None,
        &pin_args("Guide/Leader's Guideline"),
        &[],
        None,
    )
    .expect("pin commits");
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
        &splice(
            &root,
            None,
            &pin_args("Guide/Leader's Guideline"),
            &[],
            None,
        )
        .unwrap()
        .body,
    );
    let target_after_first = read_page(&root, "guide.md");

    let second = pin_fact(
        &splice(
            &root,
            None,
            &pin_args("Guide/Leader's Guideline"),
            &[],
            None,
        )
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

    let err = splice(
        &root,
        None,
        &pin_args("Guide/Leader's Guideline"),
        &[],
        None,
    )
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
        &splice(
            &root,
            None,
            &pin_args("Guide/Leader's Guideline"),
            &[],
            None,
        )
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
        &splice(
            &root,
            None,
            &pin_args("Guide/Leader's Guideline"),
            &[],
            None,
        )
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

// GATE 2: the pin-proof gate (§ A.3 proof law), single-session in-process

/// A sections read as the engine serves it, returning the served section's
/// proof pair: its `fp1.…` fingerprint and its `sec_rev` — the tokens a
/// later pin of that section carries back.
fn proof_read(root: &fs::WorkspaceRoot, rel: &str, selector: &str) -> (String, String) {
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    let params = wire_serve::read::ReadParams {
        sections: Some(vec![wire::ReadSel::parse(selector)]),
        ..Default::default()
    };
    let body = wire_serve::read::composed_read(
        &doc,
        &WPath(rel.into()),
        &wire::Root("r0".into()),
        &params,
        &wire_serve::read::NO_DECORATIONS,
    )
    .expect("the read serves");
    let ResponseBody::Read { sections, .. } = body else {
        panic!("read body");
    };
    let row = &sections.expect("sections mode")[0];
    (
        row.fingerprint
            .clone()
            .expect("a served section carries its proof token"),
        row.sec_rev.0.clone(),
    )
}

/// An actor pin carrying `proof` (and, when given, the read's `sec_rev`).
fn agent_pin_args(
    actor: &str,
    selector: &str,
    proof: Option<&str>,
    sec_rev: Option<&str>,
) -> SpliceArgs {
    let mut args = pin_args(selector);
    args.actor = Some(actor.to_owned());
    let pin = args.pin.as_mut().expect("pin_args carries a pin");
    pin.fingerprint = proof.map(str::to_owned);
    pin.sec_rev = sec_rev.map(str::to_owned);
    args
}

/// Unproven actor pin refuses; the read's own token then admits the same
/// request — one round trip, no server-side state anywhere.
#[test]
fn the_gate_refuses_an_unproven_pin_and_admits_it_with_the_reads_own_token() {
    let (_dir, root) = workspace();

    let err = splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", None, None),
        &[],
        None,
    )
    .expect_err("an unproven actor pin refuses");
    assert_eq!(err.code, ErrorCode::PinProofRequired);
    assert_eq!(err.recovery, Recovery::Fix, "the caller reads, then pins");
    assert_eq!(err.path, Some(WPath("guide.md".into())));
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("never in your context")),
        "the refusal teaches: {:?}",
        err.message
    );
    // No surface carries a `mode` parameter (MCP: none; wire: `sections`
    // presence IS the mode; CLI: `--section`) — the remedy must
    // not teach one.
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| !m.contains("mode sections") && !m.contains("mode toc")),
        "the remedy names a mode parameter no surface carries: {:?}",
        err.message
    );
    // F-R3 face-wide law (the stale-teaching sweep): a teaching
    // never joins target#selector — the retired fragment grammar refuses at
    // every tool's ref door, so a refusal that spells it hands the caller an
    // invalid address. The selector is named on its own.
    assert!(
        err.message.as_deref().is_some_and(|m| !m.contains(".md#")),
        "the refusal joins target#selector (retired fragment grammar): {:?}",
        err.message
    );
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("\"Guide/Leader's Guideline\" in guide.md")),
        "the refusal names the selector on its own, beside the page: {:?}",
        err.message
    );
    assert_eq!(read_page(&root, "guide.md"), TARGET, "target untouched");
    assert_eq!(read_page(&root, "plan.md"), PINNER, "pinner untouched");

    let (proof, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");
    let out = splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", Some(&proof), None),
        &[],
        None,
    )
    .expect("the read-backed pin commits");
    assert_eq!(
        pin_fact(&out.body).fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Leader's Guideline")
    );
    assert_eq!(
        pin_fact(&out.body).fingerprint,
        proof,
        "the minted fact IS the carried proof — one token, read to pin"
    );
}

/// The token is the NODE's, never the selector spelling's (dogfood r7 F2
/// carried forward): a dewey read's token spends an hpath pin of the same
/// node, and the reverse.
#[test]
fn a_dewey_reads_token_spends_an_hpath_pin_and_the_reverse() {
    let (_dir, root) = workspace();

    // `## Leader's Guideline` is dewey 1.1 in TARGET's toc.
    let (by_dewey, _) = proof_read(&root, "guide.md", "1.1");
    let (by_hpath, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");
    assert_eq!(
        by_dewey, by_hpath,
        "two spellings of one node serve one token"
    );

    splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", Some(&by_dewey), None),
        &[],
        None,
    )
    .expect("a dewey read's token spends a heading-path pin of the node");
}

/// Forms crossed the other way on a fresh workspace: an hpath read's token
/// spends a dewey pin.
#[test]
fn an_hpath_reads_token_spends_a_dewey_pin_of_the_same_node() {
    let (_dir, root) = workspace();

    let (proof, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");
    splice(
        &root,
        None,
        &agent_pin_args("agent-7", "1.1", Some(&proof), None),
        &[],
        None,
    )
    .expect("a heading-path read's token spends a dewey pin of the node");
}

/// Proof is selector-grained: a sibling section's token does not cover this
/// pin — you cannot attest content you did not read.
#[test]
fn a_sibling_sections_token_fails_the_gate() {
    let (_dir, root) = workspace();

    let (other, _) = proof_read(&root, "guide.md", "Guide/Other");
    let err = splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", Some(&other), None),
        &[],
        None,
    )
    .expect_err("a sibling's token does not cover this selector");
    assert_eq!(err.code, ErrorCode::PinProofRequired);
    assert_eq!(read_page(&root, "guide.md"), TARGET, "nothing was written");
}

/// An in-process actor pin with proof commits: no session layer is needed,
/// because there is no server-side state to hold — the proof IS the request.
/// (The old ledgerless-host refusal class died with the ledger.)
#[test]
fn an_in_process_actor_pin_with_proof_commits() {
    let (_dir, root) = workspace();
    let (proof, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");
    splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", Some(&proof), None),
        &[],
        None,
    )
    .expect("proof rides the request, so an in-process actor pin serves");
}

/// The CLI may pin proofless — but a token it DOES supply is verified: trust
/// excuses absence, never a wrong token.
#[test]
fn a_cli_supplied_token_is_still_verified() {
    let (_dir, root) = workspace();

    let mut wrong = pin_args("Guide/Leader's Guideline");
    let pin = wrong.pin.as_mut().expect("pin");
    pin.fingerprint =
        Some("fp1.b3:0000000000000000000000000000000000000000000000000000000000000000".into());
    let err = splice(&root, None, &wrong, &[], None)
        .expect_err("a wrong token refuses even on the trusted door");
    assert_eq!(err.code, ErrorCode::PinProofRequired);
    assert_eq!(read_page(&root, "guide.md"), TARGET, "nothing was written");

    let (proof, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");
    let mut right = pin_args("Guide/Leader's Guideline");
    let pin = right.pin.as_mut().expect("pin");
    pin.fingerprint = Some(proof);
    splice(&root, None, &right, &[], None).expect("a correct token passes the same door");
}

/// A re-pin after promotion passes on the SAME token: the marker moved the
/// section's `sec_rev` but not its fingerprint (anchor removals), so no
/// refresh of anything is owed — the D16 refresh class died with the ledger.
#[test]
fn a_re_pin_after_promotion_passes_on_the_same_token() {
    let (_dir, root) = workspace();
    let (proof, read_rev) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");

    let first = pin_fact(
        &splice(
            &root,
            None,
            &agent_pin_args(
                "agent-7",
                "Guide/Leader's Guideline",
                Some(&proof),
                Some(&read_rev),
            ),
            &[],
            None,
        )
        .expect("the first pin commits")
        .body,
    );
    assert!(first.promoted, "the first pin wrote the marker");

    // Same token, same (now raw-byte-stale) read rev: the fingerprint compare
    // passes first, so the moved `sec_rev` never refuses — a matching proof
    // means the content is current, and anchor churn is not drift.
    let second = pin_fact(
        &splice(
            &root,
            None,
            &agent_pin_args(
                "agent-7",
                "Guide/Leader's Guideline",
                Some(&proof),
                Some(&read_rev),
            ),
            &[],
            None,
        )
        .expect("the re-pin passes on the very token the first read served")
        .body,
    );
    assert!(!second.promoted, "the marker is reused, not re-written");
    assert_eq!(second.fingerprint, proof);
}

/// GATE 7: proof binds content currency → `write_conflict` on drift when the
/// read's rev is carried, with both revs named.
#[test]
fn a_rev_change_between_read_and_pin_is_a_write_conflict_not_a_silent_pin() {
    let (_dir, root) = workspace();
    let (proof, read_rev) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");

    std::fs::write(
        root.0.join("guide.md"),
        TARGET.replace("review before you close.", "review AFTER you close."),
    )
    .expect("foreign edit");

    let err = splice(
        &root,
        None,
        &agent_pin_args(
            "agent-7",
            "Guide/Leader's Guideline",
            Some(&proof),
            Some(&read_rev),
        ),
        &[],
        None,
    )
    .expect_err("the stale proof refuses");
    assert_eq!(err.code, ErrorCode::WriteConflict);
    assert!(
        err.expected.is_some() && err.actual.is_some(),
        "the refusal carries both revs"
    );
    assert!(
        err.message.as_deref().is_some_and(|m| !m.contains(".md#")),
        "the drift refusal joins target#selector (retired fragment grammar): {:?}",
        err.message
    );
    assert!(
        !read_page(&root, "guide.md").contains("^leaders-guideline"),
        "and it refused before the promotion — the gate is ordered first"
    );

    let (fresh, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");
    splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", Some(&fresh), None),
        &[],
        None,
    )
    .expect("the re-read's fresh token authorizes the pin");
}

/// Without the read's rev the gate cannot tell a moved world from a bad
/// token, and the refusal says so — both causes, one remedy.
#[test]
fn without_the_read_rev_a_moved_world_refuses_as_a_proof_mismatch_naming_both_causes() {
    let (_dir, root) = workspace();
    let (proof, _) = proof_read(&root, "guide.md", "Guide/Leader's Guideline");

    std::fs::write(
        root.0.join("guide.md"),
        TARGET.replace("review before you close.", "review AFTER you close."),
    )
    .expect("foreign edit");

    let err = splice(
        &root,
        None,
        &agent_pin_args("agent-7", "Guide/Leader's Guideline", Some(&proof), None),
        &[],
        None,
    )
    .expect_err("the stale proof refuses");
    assert_eq!(err.code, ErrorCode::PinProofRequired);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("either the content moved")),
        "with no rev to split on, the refusal names both causes: {:?}",
        err.message
    );
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
        &splice(&root, None, &pin_args("^claim"), &[], None)
            .unwrap()
            .body,
    );
    assert_eq!(
        fact.selector,
        wire::ReadSel::Anchor {
            anchor: "claim".into()
        },
        "a block pin's canonical selector is the anchor plane's id"
    );
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

    let mut ghost = pin_args("Guide/Leader's Guideline");
    ghost.pin.as_mut().expect("pin").target = WPath("nope.md".into());
    let err = splice(&root, None, &ghost, &[], None).expect_err("no such page");
    assert_eq!(err.code, ErrorCode::PinTargetMissing);
    assert_eq!(err.recovery, Recovery::Fix);

    let err = splice(&root, None, &pin_args("Guide/No Such Section"), &[], None)
        .expect_err("no such selector");
    assert_eq!(err.code, ErrorCode::PinTargetMissing);
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
        &splice(&root, None, &pin_args("Guide/Other"), &[], None)
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
    let eager = pin_fact(&splice(&root, None, &vibe, &[], None).unwrap().body);
    let eager_oid = eager.blob.clone().expect("oid written");
    assert!(
        repo.object_exists(&eager_oid).expect("git answers"),
        "--vibe writes the blob eagerly: {eager_oid}"
    );
}

/// Outside git the pin refuses: R4 folds the hash into the claim, so there is
/// no legal hashless row. Still no fabricated sha (D5) — honesty moved from
/// omission to refusal.
#[test]
fn without_git_the_pin_refuses_because_r4_admits_no_hashless_row() {
    let (_dir, root) = bare_workspace();
    let err = splice(&root, None, &pin_args("Guide/Other"), &[], None)
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
    let mut args = pin_args("Guide/Leader's Guideline");
    args.dry = true;

    let out = splice(&root, None, &args, &[], None).expect("dry rehearses");
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
        &splice(
            &root,
            None,
            &pin_args("Guide/Leader's Guideline"),
            &[],
            None,
        )
        .unwrap()
        .body,
    );
    let second = pin_fact(
        &splice(&root, None, &pin_args("Guide/Other"), &[], None)
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
    let mut args = pin_args("Guide/Leader's Guideline");
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

    let out = splice(&root, None, &args, &[], None).expect("content + lock commit together");
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
        2,
        "content and lock are the same rename — ONE row; the promotion into \
         the target is this call's other write, told as its own row (r8 D4)"
    );
    assert_eq!(frame.delta.files[0].path, WPath("plan.md".into()));
    assert_eq!(frame.delta.files[1].path, WPath("guide.md".into()));
}

/// Self-pin of any own section: the preamble-placed lock sits outside every
/// section span, so even the LAST section — the one the old EOF birth landed
/// inside — pins green.
#[test]
fn a_page_can_pin_its_own_section() {
    let (_dir, root) = workspace();
    std::fs::write(
        root.0.join("plan.md"),
        "---\ntitle: Plan\n---\n\n# Premise\n\nthe premise.\n\n# Plan\n\ndraws from it.\n",
    )
    .expect("two-section pinner");
    // The LAST section, deliberately — the placement the EOF law refused.
    let mut args = pin_args("Plan");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let fact = pin_fact(
        &splice(&root, None, &args, &[], None)
            .expect("self-pin commits")
            .body,
    );
    assert_eq!(
        fact.selector,
        wire::ReadSel::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "Plan".into(),
                n: None
            }]
        }
    );
    assert_eq!(fact.anchor, "plan");

    let page = read_page(&root, "plan.md");
    assert!(
        page.contains("# Plan\n^plan\n"),
        "the promotion landed on its own line: {page}"
    );
    assert!(
        page.starts_with("---\ntitle: Plan\n---\n```meridian-lock\n"),
        "and the lock block sits in the file preamble: {page}"
    );
    // Lock sits in the page but outside the pinned section — green immediately.
    assert_eq!(
        fact.fingerprint,
        live_fingerprint(&root, "plan.md#Plan"),
        "a self-pin verifies green immediately under preamble placement"
    );
}

/// Self-pin of the section a LEGACY block still sits in refuses (permanently
/// red otherwise) — fresh births land in the preamble, but an existing block
/// is replaced in place, so a page carrying its block inside a section keeps
/// the hazard until re-homed.
#[test]
fn a_self_pin_of_the_section_holding_the_lock_refuses() {
    let (_dir, root) = workspace();
    std::fs::write(
        root.0.join("plan.md"),
        format!(
            "{PINNER}\n```meridian-lock\nversion: 2\npins:\n  - object: \"[[guide]]\"\n    \
             hash: \"9ae3f1c0deadbeef9ae3f1c0deadbeef9ae3f1c0\"\n    path: [\"Guide\", \"Steps\"]\n    \
             fingerprint: \"fp1.span2.b3.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n```\n"
        ),
    )
    .expect("legacy pinner: the block sits at EOF, inside # Plan");
    let mut args = pin_args("Plan");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let err = splice(&root, None, &args, &[], None).expect_err("unverifiable by construction");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("lock-is-content")),
        "the refusal names the reason: {:?}",
        err.message
    );
    assert_eq!(
        read_page(&root, "plan.md")
            .matches("```meridian-lock")
            .count(),
        1,
        "and no second block was written"
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

    let err = splice(
        &root,
        None,
        &pin_args("Guide/Leader's Guideline"),
        &[],
        None,
    )
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
        &splice(&root, None, &pin_args("Guide/Padded Title"), &[], None)
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

/// Stale `if_root` refuses before promotion. The token is grammatical on
/// purpose — an ungrammatical value is `bad_request` at §5.7's malformed
/// arm, never this staleness verdict.
#[test]
fn a_stale_world_guard_refuses_before_the_promotion() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Guide/Leader's Guideline");
    args.if_root = Some(wire::Root(format!("b3:{}", "deadbeef".repeat(8))));

    let err = splice(&root, None, &args, &[], None).expect_err("stale plan refuses");
    assert_eq!(err.code, ErrorCode::RootMismatch);
    assert_eq!(read_page(&root, "guide.md"), TARGET, "no promotion");
    assert_eq!(read_page(&root, "plan.md"), PINNER, "no lock");
}

/// Fresh world guard survives the pin's own root advance (guard on client-pinned root).
#[test]
fn a_fresh_world_guard_survives_the_pins_own_root_advance() {
    let (_dir, root) = workspace();
    let live = wire_serve::ambient_root(&root).expect("ambient");
    let mut args = pin_args("Guide/Leader's Guideline");
    args.if_root = Some(live.clone());

    let out = splice(&root, None, &args, &[], None).expect("the guarded pin commits");
    let ResponseBody::Splice {
        root_before,
        root_after,
        ..
    } = &out.body
    else {
        panic!("splice body");
    };
    assert_eq!(
        root_before, &live,
        "the reported root_before is the root the client pinned — the \
         promotion is a real write and the frame tells it as a row, never by \
         silently moving the baseline (r8 D4)"
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

/// A `/`-bearing heading is representable and pinnable on the MACHINE surface
/// (the wire array — what agents and MCP callers send). The CLI coat
/// (`mrd pin --section`) still splits its string on `/` and cannot address it;
/// widening the coat is C2, reserved. The load-bearing negative is the element
/// COUNT: a join-then-split implementation yields `["Guide", "A", "B"]` — a
/// well-formed address resolving to nothing.
#[test]
fn a_slash_bearing_heading_pins_end_to_end_and_stores_as_one_array_element() {
    let (_dir, root) = workspace();
    std::fs::write(
        root.0.join("guide.md"),
        "# Guide\n\n## A/B\n\nreview before you close.\n",
    )
    .expect("rewrite the target with a `/`-bearing heading");

    // The MACHINE surface: two segments, the second carrying the `/` as TEXT.
    let mut args = pin_args("unused");
    args.pin = Some(PinSpec {
        target: WPath("guide.md".into()),
        selector: wire::ReadSel::Hpath {
            hpath: vec![
                wire::HpathSeg {
                    h: "Guide".into(),
                    n: None,
                },
                wire::HpathSeg {
                    h: "A/B".into(),
                    n: None,
                },
            ],
        },
        vibe: None,
        fingerprint: None,
        sec_rev: None,
    });

    let out = splice(&root, None, &args, &[], None)
        .expect("a `/`-bearing heading pins — the refusal that blocked it is lifted");
    let fact = pin_fact(&out.body);

    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let found = lock::find(&doc).expect("parses").expect("present");
    assert_eq!(
        found.lock.pins[0].selector,
        lock::Selector::Path(vec!["Guide".into(), "A/B".into()]),
        "TWO elements: the `/` is heading TEXT, not a delimiter. A joined \
         spelling re-split anywhere on this path would give three."
    );
    assert!(
        read_page(&root, "plan.md").contains("path: [\"Guide\", \"A/B\"]"),
        "and the stored form carries it verbatim: {}",
        read_page(&root, "plan.md")
    );
    assert!(
        fact.fingerprint.starts_with("fp1."),
        "the pin is live, not merely well-formed: {}",
        fact.fingerprint
    );
}

/// The other half of the boundary: `ReadSel::parse` splits on `/`, so
/// `"Guide/A/B"` is three segments resolving to nothing — the coat refuses
/// rather than silently pinning the wrong section.
///
/// The refusal is where the reservation is PAID FOR (`laws.md` D-1): the coat
/// stays un-widened, so the miss owes the caller the two forms that do address
/// the heading — the segment array and the dewey ordinal. A remedy naming only
/// the toc read hands back the same un-feedable title and the recovery loops
/// (dogfood finding #1, reproduced on v1.0.0).
#[test]
fn the_cli_string_coat_still_cannot_address_a_slash_bearing_heading() {
    let (_dir, root) = workspace();
    std::fs::write(
        root.0.join("guide.md"),
        "# Guide\n\n## A/B\n\nreview before you close.\n",
    )
    .expect("rewrite the target with a `/`-bearing heading");

    // Positive control: same coat, same fixture, a heading with no `/` — the
    // miss below is attributable to the split, not a broken fixture.
    splice(&root, None, &pin_args("Guide"), &[], None)
        .expect("the coat addresses an ordinary heading in this very fixture");

    let err = splice(&root, None, &pin_args("Guide/A/B"), &[], None)
        .expect_err("the coat splits on `/`, so this address resolves to nothing");
    assert_eq!(
        err.code,
        ErrorCode::PinTargetMissing,
        "it MISSES — it must never silently pin a different section: {err:?}"
    );
    let msg = err.message.as_deref().expect("the miss carries a message");
    assert!(
        msg.contains("hpath array") && msg.contains("dewey ordinal"),
        "the refusal teaches both working forms, not the toc read alone: {msg}"
    );
    assert!(
        msg.contains("splits on `/`"),
        "and it names the delimiter that made this spelling unaddressable: {msg}"
    );
}

/// A duplicated block id refuses the pin — no door may pin an occurrence the
/// caller did not name (wire-contract A.3, door symmetry over duplicate block
/// ids). The old door silently pinned the FIRST carrier: the one silent-pick
/// surface left after the read and write doors both refused.
#[test]
fn a_duplicated_anchor_refuses_the_pin_rather_than_picking_the_first() {
    let (dir, root) = workspace();
    std::fs::write(
        dir.path().join("guide.md"),
        "# Guide\n\n- first ^same-id\n\n- second ^same-id\n",
    )
    .expect("duplicated target");
    let err = splice(&root, None, &pin_args("^same-id"), &[], None)
        .expect_err("a duplicated id must refuse the pin");
    assert_eq!(err.code, ErrorCode::AmbiguousRef);
    let msg = err.message.as_deref().expect("message");
    assert!(msg.contains("2 blocks carry this id"), "{msg}");
    assert!(
        !msg.contains("rename one heading"),
        "the remedy speaks the anchor grammar: {msg}"
    );
}

// GATE 12: duplicate-heading occurrences — the mint de-collides its slug by the
// occurrence ordinal, and the lock stores the RESOLVED selector (dogfood r8
// D2+D3, card pin-mint-occurrence-handling: a pin that greys `ambiguous` in the
// session that minted it is a broken attestation).

/// Two same-named siblings under one parent — the r8 fixture's shape.
const DUP_TARGET: &str = "# Guide\n\n## Dup\n\nfirst dup body.\n\n## Dup\n\nsecond dup body.\n";

fn dup_workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let (dir, root) = workspace();
    std::fs::write(root.0.join("guide.md"), DUP_TARGET).expect("dup target");
    (dir, root)
}

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn seg_n(h: &str, n: u32) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: Some(n),
    }
}

/// Pin-only splice addressing `guide.md` by segment array (the machine surface —
/// the string coat has no occurrence spelling by design).
fn pin_hpath_args(hpath: Vec<wire::HpathSeg>) -> SpliceArgs {
    let mut args = pin_args("unused");
    args.pin = Some(PinSpec {
        target: WPath("guide.md".into()),
        selector: wire::ReadSel::Hpath { hpath },
        vibe: None,
        fingerprint: None,
        sec_rev: None,
    });
    args
}

/// Live fingerprint of the n-th `Guide/Dup` occurrence, read from disk — the
/// same mint the verify plane recomputes.
fn live_occurrence_fingerprint(root: &fs::WorkspaceRoot, occurrence: u32) -> String {
    let doc = fs::load(root, std::path::Path::new("guide.md")).expect("load");
    let target = model::resolve(
        &doc,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Guide".into(),
                n: None,
            },
            model::HpathSeg {
                h: "Dup".into(),
                n: Some(occurrence),
            },
        ]),
    )
    .expect("the occurrence resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
        .expect("the fixture section has content")
        .into_string()
}

/// r8 D2: the SECOND same-named sibling is pinnable. The mint de-collides its
/// slug with the occurrence ordinal (`^dup-2`) instead of refusing with a
/// remedy that names an own `^id` the sibling does not have.
#[test]
fn a_pin_on_the_second_duplicate_sibling_mints_a_de_collided_slug() {
    let (_dir, root) = dup_workspace();

    let out = splice(
        &root,
        None,
        &pin_hpath_args(vec![seg("Guide"), seg_n("Dup", 2)]),
        &[],
        None,
    )
    .expect("the second occurrence pins — de-collision, not refusal");
    let fact = pin_fact(&out.body);

    assert_eq!(
        fact.anchor, "dup-2",
        "the anchor is the title slug plus the occurrence ordinal — \
         self-describing, and free even though ^dup is not minted yet"
    );
    assert!(
        fact.promoted,
        "the sibling had no id, so this pin wrote one"
    );
    assert_eq!(
        read_page(&root, "guide.md"),
        "# Guide\n\n## Dup\n\nfirst dup body.\n\n## Dup\n^dup-2\n\nsecond dup body.\n",
        "the marker lands under the SECOND sibling, own line, heading untouched"
    );
    assert_eq!(
        fact.fingerprint,
        live_occurrence_fingerprint(&root, 2),
        "a freshly minted occurrence pin verifies green immediately"
    );
    assert!(
        read_page(&root, "plan.md").contains("path: [\"Guide\", \"Dup#2\"]"),
        "r8 D3: the lock stores the RESOLVED selector — the occurrence rides \
         the stored segment, so the pin cannot walk grey in its own session: {}",
        read_page(&root, "plan.md")
    );
}

/// The occurrence ids are order-independent: `^dup-2` names the second sibling
/// whichever sibling pins first, the first occurrence keeps the bare title slug,
/// and every occurrence pin stores its ordinal in its lock row.
#[test]
fn occurrence_pins_are_order_independent_and_store_their_ordinals() {
    let (_dir, root) = dup_workspace();

    // Second sibling FIRST — the bare slug is free, the ordinal still names it.
    let second = pin_fact(
        &splice(
            &root,
            None,
            &pin_hpath_args(vec![seg("Guide"), seg_n("Dup", 2)]),
            &[],
            None,
        )
        .expect("second occurrence pins")
        .body,
    );
    assert_eq!(second.anchor, "dup-2");

    let first = pin_fact(
        &splice(
            &root,
            None,
            &pin_hpath_args(vec![seg("Guide"), seg_n("Dup", 1)]),
            &[],
            None,
        )
        .expect("first occurrence pins")
        .body,
    );
    assert_eq!(
        first.anchor, "dup",
        "the first occurrence keeps the bare title slug — r8 G1's receipts hold"
    );

    let pinner = read_page(&root, "plan.md");
    assert!(
        pinner.contains("path: [\"Guide\", \"Dup#1\"]"),
        "the first occurrence stores its ordinal too — a bare `Dup` row would \
         be born ambiguous: {pinner}"
    );
    assert!(pinner.contains("path: [\"Guide\", \"Dup#2\"]"), "{pinner}");
    assert_eq!(
        read_page(&root, "guide.md"),
        "# Guide\n\n## Dup\n^dup\n\nfirst dup body.\n\n## Dup\n^dup-2\n\nsecond dup body.\n",
        "each sibling carries its own marker"
    );
}

/// A re-pin of the same occurrence reuses the marker it minted — idempotence
/// holds through the de-collided spelling.
#[test]
fn a_de_collided_pin_is_idempotent_on_re_pin() {
    let (_dir, root) = dup_workspace();
    let args = pin_hpath_args(vec![seg("Guide"), seg_n("Dup", 2)]);

    let minted = pin_fact(&splice(&root, None, &args, &[], None).expect("mints").body);
    assert!(minted.promoted);
    let target_after_mint = read_page(&root, "guide.md");

    let reused = pin_fact(&splice(&root, None, &args, &[], None).expect("re-pins").body);
    assert_eq!(reused.anchor, "dup-2");
    assert!(
        !reused.promoted,
        "the slot already bears the id — reuse, no write"
    );
    assert_eq!(
        read_page(&root, "guide.md"),
        target_after_mint,
        "no second marker"
    );
    assert_eq!(reused.fingerprint, minted.fingerprint);
}

/// r8 D2's residual arm: when even the de-collided id is taken, the refusal's
/// remedy is EXECUTABLE against the target as it stands. The old remedy said
/// "give that node's own ^id as the selector" — an id the sibling did not have.
#[test]
fn a_taken_de_collided_slug_refuses_with_an_executable_remedy() {
    const PRE_TAKEN: &str =
        "# Guide\n\nnoise ^dup-2\n\n## Dup\n\nfirst dup body.\n\n## Dup\n\nsecond dup body.\n";
    let (_dir, root) = dup_workspace();
    std::fs::write(root.0.join("guide.md"), PRE_TAKEN).expect("pre-taken de-collided id");

    let err = splice(
        &root,
        None,
        &pin_hpath_args(vec![seg("Guide"), seg_n("Dup", 2)]),
        &[],
        None,
    )
    .expect_err("the de-collided id is taken by another node");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("^dup-2") && msg.contains("already taken"),
        "the refusal names the id that collided: {msg}"
    );
    assert!(
        msg.contains("append") && msg.contains("read") && msg.contains("pin at"),
        "the remedy teaches the escape that exists — append your own id under \
         the heading, read it back, pin it: {msg}"
    );
    assert!(
        !msg.contains("give that node's own ^id"),
        "the unfollowable r8 remedy is gone: {msg}"
    );
    assert_eq!(
        read_page(&root, "guide.md"),
        PRE_TAKEN,
        "nothing was written"
    );
}

/// The strict-plane guard around the de-collision: an occurrence the caller did
/// NOT name still refuses `ambiguous_ref` — de-collision names ids, it never
/// picks siblings (wire-contract A.3).
#[test]
fn a_bare_duplicate_heading_pin_still_refuses_ambiguous_ref() {
    let (_dir, root) = dup_workspace();
    let err = splice(
        &root,
        None,
        &pin_hpath_args(vec![seg("Guide"), seg("Dup")]),
        &[],
        None,
    )
    .expect_err("no door may pin an occurrence the caller did not name");
    assert_eq!(err.code, ErrorCode::AmbiguousRef);
    assert_eq!(read_page(&root, "guide.md"), DUP_TARGET, "nothing written");
}
