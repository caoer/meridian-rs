//! Dogfood 2026-08-08, P3-d (card `p2-dogfood-refusal-teaching`): a missing
//! preset def refused with "cannot read the def: No such file or directory
//! (os error 2)" — which file, looked for where, unsaid. The refusal must name
//! the def page it wanted, the one path it searched, and the resolution rule
//! that produced it. Exit taxonomy and error variants stay frozen — teaching
//! text only.

use preset::load_def;

#[test]
fn missing_def_names_the_page_the_searched_path_and_the_resolution_rule() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));

    let err = load_def(&root, "presets/task.md").expect_err("a preset-less root refuses");
    let m = err.to_string();

    // The def page it wanted, as the caller's token resolved.
    assert!(m.contains("presets/task.md"), "names the def page: {m}");
    // The one absolute path it searched.
    let searched = root.0.join("presets/task.md");
    assert!(
        m.contains(searched.to_str().expect("utf8")),
        "names the searched path: {m}"
    );
    // The resolution rule, so `mrd new task t1` explains where `presets/task.md`
    // came from and how to aim elsewhere.
    assert!(
        m.contains("presets/<kind>.md"),
        "teaches the bare-kind resolution rule: {m}"
    );
}

/// The anchor rule rides the missing-`^properties` refusal (run-plane § presets,
/// dogfood s13-20): the loader finds the block by its `^` id ON the heading
/// line, so a def with a visually complete `# Properties` heading and no id
/// declares no block — and the refusal must say which byte is absent instead of
/// telling the author that a heading they are looking at does not exist.
#[test]
fn the_missing_properties_refusal_teaches_the_anchor_on_the_heading_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));
    std::fs::create_dir_all(root.0.join("presets")).expect("presets dir");

    let def = |body: &str| {
        format!("---\ntype: def\ndefines: session\nbirths: s/{{{{id}}}}.md\n---\n{body}")
    };
    std::fs::write(
        root.0.join("presets/noanchor.md"),
        def("# Properties\n\n- title\n\n# Template ^template\n\n```\n# x\n```\n"),
    )
    .expect("write");
    std::fs::write(
        root.0.join("presets/nothing.md"),
        def("# Template ^template\n\n```\n# x\n```\n"),
    )
    .expect("write");

    let refusal =
        |page: &str| match preset::new_record(&root, page, "x1", &preset::BirthOptions::default())
            .expect("a def defect is a refusal, not a tool failure")
        {
            preset::NewOutcome::Refused(r) => format!("{:?}", r.reason),
            preset::NewOutcome::Born(_) => panic!("an invalid def must not birth"),
        };

    // Both arms carry the RULE — it is always true of the loader.
    for page in ["presets/noanchor.md", "presets/nothing.md"] {
        let m = refusal(page);
        assert!(
            m.contains("^properties") && m.contains("heading line"),
            "{page} states the anchor rule: {m}"
        );
    }
    // Only the MEASURED arm carries the diagnosis of an anchor-less heading.
    let anchorless = refusal("presets/noanchor.md");
    assert!(
        anchorless.contains("HAS a `# Properties` heading"),
        "the measured arm names the offending heading: {anchorless}"
    );
    let absent = refusal("presets/nothing.md");
    assert!(
        !absent.contains("HAS a `# Properties` heading"),
        "a def with no such heading is never told it has one: {absent}"
    );
}
