//! U11 — post-resolution path confinement on `splice` (lexical `path_confined`).
//!
//! Pins: `..` / absolute / `root:` paths → `bad_path` + victim bytes untouched;
//! in-workspace path still commits (S3-R8(c) acceptance).

use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// In-workspace page (acceptance half).
const PAGE: &str = "# Alpha\n\n## Beta\n\nkeep this.\n";
/// Sibling victim outside the workspace.
const VICTIM: &str = "# Alpha\n\n## Beta\n\nsecret.\n";

/// Outer dir with workspace + sibling victim (`../victim.md` is real).
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot, std::path::PathBuf) {
    let outer = tempfile::tempdir().expect("tempdir");
    let ws = outer.path().join("ws");
    std::fs::create_dir(&ws).expect("ws dir");
    std::fs::write(ws.join("page.md"), PAGE).expect("page");
    let victim = outer.path().join("victim.md");
    std::fs::write(&victim, VICTIM).expect("victim");
    let root = fs::WorkspaceRoot(ws);
    (outer, root, victim)
}

fn args_for(path: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path.into()),
        actor: Some("alice".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![
                    HpathSeg {
                        h: "Alpha".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "Beta".into(),
                        n: None,
                    },
                ],
            },
            edit: EditShape::Match {
                old: "secret.".into(),
                new: "OWNED.".into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    }
}

/// `..` path → `bad_path`; victim bytes untouched.
#[test]
fn splice_refuses_a_dot_dot_path_and_leaves_the_victim_untouched() {
    let (_outer, root, victim) = workspace();

    let err = splice(&root, None, &args_for("../victim.md"), &[], None)
        .expect_err("a `..` splice path escapes the workspace and must be refused");
    assert_eq!(
        err.code,
        ErrorCode::BadPath,
        "the escape is `bad_path`, echoing the offending path",
    );
    assert_eq!(
        std::fs::read_to_string(&victim).expect("victim readable"),
        VICTIM,
        "the out-of-workspace file must be byte-identical — a refusal that still wrote is no guard",
    );
}

/// Absolute path → `bad_path` (`Path::join` discards root).
#[test]
fn splice_refuses_an_absolute_path_and_leaves_the_victim_untouched() {
    let (_outer, root, victim) = workspace();
    let absolute = victim.display().to_string();

    let err = splice(&root, None, &args_for(&absolute), &[], None)
        .expect_err("an absolute splice path discards the workspace root and must be refused");
    assert_eq!(err.code, ErrorCode::BadPath);
    assert_eq!(
        std::fs::read_to_string(&victim).expect("victim readable"),
        VICTIM,
        "an absolute path must not reach the file it names",
    );
}

/// Acceptance (S3-R8(c)): ordinary in-workspace path still commits.
#[test]
fn splice_still_commits_an_ordinary_in_workspace_path() {
    let (_outer, root, _victim) = workspace();
    let mut args = args_for("page.md");
    args.edits[0].edit = EditShape::Match {
        old: "keep this.".into(),
        new: "kept.".into(),
    };

    splice(&root, None, &args, &[], None).expect("an ordinary confined splice still commits");
    assert!(
        std::fs::read_to_string(root.0.join("page.md"))
            .expect("page readable")
            .contains("kept."),
        "the confined write landed — the guard blocks the escape, not the corpus",
    );
}

/// `root:`-bearing path (§4.2 / D11) → `bad_path` at the door.
#[test]
fn splice_refuses_a_root_prefixed_path() {
    let (_outer, root, _victim) = workspace();

    let err = splice(&root, None, &args_for("sessions:page.md"), &[], None)
        .expect_err("a `root:`-bearing path is not a corpus key — the write door refuses it");
    assert_eq!(
        err.code,
        ErrorCode::BadPath,
        "a `root:` prefix is an ADDRESS, never a path — refused at the door (D11)",
    );
}
