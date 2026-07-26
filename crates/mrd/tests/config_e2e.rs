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

/// The two-root config, over roots that live INSIDE the sandbox and really
/// declare themselves.
///
/// **Changed by U7, stated rather than absorbed.** This fixture used to name
/// `/Users/Shared/projects/field-notes` and `/Users/Shared/repos/archive`
/// literally. Binding now canonicalizes every mount path, passes it through the
/// `workspace` deny ceiling, and reads each root's own declaration — so absolute
/// machine paths made the verb's exit depend on what happened to be checked out
/// on the runner, and both roots refused (`grey(undeclared)`,
/// `grey(path-unseeable)`). Sandbox roots keep every assertion below measuring
/// what it was written to measure, and remove a machine dependency that was
/// always latent.
///
/// The LINE LAYOUT is unchanged on purpose: `line 13` is asserted verbatim by
/// [`a_malformed_config_refuses_on_exit_one_and_publishes_nothing`], so only the
/// path values differ from the original.
fn single(home: &Path) -> String {
    single_named(home, "field-notes", "archive")
}

/// The same fixture with either root renamed — and the root's own declaration
/// renamed with it, since a canonical name the root does not declare is a
/// declared-vs-bound mismatch and fails loud by design (D7).
fn single_named(home: &Path, vault_name: &str, folder_name: &str) -> String {
    let vault = declare_root(home, vault_name);
    let folder = declare_root(home, folder_name);
    format!(
        "\
---
type: meridian-config
version: 1
---

# My system (one vault)

The wiki I write in.

```meridian-mount
name: {vault_name}
path: {}
kind: vault
vault: {vault_name}
```

```meridian-mount
name: {folder_name}
path: {}
kind: git-folder
```
",
        vault.display(),
        folder.display()
    )
}

/// Create a root under the sandbox and write its self-declaration.
fn declare_root(home: &Path, name: &str) -> std::path::PathBuf {
    let root = home.join("roots").join(name);
    std::fs::create_dir_all(&root).expect("create the root");
    std::fs::write(
        root.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
    )
    .expect("write the root declaration");
    root
}

fn run(home: &Path, meridian_config: Option<&Path>, args: &[&str]) -> Output {
    run_bridged(home, meridian_config, args, None, None)
}

/// The same invocation with the two BRIDGED variables driven explicitly (U9).
///
/// `run` clears both, so every other test in this file measures a determinate
/// `unset` bridge rather than inheriting whatever the runner exports — the
/// developer machine really does export `CCC_LLM_WIKI_PATH`, which would
/// otherwise make the bridge section of this verb's output depend on who ran it.
fn run_bridged(
    home: &Path,
    meridian_config: Option<&Path>,
    args: &[&str],
    wiki_path: Option<&Path>,
    repos_root: Option<&Path>,
) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mrd"));
    cmd.arg("config")
        .args(args)
        .env("HOME", home)
        .env_remove("MERIDIAN_CONFIG")
        .env_remove("CCC_LLM_WIKI_PATH")
        .env_remove("CCC_LLM_WIKI_REPOS_ROOT");
    if let Some(path) = meridian_config {
        cmd.env("MERIDIAN_CONFIG", path);
    }
    if let Some(path) = wiki_path {
        cmd.env("CCC_LLM_WIKI_PATH", path);
    }
    if let Some(path) = repos_root {
        cmd.env("CCC_LLM_WIKI_REPOS_ROOT", path);
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
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");

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
    let roots = home.path().join("roots");
    assert!(
        text.contains(&format!(
            "field-notes  vault  {}",
            roots.join("field-notes").display()
        )) && text.contains("vault:field-notes  bound"),
        "the vault root, with its Obsidian vault name and its bound state: {text}"
    );
    assert!(
        text.contains(&format!(
            "archive  git-folder  {}",
            roots.join("archive").display()
        )),
        "the git-folder root, which carries no vault name: {text}"
    );
    assert!(
        !text.contains("archive  git-folder") || !text.contains("archive  git-folder  vault:"),
        "a git-folder root has no vault leg: {text}"
    );

    // U7: the CANONICAL path is published beside the declared one whenever the
    // two differ — and on this runner they do, because the sandbox lives under
    // `/var`, a symlink to `/private/var`. That is the mount law's collapse made
    // visible at the verb rather than only asserted in a unit test: an operator
    // reading this line sees which tree the name is actually bound to.
    let canonical =
        std::fs::canonicalize(roots.join("field-notes")).expect("the sandbox root canonicalizes");
    if canonical != roots.join("field-notes") {
        assert!(
            text.contains(&format!("-> {}", canonical.display())),
            "the canonical spelling is published when it differs: {text}"
        );
    }
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
        single_named(home.path(), "field-notes", "assets"),
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
        single(home.path()).replace(
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
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");
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
    std::fs::write(&elsewhere, single_named(home.path(), "sessions", "archive")).expect("write");
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
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");

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

/// **U33.** The verb publishes the resolution's ORIGIN — which rung of the
/// bootstrap chain supplied the path — and it does so **where the two rungs
/// resolve to the SAME path**, which is the only case the printed path cannot
/// answer on its own.
///
/// Measured before this was written: `MERIDIAN_CONFIG=$HOME/MERIDIAN.md mrd
/// config` and `mrd config` with the variable unset produced **byte-identical
/// output**. Two environments differing in exactly the variable the chain is
/// made of were indistinguishable at the only surface that publishes the chain
/// — so an operator debugging a stale exported override had nothing to read,
/// and criterion 1's claim is about the CHAIN, not about its endpoint.
///
/// The assert is the DIFFERENCE between the two runs. A build that hardcoded
/// either word would pass one half and fail the other.
#[test]
fn the_verb_names_which_rung_supplied_the_path_even_when_both_agree() {
    let home = tempfile::tempdir().expect("tempdir");
    let default_path = home.path().join("MERIDIAN.md");
    std::fs::write(&default_path, single(home.path())).expect("write");

    // Rung 2 — nothing stated, the default answered.
    let fell_through = run(home.path(), None, &[]);
    assert_eq!(
        fell_through.status.code(),
        Some(0),
        "{}",
        stderr(&fell_through)
    );
    let fell_through_text = stdout(&fell_through);
    assert!(
        fell_through_text.contains("origin:$HOME/MERIDIAN.md"),
        "the default rung is NAMED, not inferred from the path: {fell_through_text}"
    );

    // Rung 1 — stated, and stating exactly what rung 2 would have produced. The
    // paths are identical; only the origin differs.
    let stated = run(home.path(), Some(&default_path), &[]);
    assert_eq!(stated.status.code(), Some(0), "{}", stderr(&stated));
    let stated_text = stdout(&stated);
    assert!(
        stated_text.contains("origin:MERIDIAN_CONFIG"),
        "the override is named even though it resolved to the default path: {stated_text}"
    );
    assert_ne!(
        fell_through_text, stated_text,
        "the whole point: these two runs used to be byte-identical"
    );

    // `--json` carries the same word. One spelling, both faces — the rule the
    // mount state words already follow.
    let json_stated: serde_json::Value =
        serde_json::from_str(&stdout(&run(home.path(), Some(&default_path), &["--json"])))
            .expect("json");
    assert_eq!(json_stated["origin"], "MERIDIAN_CONFIG");
    let json_default: serde_json::Value =
        serde_json::from_str(&stdout(&run(home.path(), None, &["--json"]))).expect("json");
    assert_eq!(json_default["origin"], "$HOME/MERIDIAN.md");

    // State A reports its origin too: the chain still ran and still answered,
    // and WHICH rung resolved a path that holds no file is exactly what an
    // operator staring at an unexpected `absent` needs.
    let empty = tempfile::tempdir().expect("tempdir");
    let absent = run(empty.path(), None, &[]);
    assert_eq!(absent.status.code(), Some(0));
    assert!(
        stdout(&absent).contains("origin:$HOME/MERIDIAN.md"),
        "absent is a resolution, and it has an origin: {}",
        stdout(&absent)
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

// ---------------------------------------------------------------------------
// The bridge period, end to end (U9)
// ---------------------------------------------------------------------------

/// **BOTH arms through the process boundary** (S3-R8(c)).
///
/// Agreement binds **silently** and names the mount it resolved onto;
/// divergence prints its note, the FILE WINS, and **the exit code does not
/// move** — a bridge period whose mismatch is fatal is not a bridge, and the
/// exit code is the fact that would brick every machine exporting the variable.
///
/// The agreeing spelling is deliberately the **symlinked** one: it is the
/// spelling that only canonicalize-at-bind can resolve, and it is the measured
/// shape of `/Users/Shared/repos/field-notes` on this machine.
#[test]
fn the_bridge_agrees_silently_diverges_loudly_and_never_moves_the_exit_code() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");
    let bound = home.path().join("roots").join("field-notes");

    // The measured topology: a second spelling of the bound root, through a
    // symlink that lives somewhere else entirely.
    let link = home.path().join("linked-wiki");
    std::os::unix::fs::symlink(&bound, &link).expect("symlink the bound root");

    // --- Arm 1: agreement, through the symlink spelling. Silent. ---
    let out = run_bridged(home.path(), None, &[], Some(&link), None);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("bridge (2):"), "{text}");
    assert!(
        text.contains("CCC_LLM_WIKI_PATH  agrees  -> field-notes"),
        "the symlinked spelling resolves onto the bound root, and the STATE CHANGE is \
         named — which mount now exists for this variable (R40): {text}"
    );
    // Asserted on the BRIDGE's own words, not on the token `note:`. This verb
    // already prints a render-face elision note on every run, so a bare
    // `!contains("note:")` is a check that fails for a reason unrelated to the
    // thing it names — measured here, on the first run of this gate.
    assert!(
        !text.contains("the FILE WINS"),
        "agreement is SILENT — a note on every agreement is what gets the variable unset: {text}"
    );
    assert!(
        text.contains("CCC_LLM_WIKI_REPOS_ROOT  unset"),
        "an unset variable states no path and is listed as such: {text}"
    );

    // --- Arm 2: divergence. Loud, once, and exit 0 all the same. ---
    let elsewhere = home.path().join("not-a-declared-root");
    std::fs::create_dir_all(&elsewhere).expect("create");
    let out = run_bridged(home.path(), None, &[], Some(&elsewhere), None);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a divergence NEVER moves the exit code: {}",
        stderr(&out)
    );
    let text = stdout(&out);
    assert!(
        text.contains("CCC_LLM_WIKI_PATH  diverges"),
        "the state word: {text}"
    );
    assert!(
        !text.contains("diverges  -> "),
        "THE FILE WINS: a diverging variable names no root: {text}"
    );
    assert!(
        text.contains("the FILE WINS"),
        "the divergence is reported, never silent: {text}"
    );
    assert!(
        text.contains("reported once per process"),
        "and the note says so, so an operator who sees it once is not left \
         wondering whether it was suppressed: {text}"
    );
    assert!(
        !text.contains("refused:"),
        "a note is not a refusal: {text}"
    );
}

/// The `--json` face carries the same bridge facts, with `mount` **null** on a
/// divergence — "the file wins" as data, in the shape a machine reader gets.
#[test]
fn the_json_face_carries_the_bridge_and_nulls_the_mount_on_divergence() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");
    let bound = home.path().join("roots").join("field-notes");
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("create");

    let out = run_bridged(
        home.path(),
        None,
        &["--json"],
        Some(&bound),
        Some(&elsewhere),
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid json");
    let bridge = v["bridge"].as_array().expect("a bridge array");
    assert_eq!(bridge.len(), 2);

    assert_eq!(bridge[0]["var"], "CCC_LLM_WIKI_PATH");
    assert_eq!(bridge[0]["state"], "agrees");
    assert_eq!(bridge[0]["mount"], "field-notes");
    assert!(
        bridge[0]["canonical"].is_string(),
        "agreement names where it resolved from"
    );
    assert!(bridge[0]["report"].is_null(), "agreement carries no report");

    assert_eq!(bridge[1]["var"], "CCC_LLM_WIKI_REPOS_ROOT");
    assert_eq!(bridge[1]["state"], "diverges");
    assert!(
        bridge[1]["mount"].is_null(),
        "THE FILE WINS — a diverging variable names no root, and the json says so \
         with null rather than with a guessed name"
    );
    assert!(
        bridge[1]["report"].is_string(),
        "the first divergence in this process reports"
    );

    // The state word is the SAME spelling in both faces — two spellings of one
    // state is how a downstream reader and an operator come to disagree.
    let human = run_bridged(home.path(), None, &[], Some(&bound), Some(&elsewhere));
    assert!(stdout(&human).contains("CCC_LLM_WIKI_REPOS_ROOT  diverges"));
}
