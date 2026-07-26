//! Capability gates (ruling 3 / decision #15): deny-by-default, the
//! explicit > convention > none ladder, narrow-only ceilings, the builtin
//! read-only conventions, and the bash-fence load refusal (check-*/verify-*
//! only — fix-* is where bash writes are wanted).
//!
//! WHERE the convention table is declared is a separate law with its own file:
//! `caps_home.rs` (the root's own `MERIDIAN.md`, marker-retirement ruling).

mod support;

use std::collections::BTreeSet;

use run::caps::{self, Cap, CapResolution, CapSet, CapSource, CapsError, Conventions};
use run::fence::TaskLanguage;
use support::doc;

fn set(caps: &[&str]) -> CapSet {
    CapSet(caps.iter().map(|c| Cap::parse(c).unwrap()).collect())
}

fn conventions(entries: &[(&str, &[&str])]) -> Conventions {
    Conventions::new(
        entries
            .iter()
            .map(|(p, caps)| ((*p).to_owned(), set(caps)))
            .collect(),
    )
    .unwrap()
}

fn resolve(
    task: &str,
    lang: TaskLanguage,
    explicit: Option<&CapSet>,
    conv: &Conventions,
) -> CapResolution {
    caps::resolve_caps(task, lang, explicit, conv).unwrap()
}

#[test]
fn undeclared_block_is_read_only_deny_by_default() {
    let r = resolve(
        "anything",
        TaskLanguage::Starlark,
        None,
        &Conventions::none(),
    );
    assert_eq!(r.source, CapSource::DenyDefault);
    assert_eq!(r.effective, CapSet::none());
    assert!(r.narrowed.is_empty());
    assert!(!r.effective.admits("md.set_field", None));
}

#[test]
fn explicit_frontmatter_beats_convention_as_grant_source() {
    let conv = conventions(&[("fix-*", &["md.set_field", "md.append_section"])]);
    let explicit = set(&["md.set_field:status"]);
    let r = resolve("fix-drift", TaskLanguage::Bash, Some(&explicit), &conv);
    assert_eq!(r.source, CapSource::Explicit);
    // The explicit grant is inside the convention ceiling — survives intact.
    assert_eq!(r.effective, explicit);
    assert!(r.narrowed.is_empty());
}

#[test]
fn convention_grants_when_no_explicit_declaration() {
    let conv = conventions(&[("fix-*", &["md.set_field"])]);
    let r = resolve("fix-drift", TaskLanguage::Bash, None, &conv);
    assert_eq!(r.source, CapSource::Convention("fix-*".to_owned()));
    assert!(r.effective.admits("md.set_field", Some("status")));
    assert!(!r.effective.admits("md.append_section", None));
}

#[test]
fn convention_narrows_an_explicit_grant_never_widens() {
    // Explicit grants MORE than the matching convention allows: the ceiling
    // drops the excess and the narrowing is visible, never silent.
    let conv = conventions(&[("fix-*", &["md.set_field"])]);
    let explicit = set(&["md.set_field", "md.append_section"]);
    let r = resolve("fix-x", TaskLanguage::Bash, Some(&explicit), &conv);
    assert_eq!(r.source, CapSource::Explicit);
    assert_eq!(r.effective, set(&["md.set_field"]));
    assert_eq!(r.narrowed, vec![Cap::parse("md.append_section").unwrap()]);
}

#[test]
fn targeted_ceiling_tightens_an_untargeted_grant() {
    // Grant `md.set_field` (any target) under ceiling `md.set_field:status`:
    // the effective cap is the TARGETED one — narrower, and reported.
    let conv = conventions(&[("fix-*", &["md.set_field:status"])]);
    let explicit = set(&["md.set_field"]);
    let r = resolve("fix-x", TaskLanguage::Bash, Some(&explicit), &conv);
    assert_eq!(r.effective, set(&["md.set_field:status"]));
    assert!(r.effective.admits("md.set_field", Some("status")));
    assert!(!r.effective.admits("md.set_field", Some("title")));
    assert_eq!(r.narrowed, vec![Cap::parse("md.set_field").unwrap()]);
}

#[test]
fn check_and_verify_names_refuse_a_bash_fence_loudly() {
    for task in ["check-links", "verify-roots"] {
        let err =
            caps::resolve_caps(task, TaskLanguage::Bash, None, &Conventions::none()).unwrap_err();
        assert!(
            matches!(err, CapsError::BashFenceRefused { .. }),
            "{task}: {err:?}"
        );
    }
}

#[test]
fn fix_names_do_not_refuse_bash() {
    // fix-* blocks DECLARE writes and are exactly where bash is wanted — the
    // load refusal is check-*/verify-* ONLY (ruling 3).
    let r = resolve("fix-wiki", TaskLanguage::Bash, None, &Conventions::none());
    assert_eq!(r.source, CapSource::DenyDefault);
}

#[test]
fn builtin_read_only_ceiling_zeroes_even_explicit_caps() {
    // A check-* block with explicit write caps stays read-only: the builtin
    // ceiling is absolute, and the dropped caps are reported.
    let explicit = set(&["md.set_field", "md.append_section"]);
    let r = resolve(
        "check-links",
        TaskLanguage::Starlark,
        Some(&explicit),
        &Conventions::none(),
    );
    assert_eq!(r.effective, CapSet::none());
    assert_eq!(r.narrowed.len(), 2);
}

#[test]
fn longest_pattern_wins() {
    let conv = conventions(&[
        ("fix-*", &["md.set_field"]),
        ("fix-drift", &["md.append_section"]),
    ]);
    let (pattern, _) = conv.matching("fix-drift").unwrap();
    assert_eq!(pattern, "fix-drift");
    let (pattern, _) = conv.matching("fix-other").unwrap();
    assert_eq!(pattern, "fix-*");
}

#[test]
fn cap_strings_are_namespaced_and_target_scoped_forward_compat() {
    let plain = Cap::parse("md.set_field").unwrap();
    assert_eq!(plain.kind, "md.set_field");
    assert_eq!(plain.target, None);

    let scoped = Cap::parse("md.set_field:status").unwrap();
    assert_eq!(scoped.target.as_deref(), Some("status"));
    assert_eq!(scoped.as_string(), "md.set_field:status");

    for bad in [
        "md",
        "set_field",
        "MD.set_field",
        "md.set field",
        "md.:x",
        "md.set_field:",
    ] {
        assert!(Cap::parse(bad).is_err(), "{bad} must refuse");
    }
}

#[test]
fn explicit_caps_reads_the_task_caps_key() {
    let d = doc(support::PAGE);
    let caps = caps::explicit_caps(&d, "fix-drift").unwrap().unwrap();
    assert!(caps.admits("md.set_field", Some("status")));
    assert!(caps.admits("md.append_section", None));
    assert!(!caps.admits("md.set_field", Some("title")));

    assert_eq!(caps::explicit_caps(&d, "check-links").unwrap(), None);
}

#[test]
fn empty_explicit_declaration_is_explicit_read_only() {
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.caps: \"\"\n---\n";
    let d = doc(page);
    let explicit = caps::explicit_caps(&d, "t").unwrap();
    assert_eq!(explicit, Some(CapSet(BTreeSet::new())));
    let r = resolve(
        "t",
        TaskLanguage::Bash,
        explicit.as_ref(),
        &Conventions::none(),
    );
    assert_eq!(r.source, CapSource::Explicit);
    assert_eq!(r.effective, CapSet::none());
}

// The convention PLANE — where the table is declared, and what each root
// situation yields — lives in `caps_home.rs` with the rest of the rehoming
// contract. What stays here is the resolution law those tables feed.

#[test]
fn conventions_parse_out_of_a_declaration_document() {
    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: md.set_field, md.append_section\n---\n");
    let conv = caps::conventions_from_declaration(&d).unwrap();
    let (pattern, set) = conv.matching("fix-drift").unwrap();
    assert_eq!(pattern, "fix-*");
    assert!(set.admits("md.set_field", None));
}

#[test]
fn a_declaration_without_caps_keys_is_the_empty_table() {
    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\n---\n");
    assert_eq!(
        caps::conventions_from_declaration(&d).unwrap(),
        Conventions::none()
    );
}

#[test]
fn a_malformed_cap_entry_is_a_loud_error_never_no_policy() {
    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: not_namespaced\n---\n");
    assert!(matches!(
        caps::conventions_from_declaration(&d).unwrap_err(),
        CapsError::BadCap { .. }
    ));

    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fi*x: md.set_field\n---\n");
    assert!(matches!(
        caps::conventions_from_declaration(&d).unwrap_err(),
        CapsError::BadPattern { .. }
    ));
}
