//! R25 fix 1 — guard the `meridian-lock` artifact, not only the pin verb (D16).
//!
//! Pins on WRITE (`is_err()`), never rendered colour (R26). Doors: native
//! `edits`, `plan_edits`, `write::create`, anchor promotion, legit `splice.pin`
//! control; `run::executor` is separate (`crates/run/tests/executor.rs`).

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PlanEdit, PutAt, SecRef};
use wire_serve::write::{CreateArgs, SpliceArgs, splice};

/// Pinning page (no lock — first pin births one as file preamble).
const PINNER: &str = "# Plan\n\ndraws from the guide.\n";
/// Pinned page — heading ref only (bare `#^anchor` is R31 empty-span false green).
const TARGET: &str = "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n";
/// Different body under same heading — control that fingerprints can differ.
const DECOY: &str = "# Guide\n\n## Leader's Guideline\n\nship it without reading.\n";

/// Git workspace: R4 pin row needs a `hash` only git can answer.
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
        premises: Vec::new(),
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

/// Live VERIFY-plane fingerprint (forged block must carry a token that would green).
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

/// Fixture blob oid (R4 `hash` mandatory; retrieval plane not under test).
const FIXTURE_BLOB: &str = "9ae3f1deadbeef";

/// One R4 pin in `lock::render` bytes; path as SEGMENTS (R1.6 / U14 — never
/// join-then-split `page#A/B`; `/` in heading text must survive).
fn lock_block(object: &str, path: &[&str], fingerprint: &str) -> String {
    let path = path
        .iter()
        .map(|seg| format!("\"{seg}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\n```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{FIXTURE_BLOB}\"\n    path: [{path}]\n    \
         fingerprint: \"{fingerprint}\"\n```\n"
    )
}

/// Control: target vs decoy fingerprints must differ (else pins are vacuous).
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

/// Unread actor: gated pin and ordinary-edit forge both refuse.
#[test]
fn an_unread_actor_cannot_forge_a_pin_through_ordinary_edits() {
    let (dir, root) = workspace();
    let store = receipt::read_mint::ReadMintStore::new();
    let mallory = Some("agent-mallory");
    let before = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");

    // 1. Gated door: unread pin.
    let gated = splice(
        &root,
        None,
        &args_for(
            "plan.md",
            mallory,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: wire::ReadSel::parse("Guide/Leader's Guideline"),
                vibe: None,
                fingerprint: None,
                sec_rev: None,
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

    // 2. Ungated door: same claim as page text (token would verify green).
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let forged = splice(
        &root,
        None,
        &args_for(
            "plan.md",
            mallory,
            vec![put_at_end(
                "Plan",
                &lock_block("guide", &["Guide", "Leader's Guideline"], &token),
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

/// CLI (`actor: None`, D16-trusted for pin) still cannot put lock bytes as text.
#[test]
fn the_local_operator_door_cannot_write_lock_bytes_as_page_text() {
    let (_dir, root) = workspace();
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let forged = splice(
        &root,
        None,
        &args_for(
            "plan.md",
            None,
            vec![put_at_end(
                "Plan",
                &lock_block("guide", &["Guide", "Leader's Guideline"], &token),
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

/// Door 2: `plan_edits` lowering cannot forge a lock.
#[test]
fn plan_edits_lowering_cannot_forge_a_lock() {
    let (dir, root) = workspace();
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let mut args = args_for("plan.md", Some("agent-mallory"), Vec::new(), None);
    args.plan_edits = vec![PlanEdit::Append {
        hpath: vec![wire::HpathSeg {
            h: "Plan".into(),
            n: None,
        }],
        body: lock_block("guide", &["Guide", "Leader's Guideline"], &token),
        rev: None,
    }];
    let forged = splice(
        &root,
        None,
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

/// Door 3: `create` body may not birth a lock (no pre-image claim).
#[test]
fn create_cannot_birth_a_page_carrying_a_lock() {
    let (dir, root) = workspace();
    let token = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    let born = wire_serve::write::create(
        &root,
        None,
        &CreateArgs {
            id: None,
            path: WPath("forged.md".into()),
            body: format!(
                "# Forged\n{}",
                lock_block("guide", &["Guide", "Leader's Guideline"], &token)
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

/// Ordinary edits cannot rewrite a minted lock fingerprint.
#[test]
fn ordinary_edits_cannot_rewrite_a_minted_lock() {
    let (dir, root) = workspace();
    // Legit mint first (CLI / D16).
    splice(
        &root,
        None,
        &args_for(
            "plan.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: wire::ReadSel::parse("Guide/Leader's Guideline"),
                vibe: None,
                fingerprint: None,
                sec_rev: None,
            }),
        ),
        &[],
        None,
    )
    .expect("the minted pin lands");
    let minted = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    let real = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");

    // Drift target; try to re-green the claim by hand.
    std::fs::write(dir.path().join("guide.md"), DECOY).expect("drift");
    let drifted = live_fingerprint(&root, "guide.md", "Guide/Leader's Guideline");
    assert_ne!(
        real, drifted,
        "the drift must actually move the fingerprint"
    );

    let forged = splice(
        &root,
        None,
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

/// Control: real mint, re-pin, and ordinary edit beside an untouched lock land.
#[test]
fn the_minted_pin_still_lands_and_re_pins_idempotently() {
    let (dir, root) = workspace();
    let pin = |sel: &str| {
        splice(
            &root,
            None,
            &args_for(
                "plan.md",
                None,
                Vec::new(),
                Some(PinSpec {
                    target: WPath("guide.md".into()),
                    selector: wire::ReadSel::parse(sel),
                    vibe: None,
                    fingerprint: None,
                    sec_rev: None,
                }),
            ),
            &[],
            None,
        )
    };
    pin("Guide/Leader's Guideline").expect("first pin lands");
    let after_first = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    assert!(after_first.contains("```meridian-lock"), "{after_first}");

    pin("Guide/Leader's Guideline").expect("the re-pin lands");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        after_first,
        "a re-pin of unchanged content is byte-idempotent"
    );

    // Content edit beside untouched lock is ordinary work.
    splice(
        &root,
        None,
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

/// Anchor promotion (`fs::replace_file`) is lock-neutral on target.
#[test]
fn the_anchor_promotion_leaves_the_targets_lock_untouched() {
    let (dir, root) = workspace();
    // Target gets its own lock first (guarded path).
    std::fs::write(
        dir.path().join("src.md"),
        "# Src\n\n## Source Guideline\n\nread me first.\n",
    )
    .expect("src");
    splice(
        &root,
        None,
        &args_for(
            "guide.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("src.md".into()),
                selector: wire::ReadSel::parse("Src/Source Guideline"),
                vibe: None,
                fingerprint: None,
                sec_rev: None,
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

    // Pin into guide.md — promotes anchor via raw replace.
    splice(
        &root,
        None,
        &args_for(
            "plan.md",
            None,
            Vec::new(),
            Some(PinSpec {
                target: WPath("guide.md".into()),
                selector: wire::ReadSel::parse("Guide/Leader's Guideline"),
                vibe: None,
                fingerprint: None,
                sec_rev: None,
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

/// R25: whole-section rewrite that would delete a LEGACY-placed lock (inside
/// the section, where the old EOF birth left it) refuses; message teaches
/// destroy/instead (`put at:end`). A preamble-placed block is out of every
/// section's reach, so only legacy pages can hit this door section-wise.
#[test]
fn a_whole_section_rewrite_that_would_delete_the_lock_refuses() {
    let (dir, root) = workspace();
    std::fs::write(
        dir.path().join("plan.md"),
        format!(
            "{PINNER}\n```meridian-lock\nversion: 2\npins:\n  - object: \"[[guide]]\"\n    \
             hash: \"9ae3f1c0deadbeef9ae3f1c0deadbeef9ae3f1c0\"\n    \
             path: [\"Guide\", \"Leader's Guideline\"]\n    \
             fingerprint: \"fp1.span2.b3.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n```\n"
        ),
    )
    .expect("legacy pinner: the block sits at EOF, inside # Plan");
    let minted = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    assert!(minted.contains("```meridian-lock"), "{minted}");

    let wiped = splice(
        &root,
        None,
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
    // R24/R32: refusal names destroy + remedy (not law alone).
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

    // Append beside lock still commits.
    splice(
        &root,
        None,
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
