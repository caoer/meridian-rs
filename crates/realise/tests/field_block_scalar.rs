//! **`FieldEquals` observes block scalars whole** — card
//! `scalar-text-trims-config-key-block-scalars`.
//!
//! `realise`'s `fm_value` read the flat frontmatter map through
//! `model::scalar::text`, whose decode opens with `value.trim()`. PR 189 made
//! the flat map store a block scalar ALREADY DECODED, so that second decode
//! trimmed away exactly the bytes the decoder had just recovered — and the
//! OBSERVED half of a convergence comparison came back short.
//!
//! **`field` is arbitrary.** It is whatever a page declares as
//! `realise.field`, so no naming convention bounds this: the first page that
//! watches a key carrying a block scalar opens the class. Measured across six
//! bound roots on 2026-08-23 there are ZERO `realise.field` declarations, so
//! the class is dormant for lack of USERS, not because key shape closes it.
//!
//! **The failure mode is the worse direction.** A trimmed observation reads as
//! DRIFTED against a page that has already converged, and a realise loop
//! answers drift by APPLYING — so the engine would keep driving a world that
//! was already where it was asked to be, forever, and each pass would report
//! the same phantom mismatch.
//!
//! Assertions drive `FieldEquals::observe` — the real check door, over a real
//! on-disk workspace — with `PyYAML` as the foreign oracle on every expected
//! value. The engine's YAML reader is hand-rolled (`model` is serde-free by the
//! `yaml_confinement.rs` law), so measuring it against itself proves nothing.

use realise::{Check, CheckOutcome, FieldEquals};

/// A page whose watched field is a LITERAL block scalar: a genuinely
/// multi-line value, the shape no single-line reader can represent.
const LITERAL_PAGE: &str = "\
---
status: |
  first
  second
---

# Body
";

/// A page whose watched field is a FOLDED clip scalar — the default chomping
/// and what every live block-scalar page in the corpus uses. Its value ends in
/// the trailing break clip leaves; that one byte is the whole defect.
const FOLDED_PAGE: &str = "\
---
status: >
  done
---

# Body
";

/// An EXPLICIT-indent strip scalar: leading spaces are content (YAML 1.2
/// § 8.1.1.1) and there is no trailing break. It exposes the trim at the
/// LEADING end, which a "just tolerate a trailing newline" patch would miss.
const INDENTED_PAGE: &str = "\
---
status: |2-
    padded
---

# Body
";

fn workspace(page: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("page.md"), page).expect("write page");
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

/// One key's value as `PyYAML` reads it — the foreign oracle, over the
/// frontmatter block exactly as the file carries it. The split keeps the
/// newline ending the last frontmatter line: for a block scalar that byte is
/// the value's own trailing break, and dropping it would leave clip chomping
/// nothing to clip.
fn pyyaml(raw: &str, key: &str) -> String {
    let fm = raw
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(fm, _)| format!("{fm}\n")))
        .expect("the fixture carries frontmatter");
    let out = std::process::Command::new("python3")
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

fn observe(page: &str, expected: &str) -> CheckOutcome {
    let (_tmp, root) = workspace(page);
    FieldEquals {
        page: "page.md".to_owned(),
        field: "status".to_owned(),
        expected: expected.to_owned(),
    }
    .observe(&root)
    .expect("the page loads")
}

/// The premise, stated by the foreign parser before anything is asserted about
/// the door: all three fixtures carry a value `trim()` would change. If this
/// ever fails, the fixtures stopped exercising the defect and every test below
/// has gone vacuous — the exact way PR 189's first gate passed while the face
/// was broken.
#[test]
fn pyyaml_says_all_three_fixtures_expose_the_trim() {
    for (name, page, expected) in [
        ("literal clip", LITERAL_PAGE, "first\nsecond\n"),
        ("folded clip", FOLDED_PAGE, "done\n"),
        ("explicit indent strip", INDENTED_PAGE, "  padded"),
    ] {
        let value = pyyaml(page, "status");
        assert_eq!(value, expected, "{name}: PyYAML's reading");
        assert_ne!(
            value,
            value.trim(),
            "{name}: the fixture must expose a trim"
        );
    }
}

/// **The phantom drift this card closes.** Each page already carries exactly
/// the value the claim expects, so the check must say Converged. Before the
/// fix the observation was trimmed and every one of these reported Drifted
/// against a converged world.
#[test]
fn a_converged_block_scalar_field_observes_as_converged() {
    for (name, page) in [
        ("literal clip", LITERAL_PAGE),
        ("folded clip", FOLDED_PAGE),
        ("explicit indent strip", INDENTED_PAGE),
    ] {
        let expected = pyyaml(page, "status");
        assert_eq!(
            observe(page, &expected),
            CheckOutcome::Converged,
            "{name}: the page carries exactly {expected:?}"
        );
    }
}

/// The converse, so the fix cannot be "always converge": the TRIMMED text is
/// not the field's value, and a claim expecting it must still read as drift.
/// This is the assertion that would fail if someone made `fm_value` trim both
/// sides to make the test above pass.
#[test]
fn the_trimmed_text_is_not_what_the_field_carries() {
    for (name, page) in [
        ("literal clip", LITERAL_PAGE),
        ("folded clip", FOLDED_PAGE),
        ("explicit indent strip", INDENTED_PAGE),
    ] {
        let trimmed = pyyaml(page, "status").trim().to_owned();
        let CheckOutcome::Drifted { detail } = observe(page, &trimmed) else {
            panic!("{name}: expected drift against the trimmed text");
        };
        assert!(
            detail.contains("status"),
            "{name}: the detail names the field: {detail}"
        );
    }
}

/// An ordinary quoted scalar under the same field still decodes exactly as
/// before — the seam changed the block-scalar branch only, and this pins that
/// the § A.6.1 decode did not move with it.
#[test]
fn an_ordinary_quoted_scalar_still_decodes() {
    let page = "---\nstatus: \"done\"\n---\n\n# Body\n";
    assert_eq!(observe(page, "done"), CheckOutcome::Converged);
}
