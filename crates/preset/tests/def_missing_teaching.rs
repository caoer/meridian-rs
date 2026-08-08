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
