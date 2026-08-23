//! Gates for the `MRD_TIMING` timing-only log mode (`docs/status.md` § The
//! timing mode; `docs/run-plane.md` § Timing phases), driving the REAL binary
//! over its process boundary — the switch is a process-global read, so only a
//! real process can prove it.
//!
//! The claims under gate: the line grammar parses; the mode is time cost and
//! nothing else; stdout and the exit code are byte-identical with it on and
//! off; the file sink keeps stderr clean; an off word never creates a file.

use std::path::Path;
use std::process::{Command, Output};

/// A single-task page whose block emits NO effect — so two runs of it leave
/// the corpus byte-identical and their reports are comparable.
const SOLO_PAGE: &str = "\
---
task.solo: \"[[#^solo-1]]\"
---

# Tasks

```starlark
def run(ctx):
    pass
```
^solo-1
";

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("solo.md"), SOLO_PAGE).expect("page");
        Self { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    /// `mrd <args>` with the workspace pinned via the tier-1 override and the
    /// switch set to `timing` (unset when `None`).
    fn mrd(&self, timing: Option<&str>, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mrd"));
        command
            .args(args)
            .env("MERIDIAN_WORKSPACE", self.path())
            .current_dir(self.path());
        if let Some(value) = timing {
            command.env("MRD_TIMING", value);
        } else {
            command.env_remove("MRD_TIMING");
        }
        command.output().expect("spawn mrd")
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every `mrd-timing` line on a stream, parsed into `(cmd, phase, us)`. A line
/// carrying the prefix that does NOT parse is a failure, not a skip: the
/// grammar is the contract.
fn timing_lines(raw: &str) -> Vec<(String, String, u128)> {
    raw.lines()
        .filter(|line| line.starts_with("mrd-timing"))
        .map(|line| {
            let mut fields = line.split(' ');
            assert_eq!(fields.next(), Some("mrd-timing"), "{line}");
            let cmd = fields
                .next()
                .and_then(|f| f.strip_prefix("cmd="))
                .unwrap_or_else(|| panic!("no cmd= field: {line}"));
            let phase = fields
                .next()
                .and_then(|f| f.strip_prefix("phase="))
                .unwrap_or_else(|| panic!("no phase= field: {line}"));
            let us: u128 = fields
                .next()
                .and_then(|f| f.strip_prefix("us="))
                .unwrap_or_else(|| panic!("no us= field: {line}"))
                .parse()
                .unwrap_or_else(|e| panic!("us= is not an integer ({e}): {line}"));
            assert!(fields.next().is_none(), "extra field: {line}");
            (cmd.to_owned(), phase.to_owned(), us)
        })
        .collect()
}

fn phases(raw: &str) -> Vec<String> {
    timing_lines(raw).into_iter().map(|(_, p, _)| p).collect()
}

/// The mode changes nothing a consumer reads: `--json` stdout is byte-for-byte
/// the same with the switch on and off, and so is the exit code. Off, stderr
/// carries no timing line at all.
#[test]
fn off_and_on_agree_on_stdout_and_exit_code() {
    let ws = Ws::new();
    let off = ws.mrd(None, &["run", "solo.md", "--json"]);
    let on = ws.mrd(Some("1"), &["run", "solo.md", "--json"]);

    assert_eq!(off.status.code(), Some(0), "{}", stderr(&off));
    assert_eq!(on.status.code(), Some(0), "{}", stderr(&on));
    assert_eq!(
        off.stdout,
        on.stdout,
        "the switch moved stdout:\noff: {}\non:  {}",
        String::from_utf8_lossy(&off.stdout),
        String::from_utf8_lossy(&on.stdout)
    );
    assert!(
        timing_lines(&stderr(&off)).is_empty(),
        "off wrote timing lines: {}",
        stderr(&off)
    );
    assert!(
        !timing_lines(&stderr(&on)).is_empty(),
        "on wrote none: {}",
        stderr(&on)
    );
}

/// `0` and the spelled-out off words are OFF — and an off word must not be
/// taken for a file path, which would create a file named `off` in the
/// caller's working directory.
#[test]
fn off_words_are_off_and_create_no_file() {
    let ws = Ws::new();
    for word in ["0", "off", "false", "no", ""] {
        let out = ws.mrd(Some(word), &["run", "solo.md", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(
            timing_lines(&stderr(&out)).is_empty(),
            "MRD_TIMING={word} emitted: {}",
            stderr(&out)
        );
        assert!(
            word.is_empty() || !ws.path().join(word).exists(),
            "MRD_TIMING={word} created a file named {word}"
        );
    }
}

/// The phase list a reader of `mrd run` gets — the grain `run-plane.md`
/// § Timing phases promises. `total` closes the stream: it contains every
/// other phase, and lines print in completion order.
#[test]
fn a_run_reports_its_phases_and_total_is_last() {
    let ws = Ws::new();
    let out = ws.mrd(Some("1"), &["run", "solo.md", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let lines = timing_lines(&stderr(&out));
    let names = phases(&stderr(&out));

    for expected in [
        "workspace.resolve",
        "page.load",
        "conventions.load",
        "task.gate",
        "pre_eval",
        "snapshot.walk",
        "snapshot.read",
        "snapshot.fold",
        "snapshot",
        "eval",
        "dispatch",
        "cascade",
        "report.render",
        "total",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "no `{expected}` phase in: {names:?}"
        );
    }
    // Every line names the process's entry verb, not a request.
    for (cmd, phase, _) in &lines {
        assert_eq!(cmd, "run", "wrong cmd on `{phase}`");
    }
    assert_eq!(
        names.last().map(String::as_str),
        Some("total"),
        "total is not last: {names:?}"
    );
    // Nesting is real, not decorative: the whole snapshot never measures less
    // than the read inside it.
    let of = |want: &str| {
        lines
            .iter()
            .find(|(_, p, _)| p == want)
            .map(|(_, _, us)| *us)
            .unwrap_or_else(|| panic!("no {want}"))
    };
    assert!(of("snapshot") >= of("snapshot.read"));
    assert!(of("total") >= of("snapshot"));
}

/// The `solo` block emits no md.* effect, so there is no batch — and no
/// `apply` line claiming there was one.
#[test]
fn a_phase_that_did_not_run_emits_no_line() {
    let ws = Ws::new();
    let out = ws.mrd(Some("1"), &["run", "solo.md", "--json"]);
    assert!(
        !phases(&stderr(&out)).iter().any(|p| p == "apply"),
        "an effect-free run reported an apply: {}",
        stderr(&out)
    );
}

/// A value that is not a word is a sink FILE: the lines land there and stderr
/// stays clean, which is what makes the mode usable on a process whose stderr
/// is not the caller's (the daemon nulls its own — `daemon.rs`).
#[test]
fn a_path_value_sinks_to_that_file_and_leaves_stderr_clean() {
    let ws = Ws::new();
    let sink_dir = tempfile::tempdir().expect("sink dir");
    let sink = sink_dir.path().join("timing.log");

    let out = ws.mrd(
        Some(sink.to_str().expect("utf-8 sink path")),
        &["run", "solo.md", "--json"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        timing_lines(&stderr(&out)).is_empty(),
        "the file sink still wrote to stderr: {}",
        stderr(&out)
    );

    let written = std::fs::read_to_string(&sink).expect("the sink file was created");
    assert!(
        phases(&written).iter().any(|p| p == "total"),
        "no phases in the sink file: {written}"
    );

    // Append, never truncate: a second run's lines join the first's.
    let first = timing_lines(&written).len();
    let _ = ws.mrd(
        Some(sink.to_str().expect("utf-8 sink path")),
        &["run", "solo.md", "--json"],
    );
    let both = std::fs::read_to_string(&sink).expect("sink file");
    assert!(
        timing_lines(&both).len() > first,
        "the second run truncated the sink instead of appending"
    );
}

/// The instrument is not `run`-shaped: `cmd=` is whatever verb entered the
/// process, and `total` is there for every one of them.
#[test]
fn every_verb_reports_a_total_under_its_own_name() {
    let ws = Ws::new();
    let out = ws.mrd(Some("1"), &["version"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        timing_lines(&stderr(&out))
            .iter()
            .any(|(cmd, phase, _)| cmd == "version" && phase == "total"),
        "no `cmd=version phase=total` line: {}",
        stderr(&out)
    );
}

/// Discoverability: the switch is on the help surface of every verb, because
/// it belongs to no verb.
#[test]
fn the_switch_is_on_the_help_surface() {
    let ws = Ws::new();
    let out = ws.mrd(None, &["run", "--help"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("MRD_TIMING"), "{text}");
}
