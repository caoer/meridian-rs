//! The shared mint's bare-name fallback (ruling 2026-08-14, design (a-1)) and
//! its two MANDATORY guards, ratified as scope: the deterministic tie-break
//! (shortest path, then lexicographic) and case-EXACT matching. Plus the
//! deliberate boundary the same ruling fixed: a pathed spelling never falls
//! back — `git/GIT.base` written where only `sources/git/GIT.base` exists is
//! genuine rot and must KEEP reading as rot (the enumerated 7-rot set).

use std::path::Path;

use fs::WorkspaceRoot;
use fs::domain::{Domain, ExclusionReason, LinkTargetProbe};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// A bare name resolves over out-of-domain files by exact basename and stamps
/// the §12.1 word — the ~495-row arm on the field-notes corpus.
#[test]
fn a_bare_name_falls_back_to_an_out_of_domain_basename() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "bases/TAG-FILES.base", "views\n");
    write(tmp.path(), "notes/plan.md", "# Plan\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.resolution("TAG-FILES.base"),
        Some((
            "bases/TAG-FILES.base".to_owned(),
            ExclusionReason::NonMarkdown
        )),
        "the bare name resolves to the subfolder file and carries its reason",
    );
    assert_eq!(
        probe.exclusion("TAG-FILES.base"),
        Some(ExclusionReason::NonMarkdown),
    );
}

/// A pathed spelling NEVER falls back. `git/GIT.base` is suffix-reachable
/// (`sources/git/GIT.base` ends with it, exact case) — and stays bare anyway:
/// stamping it would erase three of the enumerated 7 rot rows, the set whose
/// preservation is why the LIKE-filter alternative was rejected.
#[test]
fn a_pathed_spelling_never_falls_back() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "sources/git/GIT.base", "git views\n");
    write(tmp.path(), "bases/GIT.base", "git views\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.exclusion("git/GIT.base"),
        None,
        "a pathed spelling answers by the literal probe alone — rot stays rot",
    );
    // Positive control: the literal arm still answers a true pathed spelling.
    assert_eq!(
        probe.exclusion("bases/GIT.base"),
        Some(ExclusionReason::NonMarkdown),
        "the literal arm is untouched by the fallback's no-suffix rule",
    );
}

/// MANDATORY GUARD 1 — the ambiguous-basename tie-break is deterministic:
/// shortest path first, then lexicographic. Without it the stamp (and any
/// face that renders the resolved path) would be walk-order-sensitive.
#[test]
fn the_tie_break_is_shortest_path_then_lexicographic() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "bases/GIT.base", "one\n");
    write(tmp.path(), "sources/git/GIT.base", "two\n");
    // Same byte length ⇒ the lexicographic leg decides.
    write(tmp.path(), "b/X.base", "b\n");
    write(tmp.path(), "a/X.base", "a\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.resolution("GIT.base"),
        Some(("bases/GIT.base".to_owned(), ExclusionReason::NonMarkdown)),
        "shortest path wins the ambiguity",
    );
    assert_eq!(
        probe.resolution("X.base"),
        Some(("a/X.base".to_owned(), ExclusionReason::NonMarkdown)),
        "equal length falls to lexicographic order",
    );
}

/// MANDATORY GUARD 2 — case-EXACT matching. The fallback compares the string
/// the author wrote against the name on disk; an APFS case-folding probe
/// would stamp `abc.BASE` (a genuine typo) as deliberate.
#[test]
fn the_fallback_matches_case_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "bases/abc.base", "views\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.exclusion("abc.BASE"),
        None,
        "abc.BASE must NOT match abc.base — a typo carries no reason",
    );
    // Positive control: the exact spelling stamps.
    assert_eq!(
        probe.exclusion("abc.base"),
        Some(ExclusionReason::NonMarkdown)
    );
}

/// The `.md` append rule rides the fallback exactly as it rides the literal
/// probe: an extension-less bare name can name a custom-ignored page.
#[test]
fn the_md_append_rule_reaches_a_custom_ignored_page() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "drafts/hidden.md", "# Hidden\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::from_config("ignore:\n  - \"drafts/**\"\n");
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.resolution("hidden"),
        Some(("drafts/hidden.md".to_owned(), ExclusionReason::CustomIgnore)),
        "hidden -> hidden.md -> the ignored page, with the custom-ignore word",
    );
}

/// Dot-segment files are never fallback candidates: they are invisible to the
/// vault the way Obsidian's own index treats them, and indexing them would
/// walk `.git`. The LITERAL probe still answers a dot spelling written out.
#[test]
fn dot_segment_files_are_never_fallback_candidates() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".obsidian/workspace.base", "layout\n");
    write(tmp.path(), ".obsidian/snippets.md", "# Snippets\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.exclusion("workspace.base"),
        None,
        "a dot-dir file is not a fallback candidate",
    );
    assert_eq!(
        probe.exclusion("snippets"),
        None,
        "the md-append arm does not reach into dot dirs either",
    );
    // The literal arm still classifies a written-out dot path — §12.1 order:
    // the md-only floor fires first, so only an md file can read dot-segment.
    assert_eq!(
        probe.exclusion(".obsidian/snippets.md"),
        Some(ExclusionReason::DotSegment),
        "the literal arm still classifies the written-out dot path",
    );
}

/// The literal arm is byte-for-byte the old mint: a root-level real file
/// stamps, a missing file stays bare (a genuine typo carries no reason).
#[test]
fn the_literal_arm_is_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "icon.svg", "<svg/>\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let probe = LinkTargetProbe::new(&root, &domain);

    assert_eq!(
        probe.exclusion("icon.svg"),
        Some(ExclusionReason::NonMarkdown)
    );
    assert_eq!(probe.exclusion("missing.svg"), None);
    assert_eq!(
        probe.exclusion("roadmap"),
        None,
        "a broken note link stays bare"
    );
}
