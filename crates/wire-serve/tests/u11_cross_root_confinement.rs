//! **U11 — post-resolution confinement, proven NEGATIVELY.**
//!
//! `path_confined` (`write.rs`) is the only confinement this engine has, and it
//! is purely LEXICAL — empty / leading `/` / `.` / `..`. Confinement in practice
//! came from joining onto the ONE `fs::WorkspaceRoot`; multi-root removes that
//! ambient guarantee, so the path portion must be re-confined to its resolved
//! mount.
//!
//! **And `splice` — the primary write op — never called `path_confined` at all.**
//! Only `create`, `remove`, `lock_write` and `mint_pin` did. `fs::load` joins the
//! caller's path onto the root (`root.0.join(rel_path)`), and `Path::join` with an
//! ABSOLUTE path DISCARDS the root entirely — so an absolute or `..`-bearing
//! splice path reads and writes outside the workspace.
//!
//! `mrd put` makes this reachable from a shell: it bypasses the strict decode
//! entirely and builds `SpliceArgs` straight from raw argv (`mrd/src/put_cmd.rs`).
//!
//! Every test here asserts on the WRITE refusal, and the acceptance half
//! (S3-R8(c)) is asserted in the same breath — a guard proven only by what it
//! blocks is indistinguishable from a guard that blocks everything.

use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// The in-workspace page a legitimate splice edits — the acceptance half.
const PAGE: &str = "# Alpha\n\n## Beta\n\nkeep this.\n";
/// The out-of-workspace victim an escaping splice would reach.
const VICTIM: &str = "# Alpha\n\n## Beta\n\nsecret.\n";

/// An outer directory holding BOTH the workspace and a sibling victim file, so
/// `../victim.md` from inside the workspace names a real, editable document.
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

/// **The escape, via `..`.** A relative path climbing out of the workspace must
/// be refused `bad_path` — and the victim's bytes must be untouched. The byte
/// assertion is the one that carries the claim: a refusal that still wrote would
/// pass an `is_err()` check alone.
#[test]
fn splice_refuses_a_dot_dot_path_and_leaves_the_victim_untouched() {
    let (_outer, root, victim) = workspace();

    let err = splice(&root, 0, &args_for("../victim.md"), &[], None)
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

/// **The escape, via an ABSOLUTE path** — the sharper one, because
/// `Path::join` with an absolute argument discards the root outright rather than
/// walking out of it.
#[test]
fn splice_refuses_an_absolute_path_and_leaves_the_victim_untouched() {
    let (_outer, root, victim) = workspace();
    let absolute = victim.display().to_string();

    let err = splice(&root, 0, &args_for(&absolute), &[], None)
        .expect_err("an absolute splice path discards the workspace root and must be refused");
    assert_eq!(err.code, ErrorCode::BadPath);
    assert_eq!(
        std::fs::read_to_string(&victim).expect("victim readable"),
        VICTIM,
        "an absolute path must not reach the file it names",
    );
}

/// **The ACCEPTANCE half (S3-R8(c)).** The same splice shape, confined to the
/// workspace, must still COMMIT. Without this the two refusals above are equally
/// satisfied by a `splice` that refuses everything.
#[test]
fn splice_still_commits_an_ordinary_in_workspace_path() {
    let (_outer, root, _victim) = workspace();
    let mut args = args_for("page.md");
    args.edits[0].edit = EditShape::Match {
        old: "keep this.".into(),
        new: "kept.".into(),
    };

    splice(&root, 0, &args, &[], None).expect("an ordinary confined splice still commits");
    assert!(
        std::fs::read_to_string(root.0.join("page.md"))
            .expect("page readable")
            .contains("kept."),
        "the confined write landed — the guard blocks the escape, not the corpus",
    );
}

/// **The `root:`-bearing path (§ 4.2 / D11).** A first path segment carrying a
/// `:` is unaddressable by the address grammar, so a write door targeting one
/// refuses `bad_path` rather than creating a document no address can ever name.
#[test]
fn splice_refuses_a_root_prefixed_path() {
    let (_outer, root, _victim) = workspace();

    let err = splice(&root, 0, &args_for("sessions:page.md"), &[], None)
        .expect_err("a `root:`-bearing path is not a corpus key — the write door refuses it");
    assert_eq!(
        err.code,
        ErrorCode::BadPath,
        "a `root:` prefix is an ADDRESS, never a path — refused at the door (D11)",
    );
}
