//! The env-var bridge's gates.
//!
//! Every gate asserts both arms: agreement beside divergence, silence beside
//! the report. `the_measured_topology` reproduces the real shape — a wiki
//! directory, a symlink to it from a sibling repos root, the wiki path spelled
//! with a trailing slash — so the gates assert against real `canonicalize`
//! calls rather than a stub.

use std::path::{Path, PathBuf};

use super::*;
use crate::mount::{MountError, MountReason, MountState, bind};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn config_of(blocks: &[String]) -> String {
    format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# My system\n\n{}",
        blocks.join("\n")
    )
}

fn mount_block(name: &str, path: &Path, kind: &str, vault: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut block = format!(
        "```meridian-mount\nname: {name}\npath: {}\nkind: {kind}\n",
        path.display()
    );
    if let Some(vault) = vault {
        let _ = writeln!(block, "vault: {vault}");
    }
    block.push_str("```\n");
    block
}

fn vault_block(name: &str, path: &Path) -> String {
    mount_block(name, path, "vault", Some(name))
}

fn folder_block(name: &str, path: &Path) -> String {
    mount_block(name, path, "git-folder", None)
}

/// Write a root's self-declaration so the mount reaches [`MountState::Bound`] —
/// the acceptance half is only meaningful over genuinely bound roots.
fn declare(dir: &Path, name: &str) {
    std::fs::write(
        dir.join(crate::mount::DECLARATION_FILENAME),
        format!(
            "---\ntype: {}\nversion: 1\nname: {name}\n---\n\n# {name}\n",
            crate::mount::DECLARATION_TYPE
        ),
    )
    .expect("write the root declaration");
}

fn table_of(raw: &str) -> MountTable {
    bind(&crate::parse(raw, Path::new("~/MERIDIAN.md")).expect("the fixture must parse"))
        .expect("the fixture must bind")
}

fn refuse(raw: &str) -> MountError {
    bind(&crate::parse(raw, Path::new("~/MERIDIAN.md")).expect("the fixture must parse"))
        .expect_err("this config must refuse to bind")
}

/// The measured topology, reproduced.
struct Topology {
    /// Kept alive for the fixture's lifetime — dropping it removes the tree —
    /// and read directly for paths outside the two roots.
    dir: tempfile::TempDir,
    /// The wiki's real directory — `/Users/Shared/projects/field-notes`.
    real_wiki: PathBuf,
    /// The repos root — `/Users/Shared/repos`.
    repos_root: PathBuf,
    /// The symlink into the repos root — `/Users/Shared/repos/field-notes`.
    linked_wiki: PathBuf,
    /// The wiki path as the env var spells it — real path, trailing slash.
    slashed_wiki: String,
}

fn the_measured_topology() -> Topology {
    let keep = tempfile::tempdir().expect("tempdir");
    let projects = keep.path().join("projects");
    let repos_root = keep.path().join("repos");
    let real_wiki = projects.join("field-notes");
    std::fs::create_dir_all(&real_wiki).expect("create the real wiki");
    std::fs::create_dir_all(&repos_root).expect("create the repos root");
    let linked_wiki = repos_root.join("field-notes");
    std::os::unix::fs::symlink(&real_wiki, &linked_wiki).expect("symlink the wiki into repos");
    declare(&real_wiki, "field-notes");
    declare(&repos_root, "repos");

    // Asserted, never assumed: the fixture must reproduce the measured
    // topology.
    let canonical_real = std::fs::canonicalize(&real_wiki).expect("canonicalize the real wiki");
    assert_eq!(
        std::fs::canonicalize(&linked_wiki).expect("canonicalize the symlink"),
        canonical_real,
        "the symlink and the real path must be one tree"
    );
    assert_ne!(
        linked_wiki, real_wiki,
        "the two spellings must be textually different, or the gate is vacuous"
    );

    // The sandbox itself must pass the deny ceiling, or every acceptance is
    // vacuous.
    assert_eq!(
        workspace::deny_reason(&canonical_real),
        None,
        "the sandbox must pass the ceiling"
    );

    let slashed_wiki = format!("{}/", real_wiki.display());
    Topology {
        dir: keep,
        real_wiki,
        repos_root,
        linked_wiki,
        slashed_wiki,
    }
}

fn latch() -> ReportLatch {
    ReportLatch::new()
}

// ---------------------------------------------------------------------------
// Gate 1 — the measured case binds ONE tree once, through both spellings
// ---------------------------------------------------------------------------

/// The measured `field-notes` case must not produce two mounts: three textually
/// different spellings of one tree, and the table holds exactly one canonical
/// mount for it.
#[test]
fn the_measured_case_yields_one_canonical_mount_not_two() {
    let t = the_measured_topology();
    let table = table_of(&config_of(&[
        vault_block("field-notes", &t.linked_wiki),
        folder_block("repos", &t.repos_root),
    ]));

    let canonical_wiki = std::fs::canonicalize(&t.real_wiki).expect("canonicalize");
    let holding_the_wiki: Vec<&str> = table
        .mounts()
        .iter()
        .filter(|m| m.canonical_path() == Some(canonical_wiki.as_path()))
        .map(crate::mount::Mount::name)
        .collect();
    assert_eq!(
        holding_the_wiki,
        vec!["field-notes"],
        "exactly ONE mount may hold the wiki tree — two would be two canonical refs over identical bytes with identical sec_rev, and a receipt minted on one would gate a pin on the other"
    );

    // ...and every spelling of that one tree reaches that one mount. On macOS
    // a tempdir lives under `/var`, itself a symlink to `/private/var`, so
    // every path in this topology — including the "real" one — requires
    // canonicalization here.
    for spelling in [
        t.slashed_wiki.clone(),
        t.real_wiki.display().to_string(),
        t.linked_wiki.display().to_string(),
    ] {
        let env = BridgeEnv {
            wiki_path: Some(spelling.clone()),
            repos_root: Some(t.repos_root.display().to_string()),
        };
        let checked = check_in(&env, &table, &latch());
        assert_eq!(
            checked[0].mount(),
            Some("field-notes"),
            "the spelling {spelling} names the one wiki mount"
        );
        assert_eq!(
            checked[0].canonical(),
            Some(canonical_wiki.as_path()),
            "and resolves onto the one canonical path"
        );
        assert_eq!(checked[0].report(), None, "agreement is silent");
    }
}

/// A literal inversion — each env var taken as its own mount block — is the
/// read-mint bypass, and the mount law refuses it: the same two spellings
/// that agree through one mount refuse when declared as two.
#[test]
fn the_literal_inversion_is_the_bypass_and_the_mount_law_refuses_it() {
    let t = the_measured_topology();
    let err = refuse(&config_of(&[
        vault_block("field-notes", &t.linked_wiki),
        mount_block(
            "wiki-env",
            Path::new(&t.slashed_wiki),
            "vault",
            Some("wiki-env"),
        ),
    ]));
    assert_eq!(
        err.reason,
        MountReason::DuplicateMountPath,
        "the symlink spelling and the trailing-slash real path are one tree"
    );
    assert!(
        err.detail.contains("`wiki-env`") && err.detail.contains("`field-notes`"),
        "both spellings are named: {}",
        err.detail
    );
}

/// The other direction of the same trap: the repos root appears to contain
/// the wiki, so a string-prefix reading would call the two mounts nested.
/// Canonicalized they are siblings, and the nesting dissolves.
#[test]
fn canonicalization_dissolves_the_apparent_nesting_of_the_repos_root() {
    let t = the_measured_topology();

    // The declared spellings really are nested — otherwise this gate is vacuous.
    assert!(
        t.linked_wiki.starts_with(&t.repos_root),
        "precondition: the declared wiki path must lie inside the declared repos root"
    );

    let table = table_of(&config_of(&[
        vault_block("field-notes", &t.linked_wiki),
        folder_block("repos", &t.repos_root),
    ]));
    assert!(
        table.is_clear(),
        "both roots bind — canonically they are siblings: {:?}",
        table.mounts()
    );

    let wiki = table.by_name("field-notes").expect("the wiki binds");
    let repos = table.by_name("repos").expect("the repos root binds");
    let (wiki_canonical, repos_canonical) = (
        wiki.canonical_path().expect("wiki canonical"),
        repos.canonical_path().expect("repos canonical"),
    );
    assert!(
        !wiki_canonical.starts_with(repos_canonical),
        "canonically the wiki ({}) is NOT under the repos root ({})",
        wiki_canonical.display(),
        repos_canonical.display()
    );
}

// ---------------------------------------------------------------------------
// Gate 2 — BOTH arms: agreement is silent, divergence reports ONCE
// ---------------------------------------------------------------------------

/// The acceptance half: agreement binds silently — no report, on either
/// variable, through every measured spelling.
#[test]
fn agreement_binds_silently_and_names_where_it_resolved_from() {
    let t = the_measured_topology();
    let table = table_of(&config_of(&[
        vault_block("field-notes", &t.linked_wiki),
        folder_block("repos", &t.repos_root),
    ]));
    let env = BridgeEnv {
        wiki_path: Some(t.slashed_wiki.clone()),
        repos_root: Some(t.repos_root.display().to_string()),
    };

    let checked = check_in(&env, &table, &latch());
    assert_eq!(checked.len(), 2);

    for b in &checked {
        assert_eq!(b.state().word(), "agrees", "{} must agree", b.var().name());
        assert_eq!(
            b.report(),
            None,
            "agreement is SILENT — a per-agreement note is the behaviour that gets the variable unset"
        );
    }

    // The state change, named: which mount now exists for each variable, and
    // the canonical path it resolved from.
    assert_eq!(checked[0].var(), BridgeVar::WikiPath);
    assert_eq!(checked[0].mount(), Some("field-notes"));
    assert_eq!(
        checked[0].canonical(),
        Some(
            std::fs::canonicalize(&t.real_wiki)
                .expect("canonicalize")
                .as_path()
        ),
        "the wiki resolved from the REAL path, not the symlink spelling the file declares"
    );
    assert_eq!(checked[1].var(), BridgeVar::ReposRoot);
    assert_eq!(checked[1].mount(), Some("repos"));
    assert_eq!(
        checked[1].canonical(),
        Some(
            std::fs::canonicalize(&t.repos_root)
                .expect("canonicalize")
                .as_path()
        )
    );

    // And the roots really are bound, not merely present.
    assert_eq!(
        table.by_name("field-notes").expect("bound").state(),
        &MountState::Bound
    );
}

/// The refusal half: the variable states a tree the file does not bind. The
/// file wins — the variable names no root — and the divergence is reported
/// once, not once per call.
#[test]
fn divergence_reports_once_per_process_and_the_file_wins() {
    let t = the_measured_topology();
    let elsewhere = t.dir.path().join("not-declared");
    std::fs::create_dir_all(&elsewhere).expect("create the undeclared tree");

    let table = table_of(&config_of(&[vault_block("field-notes", &t.linked_wiki)]));
    let env = BridgeEnv {
        wiki_path: Some(elsewhere.display().to_string()),
        repos_root: None,
    };
    let latch = latch();

    let first = check_in(&env, &table, &latch);
    assert_eq!(first[0].state().word(), "diverges");
    assert_eq!(
        first[0].mount(),
        None,
        "THE FILE WINS: a variable naming a tree the file does not bind names no root"
    );
    let report = first[0].report().expect("the FIRST divergence reports");
    assert!(
        report.starts_with("note:"),
        "a divergence is a NOTE, never a refusal — fail-loud here would brick the CLI on every machine that exports the var: {report}"
    );
    assert!(
        report.contains(WIKI_PATH_ENV_VAR) && report.contains(&elsewhere.display().to_string()),
        "the report names the variable and the path it states: {report}"
    );

    // A per-call warning is a different behaviour — the one that gets the
    // variable unset.
    for call in 2..=5 {
        let again = check_in(&env, &table, &latch);
        assert_eq!(again[0].state().word(), "diverges", "call {call}");
        assert_eq!(
            again[0].report(),
            None,
            "call {call} must NOT report again — once per process, not once per call"
        );
    }
}

/// The latch is per variable, not once for the pair: two variables carrying
/// two different divergences each get their own report.
#[test]
fn each_variable_carries_its_own_once() {
    let t = the_measured_topology();
    let table = table_of(&config_of(&[vault_block("field-notes", &t.linked_wiki)]));
    let env = BridgeEnv {
        wiki_path: Some(t.dir.path().join("nowhere-a").display().to_string()),
        repos_root: Some(t.dir.path().join("nowhere-b").display().to_string()),
    };
    let latch = latch();

    let first = check_in(&env, &table, &latch);
    assert!(
        first[0].report().is_some() && first[1].report().is_some(),
        "both variables report their own first divergence"
    );
    assert_ne!(
        first[0].report(),
        first[1].report(),
        "the two reports are different facts"
    );

    let second = check_in(&env, &table, &latch);
    assert!(
        second[0].report().is_none() && second[1].report().is_none(),
        "and neither reports twice"
    );
}

/// The process-global latch is genuinely process-wide. Order-independent:
/// whatever ran before, a second call through the public entry point must not
/// report; the first-call arm is asserted above on an owned latch.
#[test]
fn the_public_entry_point_latches_process_wide() {
    let t = the_measured_topology();
    let table = table_of(&config_of(&[vault_block("field-notes", &t.linked_wiki)]));
    let env = BridgeEnv {
        wiki_path: Some(t.dir.path().join("absent-root").display().to_string()),
        repos_root: None,
    };

    let first = check(&env, &table);
    assert_eq!(first[0].state().word(), "diverges");
    let second = check(&env, &table);
    assert_eq!(
        second[0].report(),
        None,
        "the process-global latch does not re-arm between calls"
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — the arms that must stay SILENT, or the instrument is deleted
// ---------------------------------------------------------------------------

/// A machine in state A has an empty table with both variables exported; if
/// that were a divergence the check would fire everywhere. State A (absent)
/// and state D (zero mounts) reach the same arm.
#[test]
fn an_empty_table_is_unchecked_in_both_of_its_shapes() {
    let env = BridgeEnv {
        wiki_path: Some("/Users/Shared/projects/field-notes/".to_string()),
        repos_root: Some("/Users/Shared/repos".to_string()),
    };

    // State A — no file at all.
    let absent = crate::Resolution::Absent {
        path: PathBuf::from("/nowhere/MERIDIAN.md"),
    }
    .bind()
    .expect("the absent table binds empty");

    // State D — a clean parse declaring zero mounts.
    let empty = table_of(&config_of(&[]));

    for (shape, table) in [("state A", &absent), ("state D", &empty)] {
        let checked = check_in(&env, table, &latch());
        for b in &checked {
            assert_eq!(
                b.state().word(),
                "unchecked",
                "{shape}: {} has nothing to be checked against",
                b.var().name()
            );
            assert_eq!(b.report(), None, "{shape} must be SILENT");
            assert_eq!(b.mount(), None);
        }
    }
}

/// Unset, empty and whitespace-only all state no path — the same nil-vs-empty
/// rule `MERIDIAN_CONFIG` follows.
#[test]
fn unset_empty_and_whitespace_state_no_path() {
    let t = the_measured_topology();
    let table = table_of(&config_of(&[vault_block("field-notes", &t.linked_wiki)]));

    for stated in [None, Some(String::new()), Some("   ".to_string())] {
        let env = BridgeEnv {
            wiki_path: stated.clone(),
            repos_root: None,
        };
        let checked = check_in(&env, &table, &latch());
        assert_eq!(
            checked[0].state().word(),
            "unset",
            "{stated:?} states no path"
        );
        assert_eq!(checked[0].report(), None);
    }
}

/// A variable naming a path that does not exist diverges too — and the detail
/// says which failure it is, because the two have different fixes.
#[test]
fn an_unresolvable_path_diverges_with_its_own_detail() {
    let t = the_measured_topology();
    let table = table_of(&config_of(&[vault_block("field-notes", &t.linked_wiki)]));
    let env = BridgeEnv {
        wiki_path: Some(t.dir.path().join("no-such-dir").display().to_string()),
        repos_root: None,
    };

    let checked = check_in(&env, &table, &latch());
    let BridgeState::Diverges { detail, .. } = checked[0].state() else {
        panic!("an unresolvable path diverges: {:?}", checked[0].state());
    };
    assert!(
        detail.contains("does not resolve on this machine"),
        "the detail distinguishes an unresolvable path from an unbound one: {detail}"
    );

    // Beside it: a path that resolves but is unbound carries the other detail.
    let resolvable = t.dir.path().join("resolvable");
    std::fs::create_dir_all(&resolvable).expect("create");
    let env = BridgeEnv {
        wiki_path: Some(resolvable.display().to_string()),
        repos_root: None,
    };
    let checked = check_in(&env, &table, &latch());
    let BridgeState::Diverges { detail, .. } = checked[0].state() else {
        panic!("an unbound path diverges");
    };
    assert!(
        detail.contains("resolves here but is not a root the file binds"),
        "{detail}"
    );
}

/// The vocabulary is closed and every word is distinct — one spelling per state,
/// used in the human line and in `--json` alike.
#[test]
fn the_state_words_are_distinct() {
    let words = [
        BridgeState::Unset.word(),
        BridgeState::Unchecked.word(),
        BridgeState::Agrees {
            mount: String::new(),
            canonical: PathBuf::new(),
        }
        .word(),
        BridgeState::Diverges {
            stated: String::new(),
            detail: String::new(),
        }
        .word(),
    ];
    let mut sorted = words.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), words.len(), "no two states share a word");
    assert_eq!(
        BridgeVar::ALL.map(BridgeVar::name),
        [WIKI_PATH_ENV_VAR, REPOS_ROOT_ENV_VAR]
    );
}
