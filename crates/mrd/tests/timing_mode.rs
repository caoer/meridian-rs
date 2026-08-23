//! Gates for the `MRD_TIMING` timing-only log mode (`docs/status.md` § The
//! timing mode; `docs/run-plane.md` § Timing phases), driving the REAL binary
//! over its process boundary — the switch is a process-global read, so only a
//! real process can prove it.
//!
//! The claims under gate: the line grammar parses and a diagnostic can never be
//! read as a measurement; a line means the phase COMPLETED; the mode is time
//! cost and nothing else; stdout and the exit code are byte-identical with it
//! on and off; the two file-sink refusals degrade loudly to stderr; and no
//! switch value spelled by a human creates a stray file.

use std::path::Path;
use std::process::{Command, Output};

mod common;

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

/// The fixture the PHASE-LIST gate drives, named once so it is the gate's only
/// parameter.
///
/// It has to be a page whose run REACHES every phase in the list — the corpus
/// fold included.
///
/// **`stamp.md`, since the lazy fold landed (seat `8ed22d72`).** This constant
/// was authored as `emit.md` precisely so the change would be a rebase and not
/// a rewrite, and the rebase is this line. `emit.md` is one `notice`, and the
/// live gate for the fold is md.\*-only: the live report renders `kind` +
/// `domain` (`run::report::EffectLine`), so a `notice`'s `root_at_eval` has no
/// reader and folding for it would buy nothing (`run-plane.md` § The run
/// plane). A live `notice` therefore reaches no `snapshot` any more, and
/// neither does `solo.md`'s `pass` — so the one page that still reaches every
/// phase LIVE is the one that commits.
///
/// The cost of the swap, stated because it is real: this gate now writes.
/// `stamp.md` sets a field and lands a receipt, so it is no longer "about
/// phases and never about writes" — but each test gets a fresh `Ws`, and the
/// alternative is a gate that asserts the presence of a phase the plane no
/// longer emits.
const PHASE_LIST_PAGE: &str = "stamp.md";

/// One `notice` — a `proto.*` effect with no local executor. See
/// [`PHASE_LIST_PAGE`].
const EMIT_PAGE: &str = "\
---
task.emit: \"[[#^emit-1]]\"
---

# Tasks

```starlark
def run(ctx):
    notice(message = \"advisory\")
```
^emit-1
";

/// A page whose task COMMITS: the md.\* batch makes `apply` real, and the run
/// is the one `run-plane.md` § The `snapshot` set can repeat is about.
const EFFECTFUL_PAGE: &str = "\
---
status: draft
task.stamp: \"[[#^stamp-1]]\"
task.stamp.caps: md.edit
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"live\")
```
^stamp-1
";

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("solo.md"), SOLO_PAGE).expect("page");
        std::fs::write(tmp.path().join("emit.md"), EMIT_PAGE).expect("emit page");
        std::fs::write(tmp.path().join("stamp.md"), EFFECTFUL_PAGE).expect("effectful page");
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

    /// `links --json` forced onto the ephemeral path: a resident daemon on the
    /// host would otherwise answer, and the client's sink would then miss
    /// `snapshot.*` / `corpus.build` (those fire in the daemon, stderr nulled).
    fn links_json(&self, timing: Option<&str>) -> Output {
        // home / rt / cache are siblings of the workspace, not members of it
        // (a non-dot `.md` under the workspace root is a corpus file).
        let side = tempfile::tempdir().expect("sidecar");
        let home = side.path().join("home");
        let rt = side.path().join("rt");
        let cache = side.path().join("cache");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&rt).expect("runtime dir");
        std::fs::create_dir_all(&cache).expect("cache dir");
        std::fs::write(
            home.join("MERIDIAN.md"),
            "---\ntype: meridian-config\nversion: 1\n---\n\n# empty table\n",
        )
        .expect("empty mount table");
        let mut command = common::mrd_command(&home, &cache);
        command
            .args(["links", "--json"])
            .env("MERIDIAN_CONFIG", home.join("MERIDIAN.md"))
            .env("MERIDIAN_WORKSPACE", self.path())
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            // Linux-only: `registry::socket_path_for_cache_root` reads
            // `XDG_RUNTIME_DIR` under `#[cfg(target_os = "linux")]`. On macOS
            // it is inert; `XDG_CACHE_HOME` is the load-bearing sock-key override.
            .env("XDG_RUNTIME_DIR", &rt)
            .current_dir(self.path());
        if let Some(value) = timing {
            command.env("MRD_TIMING", value);
        } else {
            command.env_remove("MRD_TIMING");
        }
        let out = command.output().expect("spawn mrd");
        drop(side);
        out
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every MEASUREMENT on a stream, parsed into `(cmd, phase, us)`.
///
/// The filter is the documented `mrd-timing ` — WITH the space — which is what
/// separates a measurement from a `mrd-timing:` diagnostic. A line that opens
/// with the space and does not parse is a failure, not a skip: the grammar is
/// the contract.
fn timing_lines(raw: &str) -> Vec<(String, String, u128)> {
    raw.lines()
        .filter(|line| line.starts_with("mrd-timing "))
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

/// The mode's own diagnostics — the COLON shape.
fn diagnostics(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|line| line.starts_with("mrd-timing:"))
        .map(str::to_owned)
        .collect()
}

/// The mode changes nothing a consumer reads: `--json` stdout is byte-for-byte
/// the same with the switch on and off, and so is the exit code. Off, stderr
/// carries no timing line at all.
///
/// ASSUMPTION, named because it is load-bearing: this compares two SEPARATE
/// runs, which is a valid identity test only while the report of an
/// effect-free starlark task carries no clock, pid or invocation id. It does
/// not today (`run::report::render` sources `exec` from the bash path alone).
/// A time or id field added to that report breaks this test, and the fix is to
/// compare modulo that field, never to delete the assertion.
///
/// SECOND ASSUMPTION: the off case asserts the WHOLE stderr stream is empty,
/// not merely that it carries no `mrd-timing` line. That is deliberate — "off
/// costs nothing observable" is the claim — but it means an unrelated stderr
/// byte added to `mrd run` elsewhere would fail this test and blame the timing
/// mode. If that happens, narrow the assertion to
/// `timing_lines(..).is_empty() && diagnostics(..).is_empty()`; do not delete it.
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
        off.stderr.is_empty(),
        "off wrote to stderr: {}",
        stderr(&off)
    );
    assert!(
        !timing_lines(&stderr(&on)).is_empty(),
        "on wrote none: {}",
        stderr(&on)
    );
}

/// The same identity claim for `links --json` (PR 178). Ephemeral: a host
/// daemon would hide `snapshot.*` / `corpus.build` on the client sink.
#[test]
fn links_json_off_and_on_agree_and_reports_its_phases() {
    let ws = Ws::new();
    let off = ws.links_json(None);
    let on = ws.links_json(Some("1"));

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

    let lines = timing_lines(&stderr(&on));
    let names = phases(&stderr(&on));
    for expected in [
        "daemon.dial",
        "snapshot.walk",
        "snapshot.read",
        "snapshot.fold",
        "snapshot",
        "corpus.build",
        "links.read",
        "json.render",
        "json.write",
        "total",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "no `{expected}` phase in: {names:?}\n{}",
            stderr(&on)
        );
    }
    for (cmd, phase, _) in &lines {
        assert_eq!(cmd, "links", "wrong cmd on `{phase}`");
    }
    assert_eq!(
        names.last().map(String::as_str),
        Some("total"),
        "total is not last: {names:?}"
    );
    // Containment (K = 0: every corpus.build / snapshot.* is the workspace
    // build, before `links.read`). json.render is a sibling of links.read,
    // both inside total — not nested in links.read.
    let of = |want: &str| {
        lines
            .iter()
            .find(|(_, p, _)| p == want)
            .map_or_else(|| panic!("no {want}"), |(_, _, us)| *us)
    };
    assert!(of("total") >= of("links.read"));
    assert!(of("total") >= of("json.render"));
    assert!(of("snapshot") >= of("snapshot.read"));
}

/// K ≥ 1: a workspace page that names a mounted root must emit 1 + K
/// `corpus.build` lines, and the mount build sits inside `links.read`.
#[test]
fn links_json_repeats_corpus_build_per_mounted_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache = tmp.path().join("xdg-cache");
    let other = tmp.path().join("other");
    let ws = tmp.path().join("ws");
    for dir in [&home, &cache, &other, &ws] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    std::fs::write(
        other.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n\n# Other root\n",
    )
    .expect("root declaration");
    std::fs::write(other.join("present.md"), "# Present\n\nreal page.\n").expect("page");
    std::fs::write(
        home.join("MERIDIAN.md"),
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: other\npath: {}\nvault: othervault\n```\n",
            other.display()
        ),
    )
    .expect("mount table");
    std::fs::write(
        ws.join("claim.md"),
        "# Claim\n\n## Body\n\nsee [[other:present]].\n",
    )
    .expect("claim");

    let mut command = common::mrd_command(&home, &cache);
    command
        .args(["links", "--json"])
        .env("MERIDIAN_CONFIG", home.join("MERIDIAN.md"))
        .env("MERIDIAN_WORKSPACE", &ws)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .env("XDG_RUNTIME_DIR", tmp.path().join("rt"))
        .env("MRD_TIMING", "1")
        .current_dir(&ws);
    std::fs::create_dir_all(tmp.path().join("rt")).expect("runtime dir");
    let out = command.output().expect("spawn mrd");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"source\": \"ephemeral\"") || stdout.contains("\"source\":\"ephemeral\""),
        "want ephemeral: {stdout}"
    );

    let lines = timing_lines(&stderr(&out));
    let names = phases(&stderr(&out));
    let n_build = names.iter().filter(|p| *p == "corpus.build").count();
    let n_snap = names.iter().filter(|p| *p == "snapshot").count();
    assert_eq!(
        n_build,
        2,
        "want 1 + K=1 corpus.build lines (workspace + one mount); got {n_build}: {names:?}\n{}",
        stderr(&out)
    );
    assert_eq!(
        n_snap,
        2,
        "want 1 + K=1 snapshot lines (workspace + one mount); got {n_snap}: {names:?}\n{}",
        stderr(&out)
    );
    assert_eq!(
        names.last().map(String::as_str),
        Some("total"),
        "total is not last: {names:?}"
    );
    let of = |want: &str| {
        lines
            .iter()
            .find(|(_, p, _)| p == want)
            .map_or_else(|| panic!("no {want} in {names:?}"), |(_, _, us)| *us)
    };
    assert!(of("total") >= of("links.read"));
}

/// Off words are off — trimmed, in any case — and an off word must never be
/// taken for a file path. Measured on the release binary before the fix:
/// `MRD_TIMING=OFF` created a file named `OFF`, and `MRD_TIMING="1 "` created
/// one named `1 `, in the caller's working directory.
#[test]
fn off_words_are_off_in_any_case_and_create_no_file() {
    let ws = Ws::new();
    for word in ["0", "off", "OFF", "Off", "FALSE", "no", " off ", "  ", ""] {
        let out = ws.mrd(Some(word), &["run", "solo.md", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(
            out.stderr.is_empty(),
            "MRD_TIMING={word:?} wrote to stderr: {}",
            stderr(&out)
        );
    }
    let strays: Vec<String> = std::fs::read_dir(ws.path())
        .expect("read workspace")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| {
            !Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                && !name.starts_with('.')
        })
        .collect();
    assert!(
        strays.is_empty(),
        "an off word created a file in the workspace: {strays:?}"
    );
}

/// On words are on in any case and with stray whitespace — the same folding,
/// so `MRD_TIMING="1 "` measures instead of writing a file named `1 `.
#[test]
fn on_words_are_on_in_any_case_and_with_whitespace() {
    let ws = Ws::new();
    for word in ["1", "ON", "True", " 1 ", "yes"] {
        let out = ws.mrd(Some(word), &["run", "solo.md", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(
            !timing_lines(&stderr(&out)).is_empty(),
            "MRD_TIMING={word:?} measured nothing: {}",
            stderr(&out)
        );
        assert!(
            !ws.path().join(word).exists(),
            "MRD_TIMING={word:?} created a file"
        );
    }
}

/// The phase list a reader of `mrd run` gets — the grain `run-plane.md`
/// § Timing phases promises. `total` closes the stream: it contains every
/// other phase, and lines print in completion order.
///
/// Driven by [`PHASE_LIST_PAGE`], which is the only thing about this gate that
/// is allowed to change.
#[test]
fn a_run_reports_its_phases_and_total_is_last() {
    let ws = Ws::new();
    let out = ws.mrd(Some("1"), &["run", PHASE_LIST_PAGE, "--json"]);
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
    // Containment is real, not decorative: `run-plane.md` § Timing phases
    // states the "inside" column, and these are three of its rows.
    let of = |want: &str| {
        lines
            .iter()
            .find(|(_, p, _)| p == want)
            .map_or_else(|| panic!("no {want}"), |(_, _, us)| *us)
    };
    assert!(of("snapshot") >= of("snapshot.read"));
    assert!(of("dispatch") >= of("snapshot"));
    assert!(of("total") >= of("dispatch"));
}

/// The inversion this test was authored for. It read, on main: "on main TODAY
/// an effect-free run folds the corpus too … **this is the assertion the
/// lazy-snapshot work (seat `8ed22d72`) inverts**, deliberately its own test so
/// that flip is a two-line conflict rather than a rewrite of the gate that
/// matters." This is that flip.
///
/// The fold is now LAZY and gated per tense (`run-plane.md` § The run plane):
/// a run folds only when THAT tense's output will put `root_at_eval` in front
/// of a reader. Live, the report is `kind` + `domain`
/// (`run::report::EffectLine`), so only an md.\* batch — which carries the
/// token onward to the receipt's `root_pin` — buys the walk.
///
/// So two live runs that used to fold now do not, and they are the two that
/// matter: `solo.md` emits nothing at all, and `emit.md` emits one `notice`,
/// which is the shape the hook plane fires once per event. On a 37 800-member
/// root that fold was 99.5% of the run.
#[test]
fn a_live_run_whose_gate_does_not_fire_never_folds() {
    let ws = Ws::new();
    for page in ["solo.md", "emit.md"] {
        let out = ws.mrd(Some("1"), &["run", page, "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let names = phases(&stderr(&out));
        // `eval` proves the block ran: a run that emitted no phase at all
        // would pass the absence check below for the wrong reason.
        assert!(
            names.iter().any(|n| n == "eval"),
            "{page} did not run: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("snapshot")),
            "{page} folded the corpus for a token with no reader: {names:?}"
        );
    }
}

/// The `solo` block emits no md.* effect, so there is no batch — and no
/// `apply` line claiming there was one.
///
/// The `snapshot` half of "did not run" belongs to
/// [`a_live_run_whose_gate_does_not_fire_never_folds`], which owns it for both
/// pages; asserting it here too would state one fact in two places and drift
/// in one of them.
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

/// A phase that FAILED emits no line either — a failed page load costing 312 us
/// and a successful one costing 312 us are not the same fact, and the mode
/// reports COMPLETED phases only. `total` still reports: the process finished.
#[test]
fn a_phase_that_failed_emits_no_line() {
    let ws = Ws::new();
    let out = ws.mrd(Some("1"), &["run", "no-such-page.md", "--json"]);
    assert_ne!(out.status.code(), Some(0), "the run must refuse");
    let names = phases(&stderr(&out));

    assert!(
        !names.iter().any(|p| p == "page.load"),
        "a FAILED page load reported as completed: {names:?}"
    );
    // The phase before it completed, so it reports; and the process completed,
    // so `total` reports.
    assert!(
        names.iter().any(|p| p == "workspace.resolve"),
        "the completed phase before the failure is missing: {names:?}"
    );
    assert!(
        names.iter().any(|p| p == "total"),
        "total must report on a refusal — the process is what it measures: {names:?}"
    );
    // And nothing downstream of the failure invented itself.
    for never in ["snapshot", "eval", "dispatch", "report.render"] {
        assert!(
            !names.iter().any(|p| p == never),
            "`{never}` reported after the run refused: {names:?}"
        );
    }
}

/// An effectful run: `apply` becomes real, and — on the CLI lane — the
/// `snapshot` set appears exactly ONCE. The second fold documented in
/// `run-plane.md` is the executor's pre-commit fold, which is gated on a
/// `DeltaSink`; the CLI passes `delta: None`, and the cascade fold needs a
/// non-empty ruleset the CLI never hands it.
///
/// **The `count == 1` below is as exposed to the lazy-snapshot work (seat
/// `8ed22d72`) as `an_effect_free_run_folds_today` is** — if the fold moves,
/// this count moves with it. Same rebase, same two lines.
#[test]
fn an_effectful_run_reports_apply_and_folds_once_on_the_cli() {
    let ws = Ws::new();
    let out = ws.mrd(Some("1"), &["run", "stamp.md", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let names = phases(&stderr(&out));

    assert!(
        names.iter().any(|p| p == "apply"),
        "a committing run reported no apply: {names:?}"
    );
    assert_eq!(
        names.iter().filter(|p| *p == "snapshot").count(),
        1,
        "the CLI lane folds once per run: {names:?}"
    );
    for part in ["snapshot.walk", "snapshot.read", "snapshot.fold"] {
        assert_eq!(
            names.iter().filter(|p| *p == part).count(),
            1,
            "`{part}` should appear once per fold: {names:?}"
        );
    }
}

/// A value that is not a word is a sink FILE: the lines land there and stderr
/// stays clean, which is what makes the mode usable on a process whose stderr
/// is not the caller's (the daemon nulls its own — `daemon.rs`).
#[test]
fn a_path_value_sinks_to_that_file_and_leaves_stderr_clean() {
    let ws = Ws::new();
    let sink_dir = tempfile::tempdir().expect("sink dir");
    let sink = sink_dir.path().join("timing.log");
    let sink_arg = sink.to_str().expect("utf-8 sink path");

    let out = ws.mrd(Some(sink_arg), &["run", "solo.md", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        out.stderr.is_empty(),
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
    let _ = ws.mrd(Some(sink_arg), &["run", "solo.md", "--json"]);
    let both = std::fs::read_to_string(&sink).expect("sink file");
    assert!(
        timing_lines(&both).len() > first,
        "the second run truncated the sink instead of appending"
    );
}

/// A sink the engine would FOLD is a loop: the file is a corpus member, so
/// every line it gains changes the corpus and the next fold reports more lines
/// to append. Refused by extension, before the file is opened — and said out
/// loud, because a silent degrade reads as "the code never ran there".
#[test]
fn a_corpus_extension_sink_is_refused_and_says_why() {
    let ws = Ws::new();
    let sink_dir = tempfile::tempdir().expect("sink dir");
    for name in ["timing.md", "timing.base", "TIMING.MD"] {
        let sink = sink_dir.path().join(name);
        let out = ws.mrd(
            Some(sink.to_str().expect("utf-8 sink path")),
            &["run", "solo.md", "--json"],
        );
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(!sink.exists(), "`{name}` was opened as a sink");

        let said = diagnostics(&stderr(&out));
        assert_eq!(
            said.len(),
            1,
            "expected one diagnostic for {name}: {said:?}"
        );
        assert!(
            said[0].contains("stderr"),
            "the diagnostic must name where the lines went instead: {}",
            said[0]
        );
        // Degrade, never silence: the measurements are on stderr.
        assert!(
            phases(&stderr(&out)).iter().any(|p| p == "total"),
            "refusing the sink also lost the measurements: {}",
            stderr(&out)
        );
    }
}

/// A sink that will not open degrades the same way, loudly.
#[test]
fn an_unopenable_sink_degrades_to_stderr_and_says_why() {
    let ws = Ws::new();
    let sink_dir = tempfile::tempdir().expect("sink dir");
    let sink = sink_dir.path().join("no-such-dir").join("timing.log");

    let out = ws.mrd(
        Some(sink.to_str().expect("utf-8 sink path")),
        &["run", "solo.md", "--json"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a bad sink never fails the verb"
    );
    let said = diagnostics(&stderr(&out));
    assert_eq!(said.len(), 1, "expected one diagnostic: {said:?}");
    assert!(
        said[0].contains("cannot open"),
        "the diagnostic must name the failure: {}",
        said[0]
    );
    assert!(
        phases(&stderr(&out)).iter().any(|p| p == "total"),
        "the degrade lost the measurements: {}",
        stderr(&out)
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

/// `cmd=` is raw argv, and argv can carry anything. A verb with whitespace
/// would mint a fourth field and a newline would inject a line into a
/// machine-read stream, so the label is refused and the default stands — the
/// grammar survives an invocation that does not.
#[test]
fn an_argv_that_would_break_the_grammar_does_not_become_the_label() {
    let ws = Ws::new();
    for argv in ["run solo.md", "a\nb"] {
        let out = ws.mrd(Some("1"), &[argv]);
        assert_eq!(out.status.code(), Some(2), "an unknown verb refuses");
        let lines = timing_lines(&stderr(&out));
        assert!(
            lines
                .iter()
                .any(|(cmd, phase, _)| cmd == "mrd" && phase == "total"),
            "the label was not defended for {argv:?}: {}",
            stderr(&out)
        );
    }
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
