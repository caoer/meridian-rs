//! G12 over a stored cross-root link: a sections body that holds one is served
//! read back into the agent plane, and the head declares the length served.
//!
//! Its own binary by design: a stored link reads back only under a bound mount
//! table, which the engine takes from `MERIDIAN_CONFIG`, and that is
//! process-global (the `wire-serve/tests/pin_cross_root.rs` precedent). This
//! binary holds one test, so the env is set before any thread exists.

mod daemon_door;
#[path = "g12_declared_bytes/reader.rs"]
mod reader;

use reader::{bodies_by_declared_length, render_face};
use serde_json::json;

/// The canonical stored form of `[[sessions:notes.md#Design]]` under the vault
/// name the mount table below declares.
const STORED: &str = "[sessions:notes.md#Design](obsidian://advanced-uri\
                      ?vault=field-notes-sessions&filepath=notes.md&heading=Design)";
/// A hand-edited URI to the same bound vault that does not decode: the face
/// leaves it as written.
const AS_WRITTEN: &str =
    "[x](obsidian://adv-uri?vault=field-notes-sessions&filepath=notes.md&heading=Design)";

/// Bind `sessions` (vault `field-notes-sessions`) in a machine config kept for
/// the life of the process, and point `MERIDIAN_CONFIG` at it.
fn bind_sessions_mount() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).expect("mkdir");
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("root declaration");
    std::fs::write(sessions.join("notes.md"), "# Notes\n\n## Design\n\nnote.\n").expect("target");
    let config = dir.join("MERIDIAN.md");
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: sessions\npath: {}\nvault: field-notes-sessions\n```\n",
            sessions.display()
        ),
    )
    .expect("mount table");
    // SAFETY: the only test in this binary, and it sets the env before it
    // starts any thread.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config) };
}

/// Two sections in one read, the first holding the link, so a reader that cuts
/// the first body by its declared length lands on the second body's marker.
#[test]
fn a_body_holding_a_stored_link_declares_the_length_it_serves() {
    bind_sessions_mount();
    for (case, link, served) in [
        ("read-back", STORED, "[[sessions:notes.md#Design]]"),
        ("as-written", AS_WRITTEN, AS_WRITTEN),
    ] {
        let page = format!("# Page\n\n## Linked\n\nsee {link} here\n\n## After\n\nafter body\n");
        let text = render_face(
            "page.md",
            &page,
            &json!({
                "id": 1, "op": "read", "path": "corpus/page.md",
                "sections": [{"hpath": [{"h": "Page"}, {"h": "Linked"}]},
                             {"hpath": [{"h": "Page"}, {"h": "After"}]}]
            }),
        );
        let bodies = bodies_by_declared_length(case, &text);
        assert_eq!(bodies.len(), 2, "[{case}] two bodies: {text}");
        assert!(
            bodies[0].contains(&format!("see {served} here")),
            "[{case}] the first body serves the link as {served}: {:?}",
            bodies[0]
        );
        assert_eq!(
            bodies[1], "\nafter body\n",
            "[{case}] the second body, whole"
        );
    }
}
