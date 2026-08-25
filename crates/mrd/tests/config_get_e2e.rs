//! `mrd config get` end to end, against the real binary.
//!
//! The verb reads ONE block — `MERIDIAN.md`'s `^config` — evaluates its `config()` and prints
//! what came back. These tests pin both halves: that a real config really prints (without which
//! every refusal below is satisfied by a build that refuses everything), and that each way of
//! being wrong refuses loudly rather than printing an empty line and exiting 0.

use std::path::Path;
use std::process::{Command, Output};

/// A `MERIDIAN.md` with no mounts and the given tail appended — the config plane's state D, so
/// these tests measure the config block and nothing else.
fn page(tail: &str) -> String {
    format!("---\ntype: meridian-config\nversion: 1\n---\n\n# My machine\n\n{tail}")
}

/// The canonical block: a starlark fence with the id on its own line beneath it.
fn block(body: &str) -> String {
    format!("```starlark\n{body}\n```\n^config\n")
}

fn write_home(home: &Path, contents: &str) {
    std::fs::write(home.join("MERIDIAN.md"), contents).expect("write the config");
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .arg("config")
        .arg("get")
        .args(args)
        .env("HOME", home)
        .env_remove("MERIDIAN_CONFIG")
        .env_remove("CCC_LLM_WIKI_PATH")
        .env_remove("CCC_LLM_WIKI_REPOS_ROOT")
        .output()
        .expect("mrd runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The acceptance, and the shape ZT asked for: a `^config` block whose `config()` returns a
/// mapping, one key addressed by name, printed BARE so a shell can capture it.
#[test]
fn a_key_prints_its_value_bare() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": \"/Users/Shared/projects/coscene-io\"}",
        )),
    );

    let out = run(home.path(), &["repos_root"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "/Users/Shared/projects/coscene-io\n",
        "the value alone, unquoted — `r=$(mrd config get repos_root)` is the point of the verb"
    );
}

/// No KEY prints the whole returned value. The config is arbitrary data, so the whole value is
/// JSON on the human face too — there is no schema to render it against.
#[test]
fn no_key_prints_the_whole_config() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": \"/repos\", \"jobs\": 4, \"tags\": [\"a\", \"b\"]}",
        )),
    );

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    let text = stdout(&out);
    let value: serde_json::Value = serde_json::from_str(&text).expect("the whole value is JSON");
    assert_eq!(value["repos_root"], "/repos");
    assert_eq!(value["jobs"], 4);
    assert_eq!(value["tags"], serde_json::json!(["a", "b"]));
}

/// "The config can be anything, we don't limit it" — a list, a bare string, a number and a bool
/// all come back, and none of them is a schema violation.
#[test]
fn the_config_is_not_limited_to_a_mapping() {
    let home = tempfile::tempdir().expect("tempdir");

    write_home(
        home.path(),
        &page(&block("def config():\n    return [1, 2, 3]")),
    );
    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "a list: {}", stderr(&out));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout(&out)).expect("json"),
        serde_json::json!([1, 2, 3])
    );

    write_home(
        home.path(),
        &page(&block("def config():\n    return \"just a string\"")),
    );
    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "a string: {}", stderr(&out));
    assert_eq!(stdout(&out), "just a string\n", "bare on the human face");

    write_home(home.path(), &page(&block("def config():\n    return True")));
    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "a bool: {}", stderr(&out));
    assert_eq!(stdout(&out), "true\n");
}

/// `--json` publishes JSON for every shape, including the scalar the human face prints bare —
/// a client that parses the output must not have to know which shape it asked for.
#[test]
fn the_json_face_quotes_the_scalar_the_human_face_prints_bare() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": \"/repos\"}",
        )),
    );

    let human = run(home.path(), &["repos_root"]);
    assert_eq!(stdout(&human), "/repos\n");

    let json = run(home.path(), &["repos_root", "--json"]);
    assert_eq!(json.status.code(), Some(0), "clean: {}", stderr(&json));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout(&json)).expect("json"),
        serde_json::json!("/repos")
    );
}

/// The config block is computed, not a literal table: `config()` is a function and the value it
/// returns is whatever it computed.
#[test]
fn the_block_is_starlark_and_may_compute() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "ROOT = \"/Users/Shared/projects\"\n\ndef config():\n    return {\"repos_root\": ROOT + \"/coscene-io\"}",
        )),
    );

    let out = run(home.path(), &["repos_root"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(stdout(&out), "/Users/Shared/projects/coscene-io\n");
}

/// The evaluator is the sealed kernel: no `load`, no ambient I/O, no unbound reach. A block that
/// tries faults — it does not silently succeed with a partial value.
#[test]
fn the_block_reaches_nothing() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"secret\": open(\"/etc/passwd\").read()}",
        )),
    );

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(
        stdout(&out).is_empty(),
        "nothing published: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("open"),
        "the fault names the unbound reach: {}",
        stderr(&out)
    );
}

/// A file with no `^config` block refuses and teaches the block's shape. This is the common
/// first run, so it is the refusal that has to be worth reading.
#[test]
fn a_file_with_no_config_block_refuses_and_teaches_the_shape() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(home.path(), &page("I have not declared a config yet.\n"));

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(stdout(&out).is_empty(), "nothing published");
    let text = stderr(&out);
    assert!(text.contains("no block carries `^config`"), "{text}");
    assert!(text.contains("```starlark"), "shows the shape: {text}");
    assert!(text.contains("^config"), "{text}");
}

/// No file at all is not the same refusal as a file with no block, and says so.
#[test]
fn an_absent_config_file_refuses_by_naming_the_path() {
    let home = tempfile::tempdir().expect("tempdir");

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    let text = stderr(&out);
    assert!(text.contains("no config file at"), "{text}");
    assert!(text.contains("MERIDIAN.md"), "names the path: {text}");
}

/// Two blocks carrying the id is ambiguous, and the mint plane never picks: nothing is read.
#[test]
fn two_config_blocks_refuse_as_ambiguous() {
    let home = tempfile::tempdir().expect("tempdir");
    let two = format!(
        "{}\n{}",
        block("def config():\n    return {\"which\": \"first\"}"),
        block("def config():\n    return {\"which\": \"second\"}")
    );
    write_home(home.path(), &page(&two));

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(stdout(&out).is_empty(), "no winner published");
    let text = stderr(&out);
    assert!(text.contains("ambiguous"), "{text}");
    assert!(text.contains("2 blocks"), "names the count: {text}");
}

/// The fence language is part of the contract: a `bash` block carrying the id refuses by naming
/// the language it found and the one it needs.
#[test]
fn a_non_starlark_fence_refuses_by_naming_the_language() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page("```bash\necho repos_root=/repos\n```\n^config\n"),
    );

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    let text = stderr(&out);
    assert!(text.contains("`bash` fence"), "{text}");
    assert!(text.contains("starlark"), "{text}");
}

/// A block that defines no `config()` refuses by naming the entry it owes.
#[test]
fn a_block_without_the_entry_refuses() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "REPOS = \"/repos\"\n\ndef settings():\n    return {}",
        )),
    );

    let out = run(home.path(), &[]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(
        stderr(&out).contains("defines no `config` entry"),
        "{}",
        stderr(&out)
    );
}

/// A KEY the config does not carry refuses and names the keys it does — the reader's next move
/// is in the refusal, not in a second command.
#[test]
fn an_absent_key_refuses_and_names_the_keys_that_exist() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": \"/repos\", \"editor\": \"nvim\"}",
        )),
    );

    let out = run(home.path(), &["repo_root"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(stdout(&out).is_empty(), "no empty-success line");
    let text = stderr(&out);
    assert!(text.contains("no `repo_root`"), "{text}");
    assert!(text.contains("editor"), "names what is there: {text}");
    assert!(text.contains("repos_root"), "{text}");
}

/// A KEY asked of a config that is not a mapping refuses by naming what came back.
#[test]
fn a_key_against_a_non_mapping_refuses() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block("def config():\n    return [\"a\", \"b\"]")),
    );

    let out = run(home.path(), &["repos_root"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(
        stderr(&out).contains("a list, not a mapping"),
        "{}",
        stderr(&out)
    );
}

/// The shape the first real config has: `repos_root` keyed BY WIKI, because a repos root is a fact
/// about a wiki and not about a machine. A dot-path reaches the member; the bare key prints the
/// mapping.
#[test]
fn a_dot_path_reaches_a_nested_member() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": {\"coscene-wiki\": \"/Users/Shared/projects/coscene-io\", \"field-notes\": \"/Users/Shared/repos\"}}",
        )),
    );

    let out = run(home.path(), &["repos_root.coscene-wiki"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(stdout(&out), "/Users/Shared/projects/coscene-io\n");

    let out = run(home.path(), &["repos_root.field-notes"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(stdout(&out), "/Users/Shared/repos\n");

    let out = run(home.path(), &["repos_root"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    let value: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("the branch prints as JSON");
    assert_eq!(value["coscene-wiki"], "/Users/Shared/projects/coscene-io");
}

/// The walk has no depth limit of its own — a path is as deep as the config is.
#[test]
fn a_dot_path_walks_as_deep_as_the_config_goes() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"a\": {\"b\": {\"c\": {\"d\": \"bottom\"}}}}",
        )),
    );

    let out = run(home.path(), &["a.b.c.d"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(stdout(&out), "bottom\n");
}

/// The rule that keeps every key reachable: the EXACT key wins over the split, so a config whose
/// member is really named `a.b` is addressable by its own name. A split-first grammar would have
/// made that key unaddressable and said nothing.
#[test]
fn an_exact_key_wins_over_the_split() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"a.b\": \"literal\", \"a\": {\"b\": \"nested\"}}",
        )),
    );

    let out = run(home.path(), &["a.b"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(stdout(&out), "literal\n");
}

/// A dot-path that dies mid-walk says WHERE it stopped and what that level does carry — the
/// difference between "no such key" and "no such key HERE" is the reader's next move.
#[test]
fn a_dot_path_that_dies_names_where_it_stopped() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": {\"coscene-wiki\": \"/co\", \"field-notes\": \"/home\"}}",
        )),
    );

    let out = run(home.path(), &["repos_root.team-wiki"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    assert!(stdout(&out).is_empty(), "no empty-success line");
    let text = stderr(&out);
    assert!(text.contains("`repos_root` has no `team-wiki`"), "{text}");
    assert!(text.contains("coscene-wiki"), "names what is there: {text}");

    // …and a segment asked of a leaf names the leaf, not the whole config.
    let out = run(home.path(), &["repos_root.field-notes.deeper"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", stdout(&out));
    let text = stderr(&out);
    assert!(
        text.contains("`repos_root.field-notes` is a string, not a mapping"),
        "{text}"
    );
}

/// A starlark block that is NOT the config block is prose to this door and to the mount parser
/// alike: nothing scans for a fence, one id is addressed.
#[test]
fn only_the_addressed_block_is_read() {
    let home = tempfile::tempdir().expect("tempdir");
    let decoys = format!(
        "```starlark\ndef config():\n    return {{\"which\": \"decoy\"}}\n```\n\n{}",
        block("def config():\n    return {\"which\": \"addressed\"}")
    );
    write_home(home.path(), &page(&decoys));

    let out = run(home.path(), &["which"]);
    assert_eq!(out.status.code(), Some(0), "clean: {}", stderr(&out));
    assert_eq!(stdout(&out), "addressed\n");
}

/// The two legs are independent: an unbound root refuses `mrd config` (the mount plane's
/// business) and leaves `mrd config get` answering. Coupling them would cost a machine its
/// config over a root it was not asking about.
#[test]
fn an_unbound_root_refuses_the_table_and_not_the_value() {
    let home = tempfile::tempdir().expect("tempdir");
    let missing = home.path().join("roots").join("gone");
    let contents = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# My machine\n\n\
         ```meridian-mount\nname: gone\npath: {}\n```\n\n{}",
        missing.display(),
        block("def config():\n    return {\"repos_root\": \"/repos\"}")
    );
    write_home(home.path(), &contents);

    let table = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .arg("config")
        .env("HOME", home.path())
        .env_remove("MERIDIAN_CONFIG")
        .env_remove("CCC_LLM_WIKI_PATH")
        .env_remove("CCC_LLM_WIKI_REPOS_ROOT")
        .output()
        .expect("mrd runs");
    assert_eq!(
        table.status.code(),
        Some(1),
        "the mount plane refuses an unbound root"
    );

    let value = run(home.path(), &["repos_root"]);
    assert_eq!(
        value.status.code(),
        Some(0),
        "and the value leg still answers: {}",
        stderr(&value)
    );
    assert_eq!(stdout(&value), "/repos\n");
}

/// A misspelled sub-verb is answered as one, not as a stray argument.
#[test]
fn a_bare_word_is_answered_as_a_subcommand() {
    let home = tempfile::tempdir().expect("tempdir");
    write_home(
        home.path(),
        &page(&block(
            "def config():\n    return {\"repos_root\": \"/repos\"}",
        )),
    );

    let out = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .arg("config")
        .arg("repos_root")
        .env("HOME", home.path())
        .env_remove("MERIDIAN_CONFIG")
        .output()
        .expect("mrd runs");
    assert_eq!(out.status.code(), Some(2), "bad invocation");
    assert!(
        stderr(&out).contains("unknown config subcommand: repos_root"),
        "{}",
        stderr(&out)
    );
}
