//! The `md.create` birth cap (create-task-page ruling, 2026-08-18 — option 1:
//! engine birth cap for declared tasks). Births go through the CREATE DOOR:
//! occupied-path refusal (`cas_mismatch`), workspace confinement, checks —
//! the door's own guards, never re-implemented in the run plane.
//!
//! Path resolution (md-create-ambient-paths shape (c) 2026-08-18, boundary-
//! as-data amendment 2026-08-19 #2): the descriptor's `path` is the RELATIVE
//! landing coordinate as declared; it composes under the descriptor's own
//! `base` when one rides it, under the request's `ambient` otherwise, and
//! stays workspace-root-relative under neither — the bare-door law every
//! `ambient: None` fixture here pins. A rooted spelling belongs in `base`
//! ONLY (the same-root/foreign table legs live in
//! `registry/tests/run_ambient.rs`, where the mount table is controlled); a
//! rooted `path` refuses with the base teaching. Capability grain
//! (caps-redesign ruling, 2026-08-19): `md.create` untargeted admits any
//! path; `md.create:<glob>` matches the DECLARED path — never the composed
//! landing — so every targeting lane presents one string to one glob.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError};

const PAGE: &str = "\
---
status: todo
---

# Tasks

## Log

- existing line
";

/// Empty run-birth fields (the CLI-entry shape; stamps are the wire arm's).
static EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    // Under target/, not $TMPDIR: macOS's /var→/private/var symlink makes the
    // door's cache canonicalization disagree with the fixture root (the same
    // host-env failure crates/check's e2e shows on mac at clean main; Linux
    // CI never sees it). A real path sidesteps the interplay.
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn current_root(root: &fs::WorkspaceRoot) -> MerkleRoot {
    fs::domain_snapshot(root).unwrap().1
}

fn create_effect(path: &str, body: &str, seq: u32) -> Effect {
    Effect {
        kind: EffectKind::Create,
        rule_id: "t".to_owned(),
        seq,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: [
            ("path".to_owned(), ArgValue::Str(path.to_owned())),
            ("body".to_owned(), ArgValue::Str(body.to_owned())),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    }
}

fn request<'a>(
    effects: &'a [Effect],
    authority: &'a Authority,
    observed: &'a MerkleRoot,
) -> ApplyRequest<'a> {
    ApplyRequest {
        page: "page.md",
        task: "t",
        task_rev: "cafecafecafecafe",
        invocation_id: "inv-1",
        now: None,
        effects,
        authority,
        observed_root: observed,
        receipt: None,
        exec: None,
        actor: None,
        depth: 0,
        delta: None,
        fields: &EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

fn granted(caps: &str) -> Authority {
    Authority::granted(CapSet::parse(caps).unwrap())
}

/// The happy path: a granted `md.create` births the file through the door,
/// the born bytes are exactly the body, and the count includes the birth.
#[test]
fn a_granted_birth_lands_through_the_create_door() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/new-card.md", "# A card\n\nbody\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let applied =
        executor::apply(&root, &request(&effects, &authority, &observed)).expect("the birth lands");
    assert_eq!(applied.applied, 1, "the birth counts as applied");
    let born = std::fs::read_to_string(tmp.path().join("tasks/new-card.md")).unwrap();
    assert!(
        born.contains("# A card"),
        "the born bytes carry the body: {born}"
    );
}

/// The occupied-path law: the same birth twice — the second refuses through
/// the door (`cas_mismatch`) and the file keeps its first bytes.
#[test]
fn an_occupied_path_refuses_the_second_birth() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/new-card.md", "# First\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    executor::apply(&root, &request(&effects, &authority, &observed)).expect("first birth lands");

    let again = [create_effect("tasks/new-card.md", "# Second\n", 0)];
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&again, &authority, &observed))
        .expect_err("an occupied path refuses");
    let ExecError::BirthRefused { path, detail, .. } = err else {
        panic!("expected BirthRefused, got {err:?}");
    };
    assert_eq!(path, "tasks/new-card.md");
    assert!(
        detail.contains("cas_mismatch"),
        "the door's own refusal code rides the detail: {detail}"
    );
    let kept = std::fs::read_to_string(tmp.path().join("tasks/new-card.md")).unwrap();
    assert!(kept.contains("# First"), "the first bytes stand: {kept}");
}

/// Deny-by-default: no `md.create` grant — the choke point refuses before
/// any I/O and nothing is born.
#[test]
fn an_ungranted_birth_is_cap_denied_before_io() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/new-card.md", "# A card\n", 0)];
    let authority = granted("md.edit");
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect_err("deny-by-default holds");
    assert!(
        matches!(err, ExecError::CapDenied { ref kind, .. } if kind == "md.create"),
        "the refusal names the kind: {err:?}"
    );
    assert!(
        !tmp.path().join("tasks/new-card.md").exists(),
        "nothing was born"
    );
}

/// The scoped grant: `md.create:tasks/*.md` admits a birth declared under
/// `tasks/` and refuses one declared under `notes/` — the capability grain
/// is the DECLARED path through the one glob grammar.
#[test]
fn a_scoped_grant_binds_the_declared_path() {
    let (tmp, root) = workspace();
    let authority = granted("md.create:tasks/*.md");

    let inside = [create_effect("tasks/in-scope.md", "# In\n", 0)];
    let observed = current_root(&root);
    executor::apply(&root, &request(&inside, &authority, &observed))
        .expect("a tasks/ birth is admitted");
    assert!(tmp.path().join("tasks/in-scope.md").exists());

    let outside = [create_effect("notes/out-of-scope.md", "# Out\n", 0)];
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&outside, &authority, &observed))
        .expect_err("a notes/ birth is refused");
    assert!(
        matches!(err, ExecError::CapDenied { ref target, .. } if target == "notes/out-of-scope.md"),
        "the refusal names the declared path: {err:?}"
    );
    assert!(!tmp.path().join("notes/out-of-scope.md").exists());
}

/// THE JAIL CASE (caps-redesign ruling): a declared path that buries the
/// granted board under an extra head segment must NOT match — `*` stays
/// inside a segment, and the glob reads the whole declared string.
#[test]
fn an_extra_head_segment_never_matches_the_board_glob() {
    let (tmp, root) = workspace();
    let authority = granted("md.create:tasks/*.md");
    let effects = [create_effect("evil/tasks/x.md", "# Evil\n", 0)];
    let observed = current_root(&root);
    let err = executor::apply(
        &root,
        &request_with_ambient(
            &effects,
            &authority,
            &observed,
            "year=2026/month=08/18-00-adhoc",
        ),
    )
    .expect_err("evil/tasks/x.md must not match tasks/*.md");
    assert!(
        matches!(err, ExecError::CapDenied { ref target, .. } if target == "evil/tasks/x.md"),
        "the refusal names the declared path: {err:?}"
    );
    assert!(!tmp.path().join("year=2026").exists(), "nothing was born");
}

/// A birth escaping the workspace refuses through the door's confinement.
#[test]
fn a_workspace_escaping_birth_refuses() {
    let (tmp, root) = workspace();
    let effects = [create_effect("../outside.md", "# Escape\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect_err("confinement holds");
    assert!(
        matches!(err, ExecError::BirthRefused { .. }),
        "the door refuses the escape: {err:?}"
    );
    assert!(!tmp.path().parent().unwrap().join("outside.md").exists());
}

/// A mixed generation: the birth realizes first, then the page batch lands —
/// one apply, both facts on disk.
#[test]
fn a_birth_and_a_page_edit_compose_in_one_generation() {
    let (tmp, root) = workspace();
    let effects = [
        create_effect("tasks/new-card.md", "# A card\n", 0),
        Effect {
            kind: EffectKind::SetField,
            rule_id: "t".to_owned(),
            seq: 1,
            depth: 0,
            provenance: Provenance::Run {
                invocation_id: "inv-1".to_owned(),
                root_at_eval: "b3:x".to_owned(),
            },
            args: [
                ("field".to_owned(), ArgValue::Str("status".to_owned())),
                ("value".to_owned(), ArgValue::Str("done".to_owned())),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        },
    ];
    let authority = granted("md.create, md.set_field");
    let observed = current_root(&root);
    let applied = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect("the generation lands");
    assert_eq!(applied.applied, 2, "both effects count");
    assert!(tmp.path().join("tasks/new-card.md").exists());
    let page = std::fs::read_to_string(tmp.path().join("page.md")).unwrap();
    assert!(
        page.contains("status: done"),
        "the page edit landed: {page}"
    );
}

/// A fixture request carrying the caller's ambient directory.
fn request_with_ambient<'a>(
    effects: &'a [Effect],
    authority: &'a Authority,
    observed: &'a MerkleRoot,
    ambient: &'a str,
) -> ApplyRequest<'a> {
    let mut req = request(effects, authority, observed);
    req.ambient = Some(ambient);
    req
}

/// The ambient lane (shape (c)): a bare birth path resolves under the
/// caller's ambient directory — the card lands on the CALLER's board, the
/// workspace root's stays empty, and `md.create:tasks` covers the resolved
/// partition wherever it lands.
#[test]
fn a_bare_birth_resolves_under_the_callers_ambient() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/card.md", "# A card\n", 0)];
    let authority = granted("md.create:tasks/*.md");
    let observed = current_root(&root);
    let ambient = "year=2026/month=08/18-00-adhoc";
    executor::apply(
        &root,
        &request_with_ambient(&effects, &authority, &observed, ambient),
    )
    .expect("the ambient birth lands");
    let born = tmp
        .path()
        .join("year=2026/month=08/18-00-adhoc/tasks/card.md");
    assert!(born.exists(), "the card lands on the caller's board");
    assert!(
        !tmp.path().join("tasks/card.md").exists(),
        "nothing lands at the workspace root — the original failure mode"
    );
}

/// Admission judges the DECLARED path — the
/// ambient never joins the matched string, so the refusal names exactly what
/// the block wrote, on whichever board it would have landed.
#[test]
fn the_capability_grain_is_judged_on_the_declared_path() {
    let (tmp, root) = workspace();
    let effects = [create_effect("notes/card.md", "# A card\n", 0)];
    let authority = granted("md.create:tasks/*.md");
    let observed = current_root(&root);
    let ambient = "year=2026/month=08/18-00-adhoc";
    let err = executor::apply(
        &root,
        &request_with_ambient(&effects, &authority, &observed, ambient),
    )
    .expect_err("a notes/ path refuses under md.create:tasks/*.md");
    assert!(
        matches!(err, ExecError::CapDenied { ref target, .. } if target == "notes/card.md"),
        "the refusal names the declared path: {err:?}"
    );
    assert!(!tmp.path().join(ambient).exists(), "nothing was born");
}

/// THREE-LANE EQUIVALENCE (the core guarantee of the redesign): one cap, one
/// declared path — the ambient lane, the base lane, and the bare lane all
/// ADMIT identically; only the landings differ. The rooted-base lane runs in
/// `registry/tests/run_ambient.rs`, where the mount table is controlled.
#[test]
fn one_declared_path_admits_identically_on_every_lane() {
    let authority = granted("md.create:tasks/*.md");

    // Ambient lane: lands on the caller board.
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/same.md", "# X\n", 0)];
    let observed = current_root(&root);
    executor::apply(
        &root,
        &request_with_ambient(
            &effects,
            &authority,
            &observed,
            "year=2026/month=08/18-00-adhoc",
        ),
    )
    .expect("the ambient lane admits");
    assert!(
        tmp.path()
            .join("year=2026/month=08/18-00-adhoc/tasks/same.md")
            .exists()
    );

    // Base lane (a confined directory base riding the descriptor): the base
    // OVERRIDES the ambient, and admission still reads only the declared path.
    let (tmp, root) = workspace();
    let mut based = create_effect("tasks/same.md", "# X\n", 0);
    based.args.insert(
        "base".to_owned(),
        ArgValue::Str("year=2026/month=08/19-01-elsewhere".to_owned()),
    );
    let effects = [based];
    let observed = current_root(&root);
    executor::apply(
        &root,
        &request_with_ambient(
            &effects,
            &authority,
            &observed,
            "year=2026/month=08/18-00-adhoc",
        ),
    )
    .expect("the base lane admits");
    assert!(
        tmp.path()
            .join("year=2026/month=08/19-01-elsewhere/tasks/same.md")
            .exists(),
        "the descriptor base wins over the ambient"
    );

    // Bare lane: no base, no ambient — the root board.
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/same.md", "# X\n", 0)];
    let observed = current_root(&root);
    executor::apply(&root, &request(&effects, &authority, &observed))
        .expect("the bare lane admits");
    assert!(tmp.path().join("tasks/same.md").exists());
}

/// A rooted spelling in the PATH refuses with the base teaching: the path is
/// the relative landing coordinate the cap glob judges, and targeting is the
/// base axis — two facts, two arguments, never one glued string.
#[test]
fn a_rooted_path_spelling_refuses_toward_the_base_argument() {
    let (_tmp, root) = workspace();
    let effects = [create_effect("sessions:elsewhere/tasks/x.md", "# X\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect_err("a rooted path spelling refuses");
    let ExecError::BirthRefused { detail, .. } = err else {
        panic!("expected BirthRefused, got {err:?}");
    };
    assert!(
        detail.contains("base"),
        "the refusal teaches the base argument: {detail}"
    );
}

/// A bare path escaping THROUGH the ambient join refuses at the resolution
/// seam — before the capability grain and before any I/O.
#[test]
fn an_ambient_escaping_join_refuses() {
    let (tmp, root) = workspace();
    let effects = [create_effect("../../escape.md", "# Out\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let err = executor::apply(
        &root,
        &request_with_ambient(
            &effects,
            &authority,
            &observed,
            "year=2026/month=08/18-00-adhoc",
        ),
    )
    .expect_err("the joined path is unconfined");
    assert!(
        matches!(err, ExecError::BirthRefused { .. }),
        "the resolution seam refuses the escape: {err:?}"
    );
    assert!(!tmp.path().parent().unwrap().join("escape.md").exists());
}

/// A malformed ambient refuses every birth it would have resolved — a host
/// defect surfaces loud, never as a silent root-relative landing.
#[test]
fn a_malformed_ambient_refuses_the_birth() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/card.md", "# A card\n", 0)];
    let authority = granted("md.create:tasks/*.md");
    let observed = current_root(&root);
    let err = executor::apply(
        &root,
        &request_with_ambient(&effects, &authority, &observed, "../outside"),
    )
    .expect_err("an unconfined ambient refuses");
    assert!(
        matches!(err, ExecError::BirthRefused { ref detail, .. }
            if detail.contains("ambient")),
        "the refusal names the ambient fault: {err:?}"
    );
    assert!(
        !tmp.path().join("tasks/card.md").exists(),
        "nothing was born"
    );
}

// ── The machinery floor (card create-door-machinery-containment, 2026-08-20) ──
// A create scope judges the DECLARED path's SHAPE, never the landing, so the
// grant `md.create:tasks/*.md` reached `.git/tasks/x.md` through the
// descriptor's own `base` — probed in the round-2 review of caps-redesign-docs.
// The door now refuses a landing carrying `.git`, `.meridian`, `meridian` or
// `receipts` as a segment, at any depth, case-insensitively. These tests pin
// the floor at the shared choke both run-plane lanes converge on; the per-lane
// end-to-end proofs live in `dispatch_bash.rs` and `dispatch_starlark.rs`.

/// The measured escape, closed: one grant, four machinery landings, reached
/// through the BASE axis the capability never reads. Each refuses and lands
/// nothing.
#[test]
fn a_based_birth_into_each_machinery_dir_refuses() {
    for dir in [".git", ".meridian", "meridian", "receipts"] {
        let (tmp, root) = workspace();
        let mut based = create_effect("tasks/card.md", "# Escaped\n", 0);
        based
            .args
            .insert("base".to_owned(), ArgValue::Str(dir.to_owned()));
        let effects = [based];
        // The narrow grant that MATCHES — the point is that admission passes
        // and the door still refuses.
        let authority = granted("md.create:tasks/*.md");
        let observed = current_root(&root);
        let err = executor::apply(&root, &request(&effects, &authority, &observed))
            .expect_err("a machinery landing refuses at the door");
        let ExecError::BirthRefused { detail, .. } = &err else {
            panic!("expected BirthRefused for base `{dir}`, got {err:?}");
        };
        assert!(
            detail.contains("bad_path"),
            "the machinery floor refuses `bad_path` for base `{dir}`: {detail}"
        );
        assert!(
            detail.contains(dir),
            "the refusal names the offending segment `{dir}`: {detail}"
        );
        assert!(
            !tmp.path().join(dir).join("tasks/card.md").exists(),
            "nothing was born under `{dir}`"
        );
    }
}

/// The same floor on the DECLARED path, with no base and an untargeted grant —
/// the lane that needs no `base` argument at all.
#[test]
fn a_declared_machinery_path_refuses_without_any_base() {
    for path in [
        ".git/tasks/card.md",
        ".meridian/runs/card.md",
        "meridian/armed-rules.md",
        "receipts/run.md",
    ] {
        let (tmp, root) = workspace();
        let effects = [create_effect(path, "# Escaped\n", 0)];
        let authority = granted("md.create");
        let observed = current_root(&root);
        let err = executor::apply(&root, &request(&effects, &authority, &observed))
            .expect_err("a machinery landing refuses at the door");
        assert!(
            matches!(err, ExecError::BirthRefused { ref detail, .. }
                if detail.contains("bad_path")),
            "`{path}` refuses at the machinery floor: {err:?}"
        );
        assert!(!tmp.path().join(path).exists(), "`{path}` was not born");
    }
}

/// **At any depth.** A nested root's machinery is machinery too: a birth into
/// a probe workspace's own `.git/` corrupts a repository exactly as a
/// root-level one does, and the live corpus carries 150+ such nested dirs.
#[test]
fn a_nested_machinery_dir_refuses_at_any_depth() {
    for path in [
        "results/probe/ws/.git/tasks/card.md",
        "results/probe/ws/.meridian/card.md",
        "results/probe/ws/meridian/armed-rules.md",
        "results/probe/ws/receipts/2026-08-20.md",
    ] {
        let (tmp, root) = workspace();
        let effects = [create_effect(path, "# Nested\n", 0)];
        let authority = granted("md.create");
        let observed = current_root(&root);
        let err = executor::apply(&root, &request(&effects, &authority, &observed))
            .expect_err("a nested machinery landing refuses");
        assert!(
            matches!(err, ExecError::BirthRefused { ref detail, .. }
                if detail.contains("bad_path")),
            "`{path}` refuses at any depth: {err:?}"
        );
        assert!(!tmp.path().join(path).exists(), "`{path}` was not born");
    }
}

/// **Case-insensitively.** A case-insensitive filesystem lands `.GIT/x.md`
/// inside `.git/`, so a guard a spelling defeats is not a guard.
#[test]
fn a_case_variant_machinery_dir_still_refuses() {
    for path in [".GIT/tasks/card.md", "Receipts/run.md", "MERIDIAN/x.md"] {
        let (tmp, root) = workspace();
        let effects = [create_effect(path, "# Case\n", 0)];
        let authority = granted("md.create");
        let observed = current_root(&root);
        let err = executor::apply(&root, &request(&effects, &authority, &observed))
            .expect_err("a case-variant machinery landing refuses");
        assert!(
            matches!(err, ExecError::BirthRefused { ref detail, .. }
                if detail.contains("bad_path")),
            "`{path}` refuses whatever its case: {err:?}"
        );
        assert!(!tmp.path().join(path).exists(), "`{path}` was not born");
    }
}

/// THE CARVE-OUT: `meridian/domain.md` is the hash-domain config — authored
/// content declaring the ignore list, deliberately inside its own hash domain,
/// and born through this door by the resident write path. The floor must let
/// it through at any depth while its SIBLINGS in the same directory still
/// refuse. Measured, not reasoned: the floor's first CI run refused it and
/// took down three wire-serve door tests.
#[test]
fn the_domain_config_is_the_one_machinery_carve_out() {
    for path in ["meridian/domain.md", "results/probe/ws/meridian/domain.md"] {
        let (tmp, root) = workspace();
        // The body shape the engine's own door test births (write.rs
        // `domain_config_write_overlays_membership`) — proven to parse.
        let effects = [create_effect(
            path,
            "---\nignore:\n  - \"drafts/**\"\n---\n# Domain\n",
            0,
        )];
        let authority = granted("md.create");
        let observed = current_root(&root);
        executor::apply(&root, &request(&effects, &authority, &observed))
            .unwrap_or_else(|e| panic!("the domain config must birth at `{path}`: {e:?}"));
        assert!(tmp.path().join(path).is_file(), "`{path}` was born");
    }

    // The carve-out is exactly one page, not the directory around it.
    let (tmp, root) = workspace();
    let effects = [create_effect("meridian/armed-rules.md", "# Forged\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect_err("the attestation artifact is not carved out");
    assert!(
        matches!(err, ExecError::BirthRefused { ref detail, .. }
            if detail.contains("bad_path")),
        "a sibling of the domain config still refuses: {err:?}"
    );
    assert!(!tmp.path().join("meridian/armed-rules.md").exists());
}

/// The floor is a FLOOR, not a ban on the words: an ordinary content landing
/// whose segments merely RESEMBLE machinery still births. Without this, the
/// guard could pass by refusing everything.
#[test]
fn look_alike_content_paths_still_birth() {
    for path in [
        "tasks/receipts.md",
        "receipts-archive/card.md",
        "notes/meridian-notes.md",
        "docs/gitignore.md",
    ] {
        let (tmp, root) = workspace();
        let effects = [create_effect(path, "# Content\n", 0)];
        let authority = granted("md.create");
        let observed = current_root(&root);
        executor::apply(&root, &request(&effects, &authority, &observed))
            .unwrap_or_else(|e| panic!("`{path}` is content and must birth: {e:?}"));
        assert!(tmp.path().join(path).is_file(), "`{path}` was born");
    }
}

/// Rooted-looking spellings in the PATH refuse BEFORE the mount table is
/// ever read — deterministic on any machine: the path argument admits no
/// `root:` head at all (the base axis owns targeting).
#[test]
fn rooted_grammar_faults_refuse_before_the_table() {
    let (_tmp, root) = workspace();
    let authority = granted("md.create");
    for bad in ["a:b:c.md", "sessions:"] {
        let effects = [create_effect(bad, "# X\n", 0)];
        let observed = current_root(&root);
        let err = executor::apply(&root, &request(&effects, &authority, &observed))
            .expect_err("a malformed rooted spelling refuses");
        assert!(
            matches!(err, ExecError::BirthRefused { .. }),
            "`{bad}` refuses through the resolution seam: {err:?}"
        );
    }
}
