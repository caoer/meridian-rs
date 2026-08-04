//! Advisor R25, structural fix 2 — the `@fp` strip moved from the PAYLOAD to the
//! CANDIDATE DOCUMENT.
//!
//! The strip used to walk named payload fields. A field list is the defect shape:
//! it missed `plan_edits.create.title` (finding 13), it could not see a token two
//! edits compose between them, and it judged each payload OUT of the document it
//! lands in — stripping a token the document law calls a code sample (finding
//! 11). One grammar, one grain: identify in the candidate, remove from the
//! payload that carries it, refuse what is left.
//!
//! The claim this file pins is R22's, unchanged in width: **no `@fp` token in a
//! claim-link POSITION on disk.** The positions that are NOT claim-link positions
//! are named and tested here as explicit exclusions, because a claim that does
//! not state its own edges is wider than its proof.

use wire::{
    CheckWriteEdit, Edit, EditShape, ErrorCode, Path as WPath, PlanEdit, PutAt, ResponseBody,
    SecRef,
};
use wire_serve::write::{CreateArgs, SpliceArgs, splice};

const TOKEN: &str = "@green.b3af12cd";

fn ws(seed: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), seed).expect("seed");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn args(edits: Vec<Edit>, plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("plan.md".into()),
        actor: Some("agent-scribe".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits,
        plan_edits,
        pin: None,
    }
}

fn hpath(chain: &str) -> SecRef {
    SecRef::Hpath {
        hpath: chain
            .split('/')
            .map(|h| wire::HpathSeg {
                h: h.into(),
                n: None,
            })
            .collect(),
    }
}

/// The live whole-node rev of a section — `replace_section` demands the rev the
/// caller read (a whole-section rewrite is destructive).
fn section_rev(root: &fs::WorkspaceRoot, chain: &str) -> String {
    let doc = fs::load(root, std::path::Path::new("plan.md")).expect("load");
    let r = model::Ref::Hpath(
        chain
            .split('/')
            .map(|h| model::HpathSeg {
                h: h.into(),
                n: None,
            })
            .collect(),
    );
    model::resolve(&doc, &r)
        .expect("the section resolves")
        .node_rev
        .0
}

fn on_disk(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("plan.md")).expect("read")
}

/// **Finding 13.** `plan_edits.create.title` is interpolated into heading bytes
/// by the lowering — a field the payload list never named. At document grain the
/// heading is just part of the candidate, so the token is stripped with the
/// body's.
#[test]
fn a_token_in_create_title_is_stripped_with_the_body() {
    let (dir, root) = ws("---\ntitle: Plan\n---\n\n# Plan\n\nbody\n");
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::Create {
                parent_hpath: "Plan".into(),
                title: format!("Draws from [[guide#^task1{TOKEN}|Task One]]"),
                body: format!("body [[guide#^task1{TOKEN}|B]]"),
            }],
        ),
        &[],
        None,
    )
    .expect("the create lowers and lands");
    let text = on_disk(&dir);
    assert!(
        !text.contains(TOKEN),
        "no @fp token may reach disk from ANY plan-edit field:\n{text}"
    );
    assert!(
        text.contains("## Draws from [[guide#^task1|Task One]]"),
        "the title landed, stripped, with its link intact:\n{text}"
    );
    assert!(text.contains("body [[guide#^task1|B]]"), "{text}");
}

/// Every plan-edit shape that carries a payload, in one batch — the coverage a
/// field list had to be maintained for, now structural.
#[test]
fn every_payload_shape_is_covered_without_a_field_list() {
    let (dir, root) = ws("---\ntitle: Plan\n---\n\n# Plan\n\nold line\n\n## Sub\n\nsub body\n");
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::Append {
                hpath: "Plan".into(),
                body: format!("appended [[guide#^a{TOKEN}|A]]"),
            }],
        ),
        &[],
        None,
    )
    .expect("append lands");
    let appended = on_disk(&dir);
    assert!(!appended.contains(TOKEN), "{appended}");
    assert!(appended.contains("appended [[guide#^a|A]]"), "{appended}");

    // A nested target cannot ride the same batch (§4.4 disjointness), so the
    // whole-section rewrite is its own splice.
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::ReplaceSection {
                hpath: "Plan/Sub".into(),
                body: format!("replaced [[guide#^b{TOKEN}|B]]"),
                rev: Some(section_rev(&root, "Plan/Sub")),
            }],
        ),
        &[],
        None,
    )
    .expect("replace_section lands");
    let text = on_disk(&dir);
    assert!(!text.contains(TOKEN), "{text}");
    assert!(text.contains("replaced [[guide#^b|B]]"), "{text}");

    // The native `match` payload, and its NEEDLE: `old` is an ADDRESS matched
    // against stored bytes (which never carry a token), so a needle copied from
    // the decorated render face must still find its line.
    splice(
        &root,
        None,
        &args(
            vec![Edit {
                target: hpath("Plan/Sub"),
                edit: EditShape::Match {
                    old: format!("replaced [[guide#^b{TOKEN}|B]]"),
                    new: format!("matched [[guide#^c{TOKEN}|C]]"),
                },
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect("a decorated needle still matches its undecorated bytes");
    let text = on_disk(&dir);
    assert!(!text.contains(TOKEN), "{text}");
    assert!(text.contains("matched [[guide#^c|C]]"), "{text}");
}

/// **Finding 11, the over-strip half.** A payload judged in isolation is not the
/// document it lands in: this token lands INSIDE an existing code fence, where
/// R22 says it is a code sample and must survive. The payload-grain strip ate it;
/// the document-grain strip leaves it, and the assertion stays quiet because the
/// one grammar does not call it a claim.
#[test]
fn a_token_landing_inside_a_fence_survives_r22() {
    let (dir, root) = ws("# Plan\n\n```text\nsample: PLACEHOLDER\n```\n");
    splice(
        &root,
        None,
        &args(
            vec![Edit {
                target: hpath("Plan"),
                edit: EditShape::Match {
                    old: "PLACEHOLDER".into(),
                    new: format!("[[guide#^goal{TOKEN}|G]]"),
                },
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect("an edit into a fence commits");
    let text = on_disk(&dir);
    assert!(
        text.contains(&format!("sample: [[guide#^goal{TOKEN}|G]]")),
        "a token inside a code fence is a code SAMPLE (R22) — stripping it would corrupt it:\n{text}"
    );
}

/// **Finding 11, the under-strip half — and the seeded missed door (gate 4).**
/// The token's bytes are RETAINED page text; the edit only closes the link that
/// turns them into a claim. No payload carries the token, so no payload-grain
/// strip could ever see it. The candidate does, and refuses LOUD.
#[test]
fn a_token_composed_out_of_retained_bytes_refuses() {
    let seed = format!("# Plan\n\nsee [[guide#^goal{TOKEN}\n");
    let (dir, root) = ws(&seed);
    let before = on_disk(&dir);
    let composed = splice(
        &root,
        None,
        &args(
            vec![Edit {
                target: hpath("Plan"),
                edit: EditShape::Match {
                    old: "b3af12cd".into(),
                    new: "b3af12cd|G]]".into(),
                },
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    );
    assert_eq!(
        composed.as_ref().err().map(|e| e.code),
        Some(ErrorCode::BadRequest),
        "a write that COMPOSES a claim token must refuse, not land it silently"
    );
    assert_eq!(on_disk(&dir), before, "nothing was written");
}

/// **A LIVE FALSE-RED in the shipped strip, found by fix8 on the run plane and
/// fixed here at its source.** Sections are contiguous, so a `put{at:end}`
/// replaces an EMPTY region sitting exactly on the byte where the next sibling
/// begins: two request targets contain it, and attribution by containment alone
/// called that ambiguous and refused. A legitimate decorated append into any
/// section whose sibling is also edited therefore could not land — the mirror
/// image of the defect this loop exists to close, and the reason the boundary
/// rule (the target that ENDS there owns it) is part of the law.
///
/// Its control is [`a_token_composed_out_of_retained_bytes_refuses`]: a token
/// that genuinely has no payload to strip still refuses. The rule narrows
/// ambiguity, it does not remove it.
#[test]
fn a_decorated_append_beside_an_edited_sibling_commits_stripped() {
    let seed = "# Plan\n\nbody\n\n## Alpha\n\nalpha body\n\n## Beta\n\nbeta body\n";
    let (dir, root) = ws(seed);
    splice(
        &root,
        None,
        &args(
            vec![
                Edit {
                    target: hpath("Plan/Alpha"),
                    edit: EditShape::Put {
                        at: PutAt::End,
                        text: format!("see [[guide#^task1{TOKEN}|Task One]]\n"),
                    },
                    if_node_rev: None,
                },
                // The sibling that shares Alpha's end byte, edited in the SAME
                // batch — which is what puts two containers on that byte.
                Edit {
                    target: hpath("Plan/Beta"),
                    edit: EditShape::Put {
                        at: PutAt::End,
                        text: "beta gains a line.\n".into(),
                    },
                    if_node_rev: None,
                },
            ],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect("a decorated append beside an edited sibling is legitimate and must LAND");

    let text = on_disk(&dir);
    assert!(
        !text.contains(TOKEN),
        "the token was attributed and stripped, not refused:\n{text}"
    );
    assert!(
        text.contains("see [[guide#^task1|Task One]]"),
        "the author's link survives, decoration removed:\n{text}"
    );
    assert!(text.contains("beta gains a line."), "{text}");
}

/// A token already on disk is not this write's to remove: deleting bytes the
/// batch never addressed would move the fingerprint of a node this write does not
/// own — reddening pins that have nothing to do with it. The write proceeds, and
/// introduces nothing.
#[test]
fn a_pre_existing_token_is_left_exactly_as_found() {
    let seed = format!("# Plan\n\nsee [[guide#^goal{TOKEN}|G]]\n\n## Sub\n\nsub body\n");
    let (dir, root) = ws(&seed);
    splice(
        &root,
        None,
        &args(
            vec![Edit {
                target: hpath("Plan/Sub"),
                edit: EditShape::Put {
                    at: PutAt::End,
                    text: "one more line.\n".into(),
                },
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect("an unrelated edit on a damaged page still commits");
    let text = on_disk(&dir);
    assert!(
        text.contains(&format!("[[guide#^goal{TOKEN}|G]]")),
        "the pre-existing token is untouched — this write does not own those bytes:\n{text}"
    );
    assert!(text.contains("one more line."), "{text}");
}

/// **Finding 22, stated at exactly the width of its proof.** `create` already
/// strips its whole body, so its grain was never the problem — the question is
/// WHICH positions the one grammar calls claim links. Frontmatter, HTML comments
/// and indented code are not among them, for the same reason a code fence is not
/// (R22): the dialect parse mints no link node there. The control below is the
/// half that makes this honest — the DECORATE side reads the same tree, so the
/// engine can never mint a token in a position the strip cannot reach.
#[test]
fn frontmatter_comments_and_indented_code_are_not_claim_link_positions() {
    let (dir, root) = ws("# Seed\n");
    let body = format!(
        "---\nsource: \"[[guide#^fm{TOKEN}|FM]]\"\n---\n\n# Born\n\n\
         live [[guide#^live{TOKEN}|L]]\n\n\
         <!-- [[guide#^cmt{TOKEN}|C]] -->\n\n\
         para\n\n    [[guide#^code{TOKEN}|D]]\n\n\
         ```text\n[[guide#^fence{TOKEN}|F]]\n```\n"
    );
    wire_serve::write::create(
        &root,
        None,
        &CreateArgs {
            id: None,
            path: WPath("born.md".into()),
            body,
            actor: Some("agent-scribe".into()),
            now: None,
            if_root: None,
            dry: false,
        },
        &[],
    )
    .expect("the birth lands");
    let text = std::fs::read_to_string(dir.path().join("born.md")).expect("read");

    assert!(
        text.contains("live [[guide#^live|L]]"),
        "the one claim-link position is stripped:\n{text}"
    );
    for (what, spelling) in [
        ("frontmatter", "^fm"),
        ("an HTML comment", "^cmt"),
        ("indented code", "^code"),
        ("a code fence", "^fence"),
    ] {
        assert!(
            text.contains(&format!("[[guide#{spelling}{TOKEN}")),
            "{what} is not a claim-link position — the strip leaves it verbatim:\n{text}"
        );
    }

    // THE CONTROL — decorate and strip read ONE tree. A position the strip does
    // not reach is a position the decorator never mints into, so the round trip
    // cannot leak a token into it.
    let doc = fs::load(&root, std::path::Path::new("born.md")).expect("load");
    let mut blocks = Vec::new();
    collect_link_blocks(&doc.root, &mut blocks);
    assert_eq!(
        blocks,
        vec!["live".to_owned()],
        "the ONE grammar sees exactly one claim link in this document"
    );
}

fn collect_link_blocks(node: &model::Node, out: &mut Vec<String>) {
    if let model::NodeKind::Wikilink { block: Some(b), .. }
    | model::NodeKind::Embed { block: Some(b), .. } = &node.kind
    {
        out.push(b.clone());
    }
    for child in &node.children {
        collect_link_blocks(child, out);
    }
}

/// **Finding 15.** The pre-flight refused the address the committer accepted:
/// `check_write.at` kept its decoration while `read::to_model_ref` peeled it. One
/// question, two answers. Both entry points now peel at the address owner.
#[test]
fn check_write_and_splice_agree_on_a_decorated_address() {
    let (dir, root) = ws("# Plan\n\nthe goal line ^goal\n");
    let prev = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let decorated = format!("^goal{TOKEN}");

    let ResponseBody::CheckWrite { refuse, .. } = wire_serve::check_write::check_write(
        &prev,
        &dir.path().join("plan.md").display().to_string(),
        "agent-scribe",
        "2026-07-25T00:00:00Z",
        &[CheckWriteEdit {
            op: "replace".into(),
            at: decorated.clone(),
            find: "the goal line".into(),
            body: "the goal line, rewritten".into(),
            rev: String::new(),
            all: false,
        }],
    ) else {
        panic!("check_write body");
    };
    assert!(
        refuse.is_none(),
        "the pre-flight must resolve the address the committer resolves: {refuse:?}"
    );

    splice(
        &root,
        None,
        &args(
            vec![Edit {
                target: SecRef::Anchor {
                    anchor: decorated.trim_start_matches('^').to_owned(),
                },
                edit: EditShape::Match {
                    old: "the goal line".into(),
                    new: "the goal line, rewritten".into(),
                },
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect("the committer accepts the same decorated address");
    assert!(
        on_disk(&dir).contains("the goal line, rewritten"),
        "{}",
        on_disk(&dir)
    );
}

/// The pre-flight judges the STRIPPED candidate, not the decorated draft — S4a's
/// thesis at document grain: a def rule reading the page's bytes sees what
/// `splice` will commit.
#[test]
fn the_pre_flight_judges_the_stripped_candidate() {
    let (_dir, root) = ws("# Plan\n\nbody\n");
    let prev = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let next = wire_serve::check_write::candidate(
        &prev,
        &[CheckWriteEdit {
            op: "append".into(),
            at: "Plan".into(),
            find: String::new(),
            body: format!("added [[guide#^goal{TOKEN}|G]]"),
            rev: String::new(),
            all: false,
        }],
    )
    .expect("the rebuild succeeds");
    assert!(
        !next.raw.contains(TOKEN),
        "the candidate the ladder judges carries no token:\n{}",
        next.raw
    );
}
