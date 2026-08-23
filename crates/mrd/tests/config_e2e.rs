//! `mrd config` end to end, against the real binary (U6).

use std::path::Path;
use std::process::{Command, Output};

/// The two-root config, over roots that live inside the sandbox and really declare
/// themselves. The line layout matters: `line 13` is asserted verbatim by
/// [`a_malformed_config_refuses_on_exit_one_and_publishes_nothing`].
fn single(home: &Path) -> String {
    single_named(home, "field-notes", "archive")
}

/// The same fixture with either root renamed — and the roots own declaration renamed with it,
/// since a canonical name the root does not declare is a declared-vs-bound mismatch and fails
/// loud by design (D7).
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
vault: {vault_name}
```

```meridian-mount
name: {folder_name}
path: {}
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

/// The same invocation with the two bridged variables driven explicitly (U9). `run` clears
/// both, so every other test measures a determinate `unset` bridge rather than inheriting
/// whatever the runner exports (developer machines really do export `CCC_LLM_WIKI_PATH`).
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

/// The acceptance: the verb publishes the parsed mount table, in document order, with the
/// configs own rev and fingerprint beside it. Without this half, every refusal assertion below
/// is satisfied by a build that refuses everything (S3-R8(c)).
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
        text.contains(&format!("field-notes  {}", roots.join("field-notes").display()))
            && text.contains("vault:field-notes  bound"),
        "the vault root, with its Obsidian vault name and its bound state: {text}"
    );
    assert!(
        text.contains(&format!("archive  {}", roots.join("archive").display())),
        "the plain root, which carries no vault name: {text}"
    );
    assert!(
        mount_row(&text, "archive").ends_with("  vault:(none)  bound"),
        "the git-folder root STATES its absent vault leg: {text}"
    );

    // The canonical path is published beside the declared one whenever the two differ — and on
    // this runner they do, because the sandbox lives under `/var`, a symlink to `/private/var`.
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
    assert!(after.contains("assets  "), "and the parsed name: {after}");
}

fn rev_line(text: &str) -> String {
    text.split_whitespace()
        .find(|t| t.starts_with("file_rev:"))
        .unwrap_or_default()
        .to_string()
}

/// The one output line for a mount, by canonical root name. Returned whole so an assertion
/// pins a cell at its leg — a `contains` over the entire output would also pass when the
/// token turned up in another roots row or section.
fn mount_row(text: &str, name: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with(&format!("{name}  ")))
        .unwrap_or_else(|| panic!("no mount row for `{name}` in:\n{text}"))
        .to_string()
}

/// Criterion 2 — a leg that is absent by construction is stated, never left blank.
///
/// `Mount::vault` is `Some` iff `kind: vault` (the parser refuses a `vault:` line on a
/// `git-folder` entry), so a git-folder root's vault leg cannot exist — and a blank cell is
/// byte-identical to a dropped value. Both halves matter (S3-R8(c)): the git-folder row
/// carries the marker at its vault leg, and the vault root carries its name there and no
/// marker.
#[test]
fn a_structurally_absent_vault_leg_renders_a_marker_never_a_blank() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");

    let out = run(home.path(), None, &[]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    let text = stdout(&out);

    // The refusal half. `ends_with` rather than `contains`, so the assertion fails on a blank
    // cell and on a marker rendered at the wrong leg.
    let folder = mount_row(&text, "archive");
    assert!(
        folder.ends_with("  vault:(none)  bound"),
        "the git-folder root's vault leg renders the MARKER, never whitespace: {folder}"
    );

    // The acceptance half.
    let vault = mount_row(&text, "field-notes");
    assert!(
        vault.ends_with("  vault:field-notes  bound"),
        "the vault root's vault leg renders its Obsidian vault NAME: {vault}"
    );
    assert!(
        !vault.contains("(none)"),
        "and never the marker — otherwise the half above is satisfied by a build \
         that calls every vault absent: {vault}"
    );

    // S3-R37: the population this arm iterates is asserted non-empty, so the fixture losing
    // its git-folder root retires the coverage loudly instead of silently.
    assert_eq!(
        text.matches("vault:(none)").count(),
        1,
        "exactly one root in this fixture is structurally without a vault: {text}"
    );
}

/// The `--json` face is not changed by the marker: `null` at a present key is already the
/// statement there; dropping the key would have been the omission.
#[test]
fn the_json_face_states_absence_as_null_and_never_carries_the_human_marker() {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(home.path().join("MERIDIAN.md"), single(home.path())).expect("write");

    let out = run(home.path(), None, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let raw = stdout(&out);
    let value: serde_json::Value = serde_json::from_str(&raw).expect("json");

    let mounts = value["mounts"].as_array().expect("mounts");
    let folder = mounts[1].as_object().expect("the git-folder mount");
    assert!(
        folder.contains_key("vault"),
        "the KEY is present — that is what makes the null a statement: {folder:?}"
    );
    assert!(
        folder["vault"].is_null(),
        "and its value is null: {folder:?}"
    );
    assert!(
        !raw.contains("(none)"),
        "the human marker never reaches the machine face, at any key: {raw}"
    );
}

/// State A: no config file. Not an error and not a warning — every machine starts here, and the
/// verb says so rather than printing a bare empty table a reader would take for a failure.
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
        single(home.path()).replace("vault: field-notes", "primary: false\nvault: field-notes"),
    )
    .expect("write");

    let out = run(home.path(), None, &[]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a refusal is a finding, not a tool failure"
    );
    let err = stderr(&out);
    assert!(
        err.contains("`primary: false` is not a designation"),
        "{err}"
    );
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

/// State C: `MERIDIAN_CONFIG` naming a path that is not a readable regular file refuses — it
/// never silently falls back to `~/MERIDIAN.md`. The home default is a perfectly good config
/// here, so a build that falls back looks completely healthy; that is what makes this the
/// load-bearing case.
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

/// `--json` is the house grammars machine face, and it carries the same facts as the human one
/// — a JSON surface that dropped the rev would make the agent-facing plane blinder than the
/// human-facing one.
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
    assert!(
        mounts[0].get("kind").is_none(),
        "kind left the schema (kind-sweep 2026-08-13) and the face with it"
    );
    assert_eq!(mounts[0]["vault"], "field-notes");
    assert!(mounts[0]["pin"].is_null());
    assert_eq!(mounts[1]["name"], "archive");
    assert!(
        mounts[1]["vault"].is_null(),
        "a mount without `vault:` has no Obsidian vault leg"
    );
}

/// U33 — the verb publishes the resolutions origin (which rung of the bootstrap chain
/// supplied the path), even where the two rungs resolve to the same path: the only case the
/// printed path cannot answer on its own.
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

    // State A reports its origin too: the chain still ran and still answered, and WHICH rung
    // resolved a path that holds no file is exactly what an operator staring at an unexpected
    // `absent` needs.
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

// The bridge period, end to end (U9)

/// Both arms through the process boundary (S3-R8(c)): agreement binds silently and names the
/// mount it resolved onto; divergence prints its note, the file wins, and the exit code does
/// not move — a bridge period whose mismatch is fatal is not a bridge.
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
    // Asserted on the bridges own words, not on the token `note:` — this verb already prints
    // a render-face elision note on every run.
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

/// The verb names WHICH PROCESS resolved the chain, and warns only where the operator has
/// re-pointed it. Two processes answer mount-table questions through the same
/// `config::Env::from_process()` call — this CLI, and the daemon's `mounts` op
/// (`crates/registry/src/mounts.rs`) — each from its own environment, so a table published here
/// can differ from the one the engine binds and nothing used to say so. Ruled (a) 2026-08-23,
/// card `serving-daemon-holds-mount-table-ignores-meridian-config`.
///
/// The line is UNCONDITIONAL by design, and this test is what stops it being narrowed back to
/// the override rung: a daemon started under a different `$HOME` resolves a different rung-2
/// file with no variable set anywhere, so an override-gated line would stay silent on the very
/// case that has no variable to notice.
#[test]
fn the_verb_names_which_process_answered_and_warns_only_on_the_override_rung() {
    let home = tempfile::tempdir().expect("tempdir");
    let default_path = home.path().join("MERIDIAN.md");
    std::fs::write(&default_path, single(home.path())).expect("write the config");

    // ── the line is on BOTH rungs, and on the absent resolution ──────────────
    let fell_through = run(home.path(), None, &[]);
    let stated = run(home.path(), Some(&default_path), &[]);
    let empty = tempfile::tempdir().expect("tempdir");
    let absent = run(empty.path(), None, &[]);

    for (label, out) in [
        ("rung 2", &fell_through),
        ("rung 1", &stated),
        ("state A", &absent),
    ] {
        let text = stdout(out);
        assert_eq!(out.status.code(), Some(0), "{label}: {}", stderr(out));
        assert!(
            text.contains("answered by: this process"),
            "{label} names the resolving PROCESS, not only the rung: {text}"
        );
        assert!(
            text.contains("a serving daemon answers the `mounts` op from ITS environment"),
            "{label} says where the other answer comes from, or the first half teaches nothing: \
             {text}"
        );
    }

    // ── the warning is the override rung ONLY ───────────────────────────────
    let warning = "is read here and never reaches a serving daemon";
    assert!(
        stdout(&stated).contains(warning),
        "the override rung warns that the variable stops at this process: {}",
        stdout(&stated)
    );
    assert!(
        stdout(&stated).contains("MERIDIAN_CONFIG is read here"),
        "the warning NAMES the variable — a warning that does not name it cannot be acted on: {}",
        stdout(&stated)
    );
    assert!(
        !stdout(&fell_through).contains(warning),
        "rung 2 has no variable to warn about, and a warning printed where nothing is set is \
         noise that trains the reader to skip the line: {}",
        stdout(&fell_through)
    );
    assert!(
        !stdout(&absent).contains(warning),
        "state A resolved through rung 2 as well: {}",
        stdout(&absent)
    );

    // ── one spelling across both faces, the rule `origin` already follows ────
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&run(home.path(), None, &["--json"]))).expect("json");
    assert_eq!(
        json["answered_by"], "this process",
        "the json face carries the resolver as data, in the human line's own words"
    );
}
