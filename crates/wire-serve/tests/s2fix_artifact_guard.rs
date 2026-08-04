//! Advisor R25, structural fix 1 — **GUARD THE ARTIFACT, not the verb.**
//!
//! The read-mint gate (D16) guards the `splice.pin` DOOR. The thing it protects
//! is the `meridian-lock` block, which is ordinary page text: every put shape can
//! compose those bytes without the pin field ever being set. The review
//! reproduced it — an actor with zero read receipts wrote a **Green** pin through
//! native `edits` (finding 4, P1).
//!
//! Every test here asserts on the WRITE (`is_err()`), never on the colour the
//! forged claim would have rendered: a fix that lets the block commit and paints
//! it grey fails by construction (R26).
//!
//! The doors driven here are the four the review names plus the two this unit
//! enumerated: native `edits`, lowered `plan_edits`, `write::create`, the pin's
//! own anchor promotion (a raw file replace outside the batch), and the legit
//! `splice.pin` mint (the control — a guard that also refuses the real mint would
//! be worthless). `run::executor` is the fifth door and is driven in
//! `crates/run/tests/executor.rs` (it bypasses `splice` entirely).

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PlanEdit, PutAt, SecRef};
use wire_serve::write::{CreateArgs, SpliceArgs, splice};

/// The pinning page — no lock block, so a first pin BIRTHS one at EOF.
const PINNER: &str = "# Plan\n\ndraws from the guide.\n";
/// The pinned page. A HEADING ref, never a bare `#^anchor`: an anchor node's
/// span is its host line, so a bare-anchor fixture fingerprints the empty span in
/// every document and proves nothing (R31, fix3's false-gate self-catch).
const TARGET: &str = "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n";
/// The same shape carrying DIFFERENT content — the control that proves the
/// fixture's fingerprints are real (see `fingerprints_in_this_fixture_differ`).
const DECOY: &str = "# Guide\n\n## Leader's Guideline\n\nship it without reading.\n";

/// Git-initialised: these tests mint pins through the production door, and an
/// R4 pin row carries a `hash` only git can answer for.
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "s2fix@example.invalid"],
        vec!["config", "user.name", "s2fix"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn args_for(path: &str, actor: Option<&str>, edits: Vec<Edit>, pin: Option<PinSpec>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path.into()),
        actor: actor.map(str::to_owned),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits,
        plan_edits: Vec::new(),
        pin,
    }
}

fn put_at_end(section: &str, text: &str) -> Edit {
    Edit {
        target: SecRef::Hpath {
            hpath: vec![wire::HpathSeg {
                h: section.into(),
                n: None,
            }],
        },
        edit: EditShape::Put {
            at: PutAt::End,
            text: text.to_owned(),
        },
        if_node_rev: None,
    }
}

/// The honest fingerprint of the target section, computed the way the VERIFY
/// plane computes it — so a forged block carries a token that would VERIFY, not
/// a made-up one. A guard that only catches nonsense tokens is not a guard.
fn live_fingerprint(root: &fs::WorkspaceRoot, rel: &str, selector: &str) -> String {
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    let sel = model::selector::Selector::parse(&format!("{rel}#{selector}"));
    let (d, resolved) =
        model::selector::resolve_selector(&sel, Some(&doc)).expect("selector resolves");
    let removals = syntax::anchor_removals(&d.raw);
    model::fingerprint::fingerprint_span(d, &resolved.span, &removals)
        .expect("the fixture target has content")
        .into_string()
}

/// The R4 blob hash a fixture pin carries when the test is not measuring the
/// retrieval plane. R4 makes `hash` MANDATORY, so there is no pin without one.
const FIXTURE_BLOB: &str = "9ae3f1deadbeef";

/// One R4 pin in the bytes `lock::render` emits. `declared_ref` is the
/// `page[#A/B]` convenience spelling this fixture speaks, split here into the
/// `object` wikilink and the `path` ARRAY — R4 admits no joined string on the
/// row, and no `ref:`.
fn lock_block(declared_ref: &str, fingerprint: &str) -> String {
    let (target, fragment) = match declared_ref.split_once('#') {
        Some((t, f)) => (t, f),
        None => (declared_ref, ""),
    };
    let object = target.strip_suffix(".md").unwrap_or(target);
    let path = if fragment.is_empty() {
        String::new()
    } else {
        fragment
            .split('/')
            .map(|seg| format!("\"{seg}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "\n```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{FIXTURE_BLOB}\"\n    path: [{path}]\n    \
         fingerprint: \"{fingerprint}\"\n```\n"
    )
}

/// R31 control: the two documents this fixture compares must fingerprint
/// DIFFERENTLY, or every assertion above them is green by accident.
#[test]
fn fingerprints_in_this_fixture_differ() {
    let (dir, root) = workspace();
    let target = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    std::fs::write(dir.path().join("guide.md"), DECOY).expect("decoy");
    let decoy = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    assert_ne!(
        target, decoy,
        "the fixture's target and decoy hash identically — the fixture proves nothing"
    );
}

/// **Finding 4 (P1), the reviewer's forge repro.** One actor, one EMPTY receipt
/// ledger: the gated door refuses, and the ungated door must refuse too.
#[test]
fn an_unread_actor_cannot_forge_a_pin_through_ordinary_edits() {
    let (dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    let mallory = Some("agent-mallory");
    let before = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");

    // 1. The GATED door: a pin from an actor who never read the target.
    let gated = splice(
        &root,
        0,
        &args_for(
            "plan.md",
            mallory,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: "Guide/Leader's-Guideline".into(),
                vibe: None,
            }),
        ),
        &[],
        Some(&store),
    );
    assert_eq!(
        gated.as_ref().err().map(|e| e.code),
        Some(ErrorCode::ReadMintRequired),
        "the read-mint gate must refuse an un-read pin"
    );

    // 2. The UNGATED door: the same claim as ordinary page text, carrying a
    //    fingerprint that would verify GREEN.
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let forged = splice(
        &root,
        0,
        &args_for(
            "plan.md",
            mallory,
            vec![put_at_end(
                "Plan",
                &lock_block("guide.md#Guide/Leader's Guideline", &token),
            )],
            None,
        ),
        &[],
        Some(&store),
    );

    assert!(
        forged.is_err(),
        "an actor with no receipt must not commit lock bytes through an ordinary edit — \
         the ARTIFACT must be guarded, not just the pin verb"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        before,
        "a refused forge must leave the file byte-unchanged"
    );
    assert!(
        store.is_empty(),
        "no read was ever minted for this actor — the ledger must still be empty"
    );
    assert!(
        lock::find(&fs::load(&root, std::path::Path::new("plan.md")).expect("load"))
            .expect("parses")
            .is_none(),
        "no lock block may exist on the page"
    );
}

/// The guard is on the ARTIFACT, so it does not care who is asking: the
/// local-operator CLI shape (`actor: None`, which D16 trusts for the PIN verb)
/// cannot write lock bytes as page text either.
#[test]
fn the_local_operator_door_cannot_write_lock_bytes_as_page_text() {
    let (_dir, root) = workspace();
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let forged = splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            vec![put_at_end(
                "Plan",
                &lock_block("guide.md#Guide/Leader's Guideline", &token),
            )],
            None,
        ),
        &[],
        None,
    );
    assert_eq!(
        forged.as_ref().err().map(|e| e.code),
        Some(ErrorCode::BadRequest),
        "lock bytes as page text refuse regardless of actor"
    );
}

/// Door 2 — the lowered `plan_edits` batch reaches the same bytes.
#[test]
fn plan_edits_lowering_cannot_forge_a_lock() {
    let (dir, root) = workspace();
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let mut args = args_for("plan.md", Some("agent-mallory"), Vec::new(), None);
    args.plan_edits = vec![PlanEdit::Append {
        hpath: "Plan".into(),
        body: lock_block("guide.md#Guide/Leader's Guideline", &token),
    }];
    let forged = splice(
        &root,
        0,
        &args,
        &[],
        Some(&receipt::read_mint::ReadMintStore::new()),
    );
    assert!(forged.is_err(), "plan_edits is a door to the same artifact");
    assert!(
        !std::fs::read_to_string(dir.path().join("plan.md"))
            .expect("read")
            .contains("meridian-lock"),
        "nothing landed"
    );
}

/// Door 3 — a BIRTH carrying a lock block. `create` has no pre-image, so any
/// lock bytes in the body are a claim nobody computed.
#[test]
fn create_cannot_birth_a_page_carrying_a_lock() {
    let (dir, root) = workspace();
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let born = wire_serve::write::create(
        &root,
        0,
        &CreateArgs {
            id: None,
            path: WPath("forged.md".into()),
            body: format!(
                "# Forged\n{}",
                lock_block("guide.md#Guide/Leader's Guideline", &token)
            ),
            actor: Some("agent-mallory".into()),
            now: None,
            if_root: None,
            dry: false,
        },
        &[],
    );
    assert!(born.is_err(), "a birth is a door to the artifact too");
    assert!(
        !dir.path().join("forged.md").exists(),
        "the refused birth must leave no file"
    );
}

/// An ordinary edit may not REPLACE a minted lock either — rewriting the pin's
/// fingerprint is the same forgery as writing one.
#[test]
fn ordinary_edits_cannot_rewrite_a_minted_lock() {
    let (dir, root) = workspace();
    // The legit mint first (CLI shape, D16 local-operator-trusted).
    splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: "Guide/Leader's-Guideline".into(),
                vibe: None,
            }),
        ),
        &[],
        None,
    )
    .expect("the minted pin lands");
    let minted = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    let real = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");

    // Now drift the target and try to "re-green" the claim by hand.
    std::fs::write(dir.path().join("guide.md"), DECOY).expect("drift");
    let drifted = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    assert_ne!(
        real, drifted,
        "the drift must actually move the fingerprint"
    );

    let forged = splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![wire::HpathSeg {
                        h: "Plan".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Match {
                    old: real.clone(),
                    new: drifted,
                },
                if_node_rev: None,
            }],
            None,
        ),
        &[],
        None,
    );
    assert!(
        forged.is_err(),
        "re-greening a drifted pin by hand must refuse"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        minted,
        "the refused rewrite must leave the lock byte-unchanged"
    );
}

/// THE CONTROL: the guard admits exactly the block the call minted — the real
/// pin path, its idempotent re-pin, and a second pin unioned into the same lock.
#[test]
fn the_minted_pin_still_lands_and_re_pins_idempotently() {
    let (dir, root) = workspace();
    let pin = |sel: &str| {
        splice(
            &root,
            0,
            &args_for(
                "plan.md",
                None,
                Vec::new(),
                Some(PinSpec {
                    target: WPath("guide.md".into()),
                    selector: sel.into(),
                    vibe: None,
                }),
            ),
            &[],
            None,
        )
    };
    pin("Guide/Leader's-Guideline").expect("first pin lands");
    let after_first = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    assert!(after_first.contains("```meridian-lock"), "{after_first}");

    pin("Guide/Leader's-Guideline").expect("the re-pin lands");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        after_first,
        "a re-pin of unchanged content is byte-idempotent"
    );

    // A content edit on the SAME page, riding beside an untouched lock, is
    // ordinary work — the guard must not refuse it.
    splice(
        &root,
        0,
        &args_for(
            "plan.md",
            Some("agent-scribe"),
            vec![put_at_end("Plan", "one more line.\n")],
            None,
        ),
        &[],
        Some(&receipt::read_mint::ReadMintStore::new()),
    )
    .expect("an ordinary edit beside an untouched lock still commits");
}

/// The pin's anchor promotion writes the TARGET through a raw `fs::replace_file`
/// outside the batch (finding 9). It must be lock-NEUTRAL: a target that carries
/// its own lock keeps it byte-for-byte across the promotion.
#[test]
fn the_anchor_promotion_leaves_the_targets_lock_untouched() {
    let (dir, root) = workspace();
    // Give the TARGET its own minted lock first, through the guarded path.
    std::fs::write(
        dir.path().join("src.md"),
        "# Src\n\n## Source Guideline\n\nread me first.\n",
    )
    .expect("src");
    splice(
        &root,
        0,
        &args_for(
            "guide.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("src.md".into()),
                selector: "Src/Source-Guideline".into(),
                vibe: None,
            }),
        ),
        &[],
        None,
    )
    .expect("guide.md mints its own lock");
    let guide_lock = {
        let doc = fs::load(&root, std::path::Path::new("guide.md")).expect("load");
        lock::block_texts(&doc)
            .first()
            .map(|s| (*s).to_owned())
            .expect("guide.md carries a lock")
    };

    // Now pin INTO guide.md — this promotes an anchor into it (a raw replace).
    splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: "Guide/Leader's-Guideline".into(),
                vibe: None,
            }),
        ),
        &[],
        None,
    )
    .expect("the pin lands and promotes");

    let doc = fs::load(&root, std::path::Path::new("guide.md")).expect("load");
    assert!(
        std::fs::read_to_string(dir.path().join("guide.md"))
            .expect("read")
            .contains("^leaders-guideline"),
        "the promotion did land (the test would be vacuous otherwise)"
    );
    assert_eq!(
        lock::block_texts(&doc),
        vec![guide_lock.as_str()],
        "the promotion must not move one byte of the target's lock"
    );
}

/// **A stated consequence of R25, asserted rather than discovered.** The lock
/// block is birthed at EOF, so it lives inside the page's LAST section: a
/// whole-section rewrite (`put at:content`, `plan_edits.replace_section`) would
/// DELETE the attestation. R25 says refuse any lock-byte change this call did not
/// mint, and a deletion is a change — an actor could otherwise erase a RED pin
/// and leave a page reading clean. The remedy the refusal implies: append
/// (`put at:end`) instead of rewriting, or hand-remove the block deliberately —
/// the same repair path a corrupt lock already documents (#8 §3).
#[test]
fn a_whole_section_rewrite_that_would_delete_the_lock_refuses() {
    let (dir, root) = workspace();
    splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: "Guide/Leader's-Guideline".into(),
                vibe: None,
            }),
        ),
        &[],
        None,
    )
    .expect("the pin lands");
    let minted = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    assert!(minted.contains("```meridian-lock"), "{minted}");

    let wiped = splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![wire::HpathSeg {
                        h: "Plan".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: PutAt::Content,
                    text: "rewritten prose, no lock.\n".into(),
                },
                if_node_rev: None,
            }],
            None,
        ),
        &[],
        None,
    );
    assert_eq!(
        wiped.as_ref().err().map(|e| e.code),
        Some(ErrorCode::BadRequest),
        "deleting the attestation through an ordinary put must refuse"
    );
    // R24/R32: a refusal that names the law and not the way out teaches the
    // caller nothing but that they are stuck. This one is the LEGITIMATE case —
    // a whole-section rewrite of the last section is an ordinary thing to want —
    // so the message must carry what it would destroy AND what to do instead,
    // naming only remedies that exist today.
    let taught = wiped.as_ref().err().and_then(|e| e.message.clone());
    let taught = taught.as_deref().unwrap_or_default();
    for clause in [
        "WHAT THIS WOULD DESTROY",
        "WHAT TO DO INSTEAD",
        "put at:end",
        "does not exist yet",
    ] {
        assert!(
            taught.contains(clause),
            "the refusal must teach the remedy — missing {clause:?} in: {taught}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        minted,
        "the page is byte-unchanged"
    );

    // The append shape is unaffected — it adds beside the block, never over it.
    splice(
        &root,
        0,
        &args_for(
            "plan.md",
            None,
            vec![put_at_end("Plan", "appended beside the lock.\n")],
            None,
        ),
        &[],
        None,
    )
    .expect("an append into the same section still commits");
}
