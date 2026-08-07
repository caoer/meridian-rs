//! R25 fix 2 — `@fp` strip at candidate-document grain (not payload field list).
//!
//! Pins R22: no `@fp` in a claim-link position on disk; non-claim positions
//! (fence, frontmatter, HTML comment, indented code) are explicit exclusions.

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

/// Live section `node_rev` (`replace_section` needs the rev the caller read).
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

/// Token in `plan_edits.create.title` strips with the body.
#[test]
fn a_token_in_create_title_is_stripped_with_the_body() {
    let (dir, root) = ws("---\ntitle: Plan\n---\n\n# Plan\n\nbody\n");
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::Create {
                parent_hpath: vec![wire::HpathSeg {
                    h: "Plan".into(),
                    n: None,
                }],
                title: format!("Draws from [[guide#^task1{TOKEN}|Task One]]"),
                body: format!("body [[guide#^task1{TOKEN}|B]]"),
                rev: None,
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

/// Payload shapes (append / `replace_section` / match needle+new) strip without a field list.
#[test]
fn every_payload_shape_is_covered_without_a_field_list() {
    let (dir, root) = ws("---\ntitle: Plan\n---\n\n# Plan\n\nold line\n\n## Sub\n\nsub body\n");
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::Append {
                hpath: vec![wire::HpathSeg {
                    h: "Plan".into(),
                    n: None,
                }],
                rev: None,
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

    // Nested target: separate splice (§4.4 disjointness).
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::ReplaceSection {
                hpath: vec![
                    wire::HpathSeg {
                        h: "Plan".into(),
                        n: None,
                    },
                    wire::HpathSeg {
                        h: "Sub".into(),
                        n: None,
                    },
                ],
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

    // Native match: decorated needle must still match stored (unstripped) bytes.
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

/// Over-strip guard: token inside a code fence survives (R22 sample).
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

/// Under-strip guard: composing a claim from retained bytes refuses.
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

/// Boundary rule: target that ENDS at shared byte owns it — decorated append
/// beside an edited sibling commits stripped (not false-red ambiguous).
/// Control: [`a_token_composed_out_of_retained_bytes_refuses`].
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
                // Sibling sharing Alpha's end byte, same batch.
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

/// Pre-existing on-disk token left alone (batch does not own those bytes).
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

/// Frontmatter / HTML comment / indented code / fence are not claim-link
/// positions (R22); control — decorate sees the same one link.
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

    // Control: one grammar — strip and decorate agree on claim-link set.
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

/// `check_write` and splice agree on a decorated address.
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
            at: vec![wire::HpathSeg { h: decorated.clone(), n: None }],
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

/// Pre-flight candidate is stripped (S4a: ladder sees what splice commits).
#[test]
fn the_pre_flight_judges_the_stripped_candidate() {
    let (_dir, root) = ws("# Plan\n\nbody\n");
    let prev = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let next = wire_serve::check_write::candidate(
        &prev,
        &[CheckWriteEdit {
            op: "append".into(),
            at: vec![wire::HpathSeg { h: "Plan".into(), n: None }],
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
