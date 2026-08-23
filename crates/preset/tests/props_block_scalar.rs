//! **The `^properties` checker publishes block scalars** — card
//! `scalar-text-trims-config-key-block-scalars`.
//!
//! `preset`'s `fm_scalar` read the flat frontmatter map through
//! `model::scalar::text`, whose decode opens with `value.trim()`. That is
//! right for a key line's colon remainder and wrong for a block scalar, which
//! PR 189 made the flat map store ALREADY DECODED — so every byte the decoder
//! recovered was trimmed off again one layer later, in the checker.
//!
//! **The key is not ours to bound.** `first_violated_rule` calls `fm_scalar`
//! with `rule.key` — whatever key a def's `^properties` block happens to
//! declare. `status`, `description` and `manifest` all carry block scalars on
//! live pages today, so nothing about key SHAPE closes this; what keeps it
//! dormant is that no `type: preset` page exists yet (measured across six
//! bound roots, 2026-08-23). The first one written opens it.
//!
//! **These assertions drive the public door**, `preset::new_record`, and its
//! observable outcome (born vs refused) — not `fm_scalar`, and not the flat
//! map. That is the lesson PR 189's review charged for: its first matrix
//! asserted 459 characters one layer BELOW the publishing seam and passed
//! while the face served 458. A gate one layer off the face it covers is not a
//! gate. `PyYAML` is the third opinion on every value, because the engine's
//! YAML reader is hand-rolled (`model` is serde-free by the
//! `yaml_confinement.rs` law) and a hand-rolled parser is only as good as what
//! it is measured against.

use preset::{BirthOptions, NewOutcome, new_record};

/// A def whose `^template` writes a ruled key as a block scalar with an
/// EXPLICIT indentation indicator and strip chomping (`|2-`).
///
/// The shape is chosen because it is the one where the trim's damage is
/// expressible as a pinned rule: the true value is `"  x"` — two leading
/// spaces the indicator preserves as content (YAML 1.2 § 8.1.1.1) and no
/// trailing break (strip). `trim()` turns that into `"x"`, so the pinned rule
/// `= "  x"` reads as VIOLATED against a record that satisfies it perfectly.
/// A false violation, from a checker whose whole job is to not render one.
const INDENTED_BLOCK_DEF: &str = r#"---
type: def
defines: session
root: SESSION.md
births: "sessions/{{id}}.md"
---

# Properties ^properties

- `type` required
- `indent` required = "  x"

# Template ^template

```record
---
type: session
indent: |2-
    x
---

# Session {{id}}
```

# Unfold

- SESSION.md
"#;

/// The same door, folded clip (`>`) — the DEFAULT chomping, and the shape
/// every live block-scalar page uses. Clip leaves a trailing newline, so the
/// true value is `"done\n"` and a rule pinning `done` is genuinely not
/// satisfied. Kept as its own case because it moves in the opposite direction
/// from [`INDENTED_BLOCK_DEF`]: routing through the seam makes this one
/// REFUSE where the trim used to let it through.
const FOLDED_CLIP_DEF: &str = r#"---
type: def
defines: session
root: SESSION.md
births: "sessions/{{id}}.md"
---

# Properties ^properties

- `type` required
- `status` required = "done"

# Template ^template

```record
---
type: session
status: >
  done
---

# Session {{id}}
```

# Unfold

- SESSION.md
"#;

fn workspace(def: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("presets/session.md");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, def).expect("write def");
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn birth(root: &fs::WorkspaceRoot, id: &str) -> NewOutcome {
    let opts = BirthOptions {
        actor: Some("zt".to_owned()),
        now: Some("2026-08-23T00:00:00Z".to_owned()),
        ..Default::default()
    };
    new_record(root, "presets/session.md", id, &opts).expect("the def loads")
}

/// One key's value as `PyYAML` reads it — the foreign oracle, over the
/// frontmatter block exactly as the file carries it.
///
/// The split keeps the newline that ENDS the last frontmatter line: for a
/// block scalar that byte is the value's own trailing break, and dropping it
/// would leave clip chomping nothing to clip, making the oracle disagree with
/// every correct decoder.
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

/// The RECORD the door checks: the def's `^template`, `{{id}}`-filled. The
/// block scalars under test live here, inside the def's fence — never in the
/// def page's own frontmatter, which is what a careless oracle would read.
fn record_body(def: &str, id: &str) -> String {
    preset::template_of(def)
        .expect("the def carries a ^template")
        .replace("{{id}}", id)
}

/// The oracle first: state what the values ACTUALLY are, so the door
/// assertions below are anchored to YAML and not to the engine's opinion of
/// itself. Both are values `trim()` changes — that is the whole reachability
/// argument, and if `PyYAML` ever stops agreeing, these fixtures stopped
/// exercising the defect and the door tests below have gone vacuous.
#[test]
fn pyyaml_says_the_trim_is_lossy_for_both_fixtures() {
    let indented = pyyaml(&record_body(INDENTED_BLOCK_DEF, "s1"), "indent");
    assert_eq!(
        indented, "  x",
        "explicit-indent strip keeps the two extra spaces as content"
    );
    assert_ne!(indented, indented.trim(), "the fixture must expose a trim");

    let folded = pyyaml(&record_body(FOLDED_CLIP_DEF, "s1"), "status");
    assert_eq!(folded, "done\n", "folded clip keeps one trailing break");
    assert_ne!(folded, folded.trim(), "the fixture must expose a trim");
}

/// **The false violation this card exists to close.** The record satisfies the
/// pinned rule byte for byte; only the checker's second decode disagreed.
/// Before the fix `fm_scalar` served `"x"` against an expected `"  x"` and
/// `new_record` refused a perfectly valid birth.
#[test]
fn a_block_scalar_pinned_rule_is_satisfied_at_the_birth_door() {
    let (_tmp, root) = workspace(INDENTED_BLOCK_DEF);
    match birth(&root, "s1") {
        NewOutcome::Born(report) => assert_eq!(report.target, "sessions/s1.md"),
        NewOutcome::Refused(r) => panic!(
            "the record carries exactly the pinned value; refused instead: {:?}",
            r.reason
        ),
    }
}

/// The other direction, asserted so it cannot change silently: clip chomping
/// means the value really is `"done\n"`, so a rule pinning `"done"` is
/// violated. The trim used to hide that and birth the record anyway — the
/// checker's view and the published `read` face then disagreed about the same
/// key, which is the two-faces defect PR 189 closed one layer up.
#[test]
fn folded_clip_does_not_satisfy_a_rule_pinned_to_the_trimmed_text() {
    let (_tmp, root) = workspace(FOLDED_CLIP_DEF);
    match birth(&root, "s1") {
        NewOutcome::Refused(r) => {
            let reason = format!("{:?}", r.reason);
            assert!(
                reason.contains("status"),
                "the refusal names the failing rule: {reason}"
            );
        }
        NewOutcome::Born(_) => {
            panic!("`status` is \"done\\n\", not \"done\" — the pinned rule is not satisfied")
        }
    }
}

/// The checker must publish the SAME bytes the read face publishes. Asserted
/// against `model::fm_doc_publish` — the one owner both now call — with
/// `PyYAML` holding it to YAML rather than to itself.
#[test]
fn the_checker_and_the_published_face_agree_with_pyyaml() {
    for (def, key, expected) in [
        (INDENTED_BLOCK_DEF, "indent", "  x"),
        (FOLDED_CLIP_DEF, "status", "done\n"),
    ] {
        let body = record_body(def, "s1");
        let record = model::build(body.clone(), syntax::parse(&body));
        let published = model::fm_doc_publish(&record, key)
            .unwrap_or_else(|| panic!("key {key:?} absent from the filled template"));
        assert_eq!(
            published, expected,
            "the checker's value for {key:?} is the published value"
        );
        assert_eq!(
            published,
            pyyaml(&body, key),
            "the checker's value for {key:?} agrees with PyYAML"
        );
        assert_ne!(
            published,
            model::scalar::text(&published),
            "the fixture must be one the OLD path would have changed"
        );
    }
}
