//! The executable gate for the amended `run` op's two modes (hook-support
//! design § 2.2, as amended by § Amendments / A1): `mrd run --load <PAGE>…`
//! and `mrd run <PAGE>#^<id> [--input-json FILE|-]`.
//!
//! Every assertion here is a LAW of the design, not a shape of this
//! implementation. Each test names the law it holds:
//!
//! - the consent gate — `run` executes what the page declares, never an
//!   undeclared block;
//! - load purity — an effect at load faults `effect_at_load` at its own line;
//! - recording by declaration kind — a fire adds NO receipt rows;
//! - the caps ceiling — a constructor outside the page's `caps:` is callable
//!   and refused at `admit`, never absent;
//! - the exclusions — an argument that is meaningless where it was written
//!   refuses by name rather than being ignored.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The fixture page: a declaring block, an impure one, a bare fence, a
/// birthing block, a task-bound block, and a bash block. One page, because
/// the interactions between block kinds are half of what is being tested.
const PAGE: &str = "\
---
caps: md.create
task.arm: \"[[#^armer]]\"
---

# Probe

```starlark
def run(event):
    return {\"deny\": \"no stash\", \"saw\": event[\"name\"]}

declare(on = \"PreToolUse\", match = \"Bash\")
```
^h

```starlark
bash(cmd = \"true\")
```
^impure

```starlark
x = 1
```
^bare

```starlark
def run(event):
    create(path = \"born/\" + event[\"name\"] + \".md\", body = \"hi\")
    return None

declare(on = \"Stop\")
```
^birth

```starlark
def run(ctx):
    pass
```
^armer

```bash
echo hi
```
^shell
";

const ROOT: &str = "\
---
type: meridian-root
version: 1
name: probe
---

# probe root
";

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("probe.md"), PAGE).expect("page");
        std::fs::write(tmp.path().join("MERIDIAN.md"), ROOT).expect("root");
        Self { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    fn file(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    fn input(&self, name: &str, json: &str) -> String {
        std::fs::write(self.file(name), json).expect("input file");
        name.to_owned()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mrd"))
            .arg("run")
            .args(args)
            .env("MERIDIAN_WORKSPACE", self.path())
            // Cleared for the same reason every gate run clears it: it is
            // exported in developer shells and several tests assert it empty.
            .env_remove("CCC_MRD_BIN")
            .current_dir(self.path())
            .output()
            .expect("spawn mrd")
    }

    /// Strip the page's `caps:` line — the negative half of the caps law.
    fn drop_caps(&self) {
        let page = PAGE.replace("caps: md.create\n", "");
        std::fs::write(self.file("probe.md"), page).expect("page");
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// The one target row, parsed.
fn row(out: &Output) -> Value {
    let text = stdout(out);
    let parsed: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{text}"));
    parsed["targets"][0].clone()
}

/// One `loaded[]` entry by block id.
fn block<'a>(row: &'a Value, id: &str) -> &'a Value {
    row["loaded"]
        .as_array()
        .expect("loaded rows")
        .iter()
        .find(|b| b["block"] == id)
        .unwrap_or_else(|| panic!("no loaded row for ^{id} in {row:#}"))
}

// ── load ────────────────────────────────────────────────────────────────────

/// A load publishes every declaring block's declarations VERBATIM, reports a
/// task-bound block as the run plane's, and evaluates no bash block.
#[test]
fn a_load_publishes_declarations_and_leaves_task_blocks_alone() {
    let ws = Ws::new();
    let out = ws.run(&["--load", "probe.md"]);
    let row = row(&out);

    assert_eq!(
        block(&row, "h")["declarations"][0],
        serde_json::json!({"on": "PreToolUse", "match": "Bash"}),
        "the declaration is the author's dict, uninterpreted"
    );
    assert_eq!(block(&row, "h")["entry_kind"], "evaluated");

    // A `task.<name>`-bound block is the run plane's: reported, never
    // evaluated, and carrying no declarations.
    assert_eq!(block(&row, "armer")["entry_kind"], "task");
    assert_eq!(block(&row, "armer")["task"], "arm");
    assert_eq!(
        block(&row, "armer")["declarations"]
            .as_array()
            .expect("array")
            .len(),
        0
    );

    // A bash block is not a module and never appears — it is an exec target
    // and a `bash(block=)` helper.
    assert!(
        row["loaded"]
            .as_array()
            .expect("rows")
            .iter()
            .all(|b| b["block"] != "shell"),
        "a bash block appeared in a load answer: {row:#}"
    );

    // Provenance: the page's file rev and each block's own rev, so a caller
    // can print WHICH BYTES ran.
    assert!(row["rev"]["file"].as_str().is_some_and(|r| !r.is_empty()));
    assert_ne!(block(&row, "h")["rev"], block(&row, "birth")["rev"]);
}

/// **Load purity** (gate row 8): a top-level effect faults `effect_at_load`
/// at its own line, and the block freezes nothing.
///
/// The class is NOT `name_error`: the name is bound — that is precisely what
/// makes an effectful block loadable — and the refusal is about the phase
/// (A1). Conflating them would send an author hunting a misspelling that is
/// not there.
#[test]
fn a_top_level_effect_is_effect_at_load_at_its_own_line() {
    let ws = Ws::new();
    let out = ws.run(&["--load", "probe.md"]);
    let impure = block(&row(&out), "impure").clone();

    assert_eq!(impure["result"], "fault");
    assert_eq!(impure["fault"]["class"], "effect_at_load");
    assert_eq!(
        impure["fault"]["line"], 1,
        "the fault points at the CALL: {impure:#}"
    );
    assert!(
        impure["fault"]["reason"]
            .as_str()
            .expect("reason")
            .contains("does not act at load"),
        "the refusal must teach the phase: {impure:#}"
    );
    // A faulted block does not stop its siblings — rows are independent.
    assert_eq!(block(&row(&out), "h")["result"], "ok");
    // …and the run reports the refusal on the triad's run leg.
    assert_eq!(code(&out), 1);
}

// ── fire ────────────────────────────────────────────────────────────────────

/// A fire calls the frozen entry with the input and answers its return as
/// JSON, with the page and block revs echoed.
#[test]
fn a_fire_answers_the_entrys_return_as_json() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"PreToolUse"}"#);
    let out = ws.run(&["probe.md#^h", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(code(&out), 0);
    assert_eq!(row["result"], "ok");
    assert_eq!(
        row["value"],
        serde_json::json!({"deny": "no stash", "saw": "PreToolUse"}),
        "the input reached the entry and its answer came back: {row:#}"
    );
    assert!(row["rev"]["block"].as_str().is_some_and(|r| !r.is_empty()));
}

/// **The consent gate** (gate row 7): a fire naming a bare anchored fence
/// refuses `not_declared`. This is the law the whole amendment rests on —
/// `run` executes what the page DECLARES, and widening an authority surface
/// silently is exactly what it prevents.
#[test]
fn a_fire_on_an_undeclared_block_refuses_not_declared() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^bare", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(row["result"], "refused");
    assert_eq!(row["fault"]["class"], "not_declared");
    assert_eq!(code(&out), 1);
}

/// A fire on a block bound to a `task.<name>` refuses too — the two
/// addressings are exclusive, and the refusal SAYS which one applies rather
/// than silently running the task contract with an event.
#[test]
fn a_fire_on_a_task_bound_block_names_the_other_addressing() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^armer", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(row["fault"]["class"], "not_declared");
    assert!(
        row["fault"]["reason"]
            .as_str()
            .expect("reason")
            .contains("task `arm`"),
        "the refusal must name the task that binds it: {row:#}"
    );
}

/// A fire on a bash block refuses `not_a_module` — a non-starlark block runs
/// as an exec'd entry, never as a peer evaluator.
#[test]
fn a_fire_on_a_bash_block_refuses_not_a_module() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^shell", "--input-json", &input]);
    assert_eq!(row(&out)["fault"]["class"], "not_a_module");
}

/// A dangling block id refuses `no_block`, by the mint plane's own
/// addressing.
#[test]
fn a_fire_on_a_missing_block_refuses_no_block() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^nope", "--input-json", &input]);
    assert_eq!(row(&out)["fault"]["class"], "no_block");
}

// ── effects, caps, and recording ────────────────────────────────────────────

/// **Effects through the ordinary doors** (gate row 9): a `create()` under
/// the page's `caps:` births the file, and the row's `file_rev` is the BORN
/// file's — not the page's, which would be a plausible hash for the wrong
/// record.
#[test]
fn a_fire_births_through_the_create_door_and_reports_the_born_rev() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^birth", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(row["result"], "ok");
    let applied = &row["applied"][0];
    assert_eq!(applied["kind"], "md.create");
    assert_eq!(applied["result"], "born");
    assert_eq!(applied["path"], "born/Stop.md");
    assert_eq!(
        std::fs::read_to_string(ws.file("born/Stop.md")).expect("the file is on disk"),
        "hi"
    );
    let born_rev = applied["file_rev"].as_str().expect("a born rev");
    assert!(!born_rev.is_empty());
    assert_ne!(
        born_rev,
        row["rev"]["file"].as_str().expect("page rev"),
        "the birth row must carry the BORN file's rev, not the page's"
    );
}

/// **The caps ceiling**: with no `caps:` on the page the constructor is still
/// CALLABLE — it loads and it fires — and is refused at `admit` with
/// `cap_denied`. Absence would have been the old shape; A1's is callable and
/// refused. Nothing lands.
#[test]
fn a_constructor_outside_caps_is_callable_and_refused_at_admit() {
    let ws = Ws::new();
    ws.drop_caps();
    let input = ws.input("e.json", r#"{"name":"Denied"}"#);
    let out = ws.run(&["probe.md#^birth", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(row["result"], "refused");
    assert_eq!(row["fault"]["class"], "cap_denied");
    assert_eq!(
        row["applied"].as_array().expect("array").len(),
        0,
        "a denied batch is atomic: no row may claim it landed"
    );
    assert!(
        !ws.file("born/Denied.md").exists(),
        "the refused birth reached disk anyway"
    );
}

/// **Recording by declaration kind** (gate row 6): a fire writes NO receipt
/// rows. Asserted over repeated fires and over the receipt file's very
/// existence, because "the count did not change" is also true of a file that
/// was never there for the wrong reason.
#[test]
fn fires_add_no_receipt_rows() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"PreToolUse"}"#);
    let receipts = ws.file(run::executor::RECEIPT_FILE);

    let before = std::fs::read_to_string(&receipts).unwrap_or_default();
    for _ in 0..8 {
        let out = ws.run(&["probe.md#^h", "--input-json", &input]);
        assert_eq!(code(&out), 0, "a fire failed: {}", stdout(&out));
    }
    let after = std::fs::read_to_string(&receipts).unwrap_or_default();

    assert_eq!(
        after.matches("\n^r-").count(),
        before.matches("\n^r-").count(),
        "a fire appended a receipt row"
    );
    assert_eq!(
        after.matches("\n^p-").count(),
        before.matches("\n^p-").count(),
        "a fire appended a pre-receipt row"
    );
    assert_eq!(after, before, "the receipt file moved at all");
}

// ── the argv exclusions ─────────────────────────────────────────────────────

/// An argument that is meaningless where it was written refuses BY NAME.
/// Silently ignoring one is the guard-you-believe-is-armed trap in argv
/// costume: the caller believes the input was delivered.
#[test]
fn meaningless_arguments_refuse_by_name() {
    let ws = Ws::new();
    ws.input("e.json", "{}");
    for (args, expect) in [
        (
            vec!["--load", "probe.md#^h"],
            "addresses one block to fire",
        ),
        (vec!["probe.md", "--input-json", "e.json"], "is a fire's input"),
        (
            vec!["probe.md#^h", "--input-json", "e.json", "--env", "A=1"],
            "one input channel",
        ),
        (
            vec!["--load", "probe.md", "--env", "A=1"],
            "--load takes only pages",
        ),
    ] {
        let out = ws.run(&args);
        let text = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(
            code(&out),
            2,
            "{args:?} should be an invocation fault, got:\n{text}"
        );
        assert!(
            text.contains(expect),
            "{args:?} refused without teaching {expect:?}:\n{text}"
        );
    }
}

/// `--input-json` that is not JSON refuses at the ARGV boundary (exit 2), so
/// a caller's malformed input is never reported as the page's fault.
#[test]
fn a_malformed_input_refuses_as_an_invocation_fault() {
    let ws = Ws::new();
    let input = ws.input("bad.json", "{not json");
    let out = ws.run(&["probe.md#^h", "--input-json", &input]);
    assert_eq!(code(&out), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not JSON"));
}

/// Several pages are several targets in ONE call — the shape a resolver's
/// cold pass rides (§ 1.6).
#[test]
fn several_pages_load_as_several_rows_in_one_call() {
    let ws = Ws::new();
    std::fs::write(
        ws.file("other.md"),
        "# Other\n\n```starlark\ndeclare(on = \"Stop\")\n```\n^only\n",
    )
    .expect("second page");
    let out = ws.run(&["--load", "probe.md", "other.md"]);
    let text = stdout(&out);
    let parsed: Value = serde_json::from_str(&text).expect("JSON");
    let targets = parsed["targets"].as_array().expect("targets");

    assert_eq!(targets.len(), 2, "one row per page, in request order");
    assert_eq!(targets[0]["page"], "probe.md");
    assert_eq!(targets[1]["page"], "other.md");
    assert_eq!(targets[1]["loaded"][0]["block"], "only");
}
