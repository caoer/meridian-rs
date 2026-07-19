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
