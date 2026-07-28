//! On-disk gates for the §12 hash domain: addressability ⊋ hash domain
//! (gate 2) and `mdfs_config.yaml` structurally outside the domain (gate 3).

use std::path::{Path, PathBuf};

use fs::domain::Domain;
use fs::{WorkspaceRoot, hash_domain, walk};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// Gate 2 + gate 3: a real corpus containing an ignored dot-dir md file and the
/// non-md config file. The dot-dir md is walked (addressable) yet never hashed;
/// its bytes are readable by path; the config file is in neither set.
#[test]
fn addressable_is_superset_of_hash_domain() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "receipts/2026-07-18.md", "# Receipts\n");
    write(root_path, ".github/README.md", "# CI notes\n");
    write(root_path, "mdfs_config.yaml", "version: 0\nignore: []\n");

    let root = WorkspaceRoot(root_path.to_path_buf());
    let addressable = walk(&root).unwrap();
    let hashed = hash_domain(&root, &Domain::new()).unwrap();

    let gh = PathBuf::from(".github/README.md");
    // gate 2: the ignored dot-dir md file is addressable (present in walk) ...
    assert!(
        addressable.contains(&gh),
        "ignored md stays addressable via walk"
    );
    // ... but never enters the hash domain — its bytes never reach the root ...
    assert!(!hashed.contains(&gh), "ignored md is not hashed");
    // ... and its bytes are directly readable by path (the load path is
    // unfiltered — toc/cat/splice ride on this).
    assert_eq!(
        std::fs::read_to_string(root_path.join(&gh)).unwrap(),
        "# CI notes\n",
    );

    // gate 3: mdfs_config.yaml is non-md — in neither walk nor hash domain,
    // though it sits on disk and was just read to build the domain.
    let cfg = PathBuf::from("mdfs_config.yaml");
    assert!(!addressable.contains(&cfg), "config is not markdown");
    assert!(!hashed.contains(&cfg));
    assert!(Domain::load(&root).is_ok(), "config is still readable");

    // hash ⊂ addressable, strictly (the dot-dir file separates them).
    for h in &hashed {
        assert!(addressable.contains(h), "hash domain ⊆ addressable");
    }
    assert!(hashed.len() < addressable.len(), "strict subset");

    let mut hashed_names: Vec<String> = hashed
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    hashed_names.sort();
    assert_eq!(
        hashed_names,
        vec![
            "notes/plan.md".to_string(),
            "receipts/2026-07-18.md".to_string(),
        ],
    );
}

/// A custom `drafts/**` ignore (§12.3 v1) removes a walked md file from the
/// hash domain while leaving it walked/addressable.
#[test]
fn custom_ignore_removes_from_hash_domain_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "drafts/tmp.md", "scratch\n");
    write(
        root_path,
        "mdfs_config.yaml",
        "version: 1\nignore:\n  - \"drafts/**\"\n",
    );

    let root = WorkspaceRoot(root_path.to_path_buf());
    let domain = Domain::load(&root).unwrap();
    assert_eq!(domain.version(), 1);

    let addressable = walk(&root).unwrap();
    let hashed = hash_domain(&root, &domain).unwrap();

    let draft = PathBuf::from("drafts/tmp.md");
    assert!(addressable.contains(&draft), "draft is addressable");
    assert!(!hashed.contains(&draft), "draft excluded from hash domain");
    assert!(hashed.contains(&PathBuf::from("notes/plan.md")));
}

/// The pruning gate. `hash_domain` no longer walks everything and filters —
/// it declines to descend into soundly-ignored directories. That is only
/// legitimate if it computes the SAME SET, so this asserts the equivalence
/// directly against the definition it replaced: `walk().filter(contains)`.
///
/// A wrong answer here is a wrong merkle root, not a slow walk.
#[test]
fn pruning_computes_the_same_set_as_filtering() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "notes/deep/nested/note.md", "# Deep\n");
    write(root_path, "assets/big/a.md", "asset\n");
    write(root_path, "assets/big/deeper/b.md", "asset\n");
    write(root_path, "drafts/tmp.md", "scratch\n");
    write(root_path, ".obsidian/workspace.md", "state\n");
    write(root_path, "archive/old.md", "old\n");
    write(root_path, "archive/index.md", "kept\n");
    write(
        root_path,
        "mdfs_config.yaml",
        // A negation deliberately sits under an ignored directory.
        "version: 1\nignore:\n  - \"assets/**\"\n  - \"drafts/**\"\n  \
         - \"archive/**\"\n  - \"!archive/index.md\"\n",
    );

    let root = WorkspaceRoot(root_path.to_path_buf());
    let domain = Domain::load(&root).unwrap();

    // The definition the pruned walk must reproduce, verbatim.
    let expected: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = walk(&root)
            .unwrap()
            .into_iter()
            .filter(|rel| domain.contains(rel))
            .collect();
        v.sort();
        v
    };
    let hashed = hash_domain(&root, &domain).unwrap();

    assert_eq!(
        hashed, expected,
        "pruned traversal must equal walk().filter(contains)"
    );
}

/// Negation safety — the property that makes pruning non-trivial. `archive/**`
/// ignores the directory, but `!archive/index.md` re-includes one file beneath
/// it. A traversal that pruned `archive/` on the strength of the first rule
/// would silently drop a hashed file and change the root.
#[test]
fn negation_beneath_an_ignored_dir_survives_pruning() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "archive/old.md", "old\n");
    write(root_path, "archive/index.md", "kept\n");
    write(
        root_path,
        "mdfs_config.yaml",
        "version: 1\nignore:\n  - \"archive/**\"\n  - \"!archive/index.md\"\n",
    );

    let root = WorkspaceRoot(root_path.to_path_buf());
    let domain = Domain::load(&root).unwrap();

    assert!(
        !domain.prunes_dir(Path::new("archive")),
        "a `!` rule reaching beneath the directory must forbid pruning it"
    );

    let hashed = hash_domain(&root, &domain).unwrap();
    assert!(
        hashed.contains(&PathBuf::from("archive/index.md")),
        "the re-included file must still be hashed"
    );
    assert!(
        !hashed.contains(&PathBuf::from("archive/old.md")),
        "its ignored sibling must not be"
    );
}

/// Without any `!` rule the same directory IS prunable — the fast path the
/// whole change exists for.
#[test]
fn plain_ignored_dir_is_prunable() {
    let domain = Domain::from_config("version: 1\nignore:\n  - \"assets/**\"\n");
    assert!(domain.prunes_dir(Path::new("assets")));
    assert!(domain.prunes_dir(Path::new("assets/nested")));
    assert!(!domain.prunes_dir(Path::new("notes")));
    // Never the workspace root itself.
    assert!(!domain.prunes_dir(Path::new("")));
}

/// The markdown config surface: the ignore list rides the frontmatter, and the
/// body is prose the parser must not read as configuration.
#[test]
fn markdown_config_declares_the_domain() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "assets/big.md", "asset\n");
    write(
        root_path,
        "meridian/domain.md",
        "---\nversion: 2\nignore:\n  - \"assets/**\"\n---\n\n\
         # Domain\n\nassets/ holds generated captures, not source.\n\n\
         ignore:\n  - \"notes/**\"\n",
    );

    let root = WorkspaceRoot(root_path.to_path_buf());
    let domain = Domain::load(&root).unwrap();
    assert_eq!(domain.version(), 2, "version comes from the frontmatter");

    let hashed = hash_domain(&root, &domain).unwrap();
    assert!(!hashed.contains(&PathBuf::from("assets/big.md")));
    assert!(
        hashed.contains(&PathBuf::from("notes/plan.md")),
        "the `ignore:` block in the BODY is prose, not configuration — reading \
         it would silently widen the ignore list"
    );
    // The named wrinkle, asserted rather than assumed: a .md config is itself
    // attested, so editing the ignore list moves the root.
    assert!(
        hashed.contains(&PathBuf::from("meridian/domain.md")),
        "the file defining the attested surface is itself attested"
    );
}

/// Two configs is an ambiguity the engine reports instead of resolving: a
/// precedence rule would mean the file you are reading might not be the file in
/// force.
#[test]
fn two_domain_configs_refuse_rather_than_pick() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(
        root_path,
        "meridian/domain.md",
        "---\nignore:\n  - \"a/**\"\n---\n",
    );
    write(root_path, "mdfs_config.yaml", "ignore:\n  - \"b/**\"\n");

    let root = WorkspaceRoot(root_path.to_path_buf());
    let err = Domain::load(&root).expect_err("two live ignore lists must refuse");
    let msg = err.to_string();
    assert!(msg.contains("meridian/domain.md") && msg.contains("mdfs_config.yaml"));
    assert!(msg.contains("Remedy:"), "the refusal must teach the fix");
}

/// A config page with no frontmatter declares nothing — and therefore ignores
/// nothing. It must not be read as an empty-but-authoritative domain in some
/// other way, nor error.
#[test]
fn markdown_config_without_frontmatter_is_the_default_domain() {
    let domain = Domain::from_markdown("# Domain\n\nNotes, but no frontmatter.\n");
    assert_eq!(domain.version(), 0);
    assert!(!domain.prunes_dir(Path::new("assets")));
}

/// Reserved paths stay reachable however the ignore list is written: a
/// directory on the way to one is never pruned.
#[test]
fn reserved_paths_are_never_pruned_away() {
    let domain =
        Domain::from_config("version: 1\nignore:\n  - \"meridian/**\"\n  - \"conventions/**\"\n");
    assert!(
        !domain.prunes_dir(Path::new("meridian")),
        "meridian/ holds the reserved journal and attested marker"
    );
    assert!(
        !domain.prunes_dir(Path::new("conventions")),
        "conventions/ holds the reserved index"
    );
}
