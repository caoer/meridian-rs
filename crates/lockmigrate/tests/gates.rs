//! **U9b gates** — the lock v1 → R4 v2 field migration.
//!
//! The safety properties this unit is judged on, each asserted against a real
//! on-disk vault driven through the production door (never an in-memory
//! double): the dry run writes NOTHING, unknown legacy keys survive
//! byte-for-byte, the sweep is idempotent and resumable, an illustration of a v1
//! block inside a document is LEFT ALONE, damage is refused rather than guessed
//! through, and a vault with no restore point is refused outright.

use lockmigrate::{Options, PageVerdict, sweep};
use std::fmt::Write as _;

const FP_A: &str = "fp1.span2.b3.a8222f5a4daeff2df5ffd61fbb7cb4ea00df7af479ef747b0f14a58666a2444d";
const FP_B: &str = "fp1.span2.b3.dcda25db4a73bfdd3091e5aa0c134e740be052384844d2100ca4f799cca4a0b7";

/// A git-initialised vault. Git is not decoration: the sweep REFUSES a vault
/// that is not a repository, because a pre-sweep commit is the only restore
/// point a lock rewrite has.
fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, body) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, body).expect("write fixture");
    }
    git_init(dir.path());
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn git_init(path: &std::path::Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "u9b@example.invalid"],
        vec!["config", "user.name", "u9b"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
}

/// A v1 lock block, EOF-placed — what the engine actually minted in the field.
fn v1_block(objects: &[(&str, &str)], pins: &[&str]) -> String {
    let mut s = String::from("```meridian-lock\nversion: 1\n");
    if !objects.is_empty() {
        s.push_str("objects:\n");
        for (k, v) in objects {
            let _ = writeln!(s, "  \"{k}\": \"{v}\"");
        }
    }
    if !pins.is_empty() {
        s.push_str("pins:\n");
        for p in pins {
            s.push_str(p);
        }
    }
    s.push_str("```\n");
    s
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read back")
}

fn dry() -> Options {
    Options {
        dry: true,
        ..Options::default()
    }
}

fn wet() -> Options {
    Options::default()
}

/// A one-page vault whose lock pins `target.md` by a two-segment body path,
/// carrying two unknown legacy keys.
fn one_page_vault() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let block = v1_block(
        &[("target.md", "13c3550f41b5796dd05381fd2420451f3ef1aa40")],
        &[&format!(
            "  - ref: \"target.md#Scratch notes/Findings\"\n    \
             fingerprint: \"{FP_A}\"\n    claim: owns the verdict\n    \
             legacy-note: do not drop me\n"
        )],
    );
    vault(&[
        ("page.md", &format!("# Page\n\nbody\n\n{block}")),
        (
            "target.md",
            "# Target\n\n## Scratch notes\n\n### Findings\n\nx\n",
        ),
    ])
}

// ── The dry run ────────────────────────────────────────────────────────────

/// **THE DRY RUN WRITES NOTHING.** Asserted on bytes, not on a flag: the report
/// says what it would do and the file on disk is unmoved to the byte.
#[test]
fn dry_run_writes_nothing() {
    let (_d, root) = one_page_vault();
    let before = read(&root, "page.md");

    let report = sweep(&root, &dry()).expect("sweep runs");
    assert_eq!(report.migrated(), 1, "the dry run still PLANS the rewrite");
    assert!(report.dry);
    assert_eq!(report.refusals(), 0);

    assert_eq!(read(&root, "page.md"), before, "not one byte moved");
    assert!(
        read(&root, "page.md").contains("version: 1"),
        "the v1 block is still v1 on disk"
    );
}

// ── Conversion fidelity ────────────────────────────────────────────────────

/// **Unknown legacy keys are carried VERBATIM.** R4 allows free-form keys on a
/// pin row, engine-ignored — so a migration that drops one it did not recognise
/// is data loss, and this is the assertion that says so.
#[test]
fn unknown_keys_survive_byte_for_byte() {
    let (_d, root) = one_page_vault();
    let report = sweep(&root, &wet()).expect("sweep runs");
    assert_eq!(report.migrated(), 1);

    let after = read(&root, "page.md");
    assert!(after.contains("version: 2"), "the block is v2 now");
    assert!(
        after.contains("    claim: owns the verdict"),
        "the `claim:` key rode across verbatim:\n{after}"
    );
    assert!(
        after.contains("    legacy-note: do not drop me"),
        "an UNKNOWN legacy key rode across verbatim:\n{after}"
    );

    // And the migrated block is readable by the LIVE reader — which is the
    // whole point of the exercise.
    let doc = model::build(after.clone(), syntax::parse(&after));
    let found = lock::find(&doc)
        .expect("the v2 block parses")
        .expect("present");
    assert_eq!(found.lock.version, lock::VERSION);
    assert_eq!(found.lock.pins.len(), 1);
    let pin = &found.lock.pins[0];
    assert_eq!(pin.object, "target.md");
    assert_eq!(pin.hash, "13c3550f41b5796dd05381fd2420451f3ef1aa40");
    assert_eq!(pin.fingerprint, FP_A);
    assert_eq!(
        pin.selector,
        lock::Selector::Path(vec!["Scratch notes".into(), "Findings".into()]),
        "the `/`-joined v1 selector became a real array"
    );
    assert_eq!(
        pin.extra.get("claim").map(String::as_str),
        Some("owns the verdict")
    );
    assert_eq!(
        pin.extra.get("legacy-note").map(String::as_str),
        Some("do not drop me")
    );
}

/// **Anchor rows convert cleanly, and there is no unconvertible class.**
/// `page#^claim` → `path: ["^claim"]` — the sole `^id` element, block grain.
/// A ref with no selector at all becomes `path: []`, the whole body.
#[test]
fn anchor_and_bare_refs_convert() {
    let cases: &[(&str, lock::Selector)] = &[
        (
            "target.md#^claim",
            lock::Selector::Path(vec!["^claim".into()]),
        ),
        ("target.md", lock::Selector::Path(vec![])),
        (
            "target.md#Design",
            lock::Selector::Path(vec!["Design".into()]),
        ),
        (
            "target.md#A/B/C",
            lock::Selector::Path(vec!["A".into(), "B".into(), "C".into()]),
        ),
    ];
    for (spelling, want) in cases {
        let slice = v1_block(
            &[("target.md", "deadbeef")],
            &[&format!(
                "  - ref: \"{spelling}\"\n    fingerprint: \"{FP_A}\"\n"
            )],
        );
        let parsed = lockmigrate::v1::parse(slice.trim_end()).expect("v1 parses");
        let v2 = lockmigrate::convert(&parsed).expect("converts");
        assert_eq!(&v2.pins[0].selector, want, "`{spelling}`");
        assert_eq!(v2.pins[0].object, "target.md");
        assert_eq!(v2.pins[0].hash, "deadbeef");
    }
}

/// A pin whose target has no `objects:` entry is DAMAGE, not a class: R4 never
/// omits the hash and this tool never invents one.
#[test]
fn a_pin_with_no_recorded_blob_is_refused() {
    let slice = v1_block(
        &[],
        &[&format!(
            "  - ref: \"target.md#Design\"\n    fingerprint: \"{FP_A}\"\n"
        )],
    );
    let parsed = lockmigrate::v1::parse(slice.trim_end()).expect("v1 parses");
    let err = lockmigrate::convert(&parsed).expect_err("no blob → refuse");
    assert!(
        matches!(err, lockmigrate::ConvertError::MissingBlob { .. }),
        "{err:?}"
    );
    assert!(err.to_string().contains("never invents one"));
}

// ── Idempotence and resumability ───────────────────────────────────────────

/// **Idempotent**: a second sweep over a migrated vault is a silent no-op WITH
/// A COUNT — the page is reported `already_v2`, not rewritten, and the bytes are
/// untouched.
#[test]
fn second_run_is_a_no_op_with_a_count() {
    let (_d, root) = one_page_vault();
    assert_eq!(sweep(&root, &wet()).expect("first").migrated(), 1);
    let after_first = read(&root, "page.md");

    let second = sweep(&root, &wet()).expect("second");
    assert_eq!(second.migrated(), 0, "nothing left to migrate");
    assert_eq!(second.already_v2(), 1, "and it SAYS so, with a count");
    assert_eq!(second.refusals(), 0);
    assert_eq!(read(&root, "page.md"), after_first, "bytes unmoved");
}

/// **Resumable**: a sweep interrupted mid-run re-runs cleanly, and the report
/// shows the remainder. Modelled by migrating one page, then adding a second
/// unmigrated page and re-running — the done page skips, the remainder writes.
#[test]
fn an_interrupted_sweep_resumes_and_reports_the_remainder() {
    let block_a = v1_block(
        &[("target.md", "aaa111")],
        &[&format!(
            "  - ref: \"target.md#A\"\n    fingerprint: \"{FP_A}\"\n"
        )],
    );
    let block_b = v1_block(
        &[("target.md", "bbb222")],
        &[&format!(
            "  - ref: \"target.md#B\"\n    fingerprint: \"{FP_B}\"\n"
        )],
    );
    let (_d, root) = vault(&[
        ("a.md", &format!("# A\n\n{block_a}")),
        ("b.md", &format!("# B\n\n{block_b}")),
        ("target.md", "# Target\n\n## A\n\nx\n\n## B\n\ny\n"),
    ]);

    // Run 1 completes both.
    let first = sweep(&root, &wet()).expect("first");
    assert_eq!(first.migrated(), 2);

    // Simulate an interruption BEFORE `b.md` landed: restore it to v1.
    std::fs::write(root.0.join("b.md"), format!("# B\n\n{block_b}")).expect("restore b");

    let resumed = sweep(&root, &wet()).expect("resume");
    assert_eq!(resumed.migrated(), 1, "only the remainder writes");
    assert_eq!(resumed.already_v2(), 1, "the done page skips");
    let remainder: Vec<&str> = resumed
        .pages
        .iter()
        .filter(|p| matches!(p, PageVerdict::Migrated { .. }))
        .map(PageVerdict::path)
        .collect();
    assert_eq!(remainder, vec!["b.md"], "the report NAMES the remainder");
}

// ── The discrimination rule ────────────────────────────────────────────────

/// **A v1 block illustrated inside a document is LEFT ALONE.** This is the
/// finding that shaped the unit: measured on ZT's live vaults, 17 of 19 v1
/// blocks are illustrations in decision records, design docs and verbatim
/// session traces. Rewriting them would corrupt the historical record while
/// reporting success.
#[test]
fn an_illustrated_v1_block_is_left_alone() {
    let block = v1_block(
        &[("target.md", "deadbeef")],
        &[&format!(
            "  - ref: \"target.md#A\"\n    fingerprint: \"{FP_A}\"\n"
        )],
    );
    // Prose AFTER the closing fence — the engine's placement law births a lock
    // at EOF, so content below it means a human put it there.
    let doc_page =
        format!("# Design doc\n\n## What this replaces\n\n{block}\nThree defects in five lines.\n");
    let (_d, root) = vault(&[
        ("design.md", &doc_page),
        ("target.md", "# Target\n\n## A\n\nx\n"),
    ]);
    let before = read(&root, "design.md");

    let report = sweep(&root, &wet()).expect("sweep runs");
    assert_eq!(report.migrated(), 0, "nothing was rewritten");
    assert_eq!(report.not_engine_placed(), 1);
    assert_eq!(
        report.refusals(),
        0,
        "out of scope is not damage — the gate still passes"
    );
    assert_eq!(read(&root, "design.md"), before, "the record is untouched");
    assert!(
        report.render().contains("LEFT ALONE, review by hand"),
        "and the report puts it in front of a human"
    );
}

/// **A document that illustrates the schema MANY times is still just a
/// document** — placement is tested before arity, and the order is the
/// correctness.
///
/// Regression for a real defect in this tool: with arity checked first, a page
/// carrying six illustration blocks was classified `MultipleBlocks` — a
/// REFUSAL — and would have blocked the gate permanently for being
/// documentation. Measured on the live corpus, the two verbatim ZT session
/// traces carry exactly this shape: six blocks, none page-terminal.
#[test]
fn a_document_with_many_illustrations_is_not_corruption() {
    let block = v1_block(
        &[("target.md", "deadbeef")],
        &[&format!(
            "  - ref: \"target.md#A\"\n    fingerprint: \"{FP_A}\"\n"
        )],
    );
    // Six blocks, prose after the last — a session trace's shape.
    let mut trace = String::from("# Trace\n\nZT said:\n\n");
    for _ in 0..6 {
        trace.push_str(&block);
        trace.push_str("\nand then:\n\n");
    }
    trace.push_str("...that is the whole discussion.\n");
    let (_d, root) = vault(&[
        ("trace.md", &trace),
        ("target.md", "# Target\n\n## A\n\nx\n"),
    ]);
    let before = read(&root, "trace.md");

    let report = sweep(&root, &wet()).expect("sweep runs");
    assert_eq!(
        report.refusals(),
        0,
        "documentation is not damage — the gate must not be blocked by it"
    );
    assert_eq!(report.not_engine_placed(), 1);
    assert!(
        matches!(
            report.pages[0],
            PageVerdict::NotEnginePlaced { blocks: 6, .. }
        ),
        "classified by PLACEMENT, with the block count reported: {:?}",
        report.pages[0]
    );
    assert_eq!(read(&root, "trace.md"), before, "the record is untouched");
}

/// Two lock blocks where the last IS page-terminal is corruption by the
/// engine's own reading (sole-writer mints one, and `lock::find` refuses two).
/// Refused, never guessed through — and the gate refuses completion.
#[test]
fn multiple_blocks_are_refused_and_the_gate_fails() {
    let block = v1_block(
        &[("target.md", "deadbeef")],
        &[&format!(
            "  - ref: \"target.md#A\"\n    fingerprint: \"{FP_A}\"\n"
        )],
    );
    let (_d, root) = vault(&[
        ("page.md", &format!("# P\n\n{block}\nmid\n\n{block}")),
        ("target.md", "# Target\n\n## A\n\nx\n"),
    ]);
    let before = read(&root, "page.md");

    let report = sweep(&root, &wet()).expect("sweep runs to a verdict");
    assert_eq!(report.migrated(), 0);
    assert_eq!(report.refusals(), 1, "the gate REFUSES completion");
    assert!(matches!(
        report.pages[0],
        PageVerdict::MultipleBlocks { blocks: 2, .. }
    ));
    assert_eq!(read(&root, "page.md"), before, "nothing was written");
}

/// An unparseable v1 body is listed, skipped, and the gate refuses completion —
/// **and each fixture triggers the refusal its row NAMES.**
///
/// `PageVerdict::Unparseable` has THREE producer paths: a missing or non-integer
/// `version:` line, a version that is neither v1 nor v2, and a v1 GRAMMAR
/// failure. `matches!(.., Unparseable { .. })` cannot tell them apart, so a
/// fixture that drifted into a neighbouring path would keep this test green
/// while it silently stopped covering the case in its name (Leader all-hands
/// #2; U9a hit that class six times).
///
/// So the detail is pinned per path, and all three are exercised — which is
/// also the acceptance half: if one fixture reached another's path, two rows
/// here would carry the same detail and the loop would fail.
#[test]
fn each_unparseable_path_is_reached_by_its_own_fixture() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "grammar.md",
            "# P\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n",
            // The v1 reader refused the BODY — the version gate passed first.
            "unrecognized line: `garbage here`",
        ),
        (
            "version-word.md",
            "# P\n\n```meridian-lock\nversion: banana\n```\n",
            "is not an integer",
        ),
        (
            "version-future.md",
            "# P\n\n```meridian-lock\nversion: 7\n```\n",
            "version 7 is neither v1 nor v2",
        ),
    ];
    let files: Vec<(&str, &str)> = cases.iter().map(|(n, body, _)| (*n, *body)).collect();
    let (_d, root) = vault(&files);
    let before: Vec<String> = files.iter().map(|(n, _)| read(&root, n)).collect();

    let report = sweep(&root, &wet()).expect("sweep runs to a verdict");
    assert_eq!(report.refusals(), 3, "every case refuses");
    assert_eq!(report.migrated(), 0);

    for (name, _, want_detail) in cases {
        let page = report
            .pages
            .iter()
            .find(|p| p.path() == *name)
            .unwrap_or_else(|| panic!("{name} has a verdict"));
        let PageVerdict::Unparseable { detail, .. } = page else {
            panic!("{name}: expected Unparseable, got {page:?}");
        };
        assert!(
            detail.contains(want_detail),
            "{name} must refuse via ITS OWN path.\n  want detail containing: {want_detail}\n  \
             got: {detail}"
        );
    }

    for ((name, _, _), was) in cases.iter().zip(before) {
        assert_eq!(read(&root, name), was, "{name}: nothing was written");
    }
    assert!(report.render().contains("the migration is not complete"));
}

// ── The restore point ──────────────────────────────────────────────────────

/// **A vault that is not a git repo is REFUSED.** The only undo for a lock
/// rewrite is a pre-sweep commit in the vault; without git there is none, and
/// the runbook says ask rather than proceed. Checked on a DRY run too — the dry
/// run exists to say whether the real run may proceed.
#[test]
fn a_vault_without_git_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("page.md"), "# P\n").expect("write");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    for opts in [dry(), wet()] {
        let err = sweep(&root, &opts).expect_err("no git → refuse");
        assert!(
            matches!(err, lockmigrate::SweepError::NotAGitRepo { .. }),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("no restore point"), "{msg}");
        assert!(msg.contains("ask before proceeding"), "{msg}");
    }
}

// ── The world guard, ARMED (--expect-root) ─────────────────────────────────

/// **THE WORLD GUARD, TWO ARMS: a MATCHING root PASSES and a MISMATCHED root
/// REFUSES.** Both arms, because a guard proven only by passing is a guard that
/// might be ignoring its input.
///
/// # Why this gate exists at all
/// The sweep shipped with `if_root: None` hard-coded — the §5.1 world guard was
/// UNARMED on every page — while the runbook described it as armed and required
/// (law 4.6). The claim and the code disagreed, and nothing would have caught
/// that during a sweep: every per-page CAS still passes, because each page is
/// individually consistent even when the VAULT is not the world the operator
/// inspected. `--expect-root` is what makes the operator's "I looked at this
/// vault" mean the vault they looked at.
#[test]
fn the_world_guard_refuses_a_mismatched_root_and_passes_a_matching_one() {
    // ── ARM 1: MISMATCHED root must REFUSE, and write nothing. ──
    let (_d, root) = one_page_vault();
    let before = read(&root, "page.md");
    let err = sweep(
        &root,
        &Options {
            expect_root: Some(wire::Root("not-this-vaults-root".into())),
            ..Options::default()
        },
    )
    .expect_err("a mismatched world MUST refuse");
    let lockmigrate::SweepError::Door { detail, .. } = &err else {
        panic!("expected a door refusal, got {err:?}");
    };
    assert!(
        detail.contains("root_mismatch"),
        "the refusal must be the WORLD guard, not some neighbouring refusal: {detail}"
    );
    assert_eq!(read(&root, "page.md"), before, "the refusal wrote nothing");

    // ── ARM 2: the MATCHING root must PASS. ──
    // The acceptance half. Without it, a guard that refused EVERYTHING would
    // satisfy arm 1 — and so would a `--expect-root` that was never read.
    let ambient = wire_serve::ambient_root(&root).expect("the vault has an ambient root");
    let report = sweep(
        &root,
        &Options {
            expect_root: Some(ambient.clone()),
            ..Options::default()
        },
    )
    .expect("the matching world migrates");
    assert_eq!(
        report.migrated(),
        1,
        "the page rewrote under the armed guard"
    );
    assert_ne!(read(&root, "page.md"), before, "and the bytes really moved");

    // ── The discriminator: the two arms differ, and differ FOR THE RIGHT
    // REASON. The mismatched value must not be accidentally equal to the real
    // one, or arm 1 would be proving nothing.
    assert_ne!(
        ambient.0, "not-this-vaults-root",
        "the fixture's wrong root must actually be wrong"
    );
}

/// UNARMED stays the default: `expect_root: None` migrates, exactly as the tool
/// behaved before the flag existed. The flag ADDS a guard; it does not quietly
/// become mandatory and break an operator who omits it.
#[test]
fn an_unarmed_sweep_is_unchanged() {
    let (_d, root) = one_page_vault();
    let report = sweep(&root, &Options::default()).expect("unarmed still runs");
    assert_eq!(report.migrated(), 1);
}

// ── The expected drift, named in advance ───────────────────────────────────

/// **S7: the expected full-body-pin drift is named BEFORE the sweep runs.**
/// Lock-is-content — a page's fingerprint covers its own lock block — so
/// rewriting the block moves the page, and every `path: []` pin naming it goes
/// stale once. A drift wave nobody predicted reads as corruption.
#[test]
fn expected_full_body_drift_is_named_in_advance() {
    // `a.md` pins the WHOLE BODY of `b.md`; `b.md` carries its own v1 lock, so
    // migrating `b.md` moves `b.md`'s fingerprint and staleness `a.md`'s pin.
    let a_block = v1_block(
        &[("b.md", "bbb222")],
        &[&format!("  - ref: \"b.md\"\n    fingerprint: \"{FP_A}\"\n")],
    );
    let b_block = v1_block(
        &[("target.md", "ccc333")],
        &[&format!(
            "  - ref: \"target.md#A\"\n    fingerprint: \"{FP_B}\"\n"
        )],
    );
    let (_d, root) = vault(&[
        ("a.md", &format!("# A\n\n{a_block}")),
        ("b.md", &format!("# B\n\n{b_block}")),
        ("target.md", "# Target\n\n## A\n\nx\n"),
    ]);

    // The DRY run already predicts it — that is the point of naming it in
    // advance rather than observing it afterwards.
    let report = sweep(&root, &dry()).expect("dry sweep");
    assert_eq!(report.migrated(), 2);
    let drift = &report.expected_drift;
    assert_eq!(drift.len(), 1, "one full-body pin drifts: {drift:?}");
    assert_eq!(drift[0].pinning_page, "a.md");
    assert_eq!(drift[0].object, "b.md");
    assert_eq!(drift[0].stale_fingerprint, FP_A);
    assert!(
        report.render().contains("EXPECTED fingerprint drift"),
        "and the report says so under its own heading"
    );

    // A section-grained pin does NOT appear — the claim is about `path: []`,
    // and a check that flagged everything would be satisfied by flagging
    // nothing in particular.
    assert!(
        drift.iter().all(|d| d.object != "target.md"),
        "the `#A` pin is not a full-body pin"
    );
}

// ── The key set, and the environment-varying defect class ──────────────────

/// **THE MIGRATED PIN ROW'S KEY SET IS PINNED, and it does not vary with the
/// environment.**
///
/// U8's broadened defect class (Leader all-hands #3): the question is not "did I
/// add a field" but "did I change WHICH KEYS APPEAR" — and U8's own instance was
/// a key set that varied with the ENVIRONMENT, because `blob` was optional and a
/// pin outside git serialized the key away.
///
/// A migration changes a row's key set on purpose; that IS the migration. The
/// hazard is the U8 shape — a key whose presence depends on where the tool ran.
/// **It cannot happen here, structurally:** `hash` is read from the v1
/// `objects:` table, never computed from git, so a vault with no git object for
/// the target does not silently emit a row without the key. It REFUSES
/// (`a_pin_with_no_recorded_blob_is_refused`). The row shape is a function of
/// the INPUT BYTES alone.
#[test]
fn the_migrated_row_key_set_is_exact_and_environment_independent() {
    let (_d, root) = one_page_vault();
    sweep(&root, &wet()).expect("sweep");
    let after = read(&root, "page.md");

    // The rendered row, line by line — the on-disk key set, not a struct's.
    let keys: Vec<&str> = after
        .lines()
        .skip_while(|l| !l.starts_with("  - object: "))
        .map(str::trim_start)
        .filter_map(|l| l.split_once(": ").map(|(k, _)| k))
        .map(|k| k.trim_start_matches("- "))
        .collect();
    assert_eq!(
        keys,
        vec![
            "object",
            "hash",
            "path",
            "fingerprint",
            // The free-form tail, sorted — engine-ignored, carried verbatim.
            "claim",
            "legacy-note",
        ],
        "the exact key set, in canonical order, extras last:\n{after}"
    );

    // ENVIRONMENT INDEPENDENCE, the U8 half. The same input bytes migrated in a
    // DIFFERENT vault — different path, different git object store, and the
    // target file absent entirely — produce a byte-identical lock block.
    let (_d2, root2) = {
        let block = v1_block(
            &[("target.md", "13c3550f41b5796dd05381fd2420451f3ef1aa40")],
            &[&format!(
                "  - ref: \"target.md#Scratch notes/Findings\"\n    \
                 fingerprint: \"{FP_A}\"\n    claim: owns the verdict\n    \
                 legacy-note: do not drop me\n"
            )],
        );
        // NOTE: no `target.md` in this vault at all.
        vault(&[("page.md", &format!("# Page\n\nbody\n\n{block}"))])
    };
    sweep(&root2, &wet()).expect("sweep elsewhere");
    let elsewhere = read(&root2, "page.md");
    let block_of = |s: &str| s[s.find("```meridian-lock").expect("block")..].to_string();
    assert_eq!(
        block_of(&after),
        block_of(&elsewhere),
        "the row is a function of the INPUT BYTES, never of the environment"
    );
}

// ── The quarantine, and the retirement it is measured against ──────────────

/// **THE QUARANTINE HOLDS.** The v1 claim-plane opener — `  - ref: `, the token
/// nothing but a v1 lock body contains — appears in engine Rust ONLY inside this
/// crate.
///
/// This is the assertion P4 rests on. `crates/lock` reads v2 and fails loud on
/// v1 precisely so that no reader can drift back into interpreting an old shape
/// as a new one; that guarantee is worth exactly as much as the claim that the
/// old shape is spelled in one deletable place. **After retirement this same
/// grep is zero everywhere**, which is the retirement's own gate
/// (`RETIREMENT.md`).
///
/// # Scope, stated rather than implied
/// **Production `src/` only, above the first `#[cfg(test)]`, comment lines
/// skipped.** A doc comment quoting the old shape teaches; a fixture minting one
/// is test DATA. Neither is a reader, and P4 is a claim about readers.
///
/// Measured while writing this gate, and reported to the Leader rather than
/// silently absorbed: **seven test suites still BUILD v1 lock fixtures** —
/// `run/tests/executor.rs`, `wire-serve/src/positions.rs` (in its `mod tests`),
/// `wire-serve/tests/s2fix_artifact_guard.rs`, and four under `mrd/tests/`.
/// Those pages are unreadable by `crates/lock` as it now stands. They are not
/// this unit's to fix — the card owns the door, the tool, the runbook and the
/// docs — but they are cutover work someone owns, and this comment is where the
/// measurement lives.
#[test]
fn the_v1_grammar_is_spelled_only_in_this_crate() {
    const V1_PIN_OPENER: &str = "  - ref: ";
    let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/");

    let mut leaks = Vec::new();
    let mut quarantine_hits = 0usize;
    let mut walk = vec![crates_dir.to_path_buf()];
    while let Some(dir) = walk.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let rel = path.strip_prefix(crates_dir).unwrap_or(&path);
            // Production source only: `<crate>/src/...`.
            let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy());
            let krate = parts.next().unwrap_or_default().to_string();
            if parts.next().as_deref() != Some("src") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let production = text
                .split_once("#[cfg(test)]")
                .map_or(text.as_str(), |(above, _)| above);
            let hit = production
                .lines()
                .any(|l| !l.trim_start().starts_with("//") && l.contains(V1_PIN_OPENER));
            if !hit {
                continue;
            }
            if krate == "lockmigrate" {
                quarantine_hits += 1;
            } else {
                leaks.push(rel.display().to_string());
            }
        }
    }

    // ACCEPTANCE first — without it, a scan that found nothing anywhere would
    // pass and prove nothing at all.
    assert_eq!(
        quarantine_hits, 1,
        "the quarantine itself must contain the v1 grammar, in exactly one file \
         (`src/v1.rs`); found {quarantine_hits}"
    );
    assert!(
        leaks.is_empty(),
        "the v1 lock grammar leaked OUT of the quarantine into: {leaks:?}"
    );
}

// ── The door is OFF THE WIRE, and that is gated, not assumed ───────────────

/// **No wire session — v2 or v3 — can reach the migration door**, because the
/// frozen op surface names no migration op. So there is no response frame to
/// leak a field onto.
///
/// # Why this gate and not a v2 key-set pin
/// The Leader's all-hands (#1, #2) is right that a NEW DOOR is the most exposed
/// surface for the v3-additive-field leak, and asked for either an explicit
/// version-keyed refusal or a v2 key-set pin. Measured, the premise does not
/// hold here: **this door has no response shape at all.**
///
/// - `wire::Op` has fourteen variants — `Hello` `Toc` `Cat` `Extract` `Resolve`
///   `Splice` `Create` `Root` `Diff` `Links` `Sub` `Read` `CheckWrite`
///   `ViewPath`. U9b added none, and `git diff` over `crates/wire/` is empty.
/// - **`lock_write`, shipped since U11, is in-process only too** — its callers
///   are tests and the pin prologue, never a dispatch arm. This door is its
///   sibling, reached the same way: a CLI verb calling a library function.
/// - `LockMigrateOutcome` is a plain struct. It is never a `ResponseBody` and
///   never crosses a socket.
///
/// A refusal keyed on the contract version would therefore be a refusal for a
/// request that cannot be spelled: the strict decoder already rejects an unknown
/// op. What CAN regress is someone later putting this door on the wire without
/// thinking about v2 — so that is what this pins, at the source, the same
/// technique the door census uses.
#[test]
fn no_wire_op_names_the_migration_door() {
    let wire_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("wire/src/lib.rs");
    let text = std::fs::read_to_string(&wire_src).expect("the wire contract is readable");

    // ACCEPTANCE first: the ops that ARE on the wire are found by this scan, so
    // an empty-handed scan cannot pass as proof of absence.
    for present in ["    Splice {", "    Create {", "    CheckWrite {"] {
        assert!(
            text.contains(present),
            "the scan cannot see the op surface it is reasoning about (`{present}`)"
        );
    }

    for forbidden in ["LockMigrate", "LockWrite", "lock_migrate", "lock_write"] {
        assert!(
            !text.contains(forbidden),
            "`{forbidden}` appeared in the frozen wire contract — the lock doors are \
             IN-PROCESS by design. Putting one on the wire mints a response shape, and \
             a response shape on a v2 session is exactly the v3-additive-field leak the \
             Leader flagged. Gate the projection and pin the v2 key set before doing it."
        );
    }
}

// ── Lock-is-content, asserted rather than assumed ──────────────────────────

/// The migration MOVES the page fingerprint, every time. The report carries
/// both revs so the drift is evidence rather than a claim.
#[test]
fn the_rewrite_moves_the_page_rev() {
    let (_d, root) = one_page_vault();
    let report = sweep(&root, &wet()).expect("sweep");
    let PageVerdict::Migrated {
        rev_before,
        rev_after,
        pins,
        ..
    } = &report.pages[0]
    else {
        panic!("expected a migration, got {:?}", report.pages[0]);
    };
    assert_ne!(rev_before, rev_after, "lock-is-content: the page moved");
    assert_eq!(*pins, 1);
}
