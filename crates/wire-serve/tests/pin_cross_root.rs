//! Cross-root pin — the WRITE half (ruled design D-A..D-D, session
//! 12-04-f2-mrd-integration `results/pin-cross-root-design.md`).
//!
//! D-A: `PinSpec.target` carries the ruled `name:rel` spelling; the lock row's
//! `object` receives it verbatim minus `.md`. D-B (round 2): anchor promotion
//! is one-call and engine-internal — the marker lands in the TARGET root under
//! that root's own write flock (`LOCK_NB`, so a busy target refuses
//! `workspace_busy` and no hold-and-wait cycle can form). D-C, translated to
//! the § A.3 proof law: the proof compare runs against the TARGET root's live
//! bytes — a token from the ambient root's same-named (different-content)
//! file must NOT pass a cross-root pin (the §8.2 bypass shape, arrived at
//! through the compare instead of the dead ledger). D-D: `--vibe` writes the
//! loose blob into the TARGET root's object store, where the history that
//! diffs it lives.
//!
//! One env-dependent lifecycle fn by design (`MERIDIAN_CONFIG` is
//! process-global; the `walk_op.rs` precedent): every mount-table reading lives
//! in [`cross_root_pin_lifecycle`]. The parse-grain refusals below it consult
//! no table and stay ordinary parallel tests.

use wire::{ErrorCode, Path as WPath, PinSpec, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// The pinning page in the ambient root.
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the other root.\n";
/// The pinned page in the TARGET root.
const TARGET: &str = "# Doc\n\n## Design\n\nthe target root's real design note.\n";

struct Sandbox {
    /// Held for its Drop.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    /// The ambient (pinning) workspace — mounted as `ws`.
    ws: fs::WorkspaceRoot,
    /// The mounted target root — `other`.
    other: fs::WorkspaceRoot,
}

fn git_init(dir: &std::path::Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "xr@example.invalid"],
        vec!["config", "user.name", "xr"],
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

/// Two git-initialised roots, both declared in one machine config: the
/// ambient `ws` (so the normalization case has a name for it) and the
/// mounted `other`. `MERIDIAN_CONFIG` is pointed at the config — once, in the
/// one lifecycle fn.
fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("ws");
    let other = tmp.path().join("other");
    for dir in [&home, &ws, &other] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    std::fs::write(ws.join("plan.md"), PINNER).expect("pinner");
    std::fs::write(ws.join("plan2.md"), "# Plan2\n\na second page.\n").expect("plan2");
    std::fs::write(
        ws.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# Ambient root\n",
    )
    .expect("ws declaration");
    std::fs::write(other.join("doc.md"), TARGET).expect("target");
    std::fs::write(
        other.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n\n# Other root\n",
    )
    .expect("other declaration");
    git_init(&ws);
    git_init(&other);

    let config = home.join("MERIDIAN.md");
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: ws\npath: {}\n```\n\n\
             ```meridian-mount\nname: other\npath: {}\n```\n",
            ws.display(),
            other.display()
        ),
    )
    .expect("mount table");
    // One env-dependent test binary by design (module docs). SAFETY: set
    // before any thread this test spawns; the sibling tests in this binary
    // never read the table.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config) };

    let ws = fs::WorkspaceRoot(std::fs::canonicalize(&ws).expect("canonical ws"));
    let other = fs::WorkspaceRoot(std::fs::canonicalize(&other).expect("canonical other"));
    Sandbox { tmp, ws, other }
}

fn pin_args(pinner: &str, target: &str, selector: &str, vibe: bool) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(pinner.into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath(target.into()),
            selector: wire::ReadSel::parse(selector),
            vibe: vibe.then_some(true),
            fingerprint: None,
            sec_rev: None,
        }),
        fields: Default::default(),
    }
}

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

/// Does this root's object store hold `oid` as a loose-or-packed object?
fn store_holds(root: &fs::WorkspaceRoot, oid: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(&root.0)
        .args(["cat-file", "-e", oid])
        .status()
        .expect("git cat-file")
        .success()
}

/// Production-arm sections read against ONE root, returning the served
/// section's proof token — which root a token came from is the whole D-C
/// question.
fn proof_read(root: &fs::WorkspaceRoot, rel: &str, selector: &str) -> String {
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
    sections.expect("sections mode")[0]
        .fingerprint
        .clone()
        .expect("a served section carries its proof token")
}

/// The one mount-table-reading lifecycle: mint, blob home, promotion home,
/// idempotent re-pin, target-flock refusal, the D-C proof compare both ways,
/// the promotion-stable token, unbound-root refusal, and same-root
/// normalization.
#[test]
#[allow(clippy::too_many_lines)] // one sequential lifecycle script by design
fn cross_root_pin_lifecycle() {
    let sb = sandbox();

    // — D-A + D-B + D-D: the bare-CLI cross-root pin mints, promotes into the
    //   TARGET root, and writes the vibe blob into the TARGET root's store.
    let out = splice(
        &sb.ws,
        None,
        &pin_args("plan.md", "other:doc.md", "Doc/Design", true),
        &[],
        None,
    )
    .expect("the cross-root pin commits");
    let fact = pin_fact(&out.body);
    assert_eq!(
        fact.target.0, "other:doc.md",
        "PinFact.target echoes the rooted spelling"
    );
    assert_eq!(
        fact.anchor, "design",
        "D15 slug from the target's own title"
    );
    assert!(fact.promoted, "the target had no id, so this pin wrote one");
    let blob = fact.blob.clone().expect("--vibe answers a blob oid");

    let pinner = read_page(&sb.ws, "plan.md");
    assert!(
        pinner.contains("object: \"[[other:doc]]\""),
        "the lock row's object is the canonical `name:rel` minus .md, verbatim (A-4/P5): {pinner}"
    );
    assert!(
        pinner.contains(&format!("hash: \"{blob}\"")),
        "the row's hash is the target blob: {pinner}"
    );
    let target_after = read_page(&sb.other, "doc.md");
    assert!(
        target_after.contains("^design"),
        "the promotion marker landed in the TARGET root's file (D-B one-call): {target_after}"
    );
    assert!(
        store_holds(&sb.other, &blob),
        "the vibe blob lives in the TARGET root's object store (D-D)"
    );
    assert!(
        !store_holds(&sb.ws, &blob),
        "and NOT in the pinning root's store — the diff comes from the target's history"
    );

    // — Idempotence: a re-pin reuses the marker and writes nothing new.
    let again = splice(
        &sb.ws,
        None,
        &pin_args("plan.md", "other:doc.md", "Doc/Design", true),
        &[],
        None,
    )
    .expect("a re-pin commits");
    let refact = pin_fact(&again.body);
    assert!(!refact.promoted, "the anchor is reused, not re-written");
    assert_eq!(
        read_page(&sb.other, "doc.md"),
        target_after,
        "the target is byte-identical across a re-pin"
    );

    // — D-B: a held TARGET flock refuses `workspace_busy` (LOCK_NB, transient),
    //   and nothing is written anywhere.
    let pinner_before = read_page(&sb.ws, "plan.md");
    {
        let _held = fs::WriteLock::acquire(&sb.other).expect("hold the target flock");
        let err = splice(
            &sb.ws,
            None,
            &pin_args("plan.md", "other:doc.md", "Doc/Design", true),
            &[],
            None,
        )
        .expect_err("a busy target root refuses");
        assert_eq!(err.code, ErrorCode::WorkspaceBusy);
        assert_eq!(
            read_page(&sb.ws, "plan.md"),
            pinner_before,
            "a refusal writes nothing"
        );
    }

    // — D-C (proof form): a token read off the AMBIENT root's same-named,
    //   different-content file must not pass a cross-root pin — the compare
    //   runs against the TARGET root's live bytes (§8.2 bypass shape).
    //   `ws/doc.md` shares the path and the section name, not the content.
    std::fs::write(
        sb.ws.0.join("doc.md"),
        TARGET.replace(
            "the target root's real design note.",
            "an ambient twin that merely shares the name.",
        ),
    )
    .expect("ambient twin");
    let twin_token = proof_read(&sb.ws, "doc.md", "Doc/Design");
    let mut args = pin_args("plan.md", "other:doc.md", "Doc/Design", true);
    args.actor = Some("agent-7".into());
    args.pin.as_mut().expect("pin").fingerprint = Some(twin_token);
    let err = splice(&sb.ws, None, &args, &[], None)
        .expect_err("the ambient twin's token is a different fact — the compare fails closed");
    assert_eq!(
        err.code,
        ErrorCode::PinProofRequired,
        "peel-and-compare-ambient would have admitted this — the bypass shape: {:?}",
        err.message
    );

    // — D-C green half: the token read off the TARGET root's own file admits
    //   the same pin — and keeps admitting it across the earlier promotion
    //   (the marker line is anchor-removed from the token), which is what
    //   replaced the dead D16 ledger refresh.
    let target_token = proof_read(&sb.other, "doc.md", "Doc/Design");
    args.pin.as_mut().expect("pin").fingerprint = Some(target_token.clone());
    let out =
        splice(&sb.ws, None, &args, &[], None).expect("the target root's own token admits the pin");
    assert!(
        !pin_fact(&out.body).promoted,
        "the earlier promotion's anchor is reused"
    );
    let out = splice(&sb.ws, None, &args, &[], None)
        .expect("the same token still covers the selector across the promotion");
    assert!(!pin_fact(&out.body).promoted);

    // — An unbound root refuses at the door, naming what IS bound.
    let err = splice(
        &sb.ws,
        None,
        &pin_args("plan.md", "ghost:doc.md", "Doc/Design", true),
        &[],
        None,
    )
    .expect_err("an unbound root cannot be pinned against");
    assert_eq!(err.code, ErrorCode::BadPath);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("ghost") && m.contains("other")),
        "the refusal names the unbound root and the bound set: {:?}",
        err.message
    );

    // — Normalization: a rooted spelling naming the PINNING root itself is the
    //   same-root pin under one name — bare object, bare fact, single flock.
    let out = splice(
        &sb.ws,
        None,
        &pin_args("plan.md", "ws:plan2.md", "Plan2", true),
        &[],
        None,
    )
    .expect("a self-root rooted spelling normalizes and commits");
    let fact = pin_fact(&out.body);
    assert_eq!(
        fact.target.0, "plan2.md",
        "the spelling normalizes to the bare form — one name per thing"
    );
    assert!(
        read_page(&sb.ws, "plan.md").contains("object: \"[[plan2]]\""),
        "the lock row is byte-identical to a bare same-root pin"
    );
}

// — parse-grain refusals: no mount table is consulted (ordered before the
//   table read), so these stay ordinary parallel tests.

fn bare_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    git_init(dir.path());
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// Two colons in the head — no reading exists; refused, nothing written.
#[test]
fn a_double_colon_head_refuses_as_bad_path() {
    let (_dir, root) = bare_ws();
    let err = splice(
        &root,
        None,
        &pin_args("plan.md", "a:b:c.md", "Doc/Design", true),
        &[],
        None,
    )
    .expect_err("two colons have no ruled reading");
    assert_eq!(err.code, ErrorCode::BadPath);
    assert_eq!(read_page(&root, "plan.md"), PINNER, "nothing was written");
}

/// The rel half of a rooted spelling still obeys the §1 path law — an escape
/// refuses before any table or filesystem is consulted.
#[test]
fn a_rooted_spelling_with_an_escaping_rel_refuses_as_bad_path() {
    let (_dir, root) = bare_ws();
    for target in ["other:../victim.md", "other:/etc/passwd", "other:"] {
        let err = splice(
            &root,
            None,
            &pin_args("plan.md", target, "Doc/Design", true),
            &[],
            None,
        )
        .expect_err("an escaping rel half must refuse");
        assert_eq!(err.code, ErrorCode::BadPath, "{target}");
    }
    assert_eq!(read_page(&root, "plan.md"), PINNER, "nothing was written");
}

/// A malformed root name (uppercase) refuses — never reinterpreted as a
/// literal path (§4.1: no fallback reading).
#[test]
fn a_malformed_root_name_refuses_and_never_degrades_to_a_literal_path() {
    let (_dir, root) = bare_ws();
    let err = splice(
        &root,
        None,
        &pin_args("plan.md", "Sessions:doc.md", "Doc/Design", true),
        &[],
        None,
    )
    .expect_err("a malformed name refuses");
    assert_eq!(err.code, ErrorCode::BadPath);
    assert_eq!(read_page(&root, "plan.md"), PINNER, "nothing was written");
}
