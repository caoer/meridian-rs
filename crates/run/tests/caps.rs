//! Capability resolution gates — deny-by-default, convention narrowing,
//! check-*/verify-* bash refusal, and source reporting, in the three-verb
//! glob-scoped grammar (caps-redesign ruling, 2026-08-19): verbs
//! `md.create`/`md.edit`/`md.delete`, optional path-glob scope matched
//! against declared coordinates, legacy per-op spellings folding (bare) or
//! refusing (targeted).

mod support;

use std::collections::BTreeSet;

use run::caps::{self, Authority, Cap, CapResolution, CapSet, CapSource, CapsError, Conventions};
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

fn resolve(task: &str, explicit: Option<&CapSet>, conv: &Conventions) -> CapResolution {
    caps::resolve_caps(task, explicit, conv)
}

#[test]
fn undeclared_block_is_read_only_deny_by_default() {
    let r = resolve("anything", None, &Conventions::none());
    assert_eq!(r.source, CapSource::DenyDefault);
    assert_eq!(r.effective, CapSet::none());
    assert!(r.narrowed.is_empty());
    assert!(!r.effective.admits("md.edit", Some("page.md")));
}

#[test]
fn explicit_frontmatter_beats_convention_as_grant_source() {
    let conv = conventions(&[("fix-*", &["md.edit", "md.create"])]);
    let explicit = set(&["md.edit:tasks/*.md"]);
    let r = resolve("fix-drift", Some(&explicit), &conv);
    assert_eq!(r.source, CapSource::Explicit);
    // The explicit grant is inside the convention ceiling — survives intact.
    assert_eq!(r.effective, explicit);
    assert!(r.narrowed.is_empty());
}

#[test]
fn convention_grants_when_no_explicit_declaration() {
    let conv = conventions(&[("fix-*", &["md.edit"])]);
    let r = resolve("fix-drift", None, &conv);
    assert_eq!(r.source, CapSource::Convention("fix-*".to_owned()));
    assert!(r.effective.admits("md.edit", Some("page.md")));
    assert!(!r.effective.admits("md.create", Some("tasks/x.md")));
}

#[test]
fn convention_narrows_an_explicit_grant_never_widens() {
    // Explicit grants MORE than the matching convention allows: the ceiling
    // drops the excess and the narrowing is visible, never silent.
    let conv = conventions(&[("fix-*", &["md.edit"])]);
    let explicit = set(&["md.edit", "md.create"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.source, CapSource::Explicit);
    assert_eq!(r.effective, set(&["md.edit"]));
    assert_eq!(r.narrowed, vec![Cap::parse("md.create").unwrap()]);
}

#[test]
fn scoped_ceiling_tightens_an_unscoped_grant() {
    // Grant `md.edit` (any path) under ceiling `md.edit:tasks/*.md`: the
    // effective cap is the SCOPED one — narrower, and reported.
    let conv = conventions(&[("fix-*", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, set(&["md.edit:tasks/*.md"]));
    assert!(r.effective.admits("md.edit", Some("tasks/a.md")));
    assert!(!r.effective.admits("md.edit", Some("notes/a.md")));
    assert_eq!(r.narrowed, vec![Cap::parse("md.edit").unwrap()]);
}

// ── the subsumption meet (cap-meet-subsumption ruling, 2026-08-20) ─────────
// Scopes meet by glob CONTAINMENT (`policy::glob_subsumes`), not string
// equality: nested survives, wider tightens, non-nested drops.

#[test]
fn a_strictly_narrower_scope_survives_a_differently_spelled_ceiling() {
    // The card's case: ceiling `tasks/*.md`, grant `tasks/foo.md` —
    // semantically inside, spelled differently. Survives INTACT, unreported.
    let conv = conventions(&[("fix-*", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit:tasks/foo.md"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, explicit);
    assert!(r.narrowed.is_empty());
    assert!(r.effective.admits("md.edit", Some("tasks/foo.md")));
    assert!(!r.effective.admits("md.edit", Some("tasks/bar.md")));
}

#[test]
fn an_identically_spelled_scope_survives_intact() {
    let conv = conventions(&[("fix-*", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit:tasks/*.md"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, explicit);
    assert!(r.narrowed.is_empty());
}

#[test]
fn a_scope_wider_than_the_ceiling_tightens_to_the_ceiling() {
    // Grant `tasks/**` under ceiling `tasks/*.md`: the meet is the ceiling's
    // scope — narrower, and the tightening is reported, never silent.
    let conv = conventions(&[("fix-*", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit:tasks/**"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, set(&["md.edit:tasks/*.md"]));
    assert_eq!(r.narrowed, vec![Cap::parse("md.edit:tasks/**").unwrap()]);
    assert!(!r.effective.admits("md.edit", Some("tasks/sub/a.md")));
}

#[test]
fn a_disjoint_scope_is_dropped() {
    let conv = conventions(&[("fix-*", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit:notes/*.md"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, CapSet::none());
    assert_eq!(r.narrowed, vec![Cap::parse("md.edit:notes/*.md").unwrap()]);
}

#[test]
fn overlapping_but_non_nested_scopes_drop_because_overlap_is_not_nesting() {
    // `tasks/*.md` and `*/foo.md` both admit `tasks/foo.md`, but neither
    // contains the other — the meet has no simple normal form, so the grant
    // drops (conservative: narrow only, never widen) and the drop is visible.
    let conv = conventions(&[("fix-*", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit:*/foo.md"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, CapSet::none());
    assert_eq!(r.narrowed, vec![Cap::parse("md.edit:*/foo.md").unwrap()]);
    assert!(!r.effective.admits("md.edit", Some("tasks/foo.md")));
}

#[test]
fn a_deep_scope_survives_a_double_star_ceiling() {
    // run-plane.md's measured 2026-08-19 miss, now the fixed case:
    // `tasks/sub/*.md` sits plainly inside `tasks/**` and survives.
    let conv = conventions(&[("fix-*", &["md.edit:tasks/**"])]);
    let explicit = set(&["md.edit:tasks/sub/*.md"]);
    let r = resolve("fix-x", Some(&explicit), &conv);
    assert_eq!(r.effective, explicit);
    assert!(r.narrowed.is_empty());
}

#[test]
fn check_and_verify_names_refuse_a_bash_fence_loudly() {
    // A NAME law, not a capability: it survives the bash amendment, and it
    // runs BEFORE the short-circuit below.
    let d = doc(support::PAGE);
    for task in ["check-links", "verify-roots"] {
        let err = caps::resolve_authority(&d, task, TaskLanguage::Bash, &Conventions::none())
            .unwrap_err();
        assert!(
            matches!(err, CapsError::BashFenceRefused { .. }),
            "{task}: {err:?}"
        );
    }
}

/// Under `docs/laws.md` § Amendment there is no source to resolve — a bash
/// task that clears the `check-*` name law is `Unsandboxed`, and its
/// declaration is not read at all.
#[test]
fn a_bash_task_resolves_no_capability_at_all() {
    let d = doc(support::PAGE);
    // `fix-*` blocks DECLARE writes and are exactly where bash is wanted — the
    // load refusal is check-*/verify-* ONLY (ruling 3).
    let conv = conventions(&[("fix-*", &["md.edit"])]);
    let authority = caps::resolve_authority(&d, "fix-wiki", TaskLanguage::Bash, &conv).unwrap();
    assert_eq!(authority, Authority::Unsandboxed);
    // Not a grant of everything — the absence of a gate. Nothing to report.
    assert_eq!(authority.capabilities(), None);
    assert!(authority.admits("md.edit", Some("page.md")));
}

#[test]
fn builtin_read_only_ceiling_zeroes_even_explicit_caps() {
    // A check-* block with explicit write caps stays read-only: the builtin
    // ceiling is absolute, and the dropped caps are reported.
    let explicit = set(&["md.edit", "md.create"]);
    let r = resolve("check-links", Some(&explicit), &Conventions::none());
    assert_eq!(r.effective, CapSet::none());
    assert_eq!(r.narrowed.len(), 2);
}

#[test]
fn longest_pattern_wins() {
    let conv = conventions(&[("fix-*", &["md.edit"]), ("fix-drift", &["md.create"])]);
    let (pattern, _) = conv.matching("fix-drift").unwrap();
    assert_eq!(pattern, "fix-drift");
    let (pattern, _) = conv.matching("fix-other").unwrap();
    assert_eq!(pattern, "fix-*");
}

#[test]
fn cap_strings_are_the_three_verbs_optionally_glob_scoped() {
    let plain = Cap::parse("md.edit").unwrap();
    assert_eq!(plain.kind, "md.edit");
    assert_eq!(plain.target, None);

    let scoped = Cap::parse("md.create:tasks/*.md").unwrap();
    assert_eq!(scoped.target.as_deref(), Some("tasks/*.md"));
    assert_eq!(scoped.as_string(), "md.create:tasks/*.md");

    // The reserved verb parses today — grants can be written ahead of the
    // retire descriptor.
    let delete = Cap::parse("md.delete:agents/*/memos/**").unwrap();
    assert_eq!(delete.kind, "md.delete");

    for bad in [
        "md",
        "edit",
        "MD.edit",
        "md.set field",
        "md.:x",
        "md.edit:",
        "md.rename",           // no such verb
        "daemon.refresh_view", // descriptor kinds are not cap verbs
    ] {
        assert!(Cap::parse(bad).is_err(), "{bad} must refuse");
    }
}

/// The migration fold (caps-redesign ruling): BARE legacy per-op spellings
/// fold into `md.edit` — live grants keep working across the cutover, and
/// every surface reports the canonical verb. The two spellings land on ONE
/// cap: a set naming both dedupes to `md.edit`.
#[test]
fn bare_legacy_verbs_fold_into_md_edit() {
    for legacy in ["md.set_field", "md.append_section"] {
        let cap = Cap::parse(legacy).unwrap();
        assert_eq!(cap.kind, "md.edit", "{legacy} folds");
        assert_eq!(cap.target, None);
        assert_eq!(cap.as_string(), "md.edit", "reported canonically");
    }
    let folded = CapSet::parse("md.set_field, md.append_section").unwrap();
    assert_eq!(folded, set(&["md.edit"]), "one cap after the fold");
    assert!(folded.admits("md.edit", Some("anything/at/all.md")));
}

/// The migration refusal (the ruled split): TARGETED legacy spellings refuse
/// with the retirement teaching — the old target named a field, the new
/// target position is a path glob, and neither silent reinterpretation
/// (drop the target: widens; read it as a path: dead grant) is legal.
#[test]
fn targeted_legacy_verbs_refuse_with_the_retirement_teaching() {
    for retired in ["md.set_field:status", "md.append_section:Log"] {
        let err = Cap::parse(retired).unwrap_err();
        assert!(
            matches!(err, CapsError::RetiredTarget { ref raw } if raw == retired),
            "{retired}: {err:?}"
        );
        let m = err.to_string();
        assert!(m.contains("retired field-grain form"), "{m}");
        assert!(m.contains("md.edit"), "teaches the fold target: {m}");
        assert!(m.contains("PATH GLOB"), "teaches the new grammar: {m}");
    }
}

/// Bad globs refuse at parse, each naming its fault: caps are workspace-
/// shaped paths, never navigated, never absolute.
#[test]
fn malformed_glob_scopes_refuse_at_parse() {
    for (bad, why) in [
        ("md.edit:/abs/x.md", "empty segment"),
        ("md.edit:a//b.md", "empty segment"),
        ("md.edit:tasks/", "empty segment"),
        ("md.edit:../x.md", "`..` segment"),
        ("md.edit:./x.md", "`.` segment"),
        ("md.create:tasks/a b.md", "carries ` `"),
    ] {
        let err = Cap::parse(bad).unwrap_err();
        assert!(
            matches!(err, CapsError::BadGlob { .. }),
            "{bad} must refuse as a bad glob: {err:?}"
        );
        let m = err.to_string();
        assert!(m.contains(why), "{bad}: fault named — {m}");
    }
}

/// Glob semantics ride [`policy::glob_match`] — the one grammar: `*` stays
/// inside a segment (the jail the ruling names), `**` spans segments, an
/// unscoped cap admits every path of its verb, and a scoped cap answers no
/// path-less query.
#[test]
fn glob_scopes_match_the_one_grammar() {
    let scoped = Cap::parse("md.create:tasks/*.md").unwrap();
    assert!(scoped.admits("md.create", Some("tasks/a.md")));
    assert!(
        !scoped.admits("md.create", Some("tasks/sub/a.md")),
        "* stays in-segment"
    );
    assert!(
        !scoped.admits("md.create", Some("evil/tasks/a.md")),
        "a declared path under an extra head segment must NOT match (the jail case)"
    );
    assert!(
        !scoped.admits("md.edit", Some("tasks/a.md")),
        "verbs never cross"
    );
    assert!(
        !scoped.admits("md.create", None),
        "a scoped cap answers no path-less query"
    );

    let deep = Cap::parse("md.edit:agents/**/CARD.md").unwrap();
    assert!(deep.admits("md.edit", Some("agents/ab12/CARD.md")));
    assert!(deep.admits("md.edit", Some("agents/a/b/CARD.md")));
    assert!(!deep.admits("md.edit", Some("agents/ab12/PULSE.md")));

    let unscoped = Cap::parse("md.edit").unwrap();
    assert!(unscoped.admits("md.edit", Some("anywhere/at/all.md")));
    assert!(unscoped.admits("md.edit", None));
}

#[test]
fn explicit_caps_reads_the_task_caps_key() {
    let d = doc(support::PAGE);
    let caps = caps::explicit_caps(&d, "fix-drift").unwrap().unwrap();
    assert!(caps.admits("md.edit", Some("page.md")));
    assert!(caps.admits("md.create", Some("tasks/x.md")));
    assert!(!caps.admits("md.create", Some("notes/x.md")));

    assert_eq!(caps::explicit_caps(&d, "check-links").unwrap(), None);
}

#[test]
fn empty_explicit_declaration_is_explicit_read_only() {
    let page = "---\ntask.t: \"[[#^a-1]]\"\ntask.t.caps: \"\"\n---\n";
    let d = doc(page);
    let explicit = caps::explicit_caps(&d, "t").unwrap();
    assert_eq!(explicit, Some(CapSet(BTreeSet::new())));
    let r = resolve("t", explicit.as_ref(), &Conventions::none());
    assert_eq!(r.source, CapSource::Explicit);
    assert_eq!(r.effective, CapSet::none());
}

// The convention plane (where the table is declared) is tested in
// `caps_home.rs`; here is the resolution law those tables feed.

#[test]
fn conventions_parse_out_of_a_declaration_document() {
    let d = doc(
        "---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: md.edit, md.create:tasks/*.md\n---\n",
    );
    let conv = caps::conventions_from_declaration(&d, None).unwrap();
    let (pattern, set) = conv.matching("fix-drift").unwrap();
    assert_eq!(pattern, "fix-*");
    assert!(set.admits("md.edit", None));
    assert!(set.admits("md.create", Some("tasks/x.md")));
}

#[test]
fn a_declaration_without_caps_keys_is_the_empty_table() {
    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\n---\n");
    assert_eq!(
        caps::conventions_from_declaration(&d, None).unwrap(),
        Conventions::none()
    );
}

#[test]
fn a_malformed_cap_entry_is_a_loud_error_never_no_policy() {
    let d =
        doc("---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: not_namespaced\n---\n");
    assert!(matches!(
        caps::conventions_from_declaration(&d, None).unwrap_err(),
        CapsError::TableEntry { ref source, .. } if matches!(**source, CapsError::BadCap { .. })
    ));

    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fi*x: md.edit\n---\n");
    assert!(matches!(
        caps::conventions_from_declaration(&d, None).unwrap_err(),
        CapsError::BadPattern { .. }
    ));

    // A retired field-grain entry in the TABLE is the same loud refusal a
    // page declaration gets — a mis-spelled ceiling is reported, never read
    // as an absent one.
    let d = doc(
        "---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: md.set_field:status\n---\n",
    );
    assert!(matches!(
        caps::conventions_from_declaration(&d, None).unwrap_err(),
        CapsError::TableEntry { ref source, .. }
            if matches!(**source, CapsError::RetiredTarget { .. })
    ));
}

// ── Refusals must teach legally (card cap-refusals-teach-legally) ──────────

/// DEFECT b, closed: a poisoned convention table refuses with the DECLARATION
/// PATH and the offending KEY. One bad entry bricks the whole root by design,
/// so a refusal naming neither left an operator with every task refusing and
/// nothing to grep for. Probed on `ad547a7c2`: the operator saw
/// `invalid capability '#'` and no file.
///
/// The `#` is not a contrived value — it is what the frontmatter scanner
/// yields for a TRAILING YAML COMMENT, which it does not strip. Copy-pasting
/// a documented example carrying `# longest pattern wins` bricks a root
/// (round-1 finding #1), so this is the exact byte an operator hits.
#[test]
fn a_poisoned_convention_table_names_the_file_and_the_key() {
    let d = doc(
        "---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: md.edit # longest wins\n---\n",
    );
    let declaration = std::path::Path::new("/ws/MERIDIAN.md");
    let err = caps::conventions_from_declaration(&d, Some(declaration)).unwrap_err();

    let CapsError::TableEntry { path, key, source } = &err else {
        panic!("a poisoned table entry must carry its location: {err:?}");
    };
    assert_eq!(path.as_deref(), Some(declaration), "the file is named");
    assert_eq!(key, "run.caps.fix-*", "the offending key is named");
    assert!(
        matches!(**source, CapsError::BadCap { ref raw } if raw == "#"),
        "the inner fault still says WHAT is wrong: {source:?}"
    );

    // The rendered refusal is what the operator actually reads.
    let rendered = err.to_string();
    assert!(rendered.contains("/ws/MERIDIAN.md"), "{rendered}");
    assert!(rendered.contains("run.caps.fix-*"), "{rendered}");
    assert!(
        rendered.contains("WHOLE convention table"),
        "the blast radius is stated: {rendered}"
    );
    assert!(
        rendered.contains("TRAILING YAML COMMENT"),
        "the frequent cause is named: {rendered}"
    );
}

/// The path is carried, not invented: a caller that parsed a declaration it
/// never read from disk gets the same fault minus the path, never a fabricated
/// one.
#[test]
fn a_pathless_caller_gets_the_fault_without_a_fabricated_path() {
    let d = doc("---\ntype: meridian-root\nversion: 1\nname: r\nrun.caps.fix-*: nope\n---\n");
    let err = caps::conventions_from_declaration(&d, None).unwrap_err();
    let CapsError::TableEntry { path, key, .. } = &err else {
        panic!("expected TableEntry, got {err:?}");
    };
    assert!(path.is_none(), "no path is invented");
    assert_eq!(key, "run.caps.fix-*");
    assert!(err.to_string().contains("run.caps.fix-*"));
}

/// A denial names the ceiling that ate the grant, and ONLY where a ceiling is
/// what ate it (run-plane § capabilities, dogfood s12-50). A caller whose own
/// page declares `md.edit` and is denied `md.edit` would otherwise derive the
/// one remedy already in place.
#[test]
fn a_ceiling_narrowed_denial_names_the_ceiling_and_an_unnarrowed_one_does_not() {
    let conventions = conventions(&[("fix-note", &["md.edit:tasks/*.md"])]);
    let explicit = set(&["md.edit", "md.create"]);

    let narrowed = caps::resolve_caps("fix-note", Some(&explicit), &conventions);
    let ceiling = narrowed
        .ceiling_denying("md.edit", Some("notes/owner.md"))
        .expect("the convention ceiling took the wide grant");
    assert!(
        ceiling.to_string().contains("run.caps.fix-note"),
        "names the winning pattern: {ceiling}"
    );

    // Same descriptor, no ceiling in force: the grant simply never held it, so
    // there is no measured cause and the refusal teaches no fix.
    let scoped = set(&["md.edit:tasks/*.md"]);
    let unnarrowed = caps::resolve_caps("plain", Some(&scoped), &Conventions::none());
    assert!(
        unnarrowed
            .ceiling_denying("md.edit", Some("notes/owner.md"))
            .is_none(),
        "no ceiling is a measured absence, never an unknown"
    );
}
