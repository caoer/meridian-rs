//! Kernel entry #3 — the script entry. The module top level IS the program:
//! no hook lookup, `read()` as the one effectful builtin reaching an injected
//! [`ScriptHost`], every response recorded, replay against the recording
//! byte-identical, and the read budget a typed refusal.
//!
//! Authority: `docs/run-plane.md` § The script entry.

use std::collections::BTreeMap;

use effects::{
    ArmedEdit, EvalError, ReadFace, ReadFault, ReadPosition, ScriptCtx, ScriptHost, ScriptLimits,
    SecFacts, TocEntry, TocFacts, eval_script, replay_script,
};

/// A fixture workspace: paths → toc facts, `(path, section)` → section facts.
/// Counts its calls so a test can prove a host was (or was not) consulted.
struct FixtureHost {
    actor: String,
    pages: BTreeMap<String, TocFacts>,
    sections: BTreeMap<(String, String), SecFacts>,
    calls: usize,
}

impl FixtureHost {
    fn new() -> Self {
        let mut pages = BTreeMap::new();
        for (path, owner, status) in [
            ("tasks/0011.md", "", "todo"),
            ("tasks/0012.md", "8ab41c02", "todo"),
            ("tasks/0013.md", "de348282", "doing"),
        ] {
            let mut fm = BTreeMap::new();
            fm.insert("owner".to_owned(), owner.to_owned());
            fm.insert("status".to_owned(), status.to_owned());
            pages.insert(
                path.to_owned(),
                TocFacts {
                    rev: format!("rev-{path}"),
                    fm,
                    toc: vec![
                        TocEntry {
                            section: "Goals".to_owned(),
                            anchor: None,
                            rev: "sec-goals".to_owned(),
                        },
                        TocEntry {
                            section: "Notes".to_owned(),
                            anchor: Some("^n1".to_owned()),
                            rev: "sec-notes".to_owned(),
                        },
                    ],
                    words: 41,
                },
            );
        }
        let mut sections = BTreeMap::new();
        sections.insert(
            ("tasks/0011.md".to_owned(), "Goals".to_owned()),
            SecFacts {
                text: "ship the script entry\n".to_owned(),
                rev: "sec-goals".to_owned(),
            },
        );
        Self {
            actor: "8ab41c02".to_owned(),
            pages,
            sections,
            calls: 0,
        }
    }
}

impl ScriptHost for FixtureHost {
    fn toc(&mut self, path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        self.calls += 1;
        self.pages.get(path).cloned().ok_or_else(|| ReadFault {
            path: path.to_owned(),
            section: None,
            reason: "no such page".to_owned(),
        })
    }

    fn cat(
        &mut self,
        path: &str,
        section: &str,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        self.calls += 1;
        self.sections
            .get(&(path.to_owned(), section.to_owned()))
            .cloned()
            .ok_or_else(|| ReadFault {
                path: path.to_owned(),
                section: Some(section.to_owned()),
                reason: "no such section".to_owned(),
            })
    }

    fn actor(&self) -> &str {
        &self.actor
    }
}

/// A host that must never be asked to read. Any read is a design violation.
struct NoReadHost {
    actor: String,
}

impl NoReadHost {
    fn new() -> Self {
        Self {
            actor: "8ab41c02".to_owned(),
        }
    }
}

impl ScriptHost for NoReadHost {
    fn toc(&mut self, path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        unreachable!("the host was consulted for toc({path})");
    }

    fn cat(
        &mut self,
        path: &str,
        section: &str,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        unreachable!("the host was consulted for cat({path}, {section})");
    }

    fn actor(&self) -> &str {
        &self.actor
    }
}

fn ctx() -> ScriptCtx {
    ScriptCtx {
        id: "script".to_owned(),
        args: BTreeMap::from([("mode".to_owned(), "dry".to_owned())]),
        files: vec![
            "tasks/0011.md".to_owned(),
            "tasks/0012.md".to_owned(),
            "tasks/0013.md".to_owned(),
        ],
        effects: Vec::new(),
    }
}

fn run(src: &str) -> (effects::ScriptEval, FixtureHost) {
    let mut host = FixtureHost::new();
    let eval = eval_script(src, &ctx(), ScriptLimits::default(), &mut host);
    (eval, host)
}

/// Acceptance 1 — the module top level is the whole program; its bindings are
/// observable, and no `def` is required.
#[test]
fn a_top_level_only_script_evaluates_and_its_bindings_are_observable() {
    let (eval, _) = run("total = 1 + 2\nname = \"plan\"\n");
    let facts = eval.outcome.expect("a top-level-only script evaluates");
    println!("POPULATION bindings = {:?}", facts.bindings);
    assert_eq!(facts.bindings.get("total").map(String::as_str), Some("3"));
    assert_eq!(
        facts.bindings.get("name").map(String::as_str),
        Some("\"plan\"")
    );
    // The inert inputs are inputs, not results.
    assert!(
        !facts.bindings.contains_key("files"),
        "inputs are not results"
    );
    assert!(
        !facts.bindings.contains_key("args"),
        "inputs are not results"
    );
}

/// `args` is an inert **dict** — the caller names its inputs, so a script
/// subscripts by key and can enumerate the keys. Inert means data only: the
/// dict carries no callable and no host reach, so nothing reachable through it
/// can read or write.
#[test]
fn args_is_an_inert_dict_keyed_by_name() {
    let (eval, _) = run("mode = args[\"mode\"]\nnames = sorted(args.keys())\n");
    let facts = eval.outcome.expect("the script evaluates");
    assert_eq!(
        facts.bindings.get("mode").map(String::as_str),
        Some("\"dry\"")
    );
    assert_eq!(
        facts.bindings.get("names").map(String::as_str),
        Some("[\"mode\"]")
    );
}

/// Acceptance 1 — a script that also defines `def run` is legal; the function
/// is simply unused (no hook lookup happens at this entry).
#[test]
fn a_script_defining_def_run_is_legal_and_the_function_is_unused() {
    let (eval, host) = run("def run(ctx):\n    return read(\"tasks/0011.md\")\nx = 1\n");
    let facts = eval.outcome.expect("`def run` must not be looked up");
    assert_eq!(facts.bindings.get("x").map(String::as_str), Some("1"));
    assert_eq!(
        host.calls, 0,
        "an unused def never runs, so nothing is read"
    );
    assert!(eval.recording.reads.is_empty());
}

/// Acceptance 1 — `me()` is the injected actor; the engine mints no identity.
#[test]
fn me_returns_the_injected_actor() {
    let (eval, _) = run("who = me()\n");
    let facts = eval.outcome.expect("me() resolves");
    assert_eq!(
        facts.bindings.get("who").map(String::as_str),
        Some("\"8ab41c02\"")
    );
    assert_eq!(eval.recording.actor, "8ab41c02");
}

/// run-plane § The statement-position rule: "Suppression syntax does not exist
/// in v1 (`_ = read(…)` is rejected permanently)". The refusal is pre-eval and
/// parse-class — the rule it defends is syntactic — so the banned spelling
/// never reads, never binds, and never renders.
#[test]
fn the_banned_suppression_spelling_is_refused_before_evaluation() {
    let mut host = NoReadHost::new();
    let eval = eval_script(
        "_ = read(\"tasks/0011.md\")\n",
        &ctx(),
        ScriptLimits::default(),
        &mut host,
    );
    let error = eval
        .outcome
        .expect_err("`_ = read(…)` is rejected permanently");
    println!("POPULATION refusal = {error}");
    assert!(
        matches!(error, EvalError::Parse { .. }),
        "the statement-position rule is syntactic, so its refusal is parse-class: {error:?}"
    );
    let said = error.to_string();
    for owed in ["_ = read", "rejected permanently", "line 1"] {
        assert!(said.contains(owed), "the refusal owes `{owed}`: {said}");
    }
    // Pre-eval: nothing was read (NoReadHost would have panicked) and nothing
    // was recorded.
    assert!(
        eval.recording.reads.is_empty(),
        "a refused script reads none"
    );
}

/// The refusal is the SPELLING, at any depth — a suppression inside a function
/// body is the same non-positional grammar the law rejects. An ordinary `_`
/// binding of a non-read value is untouched: `_` is a plain Starlark name.
#[test]
fn the_suppression_spelling_is_refused_at_any_depth_and_only_for_read() {
    let mut host = NoReadHost::new();
    let nested = eval_script(
        "def scan(p):\n    _ = read(p)\n    return 1\n",
        &ctx(),
        ScriptLimits::default(),
        &mut host,
    );
    let error = nested
        .outcome
        .expect_err("the spelling is refused wherever it is written");
    assert!(matches!(error, EvalError::Parse { .. }), "{error:?}");
    assert!(error.to_string().contains("line 2"), "{error}");

    // `_` itself stays an ordinary name — only the read spelling is banned.
    let (ordinary, _) = run("_ = 1 + 2\nkept = _\n");
    let facts = ordinary
        .outcome
        .expect("`_` is a plain name when no read is suppressed");
    assert_eq!(facts.bindings.get("kept").map(String::as_str), Some("3"));
}

/// Acceptance 2 — golden scenario 5's three read positions: a top-level
/// statement echoes; a comprehension and an `if` condition stay quiet.
#[test]
fn a_top_level_read_echoes_and_nested_reads_stay_quiet() {
    let src = "card = read(\"tasks/0011.md\")\n\
               owners = [read(t)[\"fm\"][\"owner\"] for t in files]\n\
               if read(\"tasks/0012.md\")[\"fm\"][\"status\"] == \"todo\":\n\
               \x20   picked = card[\"rev\"]\n";
    let (eval, _) = run(src);
    eval.outcome.expect("the script evaluates");
    let seen: Vec<(u32, &ReadPosition, &str)> = eval
        .recording
        .reads
        .iter()
        .map(|r| (r.line, &r.position, r.path.as_str()))
        .collect();
    println!("POPULATION recorded reads = {seen:?}");

    assert_eq!(eval.recording.reads.len(), 5, "1 + 3 files + 1");
    assert_eq!(eval.recording.reads[0].position, ReadPosition::Echo);
    assert_eq!(eval.recording.reads[0].line, 1);
    for nested in &eval.recording.reads[1..4] {
        assert_eq!(
            nested.position,
            ReadPosition::Quiet,
            "comprehension is quiet"
        );
        assert_eq!(nested.line, 2);
    }
    assert_eq!(
        eval.recording.reads[4].position,
        ReadPosition::Quiet,
        "an `if` condition is quiet"
    );
    assert_eq!(eval.recording.reads[4].line, 3);

    // Call order is the record's order, and every response is recorded.
    let paths: Vec<&str> = eval
        .recording
        .reads
        .iter()
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "tasks/0011.md",
            "tasks/0011.md",
            "tasks/0012.md",
            "tasks/0013.md",
            "tasks/0012.md",
        ]
    );
}

/// The two read faces: `read(path)` is toc `{rev, fm, toc}`, `read(path,
/// section=…)` is cat `{text, rev}`. There is no whole-file body.
#[test]
fn read_serves_the_toc_face_and_the_cat_face() {
    let src = "page = read(\"tasks/0011.md\")\n\
               rev = page[\"rev\"]\n\
               first = page[\"toc\"][0][\"section\"]\n\
               anchor0 = page[\"toc\"][0][\"anchor\"]\n\
               anchor1 = page[\"toc\"][1][\"anchor\"]\n\
               sec = read(\"tasks/0011.md\", section = \"Goals\")\n\
               body = sec\n";
    let (eval, _) = run(src);
    let facts = eval.outcome.expect("both faces resolve");
    assert_eq!(
        facts.bindings.get("rev").map(String::as_str),
        Some("\"rev-tasks/0011.md\"")
    );
    assert_eq!(
        facts.bindings.get("first").map(String::as_str),
        Some("\"Goals\"")
    );
    // Absence stays absence: a section with no anchor is None, never "".
    assert_eq!(
        facts.bindings.get("anchor0").map(String::as_str),
        Some("None")
    );
    assert_eq!(
        facts.bindings.get("anchor1").map(String::as_str),
        Some("\"^n1\"")
    );
    assert_eq!(
        facts.bindings.get("body").map(String::as_str),
        Some("\"ship the script entry\\n\"")
    );
    assert!(matches!(eval.recording.reads[0].face, ReadFace::Toc(_)));
    assert!(matches!(eval.recording.reads[1].face, ReadFace::Section(_)));
    assert_eq!(eval.recording.reads[1].section.as_deref(), Some("Goals"));
}

/// Acceptance 3 — the replay law. Re-evaluating against the recorded response
/// sequence is byte-identical, and consults no host: `replay_script` takes
/// none, and a host that panics on any call proves the live path is not on it.
#[test]
fn replay_against_the_recorded_reads_is_byte_identical() {
    let src = "card = read(\"tasks/0011.md\")\n\
               owners = [read(t)[\"fm\"][\"owner\"] for t in files]\n\
               sec = read(\"tasks/0011.md\", section = \"Goals\")\n\
               who = me()\n";
    let (live, _) = run(src);
    let live_facts = live.outcome.as_ref().expect("live eval succeeds").clone();

    let replayed = replay_script(src, &ctx(), ScriptLimits::default(), &live.recording);
    let replayed_facts = replayed.outcome.expect("replay succeeds");

    assert_eq!(
        serde_json::to_string(&live_facts).unwrap(),
        serde_json::to_string(&replayed_facts).unwrap(),
        "replay must be byte-identical"
    );
    assert_eq!(
        serde_json::to_string(&live.recording).unwrap(),
        serde_json::to_string(&replayed.recording).unwrap(),
        "the replayed recording must be byte-identical"
    );

    // A script that reads nothing never asks the host to read.
    let quiet = eval_script(
        "x = 1\n",
        &ctx(),
        ScriptLimits::default(),
        &mut NoReadHost::new(),
    );
    assert!(quiet.outcome.is_ok());
}

/// Replay refuses when the script diverges from what was recorded — a replay
/// that silently served the wrong response would forge determinism.
#[test]
fn replay_refuses_when_the_script_diverges_from_the_recording() {
    let (live, _) = run("card = read(\"tasks/0011.md\")\n");
    let out = replay_script(
        "card = read(\"tasks/0013.md\")\n",
        &ctx(),
        ScriptLimits::default(),
        &live.recording,
    );
    let err = out.outcome.expect_err("a divergent replay must refuse");
    println!("POPULATION divergent replay -> {err}");
    assert!(matches!(err, EvalError::Runtime { .. }));
}

/// Acceptance 5 — decision #17 stands: no exec surface at this entry.
#[test]
fn no_exec_or_os_name_resolves_in_the_script_surface() {
    for forbidden in ["exec", "subprocess", "os", "open", "print", "load", "eval"] {
        let src = format!("x = {forbidden}\n");
        let out = eval_script(
            &src,
            &ctx(),
            ScriptLimits::default(),
            &mut FixtureHost::new(),
        );
        let err = out
            .outcome
            .expect_err("an I/O name must not resolve in the script sandbox");
        println!("POPULATION {forbidden} -> {err}");
        assert!(matches!(
            err,
            EvalError::Runtime { .. } | EvalError::Parse { .. }
        ));
    }
}

/// Acceptance 6 — the 65th read refuses typed, naming the ceiling. Never
/// truncation: the run has no result at all.
#[test]
fn the_read_past_the_ceiling_refuses_typed_and_names_it() {
    let limits = ScriptLimits {
        max_reads: 64,
        ..ScriptLimits::default()
    };
    let src = "for i in range(65):\n    x = read(\"tasks/0011.md\")\n";
    let mut host = FixtureHost::new();
    let out = eval_script(src, &ctx(), limits, &mut host);
    let err = out.outcome.expect_err("the 65th read must refuse");
    println!("POPULATION read budget -> {err}");
    assert_eq!(
        err,
        EvalError::ReadBudget {
            rule_id: "script".to_owned(),
            limit: 64
        }
    );
    assert!(
        err.to_string().contains("64"),
        "the refusal names the ceiling: {err}"
    );
    assert_eq!(host.calls, 64, "the ceiling is enforced, not exceeded");
    // Telemetry is unconditional — reported even on refusal.
    assert!(
        out.telemetry.fuel_used > 0,
        "fuel is reported on failure: {:?}",
        out.telemetry
    );
    assert_eq!(out.telemetry.reads_used, 64);
}

/// A read at exactly the ceiling is admitted — the cap is `>`, not `>=`.
#[test]
fn the_read_budget_admits_exactly_the_ceiling() {
    let limits = ScriptLimits {
        max_reads: 3,
        ..ScriptLimits::default()
    };
    let src = "for i in range(3):\n    x = read(\"tasks/0011.md\")\n";
    let out = eval_script(src, &ctx(), limits, &mut FixtureHost::new());
    assert!(out.outcome.is_ok(), "exactly at the cap is allowed");
    assert_eq!(out.recording.reads.len(), 3);
}

/// A host fault aborts the whole script — nothing partial, and the fault names
/// the path.
#[test]
fn a_host_read_fault_aborts_the_script() {
    let (eval, _) = run("card = read(\"tasks/9999.md\")\n");
    let err = eval.outcome.expect_err("a missing page faults");
    println!("POPULATION missing page -> {err}");
    assert!(err.to_string().contains("tasks/9999.md"));
}

/// Telemetry is present on every outcome (the `RuleTelemetry` precedent).
#[test]
fn telemetry_is_unconditional() {
    let (ok, _) = run("total = 0\nfor i in range(4):\n    total = total + i\n");
    assert!(
        ok.telemetry.fuel_used > 0,
        "fuel is measured: {:?}",
        ok.telemetry
    );
    let (bad, _) = run("x = (\n");
    assert!(bad.outcome.is_err());
    println!("POPULATION parse-fault telemetry = {:?}", bad.telemetry);
}
