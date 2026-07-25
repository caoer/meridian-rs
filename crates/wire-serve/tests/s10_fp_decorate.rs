//! S10: the claim-link `@fp` decorate-on-read / strip-on-put transform (D10).
//!
//! The two SAFETY tests this unit was designed around (plan §8: S10 is the
//! plan's lowest-confidence unit because the grammar is greenfield, and these
//! are its net) come first, written before the grammar existed:
//!
//! 1. **Round-trip, disk clean** — an agent reads the DECORATED face, copies a
//!    decorated link into an edit, and puts it. The stored bytes carry no `@fp`
//!    token, asserted at the byte level over the whole file.
//! 2. **Heading-`@` intact** — `[[Page#Q@Home]]` is a legitimate heading
//!    fragment containing an `@`. The shaped grammar (D10) must not see an fp
//!    token there, on the render plane or the put plane.
//!
//! Everything drives the production surfaces: `wire_serve::read::composed_read`
//! for the decorated face and `wire_serve::write::splice` for the put.

use std::collections::BTreeMap;

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PutAt, ResponseBody, SecRef};
use wire_serve::read::{ReadParams, composed_read, page_decorations};
use wire_serve::write::{SpliceArgs, splice};

/// The pinning page — it draws from the guide and cites it with a claim link.
/// The link addresses the slug S7's promotion mints (`^leaders-guideline`),
/// which is the HANDLE this unit decorates; the pin's own identity is the
/// section chain, never this slug.
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from [[guide#^leaders-guideline|Leader's Guideline]].\n";

/// The pinned page.
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// Mint a REAL pin through the production choke-point (S7), so the decoration
/// under test rests on a real lock block and a real fingerprint — never a
/// hand-written fixture that could disagree with what `mrd pin` writes.
fn mint_pin(root: &fs::WorkspaceRoot) {
    let args = SpliceArgs {
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
            selector: "Guide/Leader's-Guideline".into(),
            vibe: None,
        }),
    };
    splice(root, 0, &args, &[], None).expect("the pin commits");
}

/// The corpus the decoration builder reads: every page, plus the linkpath index
/// the link resolver uses. The registry daemon holds exactly this.
fn corpus(root: &fs::WorkspaceRoot) -> (model::CorpusIndex, BTreeMap<String, model::Document>) {
    let mut docs = BTreeMap::new();
    let mut index = model::CorpusIndex::new();
    for rel in ["plan.md", "guide.md", "notes.md"] {
        let Ok(doc) = fs::load(root, std::path::Path::new(rel)) else {
            continue;
        };
        index.insert(rel, &doc);
        docs.insert(rel.to_string(), doc);
    }
    (index, docs)
}

/// The composed read as the registry serves it: the decorated `rendered_text`
/// plus the RAW `sections[]` rows.
fn read_decorated(root: &fs::WorkspaceRoot, rel: &str, sel: &str) -> ResponseBody {
    let (index, docs) = corpus(root);
    let doc = docs.get(rel).expect("the page is in the corpus");
    let decorations = page_decorations(&index, &docs, rel);
    composed_read(
        doc,
        &WPath(rel.into()),
        &wire::Root("r".into()),
        &ReadParams {
            mode: Some("sections".into()),
            sections: Some(vec![sel.into()]),
            ..ReadParams::default()
        },
        None,
        &decorations,
    )
    .expect("the read serves")
}

fn read_page(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// An empty splice frame — the caller fills in `edits` or `plan_edits`.
fn pin_free_args(path: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: WPath(path.into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: None,
    }
}

/// A `put at:end` on a section — appends INSIDE the section without rewriting
/// what is already there (so an engine block living in it is untouched).
fn put_end(path: &str, hpath: &str, text: &str) -> SpliceArgs {
    SpliceArgs {
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: hpath.into(),
                    n: None,
                }],
            },
            edit: EditShape::Put {
                at: PutAt::End,
                text: text.into(),
            },
            if_node_rev: None,
        }],
        ..pin_free_args(path)
    }
}

/// One `Put{content}` edit on a section — the whole-section replace.
fn put_content(path: &str, hpath: &str, text: &str) -> SpliceArgs {
    SpliceArgs {
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: hpath.into(),
                    n: None,
                }],
            },
            edit: EditShape::Put {
                at: PutAt::Content,
                text: text.into(),
            },
            if_node_rev: None,
        }],
        ..pin_free_args(path)
    }
}

// ---------------------------------------------------------------------------
// SAFETY TEST 1 — the round trip, asserted at the byte level on disk
// ---------------------------------------------------------------------------

/// **Gate 1 (exit criterion 4).** The whole loop: pin → read (decorated) →
/// edit → put → disk clean.
///
/// The vector this closes is the real one. An agent's context is
/// `rendered_text`, so the link it copies into its next edit is the DECORATED
/// spelling; if the put path did not strip it, the `@fp` token would be
/// authored into the file, and the file would then claim a fingerprint that no
/// engine minted. The assertion is byte-level over the WHOLE file: no `@`
/// anywhere the page did not already have.
#[test]
fn a_decorated_link_round_trips_to_disk_with_no_fp_token() {
    let (_dir, root) = workspace();
    mint_pin(&root);

    // ── read: the rendered face carries the shaped token ──────────────────
    let body = read_decorated(&root, "plan.md", "Plan");
    let ResponseBody::Read {
        rendered_text,
        sections,
        ..
    } = &body
    else {
        panic!("read body");
    };

    let at = rendered_text
        .find("[[guide#^leaders-guideline")
        .expect("the claim link renders");
    let decorated = rendered_text[at..]
        .split_inclusive("]]")
        .next()
        .expect("a closed link")
        .to_string();
    assert!(
        decorated.contains("[[guide#^leaders-guideline@green."),
        "the decorated address is the promoted slug plus the shaped token: {decorated}"
    );
    let token = decorated
        .split_once('@')
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(t, _)| t.split('|').next().unwrap_or_default().to_string())
        .expect("token body");
    assert_eq!(
        token.len(),
        "green.".len() + 8,
        "tone word + 8-hex digest: {token}"
    );

    // The RAW face is never decorated — put reads `sections[].content`, and a
    // decorated view feeding a write is the A-K1 data-loss class.
    let raw = &sections.as_ref().expect("sections")[0].content;
    assert!(
        !raw.contains('@'),
        "sections[].content is the raw face, verbatim: {raw}"
    );

    // ── edit: the agent copies the DECORATED link into its next write ─────
    //
    // Through the PLAN batch, which is the agent-facing put vocabulary. Both
    // halves carry the decorated spelling, and both matter:
    //
    // - `old` is a NEEDLE matched against STORED bytes, which never carry a
    //   token. Unstripped, an agent could never match the link it just read.
    // - `new` is the PAYLOAD. Unstripped, the token lands on disk and the file
    //   starts claiming a fingerprint no engine minted.
    let mut plan = pin_free_args("plan.md");
    plan.plan_edits = vec![wire::PlanEdit::Match {
        hpath: "Plan".into(),
        old: decorated.clone(),
        new: format!("{decorated} — reviewed."),
        all: true,
        rev: None,
    }];
    splice(&root, 0, &plan, &[], None).expect("the plan put commits");

    // ── disk: byte-level, whole file ──────────────────────────────────────
    let on_disk = read_page(&root, "plan.md");
    assert!(
        !on_disk.contains('@'),
        "stored bytes carry NO `@fp` token anywhere:\n{on_disk}"
    );
    assert!(
        on_disk.contains("[[guide#^leaders-guideline|Leader's Guideline]] — reviewed."),
        "the edit landed, and the link is back to its stored spelling:\n{on_disk}"
    );
    // The lock block the pin wrote is still there — the agent edited its own
    // prose, and the engine's block is not in the blast radius.
    assert!(
        on_disk.contains("```meridian-lock"),
        "the lock block survives the round trip:\n{on_disk}"
    );

    // ── the NATIVE edit vocabulary strips at the same intake ──────────────
    let mut native = pin_free_args("plan.md");
    native.edits = vec![Edit {
        target: SecRef::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "Plan".into(),
                n: None,
            }],
        },
        edit: EditShape::Match {
            old: format!("{decorated} — reviewed."),
            new: format!("{decorated} — closed."),
        },
        if_node_rev: None,
    }];
    splice(&root, 0, &native, &[], None).expect("the native put commits");

    let on_disk = read_page(&root, "plan.md");
    assert!(
        !on_disk.contains('@'),
        "the native path strips at the same intake:\n{on_disk}"
    );
    assert!(
        on_disk.contains("[[guide#^leaders-guideline|Leader's Guideline]] — closed."),
        "and its decorated NEEDLE matched the stored bytes:\n{on_disk}"
    );
}

// ---------------------------------------------------------------------------
// SAFETY TEST 2 — a legitimate `@` inside a heading fragment
// ---------------------------------------------------------------------------

/// **Gate 2.** `[[Page#Q@Home]]` is a heading fragment that happens to contain
/// an `@`. A fully-opaque "anything after `@`" rule would eat it (D10's whole
/// reason for a SHAPED token). It must survive both planes untouched: the put
/// stores it verbatim, and the decorated read renders it verbatim.
#[test]
fn a_heading_fragment_at_is_never_touched() {
    let (_dir, root) = workspace();
    mint_pin(&root);

    // Every `@`-bearing shape that is NOT a well-formed fp token in a block-ref
    // slot: the plan's own case, an fp-SHAPED tail on a heading fragment, an
    // `@` in a label, and a block ref whose tail is shaped but not hex.
    let authored = "\n\
        - [[Page#Q@Home]]\n\
        - [[Page#Q@green.b3af12cd]]\n\
        - [[guide#^leaders-guideline|ping @zt]]\n\
        - [[guide#^other@green.nothex1]]\n\
        - plain text me@example.com\n\n";
    // `put at:end`, not `at:content`: the minted lock block sits at EOF inside
    // this very section, and a whole-section rewrite would DELETE it. The R25
    // artifact guard refuses that (asserted by name in
    // `s2fix_artifact_guard::a_whole_section_rewrite_that_would_delete_the_lock_refuses`)
    // — this test's claim is about the `@` shapes, so it appends instead.
    splice(&root, 0, &put_end("plan.md", "Plan", authored), &[], None).expect("the put commits");

    let on_disk = read_page(&root, "plan.md");
    for line in authored.lines().filter(|l| l.contains('@')) {
        assert!(
            on_disk.contains(line.trim_end()),
            "verbatim on disk: {line}\n--- file ---\n{on_disk}"
        );
    }

    // And the decorated READ leaves them alone too: the heading-fragment `@`
    // is not a block-ref slot, so the decorator never looks at it.
    let body = read_decorated(&root, "plan.md", "Plan");
    let ResponseBody::Read { rendered_text, .. } = &body else {
        panic!("read body");
    };
    assert!(
        rendered_text.contains("[[Page#Q@Home]]"),
        "heading-`@` renders verbatim:\n{rendered_text}"
    );
    assert!(
        rendered_text.contains("[[Page#Q@green.b3af12cd]]"),
        "even an fp-SHAPED tail is left alone in a HEADING fragment — the token \
         rides the block-ref slot only:\n{rendered_text}"
    );
    assert!(
        rendered_text.contains("ping @zt"),
        "a label `@` is parsed separately and untouched:\n{rendered_text}"
    );
}

/// The I4 ladder has ONE owner and TWO entry points (S4a/D4): the host's
/// `check_write` PRE-FLIGHT and the flocked `splice`. They must judge the same
/// bytes, so the strip runs at BOTH intakes — otherwise an agent's decorated
/// `find` would miss in the pre-flight and hit in the write, which is the
/// two-answers-to-one-question shape this milestone keeps closing.
#[test]
fn the_pre_flight_and_the_write_see_the_same_stripped_bytes() {
    let (_dir, root) = workspace();
    mint_pin(&root);
    let prev = fs::load(&root, std::path::Path::new("plan.md")).expect("load");

    let decorated = "[[guide#^leaders-guideline@green.b3af12cd|Leader's Guideline]]";
    let body = wire_serve::check_write::check_write(
        &prev,
        "plan.md",
        "tester",
        "2026-07-25T00:00:00Z",
        &[wire::CheckWriteEdit {
            op: "replace".into(),
            at: "Plan".into(),
            find: decorated.into(),
            body: format!("{decorated} — reviewed."),
            rev: String::new(),
            all: false,
        }],
    );
    let ResponseBody::CheckWrite { refuse, .. } = &body else {
        panic!("check_write body");
    };
    assert!(
        refuse.is_none(),
        "the decorated needle matched the STORED bytes in the pre-flight too: {refuse:?}"
    );
}

// ---------------------------------------------------------------------------
// The colour the token carries — the reason the whole module exists
// ---------------------------------------------------------------------------

/// The token's tone is the PIN's colour, and the pin's identity is the SECTION
/// its `ref` resolves to — never the `^slug` the link addresses.
///
/// This is the false-green trap S7 named, tested from the decoration end. The
/// slug marker's own model span is its HOST LINE, so a decorator that took the
/// slug for the identity would recompute an unchanged one-line span and render
/// **green on every body edit**. Here the pinned section's BODY moves and
/// nothing else — the heading is untouched, the slug is untouched, the link is
/// untouched — and the token must go red.
#[test]
fn a_body_edit_under_the_pinned_heading_turns_the_token_red() {
    let (_dir, root) = workspace();
    mint_pin(&root);
    assert!(
        token_body(&root).starts_with("green."),
        "a freshly minted pin decorates green"
    );

    // Move ONLY the pinned section's body.
    let guide = read_page(&root, "guide.md");
    std::fs::write(
        root.0.join("guide.md"),
        guide.replace("review before you close.", "review before you MERGE."),
    )
    .expect("drift the target body");

    let after = token_body(&root);
    assert!(
        after.starts_with("red."),
        "a body edit under the pinned heading is measured drift: {after}"
    );
    assert_eq!(
        after.len(),
        "red.".len() + 8,
        "and the digest still names the PINNED claim, not the live content: {after}"
    );
}

/// Grey never renders green. A pin whose token names a version this build does
/// not implement is `Unverifiable` — the ledger did not measure it — so the
/// decoration says so rather than showing an attested-looking green.
#[test]
fn an_unverifiable_pin_decorates_grey() {
    let (_dir, root) = workspace();
    mint_pin(&root);
    let pinner = read_page(&root, "plan.md");
    std::fs::write(
        root.0.join("plan.md"),
        pinner.replace("fingerprint: \"fp1.", "fingerprint: \"fp9."),
    )
    .expect("supersede the token's version");

    let body = token_body(&root);
    assert!(
        body.starts_with("grey."),
        "an unknown version is unverifiable, never green: {body}"
    );
}

/// The decorated token body (`green.b3af12cd`) currently on the claim link in
/// `plan.md` — read through the production composed-read arm.
fn token_body(root: &fs::WorkspaceRoot) -> String {
    let body = read_decorated(root, "plan.md", "Plan");
    let ResponseBody::Read { rendered_text, .. } = &body else {
        panic!("read body");
    };
    let at = rendered_text
        .find("#^leaders-guideline@")
        .unwrap_or_else(|| panic!("the claim link is decorated:\n{rendered_text}"));
    rendered_text[at + "#^leaders-guideline@".len()..]
        .split(['|', ']'])
        .next()
        .expect("token body")
        .to_string()
}

// ---------------------------------------------------------------------------
// GATE 3 — the refusal backstop, and the address that DOES strip
// ---------------------------------------------------------------------------

/// A malformed `@fp` on an ADDRESS refuses `bad_request` and writes nothing:
/// the block-id charset (§2.4) has no `@`, so anything the shaped strip does
/// not recognize dies at `model::Ref::anchor` validation. That refusal is the
/// backstop the strip's ordering makes unreachable for well-formed tokens.
#[test]
fn a_malformed_fp_address_refuses_and_writes_nothing() {
    let (_dir, root) = workspace();
    mint_pin(&root);
    let before = read_page(&root, "guide.md");

    let mut args = put_content("guide.md", "Guide", "x");
    args.edits[0].target = SecRef::Anchor {
        anchor: "leaders-guideline@notafingerprint".into(),
    };
    let err = splice(&root, 0, &args, &[], None).expect_err("refuses");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("[A-Za-z0-9-]")),
        "the charset refusal names the one block-id charset: {:?}",
        err.message
    );
    assert_eq!(read_page(&root, "guide.md"), before, "nothing reached disk");
}

/// The SAME law at the wire's own guard. `decode_anchor` runs `Ref::anchor`
/// before any arm does, so the strip is ordered there too — otherwise the
/// decorated address would be display-only, refused by the decoder before the
/// bridge that strips it ever ran. Both halves are asserted on the decoded
/// value: a shaped token decodes to the STORED spelling, an unshaped `@` still
/// refuses verbatim.
#[test]
fn the_wire_decoder_strips_before_its_own_mint_guard() {
    let frame = |anchor: &str| {
        let serde_json::Value::Object(o) = serde_json::json!({
            "op": "cat",
            "path": "guide.md",
            "sec": { "anchor": anchor },
        }) else {
            unreachable!()
        };
        o
    };

    let op = wire_serve::decode::decode(
        &frame("leaders-guideline@green.b3af12cd"),
        wire_serve::rev::Rev::V3,
    )
    .expect("a shaped token decodes");
    let wire::Op::Cat {
        sec: Some(SecRef::Anchor { anchor }),
        ..
    } = op
    else {
        panic!("cat with an anchor sec");
    };
    assert_eq!(
        anchor, "leaders-guideline",
        "the decoded address is the STORED spelling — nothing downstream ever \
         sees the token"
    );

    let err =
        wire_serve::decode::decode(&frame("leaders-guideline@nope"), wire_serve::rev::Rev::V3)
            .expect_err("an unshaped `@` is not a token");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("[A-Za-z0-9-]")),
        "and it refuses at the charset guard, verbatim: {:?}",
        err.message
    );
}

/// The positive half of the same law: a WELL-FORMED `@fp` on an address is
/// stripped BEFORE `model::Ref::anchor` sees it, so the agent-plane decorated
/// address addresses exactly what its stored spelling addresses.
#[test]
fn a_well_formed_fp_address_strips_and_resolves() {
    let (_dir, root) = workspace();
    mint_pin(&root);

    let mut args = put_content("guide.md", "Guide", "converged.\n");
    args.edits[0].target = SecRef::Anchor {
        anchor: "leaders-guideline@green.b3af12cd".into(),
    };
    args.edits[0].edit = EditShape::Put {
        at: PutAt::All,
        text: "^leaders-guideline\n".into(),
    };
    splice(&root, 0, &args, &[], None).expect("the decorated address resolves");

    let on_disk = read_page(&root, "guide.md");
    assert!(
        !on_disk.contains('@'),
        "and nothing about the decorated address reached disk:\n{on_disk}"
    );
}
