//! E2e gates for the `mrd realise` board directory, driving the REAL binary over its process
//! boundary against a fixture workspace (`MERIDIAN_WORKSPACE` tier-1 override).
//!
//! The folder a user files pending-agent cards in is THEIR flow, not the engine's
//! (`docs/laws.md` § Amendment — no hard-coded flow, rule 1): it is read off the realising page's
//! `realise.board_dir`, and the code constant is a fallback only.

use std::path::Path;
use std::process::{Command, Output};

/// A realising page that can never converge: `status` drifts from the expected value and no
/// `realise.apply` is declared, so the run classifies `pending-agent` and mints a card.
fn drifting_page(board_dir: Option<&str>) -> String {
    let declared = match board_dir {
        None => String::new(),
        Some(dir) => format!("realise.board_dir: {dir}\n"),
    };
    format!(
        "---\nstatus: todo\nrealise.field: status\nrealise.expected: done\n{declared}---\n\n# Drift\n"
    )
}

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    /// A workspace holding one realising page at `drift.md`.
    fn new(page: &str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("drift.md"), page).expect("page");
        Self { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mrd"))
            .arg("realise")
            .args(args)
            .env("MERIDIAN_WORKSPACE", self.path())
            .current_dir(self.path())
            .output()
            .expect("spawn mrd")
    }

    /// The `.md` files directly under a workspace-relative directory, sorted.
    fn cards(&self, dir: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path().join(dir))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| {
                        Path::new(name)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// The card is born in the directory the PAGE names, and nothing lands in the code default.
#[test]
fn board_dir_follows_page_frontmatter() {
    let ws = Ws::new(&drifting_page(Some("flow/cards")));
    let out = ws.run(&["drift.md"]);

    assert_eq!(
        code(&out),
        1,
        "expected the pending-agent leg: {}",
        stderr(&out)
    );
    assert_eq!(
        ws.cards("flow/cards"),
        vec!["drift-md-status.md".to_owned()]
    );
    assert!(
        ws.cards("board").is_empty(),
        "a card leaked into the code default"
    );
}

/// A page that says nothing about the board keeps the shipped default — the constant is a
/// fallback, so this fix changes no existing workspace.
#[test]
fn board_dir_defaults_when_the_page_is_silent() {
    let ws = Ws::new(&drifting_page(None));
    let out = ws.run(&["drift.md"]);

    assert_eq!(
        code(&out),
        1,
        "expected the pending-agent leg: {}",
        stderr(&out)
    );
    assert_eq!(ws.cards("board"), vec!["drift-md-status.md".to_owned()]);
}

/// A declared-but-blank key is a malformed declaration, refused loud (exit 2) rather than
/// silently defaulted — the author meant a folder, and `board/` is not where they are looking.
#[test]
fn blank_board_dir_is_refused_loud() {
    let ws = Ws::new(&drifting_page(Some("\"\"")));
    let out = ws.run(&["drift.md"]);

    assert_eq!(code(&out), 2, "expected a tool failure");
    assert!(
        stderr(&out).contains("empty `realise.board_dir`"),
        "refusal does not name the key:\n{}",
        stderr(&out)
    );
    assert!(ws.cards("board").is_empty(), "a card was minted anyway");
}
