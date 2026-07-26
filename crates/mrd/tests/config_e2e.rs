//! `mrd config` end to end, against the REAL binary (U6).
//!
//! This is the user-reachable surface criterion 1's evidence is measured on.
//! Two others were measured OUT before it was written, and neither was ruled
//! out by reading a `--help` line:
//!
//! - `mrd read` routes through the render face, which elides every
//!   `meridian-*` block — it prints the config's prose and none of its mounts.
//! - `mrd resolve` is the WORKSPACE-identity sense of the word: a path to a
//!   workspace and a cache drawer, no mount table.
//!
//! Every invocation carries its own `HOME` and `MERIDIAN_CONFIG` through
//! `Command::env`, so no test reads or writes the real ones and the four
//! resolution states are driven as STATES, through the process boundary.

use std::path::Path;
use std::process::{Command, Output};

const SINGLE: &str = "\
---
type: meridian-config
version: 1
---

# My system (one vault)

The wiki I write in.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```

```meridian-mount
name: archive
path: /Users/Shared/repos/archive
kind: git-folder
```
";

fn run(home: &Path, meridian_config: Option<&Path>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mrd"));
    cmd.arg("config")
        .args(args)
        .env("HOME", home)
        .env_remove("MERIDIAN_CONFIG");
    if let Some(path) = meridian_config {
        cmd.env("MERIDIAN_CONFIG", path);
    }
    cmd.output().expect("mrd runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The acceptance: the verb PUBLISHES the parsed mount table, in document
/// order, with the config's own rev and fingerprint beside it. Without this
/// half, every refusal assertion below is satisfied by a build that refuses
/// everything (S3-R8(c)).
#[test]
fn the_verb_publishes_the_parsed_mount_table_and_the_config_rev() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), SINGLE).expect("write");

    let out = run(home.path(), None, &[]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    let text = stdout(&out);

    assert!(text.contains("MERIDIAN.md  loaded"), "{text}");
    assert!(
        text.contains("file_rev:"),
        "the config carries a rev: {text}"
    );
    assert!(
        text.contains("fp:fp1."),
        "and a fingerprint like any page: {text}"
    );
    assert!(text.contains("mounts (2):"), "{text}");
    assert!(
        text.contains("field-notes  vault  /Users/Shared/projects/field-notes  vault:field-notes"),
        "the vault root, with its Obsidian vault name: {text}"
    );
    assert!(
        text.contains("archive  git-folder  /Users/Shared/repos/archive"),
        "the git-folder root, which carries no vault name: {text}"
    );
    assert!(
        text.contains("elided by the render face"),
        "the verb NAMES the elision it exists to work around — S3-R10(a): an elision that \
         renders authored content invisible must not do so silently: {text}"
    );

    // The rev is the STATE this verb reports, so an out-of-band edit moves what
    // it prints. Asserted as a state change (R40), never as an exit status.
    let before = rev_line(&text);
    std::fs::write(
        home.path().join("MERIDIAN.md"),
        SINGLE.replace("name: archive", "name: assets"),
    )
    .expect("edit");
    let after_out = run(home.path(), None, &[]);
    let after = stdout(&after_out);
    assert_eq!(after_out.status.code(), Some(0));
    assert_ne!(before, rev_line(&after), "the edit moved the reported rev");
    assert!(
        after.contains("assets  git-folder"),
        "and the parsed name: {after}"
    );
}

fn rev_line(text: &str) -> String {
    text.split_whitespace()
        .find(|t| t.starts_with("file_rev:"))
        .unwrap_or_default()
        .to_string()
}

/// State A: no config file. Not an error and not a warning — every machine
/// starts here, and the verb says so rather than printing a bare empty table a
/// reader would take for a failure.
#[test]
fn an_absent_config_exits_clean_and_says_what_absent_means() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = run(home.path(), None, &[]);
    assert_eq!(out.status.code(), Some(0), "absent is NOT an error");
    let text = stdout(&out);
    assert!(text.contains("MERIDIAN.md  absent"), "{text}");
    assert!(
        text.contains("single-root behaviour, unchanged"),
        "absent is named, not implied: {text}"
    );
    assert!(text.contains("mounts (0):"), "{text}");
    assert!(
        !text.contains("file_rev:"),
        "state A has no file, so no rev — the ONE permitted difference from state D: {text}"
    );
}

/// State B: the refusal rides to stderr VERBATIM, on exit 1, carrying the line,
/// the no-partial-load sentence and the fix — and stdout publishes nothing.
#[test]
fn a_malformed_config_refuses_on_exit_one_and_publishes_nothing() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        home.path().join("MERIDIAN.md"),
        SINGLE.replace(
            "kind: vault\nvault: field-notes",
            "kind: obsidian\nvault: field-notes",
        ),
    )
    .expect("write");

    let out = run(home.path(), None, &[]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a refusal is a finding, not a tool failure"
    );
    let err = stderr(&out);
    assert!(err.contains("`kind: obsidian` is not a root kind"), "{err}");
    assert!(err.contains(" line 13:"), "the refusal names WHERE: {err}");
    assert!(
        err.contains("No mount table was loaded; the config is not partially applied."),
        "{err}"
    );
    assert!(err.contains(" Fix: "), "{err}");
    assert!(
        stdout(&out).is_empty(),
        "a refused config publishes NO mount table: {}",
        stdout(&out)
    );
}

/// State C: `MERIDIAN_CONFIG` naming a path that is not a readable regular
/// file refuses — it never silently falls back to `~/MERIDIAN.md`. The home
/// default is a perfectly good config here, so a build that falls back looks
/// completely healthy; that is what makes this the load-bearing case.
#[test]
fn a_stated_path_that_cannot_be_honoured_never_falls_back() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), SINGLE).expect("write");
    let nowhere = home.path().join("nowhere").join("MERIDIAN.md");

    let out = run(home.path(), Some(&nowhere), &[]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains(&nowhere.display().to_string()),
        "names the path: {err}"
    );
    assert!(
        !stdout(&out).contains("field-notes"),
        "the default file's mount must NOT appear — that is the silent fallback this kills"
    );

    // The acceptance half in the same breath: a stated path that CAN be
    // honoured wins over the default.
    let elsewhere = home.path().join("elsewhere.md");
    std::fs::write(
        &elsewhere,
        SINGLE.replace("name: field-notes", "name: sessions"),
    )
    .expect("write");
    let out = run(home.path(), Some(&elsewhere), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("sessions  vault"), "{}", stdout(&out));
}

/// `--json` is the house grammar's machine face, and it carries the same facts
/// as the human one — a JSON surface that dropped the rev would make the
/// agent-facing plane blinder than the human-facing one.
#[test]
fn the_json_face_carries_the_same_facts() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), SINGLE).expect("write");

    let out = run(home.path(), None, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");

    assert_eq!(value["state"], "loaded");
    assert_eq!(value["file_rev"].as_str().expect("rev").len(), 16);
    assert!(
        value["fingerprint"]
            .as_str()
            .is_some_and(|f| f.starts_with("fp1.")),
        "{value}"
    );
    let mounts = value["mounts"].as_array().expect("mounts");
    assert_eq!(mounts.len(), 2);
    assert_eq!(mounts[0]["name"], "field-notes");
    assert_eq!(mounts[0]["kind"], "vault");
    assert_eq!(mounts[0]["vault"], "field-notes");
    assert!(mounts[0]["pin"].is_null());
    assert_eq!(mounts[1]["name"], "archive");
    assert_eq!(mounts[1]["kind"], "git-folder");
    assert!(
        mounts[1]["vault"].is_null(),
        "a git-folder root has no Obsidian vault leg"
    );
}

/// A positional argument or an unknown flag is an exit-2 tool failure, never
/// ignored.
#[test]
fn a_bad_invocation_is_exit_two() {
    let home = tempfile::tempdir().expect("tempdir");
    for args in [vec!["some/path.md"], vec!["--depth"]] {
        let out = run(home.path(), None, &args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`mrd config {args:?}` must be a tool failure: {}",
            stderr(&out)
        );
    }
}
