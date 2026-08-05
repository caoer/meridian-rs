//! The mount table end to end, against the REAL binary (U7). Criterion 2 — *"the mount table is
//! the single authority for the three-way translation … verified through a user-reachable
//! verb"* — is measured HERE, on `mrd config`, because that is the verb that publishes the
//! table. Two others were measured out before it: `mrd read` elides `meridian-*` blocks and
//! `mrd resolve` answers the workspace-identity sense of the word.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::path::Path;
use std::process::{Command, Output};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .arg("config")
        .args(args)
        .env("HOME", home)
        .env_remove("MERIDIAN_CONFIG")
        .output()
        .expect("mrd runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn write_config(home: &Path, body: &str) {
    std::fs::write(
        home.join("MERIDIAN.md"),
        format!("---\ntype: meridian-config\nversion: 1\n---\n\n# My system\n\n{body}"),
    )
    .expect("write the config");
}

fn vault_mount(name: &str, path: &Path, pin: Option<&str>) -> String {
    let mut block = format!(
        "```meridian-mount\nname: {name}\npath: {}\nkind: vault\nvault: {name}\n",
        path.display()
    );
    if let Some(pin) = pin {
        use std::fmt::Write as _;
        let _ = writeln!(block, "pin: {pin}");
    }
    block.push_str("```\n");
    block
}

fn make_root(home: &Path, name: &str) -> std::path::PathBuf {
    let root = home.join("roots").join(name);
    std::fs::create_dir_all(&root).expect("create the root");
    root
}

fn declare(root: &Path, name: &str) {
    std::fs::write(
        root.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
    )
    .expect("write the root declaration");
}

fn state_of(out: &Output) -> String {
    let value: serde_json::Value = serde_json::from_str(&stdout(out)).expect("json");
    value["mounts"][0]["state"]
        .as_str()
        .expect("a state word")
        .to_string()
}

/// **Gate 4 at the verb, both arms.** An undeclared root renders `grey(undeclared)` — in the
/// human line AND in `--json`, the same spelling — and it **refuses on exit 1**, per S3-R6.
/// Then the root declares itself, and the same config binds on exit 0. The assert is the
/// transition. A build that always greyed, and a build that always bound, each fail exactly one
/// half.
///
///
#[test]
fn an_undeclared_root_greys_on_exit_one_and_binds_once_it_declares() {
    let home = tempfile::tempdir().expect("tempdir");
    let root = make_root(home.path(), "field-notes");
    write_config(home.path(), &vault_mount("field-notes", &root, None));

    // Arm 1 — grey. The root exists and binds cleanly; it just says nothing
    // about its own name.
    let grey = run(home.path(), &[]);
    assert_eq!(
        grey.status.code(),
        Some(1),
        "grey REFUSES and rides exit 1 (S3-R6): {}",
        stderr(&grey)
    );
    let text = stdout(&grey);
    assert!(
        text.contains("grey(undeclared)"),
        "the reason word is in the human line: {text}"
    );
    assert!(
        text.contains(&root.join("MERIDIAN.md").display().to_string()),
        "the MISSING declaration is named by path: {text}"
    );
    assert!(
        !text.contains("file_not_found") && !text.contains("red("),
        "not red, not file_not_found — nothing drifted, the root just has not spoken: {text}"
    );
    assert!(
        stderr(&grey).contains("field-notes grey(undeclared)"),
        "the non-zero exit names WHICH root refuses: {}",
        stderr(&grey)
    );
    assert_eq!(
        state_of(&run(home.path(), &["--json"])),
        "grey(undeclared)",
        "`--json` carries the SAME word — two spellings is how a reader and an operator disagree"
    );

    // Arm 2 — the acceptance. One file appears, and the same config binds.
    declare(&root, "field-notes");
    let bound = run(home.path(), &[]);
    assert_eq!(
        bound.status.code(),
        Some(0),
        "a declared, matching root binds clean: {}",
        stderr(&bound)
    );
    assert!(stdout(&bound).contains("  bound"), "{}", stdout(&bound));
    assert_eq!(state_of(&run(home.path(), &["--json"])), "bound");
}

/// **Gate 1 at the verb.** A mount binding `$HOME` fails the WHOLE parse, and the verb
/// publishes **no table at all** — not a partial one with the legal root in it. The legal root
/// is declared first precisely so a partial build would have something to print.
///
#[test]
fn a_mount_binding_home_fails_the_whole_verb_and_publishes_no_table() {
    let home = tempfile::tempdir().expect("tempdir");
    let legal = make_root(home.path(), "field-notes");
    declare(&legal, "field-notes");
    write_config(
        home.path(),
        &format!(
            "{}{}",
            vault_mount("field-notes", &legal, None),
            vault_mount("poison", home.path(), None)
        ),
    );

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "a refusal is a finding");
    let err = stderr(&out);
    assert!(err.contains("home directory"), "the ceiling reason: {err}");
    assert!(
        err.contains("No mount table was loaded; the config is not partially applied."),
        "the no-partial-load clause: {err}"
    );
    assert!(err.contains(" Fix: "), "{err}");
    assert!(
        stdout(&out).is_empty(),
        "NO partial table survives — not even the legal root: {}",
        stdout(&out)
    );

    // The acceptance half: with the poisoned block gone, the same legal root binds and the table
    // publishes. So the refusal above is the ceiling, not the fixture.
    //
    write_config(home.path(), &vault_mount("field-notes", &legal, None));
    let clean = run(home.path(), &[]);
    assert_eq!(clean.status.code(), Some(0), "{}", stderr(&clean));
    assert!(stdout(&clean).contains("mounts (1):"));
}

/// **Gate 5 at the verb — mount-as-claim, end to end.** A mount pins the root it declares; the
/// verb reports it **bound** on exit 0.
///
///
///
///
///
///
#[test]
fn a_pinned_root_binds_and_reddens_when_its_declaration_drifts() {
    let home = tempfile::tempdir().expect("tempdir");
    let root = make_root(home.path(), "field-notes");
    declare(&root, "field-notes");

    // The pin is minted from the declarations own bytes by the shipped law — read back out of the
    // verb rather than hand-computed here, so the test cannot pin a value the engine would never
    // produce.
    let raw = std::fs::read_to_string(root.join("MERIDIAN.md")).expect("read");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let pin = model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the declaration has content")
        .into_string();

    write_config(home.path(), &vault_mount("field-notes", &root, Some(&pin)));
    let before = run(home.path(), &[]);
    assert_eq!(
        before.status.code(),
        Some(0),
        "a claim that verifies binds clean: {}",
        stderr(&before)
    );
    assert_eq!(state_of(&run(home.path(), &["--json"])), "bound");

    // Drift the root's declaration, keeping its declared NAME identical — so
    // what reddens is the claim and not the name check.
    std::fs::write(
        root.join("MERIDIAN.md"),
        format!("{raw}\nAn out-of-band edit to the root's own declaration.\n"),
    )
    .expect("edit the declaration");

    let after = run(home.path(), &[]);
    assert_eq!(
        after.status.code(),
        Some(1),
        "a drifted claim REFUSES, exactly as grey does: {}",
        stderr(&after)
    );
    let text = stdout(&after);
    assert!(
        text.contains("red(content-drifted)"),
        "the reason word is red, not grey — this is a measured disagreement, not the edge of sight: {text}"
    );
    assert!(
        text.contains(&pin),
        "the refusal names what was pinned: {text}"
    );
    assert_eq!(
        state_of(&run(home.path(), &["--json"])),
        "red(content-drifted)"
    );
    assert_ne!(
        before.status.code(),
        after.status.code(),
        "R40: the STATE changed — an exit that never moves reports nothing"
    );
}

/// **Gate 2 and gate 6 at the verb.** One tree bound twice under two names — reached through a
/// symlink, which is the measured topology on this machine — fails the whole parse. Then the
/// second mount is pointed at a genuinely different tree and the same config binds both.
///
#[test]
fn one_tree_under_two_names_refuses_and_two_trees_bind() {
    let home = tempfile::tempdir().expect("tempdir");
    let real = make_root(home.path(), "field-notes");
    declare(&real, "field-notes");
    let link = home.path().join("roots").join("link-to-field-notes");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    write_config(
        home.path(),
        &format!(
            "{}{}",
            vault_mount("field-notes", &real, None),
            vault_mount("repos", &link, None)
        ),
    );
    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("the same tree"),
        "the refusal says WHY two names are one tree: {err}"
    );
    assert!(
        stdout(&out).is_empty(),
        "no partial table: {}",
        stdout(&out)
    );

    // The acceptance half: two genuinely distinct trees bind under two names.
    let other = make_root(home.path(), "sessions");
    declare(&other, "sessions");
    write_config(
        home.path(),
        &format!(
            "{}{}",
            vault_mount("field-notes", &real, None),
            vault_mount("sessions", &other, None)
        ),
    );
    let clean = run(home.path(), &[]);
    assert_eq!(clean.status.code(), Some(0), "{}", stderr(&clean));
    assert!(stdout(&clean).contains("mounts (2):"), "{}", stdout(&clean));
}

/// **Gate 3 and gate 7 at the verb, together.** A declared-vs-bound mismatch fails loud and
/// names both spellings; an unseeable root greys and the table **stays loaded** around it. The
/// two are asserted in one test because the distinction between them is the point: one is a
/// statement the roots disagree about, the other is a root this machine simply cannot see.
///
#[test]
fn a_mismatch_fails_loud_while_an_unseeable_root_only_greys() {
    let home = tempfile::tempdir().expect("tempdir");
    let root = make_root(home.path(), "field-notes");
    declare(&root, "wiki");

    write_config(home.path(), &vault_mount("field-notes", &root, None));
    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("`field-notes`") && err.contains("`wiki`"),
        "both spellings are named — a silent pick would make stored links machine-dependent: {err}"
    );
    assert!(stdout(&out).is_empty(), "nothing loaded: {}", stdout(&out));

    // An unseeable root is the OTHER outcome entirely: the table loads, that one
    // root greys, and every other root keeps its own verdict.
    declare(&root, "field-notes");
    let missing = home.path().join("roots").join("not-checked-out");
    write_config(
        home.path(),
        &format!(
            "{}{}",
            vault_mount("field-notes", &root, None),
            vault_mount("archive", &missing, None)
        ),
    );
    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "grey refuses too (S3-R6)");
    let text = stdout(&out);
    assert!(
        text.contains("mounts (2):"),
        "the TABLE STAYS LOADED — one unseeable root is the topology working as designed: {text}"
    );
    assert!(text.contains("grey(path-unseeable)"), "{text}");
    assert!(
        text.contains("  bound"),
        "and the seeable root keeps its own bound verdict: {text}"
    );
    assert!(
        text.contains(&missing.display().to_string()),
        "the unseeable path is named: {text}"
    );
}
