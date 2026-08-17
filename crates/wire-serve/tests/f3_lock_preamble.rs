//! F3 (dogfood r3, D-USER): the `meridian-lock` block lives in the FILE
//! PREAMBLE — after the frontmatter, before the first heading — where no
//! section claims it.
//!
//! The EOF birth law put the block inside the page's LAST section: the
//! section's word count rose by lock machinery the author never wrote, a face
//! read of that section served YAML plumbing, and the delta feed fired
//! `edited §Last` for a write never aimed there while the receipt named only
//! the aimed sections — two surfaces, one write, different counts. Preamble
//! placement dissolves all three at once: the bytes belong to no section, so
//! counts stay honest, section reads stay prose, and the feed's node rows
//! enumerate exactly the sections the caller touched.
//!
//! An EXISTING block is still replaced across its own span wherever it sits
//! (legacy EOF placements included) — relocation would rewrite a region the
//! caller never aimed at and would stale the pin fingerprint minted before the
//! lock edit is composed. The trade: legacy pages keep their squatting block
//! until re-homed deliberately.

use wire::{Edit, EditShape, Path as WPath, PinSpec, PutAt, ResponseBody};
use wire_map::facts::{read_facts, words_total};
use wire_serve::write::{SpliceArgs, splice};

/// Pinning page: frontmatter + two sections. `Notes` is LAST — the section
/// the EOF law would have polluted.
const PINNER: &str =
    "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n\n# Notes\n\nkept notes.\n";

/// Pinned page.
const TARGET: &str = "# Guide\n\n## Steps\n\nreview before you close.\n";

/// A well-formed LEGACY page: the block sits at EOF, inside `# Notes`.
fn legacy_pinner() -> String {
    format!(
        "{PINNER}\n```meridian-lock\nversion: 2\npins:\n  - object: \"[[guide]]\"\n    \
         hash: \"9ae3f1c0deadbeef9ae3f1c0deadbeef9ae3f1c0\"\n    path: [\"Guide\", \"Steps\"]\n    \
         fingerprint: \"fp1.span2.b3.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n```\n"
    )
}

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "f3@example.invalid"],
        vec!["config", "user.name", "f3"],
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
        fields: Default::default(),
    }
}

fn read_page(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// The committed page's one lock block, found + rendered back to its
/// canonical bytes, plus its fence-to-fence span.
fn committed_block(root: &fs::WorkspaceRoot) -> (String, std::ops::Range<usize>) {
    let doc = fs::load(root, std::path::Path::new("plan.md")).expect("re-load");
    let found = lock::find(&doc).expect("clean lock state").expect("found");
    (lock::render(&found.lock), found.span)
}

/// Birth placement: the block lands between the frontmatter and the first
/// heading — the page's own bytes stand untouched on both sides of it.
#[test]
fn a_fresh_lock_births_as_file_preamble_after_the_frontmatter() {
    let (_dir, root) = workspace();
    splice(&root, None, &pin_args("Guide/Steps"), &[], None).expect("pin commits");

    let after = read_page(&root, "plan.md");
    let (block, _) = committed_block(&root);
    assert_eq!(
        after,
        format!(
            "---\ntitle: Plan\n---\n{block}\n\n# Plan\n\ndraws from the guide.\n\n# Notes\n\nkept notes.\n"
        ),
        "the block opens immediately after the frontmatter's closing fence, one \
         blank line separates it from the body, and the body's bytes are untouched"
    );
    assert_eq!(
        after.matches("```meridian-lock").count(),
        1,
        "exactly one block"
    );
}

/// No frontmatter: the preamble starts at byte zero, and the block with it.
#[test]
fn a_fresh_lock_births_at_byte_zero_when_the_page_has_no_frontmatter() {
    let (dir, root) = workspace();
    std::fs::write(
        dir.path().join("plan.md"),
        "# Plan\n\ndraws from the guide.\n",
    )
    .expect("frontmatterless pinner");

    splice(&root, None, &pin_args("Guide/Steps"), &[], None).expect("pin commits");

    let after = read_page(&root, "plan.md");
    let (block, _) = committed_block(&root);
    assert_eq!(
        after,
        format!("{block}\n\n# Plan\n\ndraws from the guide.\n"),
        "the block is the file's first construct, one blank line before the \
         first heading, body bytes untouched"
    );
}

/// The placement's point: the block's bytes sit inside NO section span, so no
/// section's word count absorbs lock machinery — section rows hold their
/// pre-pin values and the whole-file banner (raw-byte `wc -w` law) grows by
/// exactly the block's own words.
#[test]
fn the_lock_bytes_belong_to_no_section_and_counts_stay_honest() {
    let (_dir, root) = workspace();
    let before_doc = fs::load(&root, std::path::Path::new("plan.md")).expect("load");
    let before_facts = read_facts(
        &wire_map::project_toc(&before_doc),
        before_doc.raw.as_bytes(),
    );
    let banner_before = words_total(before_doc.raw.as_bytes());

    splice(&root, None, &pin_args("Guide/Steps"), &[], None).expect("pin commits");

    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("re-load");
    let (_, block) = committed_block(&root);
    let facts = read_facts(&wire_map::project_toc(&doc), doc.raw.as_bytes());
    let (b_start, b_end) = (
        u64::try_from(block.start).expect("start fits"),
        u64::try_from(block.end).expect("end fits"),
    );
    for fact in facts.iter().filter(|f| f.depth > 0) {
        assert!(
            fact.span.1 <= b_start || fact.span.0 >= b_end,
            "no section span may claim the lock bytes: section `{}` spans {:?}, block {block:?}",
            fact.title,
            fact.span
        );
    }
    for before in before_facts.iter().filter(|f| f.depth > 0) {
        let now = facts
            .iter()
            .find(|f| f.title == before.title && f.depth == before.depth)
            .expect("every pre-pin section survives the pin");
        assert_eq!(
            now.words, before.words,
            "section `{}` counts only the author's prose — the pin moved its count",
            before.title
        );
    }
    let block_words = words_total(doc.raw[block.clone()].as_bytes());
    assert_eq!(
        words_total(doc.raw.as_bytes()),
        banner_before + block_words,
        "the banner counts the FILE (raw bytes), so preamble text — the lock \
         block included — rides the banner and only the banner"
    );
}

/// One write, one truth: the committed delta's node rows name exactly the
/// sections the caller aimed at. The lock bytes land where no section claims
/// them, so the feed no longer fires `edited §Last` for a write whose receipt
/// never named it (the file-grain `modified` fact carries the preamble
/// change). The pin's anchor promotion is the OTHER file this call wrote —
/// its own row, told, not folded (r8 D4).
#[test]
fn one_pin_write_yields_one_truth_across_receipt_and_feed() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Guide/Steps");
    args.edits = vec![Edit {
        target: wire::SecRef::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "Plan".into(),
                n: None,
            }],
        },
        edit: EditShape::Put {
            at: PutAt::End,
            text: "\nan appended line.\n".into(),
        },
        if_node_rev: None,
    }];

    let out = splice(&root, None, &args, &[], None).expect("append + pin commit");
    let frame = out.committed.expect("a real write emits one delta");
    let paths: Vec<&str> = frame
        .delta
        .files
        .iter()
        .map(|f| f.path.0.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["plan.md", "guide.md"],
        "the pinning page and the promotion target — both of this call's writes"
    );
    let targets: Vec<String> = frame.delta.files[0]
        .nodes
        .iter()
        .map(|n| format!("{:?}", n.target))
        .collect();
    assert_eq!(
        frame.delta.files[0].nodes.len(),
        1,
        "exactly the aimed section — no row for the lock bytes, no row for the \
         last section: {targets:?}"
    );
    assert!(
        targets[0].contains("Plan"),
        "the one row is the aimed §Plan: {targets:?}"
    );
}

/// The EOF hazard is gone: pinning the page's own LAST section commits and
/// verifies green — the block no longer lands inside the pinned span.
#[test]
fn a_self_pin_of_the_pages_last_section_commits_green() {
    let (_dir, root) = workspace();
    let mut args = pin_args("Notes");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let out = splice(&root, None, &args, &[], None)
        .expect("the last section is pinnable from its own page under preamble placement");
    let ResponseBody::Splice { pin, .. } = &out.body else {
        panic!("splice body");
    };
    let fact = pin.as_deref().expect("pin fact");

    // Green immediately: the minted fingerprint equals the live one recomputed
    // from the committed page.
    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("re-load");
    let target = model::resolve(
        &doc,
        &model::Ref::Hpath(vec![model::HpathSeg {
            h: "Notes".into(),
            n: None,
        }]),
    )
    .expect("Notes resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    let live = model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
        .expect("Notes has content")
        .into_string();
    assert_eq!(
        fact.fingerprint, live,
        "a self-pin of the last section verifies green immediately"
    );
}

/// The guard survives where it still bites: a page carrying its block INSIDE a
/// section (legacy EOF placement) refuses a self-pin of that section — the
/// replace-in-place edit would change the very bytes the pin fingerprints.
/// The refusal speaks the ratified register: reason first, then suggestions
/// fitted by applicability.
#[test]
fn a_self_pin_of_a_section_holding_a_legacy_block_still_refuses() {
    let (dir, root) = workspace();
    std::fs::write(dir.path().join("plan.md"), legacy_pinner()).expect("legacy-placed lock");

    let mut args = pin_args("Notes");
    args.pin.as_mut().expect("pin").target = WPath("plan.md".into());

    let err = splice(&root, None, &args, &[], None)
        .expect_err("the pinned section holds the block — unverifiable by construction");
    let msg = err.message.clone().unwrap_or_default();
    assert!(
        msg.contains("lock-is-content"),
        "the refusal names the law: {msg}"
    );
    assert!(
        msg.contains("file preamble"),
        "the refusal teaches where the block lives now: {msg}"
    );
    assert!(
        !read_page(&root, "plan.md").starts_with("```meridian-lock"),
        "and nothing was written"
    );
}

/// The named trade: an existing block is replaced across its OWN span, never
/// relocated — a legacy EOF block stays where it is until re-homed
/// deliberately, because relocation would rewrite a region the caller never
/// aimed at and stale the fingerprint minted before the lock edit composes.
#[test]
fn a_legacy_placed_block_is_replaced_in_its_own_span_not_relocated() {
    let (dir, root) = workspace();
    std::fs::write(dir.path().join("plan.md"), legacy_pinner()).expect("legacy-placed lock");

    // A fresh claim on another target section unions into the SAME block.
    splice(&root, None, &pin_args("Guide"), &[], None).expect("the union pin commits");

    let after = read_page(&root, "plan.md");
    assert_eq!(
        after.matches("```meridian-lock").count(),
        1,
        "still exactly one block"
    );
    assert!(
        after.starts_with(PINNER),
        "the block was replaced in place at EOF — not relocated to the preamble:\n{after}"
    );
    assert!(after.ends_with("```\n"), "the block still closes the file");
    let doc = fs::load(&root, std::path::Path::new("plan.md")).expect("re-load");
    let found = lock::find(&doc).expect("clean").expect("found");
    assert_eq!(
        found.lock.pins.len(),
        2,
        "both claims are held in the one block"
    );
}
