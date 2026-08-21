//! S10: claim-link `@fp` decorate-on-read / strip-on-put (D10).
//!
//! SAFETY: (1) decorated link round-trips to disk with no `@fp` token;
//! (2) heading-fragment `@` (`[[Page#Q@Home]]`) is never treated as an fp token.
//! Surfaces: `composed_read` (decorated) and `write::splice` (put/strip).

use std::collections::BTreeMap;

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PinSpec, PutAt, ResponseBody, SecRef};
use wire_serve::read::{ReadParams, composed_read, page_decorations};
use wire_serve::write::{SpliceArgs, splice};

/// Pinning page citing the guide via claim-link slug `^leaders-guideline` (S7 handle; pin id is section chain).
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from [[guide#^leaders-guideline|Leader's Guideline]].\n";

/// Pinned guide page.
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

/// Git workspace ([`mint_pin`] / R4 `hash`).
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "s10@example.invalid"],
        vec!["config", "user.name", "s10"],
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

/// Mint a real pin through production choke-point (S7) — not a hand-written lock fixture.
fn mint_pin(root: &fs::WorkspaceRoot) {
    let args = SpliceArgs {
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
            selector: wire::ReadSel::parse("Guide/Leader's Guideline"),
            vibe: None,
            fingerprint: None,
            sec_rev: None,
        }),
        fields: BTreeMap::default(),
    };
    splice(root, None, &args, &[], None).expect("the pin commits");
}

/// Corpus the decoration builder reads (registry shape: every page + linkpath index).
fn corpus(root: &fs::WorkspaceRoot) -> (model::CorpusIndex, model::Docs) {
    let mut docs = BTreeMap::new();
    let mut index = model::CorpusIndex::new();
    for rel in ["plan.md", "guide.md", "notes.md"] {
        let Ok(doc) = fs::load(root, std::path::Path::new(rel)) else {
            continue;
        };
        index.insert(rel, &doc);
        docs.insert(rel.to_string(), std::sync::Arc::new(doc));
    }
    (index, docs)
}

/// Composed read as the registry serves: decorated `rendered_text` + raw `sections[]`.
fn read_decorated(root: &fs::WorkspaceRoot, rel: &str, sel: &str) -> ResponseBody {
    let (index, docs) = corpus(root);
    let doc = docs.get(rel).expect("the page is in the corpus");
    let decorations = page_decorations(&index, &docs, rel);
    composed_read(
        doc,
        &WPath(rel.into()),
        &wire::Root("r".into()),
        &ReadParams {
            sections: Some(vec![wire::ReadSel::parse(sel)]),
            ..ReadParams::default()
        },
        &decorations,
    )
    .expect("the read serves")
}

fn read_page(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// Empty splice frame — caller fills `edits` or `plan_edits`.
fn pin_free_args(path: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
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
        fields: BTreeMap::default(),
    }
}

/// `put at:end` — appends inside the section without rewriting existing content (lock block safe).
fn put_end(path: &str, hpath: &str, text: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
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

/// Whole-section `Put{content}` replace.
fn put_content(path: &str, hpath: &str, text: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
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

/// Gate 1: pin → decorated read → put decorated link → disk has no `@fp` (whole-file byte assert).
#[test]
fn a_decorated_link_round_trips_to_disk_with_no_fp_token() {
    let (_dir, root) = workspace();
    mint_pin(&root);

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

    // sections[].content is raw (A-K1).
    let raw = &sections.as_ref().expect("sections")[0].content;
    assert!(
        !raw.contains('@'),
        "sections[].content is the raw face, verbatim: {raw}"
    );

    // Plan: strip needle and payload.
    let mut plan = pin_free_args("plan.md");
    plan.plan_edits = vec![wire::PlanEdit::Match {
        hpath: vec![wire::HpathSeg {
            h: "Plan".into(),
            n: None,
        }],
        old: decorated.clone(),
        new: format!("{decorated} — reviewed."),
        all: true,
        rev: None,
    }];
    splice(&root, None, &plan, &[], None).expect("the plan put commits");

    let on_disk = read_page(&root, "plan.md");
    assert!(
        !on_disk.contains('@'),
        "stored bytes carry NO `@fp` token anywhere:\n{on_disk}"
    );
    assert!(
        on_disk.contains("[[guide#^leaders-guideline|Leader's Guideline]] — reviewed."),
        "the edit landed, and the link is back to its stored spelling:\n{on_disk}"
    );
    assert!(
        on_disk.contains("```meridian-lock"),
        "the lock block survives the round trip:\n{on_disk}"
    );

    // Native match strips at same intake.
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
    splice(&root, None, &native, &[], None).expect("the native put commits");

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

/// Gate 2: heading-fragment `@` is never an fp token (D10 shaped grammar).
#[test]
fn a_heading_fragment_at_is_never_touched() {
    let (_dir, root) = workspace();
    mint_pin(&root);

    // Non-token `@` shapes (heading, label, email, etc.).
    let authored = "\n\
        - [[Page#Q@Home]]\n\
        - [[Page#Q@green.b3af12cd]]\n\
        - [[guide#^leaders-guideline|ping @zt]]\n\
        - [[guide#^other@green.nothex1]]\n\
        - plain text me@example.com\n\n";
    // put at:end (R25: content rewrite would delete lock at EOF).
    splice(
        &root,
        None,
        &put_end("plan.md", "Plan", authored),
        &[],
        None,
    )
    .expect("the put commits");

    let on_disk = read_page(&root, "plan.md");
    for line in authored.lines().filter(|l| l.contains('@')) {
        assert!(
            on_disk.contains(line.trim_end()),
            "verbatim on disk: {line}\n--- file ---\n{on_disk}"
        );
    }

    // Decorated read leaves non-tokens alone.
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

/// I4: strip at both `check_write` pre-flight and flocked splice so they judge the same bytes.
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
            at: vec![wire::HpathSeg {
                h: "Plan".into(),
                n: None,
            }],
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

/// Token tone is pin colour by section identity (not `^slug`) — body drift must go red (false-green trap).
#[test]
fn a_body_edit_under_the_pinned_heading_turns_the_token_red() {
    let (_dir, root) = workspace();
    mint_pin(&root);
    assert!(
        token_body(&root).starts_with("green."),
        "a freshly minted pin decorates green"
    );

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

/// Unverifiable pin version decorates grey, never green.
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

/// Decorated token body (`green.b3af12cd`) on the claim link via composed-read.
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

/// Malformed `@fp` on address refuses at block-id charset; writes nothing.
#[test]
fn a_malformed_fp_address_refuses_and_writes_nothing() {
    let (_dir, root) = workspace();
    mint_pin(&root);
    let before = read_page(&root, "guide.md");

    let mut args = put_content("guide.md", "Guide", "x");
    args.edits[0].target = SecRef::Anchor {
        anchor: "leaders-guideline@notafingerprint".into(),
    };
    let err = splice(&root, None, &args, &[], None).expect_err("refuses");
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

/// Wire decoder strips shaped token before its own mint guard (`decode_anchor`).
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

/// Well-formed `@fp` on address strips before `Ref::anchor` and resolves to stored spelling.
///
/// The `text` carries NO trailing newline, and that is load-bearing rather than
/// cosmetic (`decisions/0018`, 2026-08-09). A leaf block's span EXCLUDES its
/// line terminator, so a trailing separator here would place a byte outside the
/// node this edit names — the engine refuses that now, and it committed it at
/// v1.0.0 with a `node_rev` that could not move. This fixture previously
/// carried the separator, which made it depend on that escape to pass: the
/// defect encoded as its own precondition. Re-pointed at the LAW per charter
/// 03; **the assertions below are untouched**, because the trailing newline
/// was never what this test measures — it measures that `@fp` is stripped from
/// an ADDRESS and never reaches disk.
#[test]
fn a_well_formed_fp_address_strips_and_resolves() {
    let (_dir, root) = workspace();
    mint_pin(&root);

    let mut args = put_content("guide.md", "Guide", "converged.\n");
    args.edits[0].target = SecRef::Anchor {
        anchor: "leaders-guideline@green.b3af12cd".into(),
    };
    // The promoted pin marker sits on its own line directly below the
    // heading, so the anchor keys the HEADING LINE (F-R4 attachment). The
    // lawful `at:"all"` therefore re-supplies the heading bytes — the marker
    // line lies outside the replaced region and survives by construction.
    args.edits[0].edit = EditShape::Put {
        at: PutAt::All,
        text: "## Leader's Guideline".into(),
    };
    splice(&root, None, &args, &[], None).expect("the decorated address resolves");

    let on_disk = read_page(&root, "guide.md");
    assert!(
        !on_disk.contains('@'),
        "and nothing about the decorated address reached disk:\n{on_disk}"
    );
}
