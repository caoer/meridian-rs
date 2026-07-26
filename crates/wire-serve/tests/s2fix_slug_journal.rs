//! R44 P0-1 (R45 shape B) — a convention SLUG cannot carry a forged `@fp` claim
//! into the RESERVED JOURNAL, because it cannot enter the engine at all.
//!
//! The door: `force_journal_write` strips the trailing anchor off a line
//! `receipt::journal::render_row` already returned, interpolates
//! `forced_rule={token_safe(&skip.rule)}`, and re-attaches the anchor — and
//! `ForcedSkip.rule` IS the armed convention's slug (`policy::gate` builds
//! `GateFinding.slug` from `ac.slug`, `wire-serve::gate` copies it). `token_safe`
//! is `split_whitespace().join("_")`: it removes no `[`, `]`, `@`, `#`, `^`. So a
//! convention folder named `[[guide#^goal@green.b3af12cd|G]]` lands a claim token
//! in the ledger the chain detector reads — **a forged claim in the journal forges
//! the chain detector's own input.**
//!
//! R45 ruled the INTAKE shape over per-renderer escaping: the slug takes the one
//! identifier charset at `policy::validate_slug`, so the hostile folder never
//! becomes an armed convention and the token is **unrepresentable rather than
//! removable** — this door and the INDEX door close at one owner, and so does any
//! renderer added later.
//!
//! THE ASSERT IS THE ARTIFACT (R26): `syntax::fp_removals` over the journal bytes
//! ON DISK, never a string that looks right. The absence is carried by a CONTROL
//! leg that drives the SAME force through the SAME fixture with a legal slug and
//! proves the journal row is really written — so "no token" can never be "no row".
//!
//! Redden at `c72d144c` (pre-fix), quoted in the unit card.

use wire::{Edit, EditShape, Path as WPath, PinSpec, PutAt, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// An `@fp` claim token in a claim-link position, spelled as a directory name.
/// `validate_slug` at `c72d144c` is a path-traversal guard (empty / leading dot /
/// `/` / `..`) with NO charset, so this is an admissible convention folder.
const TOKEN_SLUG: &str = "[[guide#^goal@green.b3af12cd|G]]";

/// The legal slug the control leg arms — and the placeholder the hostile INDEX is
/// rendered through, so both legs read bytes the ENGINE produced.
const LEGAL_SLUG: &str = "harness-frozen-guide";

/// A convention that refuses every change to `guide.md` — the live refusal the
/// `--force` leg must escape.
const FROZEN_GUIDE_CHECK: &str = r#"---
paths:
  - guide.md
---

# frozen guide (gate fixture)

```starlark
def check_change(change):
    refuse(
        message = "frozen-guide: guide.md is frozen by an armed convention",
        passing = "scenarios/leave-it-alone.md",
    )
```
"#;

const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

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

/// Arm `slug` in a fresh workspace: the pack on disk under its own folder name, an
/// attested INDEX pinned to the pack's live rev, and the once-armed marker.
///
/// The INDEX is rendered by the ENGINE through [`LEGAL_SLUG`] and then re-keyed to
/// `slug`, so the hostile leg reads the exact row grammar `generate_index` emits
/// (title, preamble, checkbox, middot fields) rather than a hand-typed imitation —
/// and the same fixture builds identically before and after the intake guard.
fn arm(root: &fs::WorkspaceRoot, slug: &str) {
    let pack = root.0.join("conventions").join(slug);
    std::fs::create_dir_all(pack.join("scenarios")).expect("pack dir");
    std::fs::write(pack.join("CHECK.md"), FROZEN_GUIDE_CHECK).expect("CHECK.md");
    std::fs::write(
        pack.join("scenarios/leave-it-alone.md"),
        "# leave it alone\n\nThe legal path: do not change `guide.md`.\n",
    )
    .expect("scenario");

    let files = PackFiles {
        dir: root.0.join("conventions").join(LEGAL_SLUG),
    };
    std::fs::create_dir_all(&files.dir).expect("render pack dir");
    std::fs::write(files.dir.join("CHECK.md"), FROZEN_GUIDE_CHECK).expect("render CHECK.md");
    let swept =
        policy::sweep(&files, LEGAL_SLUG, policy::CheckLimits::default()).expect("the pack loads");
    let rev = swept.rev().to_string();
    let armed = policy::arm(swept, &rev, policy::Enforcement::Block).expect("arm at the live rev");
    let index = policy::generate_index(&[armed]);
    if slug != LEGAL_SLUG {
        // Re-key the engine-rendered row to the hostile folder, and remove the
        // placeholder pack so exactly ONE convention is armed and on disk.
        std::fs::remove_dir_all(&files.dir).expect("drop the placeholder pack");
    }
    std::fs::write(
        root.0.join(fs::domain::RESERVED_INDEX_PATH),
        index.replace(LEGAL_SLUG, slug),
    )
    .expect("INDEX");

    let marker = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
    std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker dir");
    std::fs::write(marker, "").expect("once-armed marker");
}

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(
        dir.path().join("guide.md"),
        "# Guide\n\n## Omega\n\nbody.\n",
    )
    .expect("target");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn pin_args(force: bool) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: WPath("plan.md".into()),
        actor: None,
        now: Some("2026-07-25T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: "Guide/Omega".into(),
            vibe: None,
        }),
    }
}

/// The journal page's bytes, or empty when no row was ever appended — an absent
/// page is the strongest possible "no forged row", and reading it this way keeps
/// the assertion about BYTES rather than about a file's existence.
fn journal(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// **THE GATE.** The reviewer's own reproduction: the production splice choke
/// point, `force: true`, against an armed convention whose SLUG is a claim token.
/// The journal bytes on disk carry no `@fp` claim.
#[test]
fn a_convention_slug_lands_no_claim_token_in_the_reserved_journal() {
    let (_dir, root) = workspace();
    arm(&root, TOKEN_SLUG);
    let plan_before = read(&root, "plan.md");
    let guide_before = read(&root, "guide.md");

    let outcome = splice(&root, 0, &pin_args(true), &[], None);

    // THE ASSERT — the artifact, first, so a regression quotes the ranges.
    let journal = journal(&root);
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "an `@fp` claim token stands in a claim-link position in the RESERVED \
         JOURNAL — a claim nobody computed, in the ledger the chain-continuity \
         detector reads.\nfp_removals = {:?}\njournal:\n{journal}",
        syntax::fp_removals(&journal)
    );

    // And it is absent for the RIGHT reason: the intake refused the folder, so it
    // never became an armed convention and `--force` had nothing to escape. Note
    // `--force` does NOT escape this — a faulted armed set refuses before the gate
    // reads `change.force`, which is what makes the absence above unconditional.
    let err = outcome.expect_err("the intake refuses a convention slug outside the charset");
    let message = err.message.clone().unwrap_or_default();
    for named in [
        TOKEN_SLUG,
        "[a-z][a-z0-9-]*",
        "conventions/guide-goal-green-b3af12cd-g/",
    ] {
        assert!(
            message.contains(named),
            "the wire refusal carries the intake teaching ({named}): {:?} / {message}",
            err.code
        );
    }

    // A refusal writes nothing (R32 (1)): both files byte-unchanged.
    assert_eq!(
        read(&root, "plan.md"),
        plan_before,
        "the pinning page stands"
    );
    assert_eq!(read(&root, "guide.md"), guide_before, "the target stands");
}

/// **THE CONTROL for the absence above** — an assertion of absence passes on an
/// empty world, so this drives the SAME force through the SAME fixture with a
/// legal slug: the armed law refuses unforced, `--force` escapes it, and the
/// journal really does gain an `op=force` row naming `forced_rule=`. The gate
/// above is therefore measuring a door that is open, not machinery that never ran.
#[test]
fn the_same_force_journals_a_row_when_the_slug_is_in_charset() {
    let (_dir, root) = workspace();
    arm(&root, LEGAL_SLUG);

    // The armed convention genuinely refuses — the force escapes something real.
    splice(&root, 0, &pin_args(false), &[], None)
        .expect_err("the armed convention refuses the unforced pin");

    splice(&root, 0, &pin_args(true), &[], None).expect("--force escapes the armed refusal");

    let journal = journal(&root);
    assert!(
        journal.contains("op=force") && journal.contains(&format!("forced_rule={LEGAL_SLUG}")),
        "the forced row is written and names the bypassed rule:\n{journal}"
    );
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "an in-charset slug journals no claim token:\n{journal}"
    );
}

/// **THE SECOND SOURCE of `forced_rule=`, enumerated and driven.** `ForcedSkip.rule`
/// has exactly two origins in `policy::gate`: an armed convention's slug (closed by
/// intake, above) and the ENGINE-DERIVED `binding-break:{index|file}` of a forced
/// one-sided file↔index change. The second is a constant the engine writes, not
/// caller bytes — but "engine-derived" is a claim, and this milestone's rule is that
/// an undriven door of a named shape is the enumeration not closing (fix9's `sec`).
/// So it is driven: the row lands, and it carries no claim token either.
#[test]
fn the_engine_derived_forced_rule_journals_no_claim_token_either() {
    let (_dir, root) = workspace();
    arm(&root, LEGAL_SLUG);

    // A direct edit to the armed convention's own CHECK.md is a file-side binding
    // break (taxonomy row 9) — the door law refuses it before any convention runs.
    let check_path = format!("conventions/{LEGAL_SLUG}/CHECK.md");
    let break_args = |force: bool| SpliceArgs {
        id: None,
        path: WPath(check_path.clone()),
        actor: None,
        now: Some("2026-07-25T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: "frozen guide (gate fixture)".into(),
                    n: None,
                }],
            },
            edit: EditShape::Put {
                at: PutAt::End,
                text: "an out-of-band edit to the attested law.\n".into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    };

    // CONTROL — unforced, the door law refuses, so the force escapes something real.
    splice(&root, 0, &break_args(false), &[], None)
        .expect_err("the binding law refuses a one-sided change to an armed CHECK.md");

    splice(&root, 0, &break_args(true), &[], None).expect("--force escapes the binding break");

    let journal = journal(&root);
    assert!(
        journal.contains("forced_rule=binding-break:"),
        "the engine-derived rule name is journalled — or this door was not driven:\n{journal}"
    );
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "the engine-derived rule name lands no claim token:\n{journal}"
    );
}
