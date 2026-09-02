//! The rehomed convention plane (marker-retirement ruling): the caps table is
//! declared by the ROOT ITSELF in `<root>/MERIDIAN.md`, never by a marker
//! file. Contract: `docs/run-plane.md`.

use run::caps::{self, Cap, CapSet, CapSource, CapsError, ConventionSource, Conventions};

/// A root that declares itself, plus whatever `run.*` keys the test needs.
fn declare(root: &std::path::Path, body: &str) {
    std::fs::write(
        root.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: fixture-root\n{body}---\n"),
    )
    .unwrap();
}

fn set(caps: &[&str]) -> CapSet {
    CapSet(caps.iter().map(|c| Cap::parse(c).unwrap()).collect())
}

/// The table comes from the ROOT DECLARATION.
#[test]
fn conventions_load_from_the_root_declaration() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "run.caps.fix-*: md.edit, md.create\n");

    let (conv, source) = caps::load_conventions(Some(tmp.path())).unwrap();
    assert_eq!(source, ConventionSource::Declared(tmp.path().to_path_buf()));

    let (pattern, caps) = conv
        .matching("fix-drift")
        .expect("the root declared a fix-* convention");
    assert_eq!(pattern, "fix-*");
    assert!(caps.admits("md.edit", None));
}

/// THE WIDENING DETECTOR. A declared ceiling must narrow an explicit grant; if
/// the declaration goes unread, the grant passes whole and that is the widening.
#[test]
fn a_declared_ceiling_narrows_an_explicit_grant() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "run.caps.fix-*: md.edit\n");

    let (conv, _) = caps::load_conventions(Some(tmp.path())).unwrap();
    let explicit = set(&["md.edit", "md.create"]);
    let r = caps::resolve_caps("fix-x", Some(&explicit), &conv);

    assert_eq!(r.source, CapSource::Explicit);
    assert_eq!(
        r.effective,
        set(&["md.edit"]),
        "the declared ceiling must drop md.create — a wider effective set is the widening"
    );
    assert_eq!(r.narrowed, vec![Cap::parse("md.create").unwrap()]);
}

/// A present-but-invalid declaration REFUSES. Reading it as the empty table
/// would delete a declared ceiling on one typo — the widening, by silence.
#[test]
fn an_invalid_declaration_refuses_and_never_yields_no_policy() {
    let tmp = tempfile::tempdir().unwrap();
    // The config plane's own file type, at a root: a discriminator mismatch.
    std::fs::write(
        tmp.path().join("MERIDIAN.md"),
        "---\ntype: meridian-config\nversion: 1\n---\n",
    )
    .unwrap();

    let err = caps::load_conventions(Some(tmp.path())).unwrap_err();
    assert!(
        matches!(err, CapsError::Declaration { .. }),
        "a MERIDIAN.md that is not a root declaration must refuse: {err:?}"
    );
    // The refusal teaches the fix rather than only naming the fault.
    let text = err.to_string();
    assert!(text.contains("meridian-root"), "{text}");
    assert!(text.contains("No convention table was loaded"), "{text}");
}

/// A broken `version:` is the same class of fault — a ceiling that cannot be
/// read is never a ceiling that is absent.
#[test]
fn a_declaration_with_an_unimplemented_version_refuses() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 99\nname: fixture-root\nrun.caps.fix-*: md.edit\n---\n",
    )
    .unwrap();

    assert!(matches!(
        caps::load_conventions(Some(tmp.path())).unwrap_err(),
        CapsError::Declaration { .. }
    ));
}

/// No root at all (the ladder's `CwdDefault`): the empty table, said out loud,
/// and deny-by-default remains the whole chain. Never an error — refusing here
/// would delete the convenience default the ruling deliberately kept.
#[test]
fn no_root_is_the_empty_table_stated_never_an_error() {
    let (conv, source) = caps::load_conventions(None).unwrap();
    assert_eq!(conv, Conventions::none());
    assert_eq!(source, ConventionSource::NoRoot);
    assert_eq!(source.root(), None);
    assert!(
        source
            .to_string()
            .contains("no convention ceiling in force"),
        "the absence of a ceiling must be stated, not implied: {source}"
    );

    let r = caps::resolve_caps("fix-x", None, &conv);
    assert_eq!(r.source, CapSource::DenyDefault);
    assert_eq!(r.effective, CapSet::none());
}

/// An undeclared root is DISTINCT from no root: both yield the empty table, and
/// they teach different fixes, so they must not collapse into one state.
#[test]
fn an_undeclared_root_is_distinct_from_no_root() {
    let tmp = tempfile::tempdir().unwrap();
    let (conv, source) = caps::load_conventions(Some(tmp.path())).unwrap();

    assert_eq!(conv, Conventions::none());
    assert_eq!(
        source,
        ConventionSource::Undeclared(tmp.path().to_path_buf())
    );
    assert_ne!(source, ConventionSource::NoRoot);
    assert_eq!(source.root(), Some(tmp.path()));
}

/// A root that declares itself but states no `run.caps.*` is `Declared` with an
/// empty table — emptiness is read off the table, not off a fourth variant.
#[test]
fn a_declared_root_without_caps_keys_is_declared_and_empty() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "");

    let (conv, source) = caps::load_conventions(Some(tmp.path())).unwrap();
    assert_eq!(conv, Conventions::none());
    assert_eq!(source, ConventionSource::Declared(tmp.path().to_path_buf()));
}

/// THE EXPRESSIVENESS FIXTURE — the four combinations the one real-world
/// `[run.caps]` table exercised, round-tripped through the new grammar:
/// glob key, exact key, unscoped verb, colon-scoped path glob.
#[test]
fn the_live_fixture_four_combinations_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    declare(
        tmp.path(),
        "run.caps.conv-*: md.edit\nrun.caps.conv-narrow: md.edit:tasks/*.md\n",
    );

    let (conv, _) = caps::load_conventions(Some(tmp.path())).unwrap();

    // Glob key + unscoped verb: admits every target of its kind.
    let (pattern, caps) = conv.matching("conv-other").expect("glob key matches");
    assert_eq!(pattern, "conv-*");
    assert!(caps.admits("md.edit", Some("anything.md")));

    // Exact key + colon-scoped target, winning over the glob by length.
    let (pattern, caps) = conv.matching("conv-narrow").expect("exact key matches");
    assert_eq!(pattern, "conv-narrow");
    assert!(caps.admits("md.edit", Some("tasks/a.md")));
    assert!(
        !caps.admits("md.edit", Some("notes/a.md")),
        "the scoped target must stay strictly narrower than its unscoped form"
    );
}

/// A pattern may contain `.`: the pattern is taken by stripping the FIXED
/// `run.caps.` prefix, never by splitting the key on dots.
#[test]
fn a_pattern_may_contain_a_dot() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "run.caps.fix.note: md.edit\n");

    let (conv, _) = caps::load_conventions(Some(tmp.path())).unwrap();
    let (pattern, _) = conv.matching("fix.note").expect("dotted pattern matches");
    assert_eq!(pattern, "fix.note");
}

/// The bracketed spelling READS as the declared ceiling — never as an empty
/// one. The second widening-catcher, at its post-ruling trigger.
///
/// It used to refuse (`invalid capability '[md.edit'`), and refusing was the
/// right shape while a bracket was unreadable here: an unread ceiling that
/// became the empty table would be the widening. The caps-one-parser law
/// made a flow sequence a LEGAL spelling of a cap list on both
/// planes, so the bracket is now read, and the invariant this test exists for
/// is satisfied more directly — the ceiling in force is the one the author
/// wrote. What must never happen, and is what is asserted, is `[md.edit]`
/// silently yielding an EMPTY ceiling, which admits nothing and so hides a
/// declaration rather than enforcing it.
#[test]
fn the_array_spelling_reads_as_its_ceiling_never_an_empty_one() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "run.caps.fix-*: [md.edit]\n");

    let (conv, _) = caps::load_conventions(Some(tmp.path())).expect("a flow sequence is legal");
    let (pattern, caps) = conv
        .matching("fix-drift")
        .expect("the entry is in the table, not silently absent");
    assert_eq!(pattern, "fix-*");
    assert_eq!(
        caps,
        &CapSet::parse("md.edit").unwrap(),
        "the bracketed spelling must yield the ceiling it names, never an empty one"
    );
}

/// A genuinely malformed entry still refuses loudly, naming the file AND the
/// key — the widening-catcher's other half, kept on the fault that motivated
/// the whole gate: a TRAILING YAML COMMENT, which the frontmatter scanner does
/// not strip, so `#` arrives as a capability and bricks the root.
///
/// Card cap-refusals-teach-legally, defect b, proven at the entry point that
/// actually knows the path: `load_conventions` is the only caller holding the
/// declaration's location, so this is where a bricked root either gets told
/// which file and key to fix, or does not.
#[test]
fn a_malformed_entry_refuses_loudly_naming_the_file_and_the_key() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "run.caps.fix-*: md.edit # longest wins\n");

    let err = caps::load_conventions(Some(tmp.path())).unwrap_err();
    assert!(
        matches!(err, CapsError::TableEntry { ref source, .. }
            if matches!(**source, CapsError::BadCap { .. })),
        "a malformed entry must refuse, never yield a silently empty ceiling: {err:?}"
    );

    let CapsError::TableEntry { path, key, .. } = &err else {
        unreachable!("asserted above")
    };
    assert_eq!(
        path.as_deref(),
        Some(tmp.path().join("MERIDIAN.md").as_path()),
        "the refusal names the declaration it refused"
    );
    assert_eq!(key, "run.caps.fix-*", "and the key whose value failed");
    let rendered = err.to_string();
    assert!(rendered.contains("MERIDIAN.md"), "{rendered}");
    assert!(rendered.contains("run.caps.fix-*"), "{rendered}");
}

/// A bare `run.caps.<pattern>:` is the EMPTY ceiling, never an absent entry.
/// Fail-closed: a key the author wrote and left blank must not read as "no
/// ceiling for this pattern", which is the widening this whole family guards.
#[test]
fn a_bare_convention_key_is_the_empty_ceiling_never_an_absent_entry() {
    let tmp = tempfile::tempdir().unwrap();
    declare(tmp.path(), "run.caps.fix-*:\n");

    let (conv, _) = caps::load_conventions(Some(tmp.path())).expect("a bare key is not a fault");
    let (pattern, caps) = conv
        .matching("fix-drift")
        .expect("the entry is present, not absent");
    assert_eq!(pattern, "fix-*");
    assert_eq!(
        caps,
        &CapSet::none(),
        "a bare ceiling key admits nothing — it never lifts the ceiling"
    );
}

/// The retired marker is INERT: a `.meridian.toml` carrying a full `[run.caps]`
/// table grants nothing. No fallback ships — a reader that still found it would
/// defeat the retirement.
#[test]
fn a_retired_marker_grants_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(".meridian.toml"),
        "version = 1\n\n[run.caps]\n\"fix-*\" = [\"md.set_field\", \"md.append_section\"]\n",
    )
    .unwrap();

    let (conv, source) = caps::load_conventions(Some(tmp.path())).unwrap();
    assert_eq!(conv, Conventions::none(), "the marker must grant nothing");
    assert_eq!(
        source,
        ConventionSource::Undeclared(tmp.path().to_path_buf())
    );

    // And it cannot ceiling anything either: an explicit grant passes whole,
    // because a retired file is not a policy plane.
    let explicit = set(&["md.edit", "md.create"]);
    let r = caps::resolve_caps("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, explicit);
}
