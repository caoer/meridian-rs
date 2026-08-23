//! **The DECLARED half publishes block scalars too** — card
//! `scalar-text-trims-config-key-block-scalars`.
//!
//! `realise` compares two values: the OBSERVED one, read off the watched page
//! by `realise::FieldEquals`, and the DECLARED one, read off the realising
//! page's `realise.expected` here in `realise_cmd`. Both used to go through
//! `model::scalar::text`, whose decode opens with `value.trim()`.
//!
//! Unlike `preset`'s `rule.key` and `realise`'s `field`, the keys read here are
//! FIXED (`realise.*`), so no live page can carry a block scalar under one
//! today and this site could have been left with a comment. It publishes
//! through `model::fm_doc_publish` anyway, and the reason is the comparison
//! itself: **a decode on one side alone moves the mismatch instead of closing
//! it.** With only the observed half fixed, an author who writes
//! `realise.expected` as a block scalar gets a declared value trimmed against
//! an observed value that is not — permanent phantom drift, and a realise loop
//! answers drift by applying, so the engine would drive a converged world
//! forever.
//!
//! The fixture below is the discriminating shape. Both halves being block
//! scalars proves nothing (they were trimmed symmetrically and matched); the
//! mismatch only shows when ONE side is a block scalar. So: declared as an
//! explicit-indent strip block whose leading spaces are content (YAML 1.2
//! § 8.1.1.1), observed as an ordinary quoted scalar carrying exactly those
//! bytes. Before the fix this reported drift and minted a pending-agent card
//! against a page that had already converged.
//!
//! Driven through the REAL `mrd` binary over its process boundary, not through
//! `spec_of` — the door a user reaches. `PyYAML` holds the fixture to YAML.

use std::path::Path;
use std::process::{Command, Output};

/// `realise.expected` as a `|2-` block: explicit indent 2, strip chomping. The
/// value is `"  padded"` — two leading spaces of content, no trailing break.
/// `status` carries exactly that, quoted so the spaces survive the ordinary
/// § A.6.1 decode.
const CONVERGED_PAGE: &str = "\
---
status: \"  padded\"
realise.field: status
realise.expected: |2-
    padded
---

# Converged
";

/// The control: the same declared block scalar against an observed value that
/// is the TRIMMED text. This one really has drifted, and must still say so —
/// the assertion that fails if someone "fixes" the test above by trimming both
/// sides again.
const DRIFTED_PAGE: &str = "\
---
status: padded
realise.field: status
realise.expected: |2-
    padded
---

# Drifted
";

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    fn new(page: &str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("drift.md"), page).expect("page");
        Self { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mrd"))
            .arg("realise")
            .arg("drift.md")
            .env("MERIDIAN_WORKSPACE", self.path())
            .current_dir(self.path())
            .output()
            .expect("spawn mrd")
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// One key as `PyYAML` reads it — the foreign oracle over the frontmatter block
/// exactly as the file carries it.
fn pyyaml(raw: &str, key: &str) -> String {
    let fm = raw
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(fm, _)| format!("{fm}\n")))
        .expect("the fixture carries frontmatter");
    let out = Command::new("python3")
        .args([
            "-c",
            "import sys, yaml\nd = yaml.safe_load(sys.argv[1])\nsys.stdout.write(d[sys.argv[2]])\n",
            &fm,
            key,
        ])
        .output()
        .expect("python3 runs");
    assert!(
        out.status.success(),
        "PyYAML rejects the fixture: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

/// The premise, from the foreign parser: the two halves of `CONVERGED_PAGE`
/// really are the same bytes, and those bytes really are ones `trim()` would
/// change. Without this the exit-code assertion below could pass for the wrong
/// reason.
#[test]
fn pyyaml_says_the_two_halves_are_the_same_bytes() {
    let declared = pyyaml(CONVERGED_PAGE, "realise.expected");
    let observed = pyyaml(CONVERGED_PAGE, "status");
    assert_eq!(declared, "  padded", "the declared half");
    assert_eq!(observed, "  padded", "the observed half");
    assert_eq!(declared, observed, "the page has converged");
    assert_ne!(
        declared,
        declared.trim(),
        "the fixture must expose a trim, or it tests nothing"
    );
}

/// **The phantom drift this closes.** The page has converged; `mrd realise`
/// must exit 0. Before the fix the declared half was trimmed to `padded`, the
/// observed half was `  padded`, and the run took the pending-agent leg
/// (exit 1) and minted a card for work nobody needed.
#[test]
fn a_block_scalar_expectation_converges_against_the_matching_value() {
    let ws = Ws::new(CONVERGED_PAGE);
    let out = ws.run();
    assert_eq!(
        code(&out),
        0,
        "the page carries exactly the declared value: {}",
        stderr(&out)
    );
}

/// The converse: a genuinely drifted page still takes the pending-agent leg.
#[test]
fn the_trimmed_observed_value_still_drifts() {
    let ws = Ws::new(DRIFTED_PAGE);
    let out = ws.run();
    assert_eq!(
        code(&out),
        1,
        "`padded` is not `  padded` — this must still drift: {}",
        stderr(&out)
    );
}
