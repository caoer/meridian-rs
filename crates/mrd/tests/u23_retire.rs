//! U23 gates for `mrd retire` — the type-2 retirement DSL, driven through the
//! REAL binary over its process boundary.
//!
//! # Every pin here is a detector, and a detector is only known by having been red
//! Each gate names the MUTATION that must redden it, in its own doc comment, and
//! most carry a VACUITY CONTROL in the same test — a second arm proving the
//! assertion can still fail when the world changes. The mutations are executed
//! by `tools/u23-mutation-proof.py`, which carries the two measured harness
//! cures (stamp restored files forward; refuse to read `running 0 tests` as a
//! pass).
//!
//! The specific traps this file is written against:
//!
//! - an emptiness assertion that becomes true by construction goes vacuous and
//!   stays green forever — so "the second run wrote nothing" is never asserted
//!   without an arm proving the FIRST run wrote something;
//! - a refusal-asserting test whose fixture triggers a DIFFERENT refusal than
//!   the one it names reads as a wording change (all-hands #2) — so every
//!   refusal gate asserts the REASON WORD, a closed set, never a substring of
//!   the sentence;
//! - a count that is flat BY CONSTRUCTION carries no evidence (all-hands #4) —
//!   so the report's denominator is asserted wherever a zero is;
//! - a control that emits the same bytes in the passing and failing world is a
//!   decoration (all-hands #3) — which is why `retire-term-never-matched`
//!   exists at all, and why its gate states BOTH arms explicitly.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            // Spawn-impossible: deterministic in-process answers, no resident
            // daemon ever starts.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE");
        cmd.output().expect("spawn mrd")
    }

    /// A workspace carrying `files` — `(relative path, body)`.
    fn workspace(&self, files: &[(&str, &str)]) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        for (path, body) in files {
            let full = ws.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("mkdir parent");
            }
            std::fs::write(&full, body).expect("write");
        }
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}
fn json(out: &Output) -> Value {
    serde_json::from_str(&stdout(out))
        .unwrap_or_else(|e| panic!("json report ({e}): {}{}", stdout(out), stderr(out)))
}

/// The reason words present in a `--json` report — the closed set a refusal gate
/// must assert on. Asserting a SENTENCE would make a reworded RIGHT refusal and
/// a caught WRONG refusal look identical (all-hands #2).
fn reasons(v: &Value) -> Vec<String> {
    v["refusals"]
        .as_array()
        .expect("refusals array")
        .iter()
        .map(|r| r["reason"].as_str().expect("reason word").to_owned())
        .collect()
}

fn message_of(v: &Value, reason: &str) -> String {
    v["refusals"]
        .as_array()
        .expect("refusals")
        .iter()
        .find(|r| r["reason"] == reason)
        .unwrap_or_else(|| panic!("no {reason} refusal in {v}"))["message"]
        .as_str()
        .expect("message")
        .to_owned()
}

/// The declaration page: one holding section, one `meridian-retire` block.
fn declaration(id: &str, term: &str, control: &str, proof: bool) -> String {
    let proof_line = if proof {
        "proof: u23_retire::a_second_mark_writes_nothing — empty the exclusion, run 2 double-marks\n"
    } else {
        ""
    };
    format!(
        "# Retire the old spelling\n\n\
         ## The wire node carries segments\n\n\
         The old spelling is gone. Address the node by its segment array.\n\n\
         ```meridian-retire\n\
         version: 1\n\
         id: {id}\n\
         term: {term}\n\
         replacer: hpath\n\
         control: {control}\n\
         holding:\n\
         \x20 path: decisions/retire.md\n\
         \x20 hpath:\n\
         \x20   - h: Retire the old spelling\n\
         \x20   - h: The wire node carries segments\n\
         route: array-hpath\n\
         {proof_line}\
         ```\n"
    )
}

/// A prose page carrying the term exactly twice — once bare, once inside an
/// inline code span, which is the pin-7 fixture.
const PROSE: &str =
    "# Guide\n\n## Usage\n\nThe hpath_text field carried it.\nSo did `hpath_text` in the table.\n";

fn base_vault(proof: bool) -> Vec<(&'static str, String)> {
    vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "hpath", proof),
        ),
        ("guide.md", PROSE.to_owned()),
    ]
}

fn as_pairs<'a>(v: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    v.iter().map(|(p, b)| (*p, b.as_str())).collect()
}

/// The workspace fingerprint the report publishes — the SAME token
/// `--expect-root` is compared against by `splice`'s §5.1 world guard.
fn fingerprint(sb: &Sandbox, ws: &Path) -> String {
    let out = sb.run(ws, &["retire", "report", "--json"]);
    json(&out)["fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_owned()
}

// ---------------------------------------------------------------------------
// Pin 1 — idempotence
// ---------------------------------------------------------------------------

/// **Pin 1.** Mark twice. The second run writes zero bytes, leaves the workspace
/// fingerprint BYTE-IDENTICAL, and still prints its count.
///
/// *Mutation:* delete the marker-span exclusion in `scan_doc`, and run 2 marks
/// every occurrence a second time — the fingerprint moves.
///
/// *Vacuity control, same test:* run 1 is asserted to have MOVED the
/// fingerprint. Without that arm a `mark` that silently did nothing at all would
/// satisfy "run 2 changed nothing" perfectly, forever.
#[test]
fn a_second_mark_writes_nothing() {
    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&base_vault(true)));

    let before = fingerprint(&sb, &ws);
    let first = sb.run(&ws, &["retire", "mark", "--expect-root", &before, "--json"]);
    assert_eq!(code(&first), 0, "first mark: {}", stderr(&first));
    let after_one = fingerprint(&sb, &ws);

    assert_ne!(
        before, after_one,
        "VACUITY CONTROL: run 1 actually wrote — without this arm, a mark that did nothing passes the real assertion"
    );

    let second = sb.run(
        &ws,
        &["retire", "mark", "--expect-root", &after_one, "--json"],
    );
    assert_eq!(code(&second), 0, "second mark: {}", stderr(&second));
    let after_two = fingerprint(&sb, &ws);
    assert_eq!(
        after_one, after_two,
        "the second run is a silent no-op: the folded merkle root is byte-identical"
    );

    let report = json(&second);
    let m = &report["retirements"][0]["measured"];
    assert_eq!(m["marked"], 0, "run 2 marked nothing: {report}");
    assert!(
        m["already"].as_u64().expect("already") > 0,
        "run 2 still PRINTS its count, re-derived from the documents: {report}"
    );
    assert!(
        report["files_scanned"].as_u64().expect("denominator") > 0,
        "a zero is never printed without its denominator: {report}"
    );
}

// ---------------------------------------------------------------------------
// Pin 2 — the positive control
// ---------------------------------------------------------------------------

/// **Pin 2.** A retirement whose declared control matches nothing refuses
/// `retire-control-silent`.
///
/// *Mutation:* drop the `counts.control == 0` arm, and the sweep proceeds on a
/// scan it cannot vouch for.
///
/// *Vacuity control, same test:* the identical vault with a control that DOES
/// match must NOT raise this reason — otherwise the pin passes in a build where
/// everything refuses.
#[test]
fn a_blind_control_refuses_and_a_seeing_one_does_not() {
    let sb = sandbox();
    let blind = vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "zzz-absent-token", true),
        ),
        ("guide.md", PROSE.to_owned()),
    ];
    let ws = sb.workspace(&as_pairs(&blind));
    let out = sb.run(&ws, &["retire", "report", "--json"]);
    assert_eq!(
        code(&out),
        1,
        "a blind control is a finding: {}",
        stdout(&out)
    );
    assert!(
        reasons(&json(&out)).contains(&"retire-control-silent".to_owned()),
        "the reason word, not the sentence: {}",
        stdout(&out)
    );

    let sb2 = sandbox();
    let ws2 = sb2.workspace(&as_pairs(&base_vault(true)));
    let out2 = sb2.run(&ws2, &["retire", "report", "--json"]);
    assert!(
        !reasons(&json(&out2)).contains(&"retire-control-silent".to_owned()),
        "VACUITY CONTROL: a control that matches raises nothing: {}",
        stdout(&out2)
    );
}

// ---------------------------------------------------------------------------
// Pin 3 — engine-block safety
// ---------------------------------------------------------------------------

/// **Pin 3.** A term inside bytes the ENGINE writes refuses
/// `retire-would-corrupt-engine-block`; an ordinary code fence only SKIPS, with
/// a count.
///
/// *Mutation:* drop the `lock::is_engine_emitted` check, and the sweep writes a
/// `~~` marker into a `meridian-lock` block, corrupting an engine artifact.
///
/// The fixture uses `meridian-lock` deliberately — a language with a REGISTERED
/// canonical writer, which is what the predicate actually tests. The second arm
/// asserts the separation, so this pin cannot pass by refusing every fence.
#[test]
fn an_engine_written_block_refuses_but_a_code_fence_only_skips() {
    let sb = sandbox();
    let files = vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "hpath", true),
        ),
        (
            "locked.md",
            "# Locked\n\n## Body\n\n```meridian-lock\nref: hpath_text\n```\n".to_owned(),
        ),
    ];
    let ws = sb.workspace(&as_pairs(&files));
    let out = sb.run(&ws, &["retire", "report", "--json"]);
    assert_eq!(code(&out), 1, "engine block: {}", stdout(&out));
    assert!(
        reasons(&json(&out)).contains(&"retire-would-corrupt-engine-block".to_owned()),
        "the reason word: {}",
        stdout(&out)
    );

    let sb2 = sandbox();
    let files2 = vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "hpath", true),
        ),
        (
            "sample.md",
            "# S\n\n## Body\n\nprose hpath_text here\n\n```rust\nlet hpath_text = 1;\n```\n"
                .to_owned(),
        ),
    ];
    let ws2 = sb2.workspace(&as_pairs(&files2));
    let report2 = json(&sb2.run(&ws2, &["retire", "report", "--json"]));
    assert!(
        !reasons(&report2).contains(&"retire-would-corrupt-engine-block".to_owned()),
        "a plain code fence is not an engine artifact: {report2}"
    );
    assert_eq!(
        report2["retirements"][0]["measured"]["skipped_code"], 1,
        "and it is COUNTED, never silently dropped: {report2}"
    );
}

// ---------------------------------------------------------------------------
// Pin 4 — count honesty and its denominator
// ---------------------------------------------------------------------------

/// **Pin 4.** Over a fixture with a KNOWN occurrence count, run 2's `already`
/// equals run 1's `marked`, and the report always carries its denominator.
///
/// *Mutation:* count `already` from anything other than this run's documents,
/// and the equality breaks.
#[test]
fn the_second_runs_already_equals_the_first_runs_marked() {
    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&base_vault(true)));

    let fp = fingerprint(&sb, &ws);
    let first = json(&sb.run(&ws, &["retire", "mark", "--expect-root", &fp, "--json"]));
    let marked = first["retirements"][0]["measured"]["marked"]
        .as_u64()
        .expect("marked");
    assert_eq!(
        marked, 2,
        "the fixture carries the term exactly twice: {first}"
    );

    let second = json(&sb.run(&ws, &["retire", "report", "--json"]));
    let already = second["retirements"][0]["measured"]["already"]
        .as_u64()
        .expect("already");
    assert_eq!(
        already, marked,
        "the count is re-derived from the documents, never remembered: {second}"
    );
    assert!(
        second["files_scanned"].as_u64().expect("denominator") > 0,
        "every count carries its denominator: {second}"
    );
}

// ---------------------------------------------------------------------------
// Pin 5 — orphan detection
// ---------------------------------------------------------------------------

/// **Pin 5.** A marker whose id no declaration carries is REPORTED with its file
/// and its id, and exits 1.
///
/// *Mutation:* make the orphan path a silent skip, and a reader following the
/// marker reaches nothing, forever, with a green board.
#[test]
fn an_undeclared_marker_id_is_reported_not_skipped() {
    let sb = sandbox();
    let files = vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "hpath", true),
        ),
        (
            "stray.md",
            "# S\n\n## Body\n\nthe ~~gone_name~~ new_name (retired: not-declared) field\nand hpath_text here\n".to_owned(),
        ),
    ];
    let ws = sb.workspace(&as_pairs(&files));
    let out = sb.run(&ws, &["retire", "report", "--json"]);
    assert_eq!(code(&out), 1, "orphan: {}", stdout(&out));
    let report = json(&out);
    assert!(
        reasons(&report).contains(&"retire-marker-orphaned".to_owned()),
        "the reason word: {report}"
    );
    let message = message_of(&report, "retire-marker-orphaned");
    assert!(
        message.contains("stray.md") && message.contains("not-declared"),
        "names the file and the id it could not resolve: {message}"
    );
}

// ---------------------------------------------------------------------------
// Pin 6 — the refusal contract, and the reader/writer split
// ---------------------------------------------------------------------------

/// **Pin 6.** Every refusal carries the house four properties — subject, cause,
/// partial state, and a fix naming a RUNNABLE COMMAND — and the READER's
/// partial-state clause differs from the WRITERS'.
///
/// That last clause is the point. Five refusals all asserting the same sentence
/// would be one assertion wearing five names; and copying "no file was marked"
/// onto a report that served its whole table is a false negative — the exact
/// defect the engine's own `assert_refusal_contract` exists to catch.
///
/// *Mutation:* splice `NO_MARK_CLAUSE` into the orphan refusal too, and the
/// inequality below reddens.
#[test]
fn the_refusals_are_not_one_assertion_wearing_many_names() {
    let sb = sandbox();
    let files = vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "zzz-absent-token", true),
        ),
        (
            "stray.md",
            "# S\n\n## Body\n\nthe ~~gone~~ new (retired: not-declared) field\nand hpath_text\n"
                .to_owned(),
        ),
    ];
    let ws = sb.workspace(&as_pairs(&files));
    let report = json(&sb.run(&ws, &["retire", "report", "--json"]));
    let all = report["refusals"].as_array().expect("refusals");
    assert!(all.len() >= 2, "two distinct refusals fired: {report}");

    for r in all {
        let m = r["message"].as_str().expect("message");
        assert!(m.starts_with("refused: "), "subject leads: {m}");
        assert!(m.contains("Fix: "), "carries a fix clause: {m}");
        assert!(
            m.contains("mrd "),
            "the fix names a RUNNABLE COMMAND, never an internal mode: {m}"
        );
    }

    let writer = message_of(&report, "retire-control-silent");
    let reader = message_of(&report, "retire-marker-orphaned");
    assert!(
        writer.contains("No file was marked"),
        "the writer says nothing landed: {writer}"
    );
    assert!(
        !reader.contains("No file was marked"),
        "the READER must not claim it wrote nothing — it served its whole table, and copying the writer's clause here is a false negative: {reader}"
    );
    assert!(
        reader.contains("report above is complete"),
        "the reader states its OWN partial state: {reader}"
    );
}

// ---------------------------------------------------------------------------
// Pin 7 — code-span wrapping, asserted on RENDERED text
// ---------------------------------------------------------------------------

/// **Pin 7.** A term inside an inline code span is wrapped OUTSIDE the
/// backticks, so the rendered output strikes the term.
///
/// *Mutation:* wrap inside the backticks. The raw bytes still look plausible —
/// `` `~~hpath_text~~` `` is a perfectly ordinary string — which is why the
/// instrument here is the RENDERED text and never the bytes. Rendered, the
/// mutated form emits the tilde characters literally and strikes nothing.
#[test]
fn a_code_span_term_is_struck_in_the_rendered_text_not_merely_in_the_bytes() {
    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&base_vault(true)));
    let fp = fingerprint(&sb, &ws);
    let mark = sb.run(&ws, &["retire", "mark", "--expect-root", &fp, "--json"]);
    assert_eq!(code(&mark), 0, "mark: {}", stderr(&mark));

    let raw = std::fs::read_to_string(ws.join("guide.md")).expect("read back");
    assert!(
        raw.contains("~~`hpath_text`~~"),
        "the tildes wrap the FULL inline span, backticks included: {raw}"
    );
    assert!(
        !raw.contains("`~~hpath_text~~`"),
        "and never sit inside them, where markdown reads them as literal text: {raw}"
    );

    // The instrument that distinguishes the two: the RENDERED face, asked
    // through the same projection the engine serves.
    let read = sb.run(&ws, &["read", "guide.md"]);
    assert_eq!(code(&read), 0, "read: {}", stderr(&read));
    let rendered = stdout(&read);
    assert!(
        !rendered.contains("`~~") && !rendered.contains("~~`"),
        "rendered, no literal tilde-inside-code survives: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// Pin 8 — the arm the control cannot see (all-hands #3)
// ---------------------------------------------------------------------------

/// **Pin 8.** A term that matches nothing, with no marker of its id anywhere,
/// refuses `retire-term-never-matched`.
///
/// # Both arms, stated — this pin exists BECAUSE of a two-arm check
/// The positive control proves the scanner REACHED THE FILES. It cannot see the
/// term at all:
///
/// - **Arm A, healthy and complete:** term 0, control > 0.
/// - **Arm C, term never right:** term 0, control > 0 — *byte-identical to A*.
///
/// The control emits the same row in both worlds, which by all-hands #3 makes it
/// a decoration there. This refusal covers that world, and covers it by REFUSING
/// rather than guessing which world it is in: a wrong pattern, a term already
/// removed by hand, and a retirement declared before its term exists all produce
/// the same pair, so the reason word says `never-matched` and names none of the
/// three.
///
/// *Mutation:* drop the `would_mark == 0 && already == 0` arm, and a retirement
/// declared for a term that was never in the vault reports a perfect clean
/// sweep, greenly, forever.
#[test]
fn a_term_that_never_matched_refuses_rather_than_reporting_a_clean_sweep() {
    let sb = sandbox();
    let files = vec![
        (
            "decisions/retire.md",
            // `hpath` IS in the vault (the control sees the files); `hpath_txt`
            // is not (the term never matches). Precisely arm C.
            declaration("hpath-text", "hpath_txt", "hpath", true),
        ),
        ("guide.md", PROSE.to_owned()),
    ];
    let ws = sb.workspace(&as_pairs(&files));
    let out = sb.run(&ws, &["retire", "report", "--json"]);
    assert_eq!(code(&out), 1, "never matched: {}", stdout(&out));
    let report = json(&out);
    assert!(
        reasons(&report).contains(&"retire-term-never-matched".to_owned()),
        "the reason word: {report}"
    );
    assert!(
        report["retirements"][0]["measured"]["control"]
            .as_u64()
            .expect("control")
            > 0,
        "ARM C: the control matched, so this row reads exactly as arm A does — the term is the only difference. If this fails, the fixture is testing pin 2 and pin 8 is vacuous: {report}"
    );
    let message = message_of(&report, "retire-term-never-matched");
    assert!(
        message.contains("the pattern is wrong") && message.contains("already done"),
        "the refusal teaches BOTH branches rather than accusing the operator of a typo they may not have made: {message}"
    );

    let sb2 = sandbox();
    let ws2 = sb2.workspace(&as_pairs(&base_vault(true)));
    assert!(
        !reasons(&json(&sb2.run(&ws2, &["retire", "report", "--json"])))
            .contains(&"retire-term-never-matched".to_owned()),
        "VACUITY CONTROL: a term that matches raises nothing"
    );
}

// ---------------------------------------------------------------------------
// Q4's ruling, executable
// ---------------------------------------------------------------------------

/// `mark` demands a world guard, and a STALE one refuses at the write door.
///
/// *Mutation:* make the flag optional, and the sweep can land on a vault that
/// moved under it — U9b's measured failure, industrialized.
#[test]
fn mark_demands_a_world_guard_and_a_stale_one_refuses() {
    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&base_vault(true)));

    let bare = sb.run(&ws, &["retire", "mark", "--json"]);
    assert_eq!(
        code(&bare),
        2,
        "a bare mark is a bad invocation: {}",
        stdout(&bare)
    );
    assert!(
        stderr(&bare).contains("--expect-root"),
        "and it names the flag it wants: {}",
        stderr(&bare)
    );

    let stale = sb.run(
        &ws,
        &[
            "retire",
            "mark",
            "--expect-root",
            "b3:0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
    );
    assert_eq!(code(&stale), 1, "a stale guard refuses: {}", stdout(&stale));

    let fp = fingerprint(&sb, &ws);
    let fresh = sb.run(&ws, &["retire", "mark", "--expect-root", &fp, "--json"]);
    assert_eq!(
        code(&fresh),
        0,
        "VACUITY CONTROL: the CURRENT root is accepted, so the assertion above is about staleness and not about the guard refusing everything: {}",
        stderr(&fresh)
    );
}

// ---------------------------------------------------------------------------
// The design position, executable
// ---------------------------------------------------------------------------

/// The report keeps `measured` and `declared` in separate keys, says in the data
/// that it verified none of the declared evidence, and holds a retirement with
/// no declared proof `open`.
///
/// *Mutation:* merge the two objects into one flat table, and a reader can no
/// longer tell which numbers the tool measured and which it was told — the
/// failure the whole §3 position exists to prevent.
#[test]
fn the_report_never_mixes_what_it_measured_with_what_it_was_told() {
    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&base_vault(false)));
    let out = sb.run(&ws, &["retire", "report", "--json"]);
    let report = json(&out);
    let r = &report["retirements"][0];

    assert!(
        r["measured"].is_object(),
        "measured is its own key: {report}"
    );
    assert!(
        r["declared"].is_object(),
        "declared is its own key: {report}"
    );
    assert_eq!(
        r["declared"]["verified_by_this_tool"], false,
        "the tool says, in the data, that it verified none of this: {report}"
    );
    assert_eq!(r["state"], "open", "no declared proof means open: {report}");
    assert_eq!(code(&out), 1, "and an open retirement is a finding");

    let sb2 = sandbox();
    let ws2 = sb2.workspace(&as_pairs(&base_vault(true)));
    let report2 = json(&sb2.run(&ws2, &["retire", "report", "--json"]));
    assert_eq!(
        report2["retirements"][0]["state"], "closed",
        "VACUITY CONTROL: a declared proof closes it, so `open` is about the proof: {report2}"
    );
}

/// The ruled link is an ARRAY, and it must be the thing that actually resolves:
/// a holding hpath addressing no section refuses `retire-holding-unresolvable`.
///
/// *Mutation:* skip the `section_span` resolution, and every marker the sweep
/// writes points at nothing, with no complaint.
#[test]
fn a_holding_hpath_that_addresses_nothing_refuses() {
    let sb = sandbox();
    let broken = declaration("hpath-text", "hpath_text", "hpath", true).replace(
        "- h: The wire node carries segments",
        "- h: No Such Section",
    );
    let files = vec![
        ("decisions/retire.md", broken),
        ("guide.md", PROSE.to_owned()),
    ];
    let ws = sb.workspace(&as_pairs(&files));
    let out = sb.run(&ws, &["retire", "report", "--json"]);
    assert_eq!(code(&out), 1, "unresolvable holding: {}", stdout(&out));
    assert!(
        reasons(&json(&out)).contains(&"retire-holding-unresolvable".to_owned()),
        "the reason word: {}",
        stdout(&out)
    );
}
