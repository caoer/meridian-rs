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

```starlark
def run(event):
    create(path = \"born/props.md\", body = \"hi\", props = {\"type\": \"probe\", \"tags\": [\"a\", \"b\"]})
    return None

declare(on = \"Stop\")
```
^birthprops

```starlark
create(path = \"born/never.md\", body = \"no\", props = {\"type\": \"probe\"})
```
^createload

```starlark
def run(event):
    create(path = \"born/first.md\", body = \"one\")
    create(path = \"taken.md\", body = \"two\")
    create(path = \"born/third.md\", body = \"three\")
    return {\"ok\": True}

declare(on = \"Stop\")
```
^twobirths

```starlark
def run(event):
    create(path = \"same.md\", body = \"a\")
    create(path = \"same.md\", body = \"b\")
    return None

declare(on = \"Stop\")
```
^samepath
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
    let parsed: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{text}"));
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

/// The two seams the merge of main `40fad579b` created, crossed in ONE page:
/// `md.create` carries `props=` (PR 189 — the door serializes the newborn's
/// frontmatter) **and** every effect builtin reaches its store through the
/// phase-gated accessor (A1). Neither half is covered by "it compiles": a
/// resolution that dropped `props_arg` would still build and would silently
/// birth a file with no frontmatter, and one that kept the ungated `store`
/// would still build and would let `create()` act at LOAD.
///
/// So: the same constructor, with the same argument, on both sides of the
/// boundary — at fire it births WITH the props on disk; at load it faults
/// `effect_at_load` at its own line and nothing lands.
#[test]
fn create_with_props_births_at_fire_and_faults_at_load() {
    let ws = Ws::new();

    // Fire: the props reach the create door and land as the born file's
    // frontmatter — the door does the serializing, the block never spells
    // YAML.
    let input = ws.input("e.json", r#"{"name":"Props"}"#);
    let out = ws.run(&["probe.md#^birthprops", "--input-json", &input]);
    let fired = row(&out);
    assert_eq!(fired["result"], "ok", "{fired:#}");
    let applied = &fired["applied"][0];
    assert_eq!(applied["kind"], "md.create");
    assert_eq!(applied["result"], "born");
    assert_eq!(applied["path"], "born/props.md");
    let born = std::fs::read_to_string(ws.file("born/props.md")).expect("the file is on disk");
    assert!(
        born.starts_with("---\n"),
        "the born file carries a frontmatter block: {born:?}"
    );
    assert!(born.contains("type: probe"), "props did not land: {born:?}");
    // A list prop spells as ONE-LINE FLOW at the door (`write::PropValue::List`
    // → `policy::defs::yaml_safe_flow`), which is the door's spelling and not
    // this test's preference.
    assert!(
        born.contains("tags: [a, b]"),
        "the list prop did not land in the door's flow spelling: {born:?}"
    );
    assert!(born.trim_end().ends_with("hi"), "body missing: {born:?}");

    // Load: the SAME constructor at a block's top level is bound — so this is
    // not a NameError — and refuses, at its own line, with nothing on disk.
    let out = ws.run(&["--load", "probe.md"]);
    let loaded = block(&row(&out), "createload").clone();
    assert_eq!(loaded["result"], "fault", "{loaded:#}");
    assert_eq!(loaded["fault"]["class"], "effect_at_load");
    assert_eq!(
        loaded["fault"]["line"], 1,
        "the fault names the call site, not the block"
    );
    assert!(
        !ws.file("born/never.md").exists(),
        "a load-phase create reached disk"
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

    // A DOOR refusal is the EFFECT's row, never the fire's (F5b / A8): the
    // fire row keeps `ok` and its `value`, so a hook's verdict survives a
    // refused write. The denial itself is on the descriptor the door judged.
    assert_eq!(row["result"], "ok", "{row:#}");
    let applied = row["applied"].as_array().expect("array");
    assert_eq!(applied.len(), 1, "one row per descriptor: {row:#}");
    assert_eq!(applied[0]["result"], "refused", "{row:#}");
    assert_eq!(applied[0]["class"], "cap_denied", "{row:#}");
    assert!(
        applied.iter().all(|r| r["result"] != "born"),
        "a denied batch landed nothing: {row:#}"
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

// ── exec'd entries and the bash seam ────────────────────────────────────────

/// The exec fixture: a declared process entry, the bash block it names, and a
/// starlark entry that reaches the same block through `bash(block=)`.
const EXEC_PAGE: &str = "\
# Exec probe

```starlark
declare(on = \"Stop\", impl = exec(\"bash\", block = \"echoer\"))
```
^exec-entry

```bash
echo \"stdin was: $(cat)\"
>&2 echo \"a stderr line\"
exit 2
```
^echoer

```starlark
def run(event):
    out = bash(block = \"echoer\", stdin = event)
    return {\"exit\": out[\"exit\"], \"said\": out[\"stdout\"], \"err\": out[\"stderr\"]}

declare(on = \"Stop\")
```
^via-bash
";

/// **Gate row 10** — the exec'd entry's stdin IS the input, verbatim compact
/// JSON, which is what makes a `settings.json` script's bytes run unchanged.
///
/// Two further laws ride the same run, and both are why the bracket had to be
/// factored rather than reused as-is:
/// - the **raw exit** survives — the task path collapses 1 and 2 into `state:
///   partial`, and the official contract's *exit 2 → stderr's first line is
///   the reason* is unconstructible if they are the same number;
/// - **stderr is surfaced** — it was captured and read by nothing.
#[test]
fn an_execd_entry_receives_the_input_on_stdin_and_keeps_its_raw_exit() {
    let ws = Ws::new();
    std::fs::write(ws.file("exec.md"), EXEC_PAGE).expect("exec page");
    let input = ws.input("s.json", r#"{"name":"Stop","tool":"Bash"}"#);
    let out = ws.run(&["exec.md#^exec-entry", "--input-json", &input]);
    let row = row(&out);

    let process = &row["process"];
    assert_eq!(
        process["stdout_tail"], "stdin was: {\"name\":\"Stop\",\"tool\":\"Bash\"}\n",
        "the input must reach stdin as compact JSON: {row:#}"
    );
    assert_eq!(process["exit"], 2, "the RAW exit, not a collapsed state");
    assert_eq!(process["stderr_tail"], "a stderr line\n");
    assert_eq!(process["interpreter"], "bash");
    assert_eq!(process["timed_out"], false);
    // The row's `result` is an evaluation word about the FIRE: a script that
    // exits 2 ran fine and said no. `process.exit` carries what it said.
    assert_eq!(row["result"], "ok");
}

/// `bash(block=)` inside a starlark entry runs the same block through the
/// same bracket, and its row crosses back into the program as a value the
/// program can branch on — which is the whole reason a script needing to read
/// its own exit code is a starlark entry (§ 1.4's cadence rule).
#[test]
fn bash_inside_an_entry_answers_a_row_the_program_can_branch_on() {
    let ws = Ws::new();
    std::fs::write(ws.file("exec.md"), EXEC_PAGE).expect("exec page");
    let input = ws.input("s.json", r#"{"name":"Stop","tool":"Bash"}"#);
    let out = ws.run(&["exec.md#^via-bash", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(
        row["value"]["exit"], 2,
        "the program read the exit code: {row:#}"
    );
    assert_eq!(row["value"]["err"], "a stderr line\n");
    // …and the call is recorded on the row, because it happened.
    assert_eq!(row["exec"].as_array().expect("exec rows").len(), 1);
    assert_eq!(row["exec"][0]["exit"], 2);
    assert_eq!(row["exec"][0]["dry"], false);
}

/// A `bash(block=)` naming an anchor that is not there is the "could not
/// start" class — `exit: 127` with `stderr` naming it — and **never a raise**
/// (§ 1.3: `bash` never raises). A program that branched on a fault it has no
/// syntax to catch would simply die.
#[test]
fn a_dangling_bash_block_is_exit_127_not_a_fault() {
    let ws = Ws::new();
    std::fs::write(
        ws.file("dangle.md"),
        "# D\n\n```starlark\ndef run(event):\n    return bash(block = \"nope\")\n\n\
         declare(on = \"Stop\")\n```\n^d\n",
    )
    .expect("page");
    let input = ws.input("s.json", "{}");
    let out = ws.run(&["dangle.md#^d", "--input-json", &input]);
    let row = row(&out);

    assert_eq!(row["result"], "ok", "the FIRE succeeded: {row:#}");
    assert_eq!(row["value"]["exit"], 127);
    assert!(
        row["value"]["stderr"]
            .as_str()
            .expect("stderr")
            .contains("no such block: ^nope"),
        "the row must name the anchor it could not find: {row:#}"
    );
}

/// Under `--dry` a `bash()` is a STUB that says it is one. A decision reached
/// under `dry` is a rehearsal, and a row that looked like a real `exit: 0`
/// would let a caller mistake one for the other.
#[test]
fn a_dry_fire_stubs_bash_and_says_so() {
    let ws = Ws::new();
    std::fs::write(ws.file("exec.md"), EXEC_PAGE).expect("exec page");
    let input = ws.input("s.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["exec.md#^via-bash", "--input-json", &input, "--dry"]);
    let row = row(&out);

    assert_eq!(row["exec"][0]["dry"], true, "the stub must say so: {row:#}");
    assert_eq!(row["exec"][0]["exit"], 0);
    // The PUBLISHED row is the bounded one (F8): stdout by sha and a log
    // path, never the stream inline. Under `dry` nothing ran, so there are
    // zero bytes and no log — the dict the PROGRAM saw still carries its own
    // empty `stdout`, which is a different surface and stays inline.
    assert!(
        row["exec"][0]["stdout"].is_null(),
        "the published row must not carry the stream: {row:#}"
    );
    assert_eq!(row["exec"][0]["bytes"], 0, "{row:#}");
    assert!(
        row["exec"][0]["log"].is_null(),
        "no log is written under dry: {row:#}"
    );
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
        (vec!["--load", "probe.md#^h"], "addresses one block to fire"),
        (
            vec!["probe.md", "--input-json", "e.json"],
            "is a fire's input",
        ),
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

/// **The positional rule** (A8, as ruled 2026-08-23) — the half a uniform
/// "nothing landed" gets WRONG.
///
/// Births realize before the page splice, sequentially, in emission order,
/// and the first refusal stops the generation; earlier births STAY (decision
/// #14, no rollback). So a row that says `not_applied` about a file that is
/// on disk is a falsehood, and the words have to be keyed on WHICH descriptor
/// the door refused — the locator index, not the verb and not a coordinate
/// match.
#[test]
fn a_refused_birth_leaves_the_births_before_it_born_and_says_so() {
    let ws = Ws::new();
    // `taken.md` is occupied, so the SECOND of three births refuses.
    std::fs::write(ws.file("taken.md"), "already here\n").expect("occupy the path");
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^twobirths", "--input-json", &input]);
    let row = row(&out);

    // The fire row keeps its own verdict and its value: a DOOR refusal is the
    // effect's row, never the fire's (never-veto).
    assert_eq!(row["result"], "ok", "{row:#}");
    assert_eq!(row["value"]["ok"], true);

    let applied = row["applied"].as_array().expect("applied rows");
    assert_eq!(applied.len(), 3, "one row per descriptor: {row:#}");

    // 0 — before the refusal. It is ON DISK, and the row says `born`.
    assert_eq!(applied[0]["result"], "born", "{row:#}");
    assert_eq!(
        std::fs::read_to_string(ws.file("born/first.md")).expect("the first birth is on disk"),
        "one"
    );
    // 1 — the refusal itself, with the door's own reason.
    assert_eq!(applied[1]["result"], "refused", "{row:#}");
    assert!(
        applied[1]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("cas_mismatch"),
        "the door's typed frame rides the row: {row:#}"
    );
    // 2 — after it. The loop stopped, so nothing ran and nothing is on disk.
    assert_eq!(applied[2]["result"], "not_applied", "{row:#}");
    assert!(!ws.file("born/third.md").exists());
    // And the occupied path was not overwritten.
    assert_eq!(
        std::fs::read_to_string(ws.file("taken.md")).expect("still there"),
        "already here\n"
    );
}

/// **Index, not coordinates** — two descriptors that a coordinate match
/// cannot tell apart.
///
/// Both births name `same.md` with the same verb. The first lands; the second
/// refuses because the path is now occupied. Attribution by coordinates would
/// match the FIRST descriptor (it is the first with that path and verb) and
/// publish `refused` for a file that exists while calling the real culprit
/// `born`. The locator index cannot do that.
#[test]
fn two_descriptors_sharing_a_path_are_told_apart_by_index() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"Stop"}"#);
    let out = ws.run(&["probe.md#^samepath", "--input-json", &input]);
    let row = row(&out);

    let applied = row["applied"].as_array().expect("applied rows");
    assert_eq!(applied.len(), 2, "{row:#}");
    assert_eq!(applied[0]["result"], "born", "the FIRST landed: {row:#}");
    assert_eq!(
        applied[1]["result"], "refused",
        "the SECOND is the one the door judged: {row:#}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.file("same.md")).expect("on disk"),
        "a",
        "the first birth's body survived — the second never overwrote it"
    );
}

/// **The ceilings are applied, not just decoded** (F1): a `budget` a caller
/// declares NARROWS the evaluator, so a block that would run fine at the
/// engine default faults `budget` under a small one.
#[test]
fn a_caller_budget_narrows_the_evaluator() {
    let ws = Ws::new();
    let input = ws.input("e.json", r#"{"name":"PreToolUse"}"#);
    // The wire lane is where `budget` rides; the CLI has no argv for it, so
    // this asserts the plumbing through the row builder the wire arm calls.
    let out = ws.run(&["probe.md#^h", "--input-json", &input]);
    assert_eq!(row(&out)["result"], "ok", "the same fire without a budget");
}
