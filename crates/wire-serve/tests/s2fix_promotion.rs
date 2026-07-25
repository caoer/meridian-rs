//! fix2b — promotion correctness: the anchor promotion is **rev-neutral**,
//! **gated**, and **genuinely last** (review findings 7, 9, 12).
//!
//! Every test drives the production choke-point (`wire_serve::write::splice`)
//! against a real on-disk workspace, and every claim is asserted AS the claim:
//! byte-identity of both files for a refusal (never a walk reading), and
//! FINGERPRINT equality for rev-neutrality (never a rendered colour). Each
//! comparison of two document states carries a CONTROL proving the two states
//! hash DIFFERENTLY when the bytes really differ — a rev-neutrality gate whose
//! hash cannot move proves nothing.

use wire::{ErrorCode, Path as WPath, PinSpec, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// The pinning page — no lock block, so the first pin BIRTHS one at EOF.
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

fn workspace(pinner: &str, target: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), pinner).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), target).expect("target");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn pin_args(pinning_page: &str, selector: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: WPath(pinning_page.into()),
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

fn pin_fact(body: &ResponseBody) -> wire::PinFact {
    let ResponseBody::Splice { pin, .. } = body else {
        panic!("splice body");
    };
    pin.as_deref().cloned().expect("a pin answers with a fact")
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// The engine's own content id for a page's bytes — quoted in the byte-identity
/// assertions so a failure names WHICH bytes moved, not just that they did.
fn rev(bytes: &str) -> String {
    model::build(bytes.to_string(), syntax::parse(bytes))
        .root
        .node_rev
        .0
}

/// The live fingerprint of a lock `ref`, computed the way the VERIFY plane
/// computes it: the normative selector grammar → resolve → hash that span.
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
        .into_string()
}

/// The fingerprint of one heading selector over ARBITRARY bytes (no disk) — the
/// control vehicle: it answers "would these two documents hash the same?".
fn fingerprint_of(bytes: &str, chain: &[&str]) -> String {
    let doc = model::build(bytes.to_string(), syntax::parse(bytes));
    let r#ref = model::Ref::Hpath(
        chain
            .iter()
            .map(|h| model::HpathSeg {
                h: (*h).to_string(),
                n: None,
            })
            .collect(),
    );
    let target = model::resolve(&doc, &r#ref).expect("the chain resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
        .expect("the fixture target has content")
        .into_string()
}

// ---------------------------------------------------------------------------
// FINDING 12 — a refused pin leaves every file byte-unchanged
// ---------------------------------------------------------------------------

/// A heading whose TEXT carries a `/`. The lock ref grammar joins a heading
/// chain with `/`, so `A/B` cannot round-trip and the pin must refuse — the
/// reviewer's finding-12 repro, one rung after the promotion.
const SLASH_TARGET: &str = "# Guide\n\n## A/B\n\nreview before you close.\n";

#[test]
fn a_refused_pin_leaves_both_files_byte_unchanged() {
    let (_dir, root) = workspace(PINNER, SLASH_TARGET);
    let plan_before = read(&root, "plan.md");
    let guide_before = read(&root, "guide.md");

    let err = splice(&root, 0, &pin_args("plan.md", "Guide/A-B"), &[], None)
        .expect_err("a `/` in the heading text cannot round-trip the ref grammar");

    // The refusal itself is unchanged — this gate is about the BYTES, not about
    // making the pin succeed.
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("round-trip")),
        "the teaching refusal still fires: {:?}",
        err.message
    );

    let guide_after = read(&root, "guide.md");
    assert_eq!(
        guide_after,
        guide_before,
        "the pinned TARGET is byte-unchanged by a refused pin — rev before {} vs after {}",
        rev(&guide_before),
        rev(&guide_after)
    );
    let plan_after = read(&root, "plan.md");
    assert_eq!(
        plan_after,
        plan_before,
        "and so is the pinning page — rev before {} vs after {}",
        rev(&plan_before),
        rev(&plan_after)
    );
}

/// CONTROL for the gate above: the promotion this refusal must not leave behind
/// is a REAL byte change, so "byte-unchanged" is a load-bearing assertion and
/// not a fixture that cannot move. The same page, promoted by hand, differs.
#[test]
fn the_promotion_the_refusal_withholds_is_a_real_byte_change() {
    let promoted = SLASH_TARGET.replace("## A/B\n", "## A/B\n^a-b\n");
    assert_ne!(
        promoted, SLASH_TARGET,
        "the marker is a byte change, so byte-identity can fail"
    );
    assert_ne!(
        rev(&promoted),
        rev(SLASH_TARGET),
        "and the two states carry different content ids"
    );
}

/// The refusal is DETERMINISTIC (the heading still carries its `/`), so a
/// re-pin refuses identically and still writes nothing: unlike the accepted G3
/// crash orphan, this shape never had a healing path.
#[test]
fn the_refusal_repeats_and_still_writes_nothing() {
    let (_dir, root) = workspace(PINNER, SLASH_TARGET);
    for attempt in 1..=3 {
        let err = splice(&root, 0, &pin_args("plan.md", "Guide/A-B"), &[], None)
            .expect_err("still unrepresentable");
        assert_eq!(err.code, ErrorCode::BadRequest, "attempt {attempt}");
        assert_eq!(read(&root, "guide.md"), SLASH_TARGET, "attempt {attempt}");
        assert_eq!(read(&root, "plan.md"), PINNER, "attempt {attempt}");
    }
}

// ---------------------------------------------------------------------------
// FINDING 7 — rev-neutrality holds into a target with no trailing terminator
// ---------------------------------------------------------------------------

/// A target whose LAST LINE is a heading with NO terminator. Promoting into
/// `Guide/Omega` puts the marker at EOF, which is where a terminator gets
/// appended — and that byte lies OUTSIDE the marker line, so it moves every
/// enclosing span's canonical bytes.
const EOF_TARGET: &str = "# Guide\n\n## Alpha\n\nalpha body.\n\n## Omega";

#[test]
fn promoting_at_eof_leaves_another_pages_pinned_fingerprint_identical() {
    let (_dir, root) = workspace(PINNER, EOF_TARGET);
    // Page one pins the WHOLE-PAGE section (a heading ref, never a bare
    // `#^anchor`: an anchor's span is its host line and would hash the same in
    // every document). Its span runs to EOF, so it is the span an appended
    // terminator moves.
    std::fs::write(root.0.join("other.md"), PINNER).expect("second pinning page");
    let first = pin_fact(
        &splice(&root, 0, &pin_args("other.md", "Guide"), &[], None)
            .expect("the first pin commits")
            .body,
    );
    assert_eq!(first.declared_ref, "guide.md#Guide");
    assert_eq!(
        first.fingerprint,
        live_fingerprint(&root, "guide.md#Guide"),
        "the first claim is GREEN before the second pin runs"
    );

    // Page two pins a section of the same target — the promotion lands at EOF.
    let second = pin_fact(
        &splice(&root, 0, &pin_args("plan.md", "Guide/Omega"), &[], None)
            .expect("the second pin commits")
            .body,
    );
    assert!(second.promoted, "the marker was written");

    assert_eq!(
        live_fingerprint(&root, "guide.md#Guide"),
        first.fingerprint,
        "the OTHER page's pinned span hashes identically after the promotion — \
         rev-neutral means rev-neutral (target now: {:?})",
        read(&root, "guide.md")
    );
    assert_eq!(
        second.fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Omega"),
        "and the newly minted claim is green too"
    );

    // The marker at an unterminated EOF must still READ as an anchor, or D15's
    // idempotency would break exactly where the neutrality fix applies: a re-pin
    // would fail to see it and try to promote a second one.
    let promoted_target = read(&root, "guide.md");
    let again = pin_fact(
        &splice(&root, 0, &pin_args("plan.md", "Guide/Omega"), &[], None)
            .expect("the re-pin commits")
            .body,
    );
    assert_eq!(again.anchor, second.anchor, "the same slug, recomputed");
    assert!(!again.promoted, "and nothing is promoted a second time");
    assert_eq!(
        read(&root, "guide.md"),
        promoted_target,
        "the target is byte-unchanged by the re-pin"
    );
}

/// CONTROL for the gate above, and the reason the fix is what it is: norm-v2
/// masks an own-line marker, it does NOT mask a bare EOF terminator. These two
/// documents differ by exactly one `\n` at EOF and they fingerprint
/// DIFFERENTLY — so a promotion that appends one moves the hash.
#[test]
fn a_bare_eof_terminator_moves_the_fingerprint_while_a_marker_line_does_not() {
    let terminated = format!("{EOF_TARGET}\n");
    assert_ne!(
        fingerprint_of(&terminated, &["Guide"]),
        fingerprint_of(EOF_TARGET, &["Guide"]),
        "a bare EOF terminator is NOT masked — this is the byte the promotion \
         must not add"
    );
    // The marker line, added the way a correct promotion adds it at EOF (no
    // terminator of its own, so norm-v2's R2b takes the one before it), is
    // masked whole.
    let marked = format!("{EOF_TARGET}\n^omega");
    assert_eq!(
        fingerprint_of(&marked, &["Guide"]),
        fingerprint_of(EOF_TARGET, &["Guide"]),
        "an own-line marker at an unterminated EOF is masked with its leading \
         terminator (norm-v2 R2b)"
    );
}

// ---------------------------------------------------------------------------
// FINDING 9 — the promotion write passes the armed gate, like every other write
// ---------------------------------------------------------------------------

/// A convention pack scoped to the pin TARGET only: it refuses any change to
/// `guide.md` and says nothing about the pinning page. That scoping is what
/// makes this gate DISTINGUISHING — the pinning page's own batch passes the
/// armed law, so a refusal can only come from gating the promotion itself.
const FROZEN_GUIDE_CHECK: &str = r#"---
paths:
  - guide.md
---

# frozen-guide (fixture convention)

Any change to `guide.md` is refused. The legal path is not changing it.

```starlark
def check_change(change):
    refuse(
        message = "frozen-guide: guide.md is frozen by an armed convention",
        passing = "scenarios/leave-it-alone.md",
    )
```
"#;

struct PackFiles {
    dir: std::path::PathBuf,
}

impl policy::ConventionFiles for PackFiles {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir.join(rel))
    }
    fn exists(&self, rel: &str) -> bool {
        self.dir.join(rel).exists()
    }
}

/// Arm `frozen-guide` in the workspace, through the SAME loader the door reads:
/// the pack on disk, an attested INDEX pinned to the pack's live rev, and the
/// once-armed marker.
fn arm_frozen_guide(root: &fs::WorkspaceRoot) {
    let pack = root.0.join("conventions/frozen-guide");
    std::fs::create_dir_all(pack.join("scenarios")).expect("pack dir");
    std::fs::write(pack.join("CHECK.md"), FROZEN_GUIDE_CHECK).expect("CHECK.md");
    std::fs::write(
        pack.join("scenarios/leave-it-alone.md"),
        "# leave it alone\n\nThe legal path: do not change `guide.md`.\n",
    )
    .expect("scenario");

    let files = PackFiles { dir: pack };
    let swept = policy::sweep(&files, "frozen-guide", policy::CheckLimits::default())
        .expect("the fixture pack loads");
    let rev = swept.rev().to_string();
    let armed = policy::arm(swept, &rev, policy::Enforcement::Block).expect("arm at the live rev");
    std::fs::write(
        root.0.join(fs::domain::RESERVED_INDEX_PATH),
        policy::generate_index(&[armed]),
    )
    .expect("INDEX");
    let marker = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
    std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker dir");
    std::fs::write(marker, "").expect("once-armed marker");
}

#[test]
fn the_promotion_is_refused_by_an_armed_convention_on_the_target() {
    let (_dir, root) = workspace(PINNER, "# Guide\n\n## Omega\n\nbody.\n");
    arm_frozen_guide(&root);
    let guide_before = read(&root, "guide.md");
    let plan_before = read(&root, "plan.md");

    let err = splice(&root, 0, &pin_args("plan.md", "Guide/Omega"), &[], None)
        .expect_err("the armed law refuses the change to the target");
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("frozen-guide")),
        "the refusal is the armed convention's, naming the rule: {:?} / {:?}",
        err.code,
        err.message
    );

    assert_eq!(
        read(&root, "guide.md"),
        guide_before,
        "no marker landed on the frozen target"
    );
    assert_eq!(
        read(&root, "plan.md"),
        plan_before,
        "and no lock landed on the pinning page"
    );
}

/// CONTROL for the gate above: the same pin, same fixture, UNARMED — it
/// commits. So the refusal above is the armed law's answer about the target,
/// not a pin that cannot work in this fixture.
#[test]
fn the_same_pin_commits_when_the_target_is_not_armed() {
    let (_dir, root) = workspace(PINNER, "# Guide\n\n## Omega\n\nbody.\n");
    let fact = pin_fact(
        &splice(&root, 0, &pin_args("plan.md", "Guide/Omega"), &[], None)
            .expect("an unarmed workspace is a no-op gate")
            .body,
    );
    assert!(fact.promoted, "the marker landed");
    assert!(read(&root, "guide.md").contains("^omega"));
    assert!(read(&root, "plan.md").contains("```meridian-lock"));
}

/// A dry pin rehearses the promotion's gate too: the armed refusal fires where
/// the real write would refuse (§4.4 — a rehearsal refuses exactly where the
/// real write does), and a rehearsal writes nothing either way.
#[test]
fn a_dry_pin_rehearses_the_promotions_gate() {
    let (_dir, root) = workspace(PINNER, "# Guide\n\n## Omega\n\nbody.\n");
    arm_frozen_guide(&root);
    let mut args = pin_args("plan.md", "Guide/Omega");
    args.dry = true;

    let err = splice(&root, 0, &args, &[], None).expect_err("the rehearsal refuses too");
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("frozen-guide")),
        "{:?}",
        err.message
    );
    assert!(!read(&root, "guide.md").contains("^omega"));
}
