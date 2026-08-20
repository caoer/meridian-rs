//! U23 gates for `mrd retire` — the type-2 retirement DSL, driven through the
//! REAL binary over its process boundary. Each gate names the MUTATION that must
//! redden it in its own doc comment, and most carry a VACUITY CONTROL in the same
//! test: a second arm proving the assertion can still fail when the world
//! changes. `tools/u23-mutation-proof.py` executes the mutations.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
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
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
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

/// A hash-domain member the corpus does not serve: a real markdown prefix, then
/// a byte no UTF-8 decode accepts. Written directly — the fixture table carries
/// `&str` bodies, which cannot hold invalid UTF-8 by construction.
fn poison(ws: &Path, rel: &str, line: &str) {
    let mut bytes = format!("# P\n\n## Body\n\n{line}\n").into_bytes();
    bytes.extend_from_slice(b"\xFF\n");
    std::fs::write(ws.join(rel), bytes).expect("write poison member");
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

/// Pin 1. Mark twice: the second run writes zero bytes, leaves the workspace
/// fingerprint BYTE-IDENTICAL, and still prints its count.
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

/// Pin 2. A retirement whose declared control matches nothing refuses
/// `retire-control-silent`. *Mutation:* drop the `counts.control == 0` arm, and
/// the sweep proceeds on a scan it cannot vouch for. *Vacuity control:* the same
/// vault with a control that DOES match must not raise this reason.
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

/// Pin 3. A term inside bytes the ENGINE writes refuses
/// `retire-would-corrupt-engine-block`; an ordinary code fence only SKIPS, with
/// a count.
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

/// Pin 4. Over a fixture with a KNOWN occurrence count, run 2's `already` equals
/// run 1's `marked`, and the report always carries its denominator. *Mutation:*
/// count `already` from anything other than this run's documents, and the
/// equality breaks.
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

/// Pin 5. A marker whose id no declaration carries is REPORTED with its file and
/// its id, and exits 1. *Mutation:* make the orphan path a silent skip, and a
/// reader following the marker reaches nothing, forever, with a green board.
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

/// Pin 6. Every refusal carries the house four properties — subject, cause,
/// partial state, and a fix naming a RUNNABLE COMMAND — and the READER's
/// partial-state clause differs from the WRITERS'.
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

/// Pin 7. A term inside an inline code span is wrapped OUTSIDE the backticks, so
/// the rendered output strikes the term. *Mutation:* wrap inside the backticks —
/// the raw bytes still look plausible, which is why the instrument here is the
/// RENDERED text and never the bytes.
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

    // The RENDERED face, asked through the same projection the engine serves.
    let read = sb.run(&ws, &["read", "guide.md"]);
    assert_eq!(code(&read), 0, "read: {}", stderr(&read));
    let rendered = stdout(&read);
    assert!(
        !rendered.contains("`~~") && !rendered.contains("~~`"),
        "rendered, no literal tilde-inside-code survives: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// Pin 8 — the arm the control cannot see
// ---------------------------------------------------------------------------

/// Pin 8. A term that matches nothing, with no marker of its id anywhere,
/// refuses `retire-term-never-matched`. The control proves the scanner reached
/// the files, so arm A (healthy) and arm C (term never right) read
/// byte-identically: term 0, control > 0 in both.
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
/// *Mutation:* make the flag optional, and the sweep can land on a vault that
/// moved under it.
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
/// no declared proof `open`. *Mutation:* merge the two objects into one flat
/// table, and a reader can no longer tell which numbers the tool measured and
/// which it was told.
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

// ---------------------------------------------------------------------------
// Pin 9 — the sweep certifies only what its corpus serves
// ---------------------------------------------------------------------------

/// Pin 9. A retirement term whose ONLY live reference sits in a non-UTF-8
/// member must NOT report a clean sweep. The corpus does not serve that member
/// (node-rev-merkle-spec §3 per-file degradation), so the scan cannot see
/// inside it — the sweep refuses `retire-member-unserved` on both faces and
/// `mark` writes nothing. *Mutation:* drop the unserved refusal, and `mark`
/// closes a retirement while a real reference survives in the skipped file.
/// *Vacuity control:* the same vault without the poisoned member raises
/// nothing.
#[test]
fn a_poisoned_member_with_the_only_live_reference_refuses_rather_than_reporting_clean() {
    // Mid-retirement vault: guide.md's occurrence is already marked, so the
    // scan over SERVED files sees nothing left to do — the exact state in
    // which a silent skip reads as a completed retirement.
    let files = vec![
        (
            "decisions/retire.md",
            declaration("hpath-text", "hpath_text", "hpath", true),
        ),
        (
            "guide.md",
            "# Guide\n\n## Usage\n\nThe ~~hpath_text~~ hpath (retired: hpath-text) field carried it.\nSee the hpath column.\n"
                .to_owned(),
        ),
    ];

    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&files));
    poison(&ws, "poison.md", "hpath_text survives here");

    let report_out = sb.run(&ws, &["retire", "report", "--json"]);
    let report = json(&report_out);
    assert!(
        reasons(&report).contains(&"retire-member-unserved".to_owned()),
        "the unserved member is a REFUSAL, never a silent skip: {report}"
    );
    assert_eq!(code(&report_out), 1, "a refusal is a finding");
    let fp = report["fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_owned();

    let mark = sb.run(&ws, &["retire", "mark", "--expect-root", &fp, "--json"]);
    assert_eq!(
        code(&mark),
        1,
        "mark refuses the sweep whole: {}",
        stdout(&mark)
    );
    assert!(
        reasons(&json(&mark)).contains(&"retire-member-unserved".to_owned()),
        "mark carries the same refusal: {}",
        stdout(&mark)
    );
    assert_eq!(
        fingerprint(&sb, &ws),
        fp,
        "nothing was written under the refusal"
    );

    // VACUITY CONTROL: the same vault, fully served, raises nothing.
    let sb2 = sandbox();
    let ws2 = sb2.workspace(&as_pairs(&files));
    let out2 = sb2.run(&ws2, &["retire", "report", "--json"]);
    assert!(
        !reasons(&json(&out2)).contains(&"retire-member-unserved".to_owned()),
        "VACUITY CONTROL: a fully served corpus raises nothing: {}",
        stdout(&out2)
    );
    assert_eq!(code(&out2), 0, "and the report is clean: {}", stdout(&out2));
}

/// Pin 9b. The published denominator counts files the scan actually READ:
/// `files_scanned` is the SERVED corpus, and the unserved population is its own
/// key — never folded into the total. *Mutation:* set `scanned` back to the raw
/// pre-split domain count, and "matched 0 of N scanned files" overclaims
/// coverage by exactly the files the scan never saw.
#[test]
fn the_scan_denominator_counts_served_files_never_the_raw_domain() {
    // The clean twin measures the served population; the poisoned workspace
    // must publish the SAME `files_scanned`, not one more.
    let sb_clean = sandbox();
    let ws_clean = sb_clean.workspace(&as_pairs(&base_vault(true)));
    let clean = json(&sb_clean.run(&ws_clean, &["retire", "report", "--json"]));
    let served = clean["files_scanned"].as_u64().expect("files_scanned");
    assert_eq!(
        clean["files_unserved"].as_u64(),
        Some(0),
        "the clean shape keeps the key, at zero: {clean}"
    );

    let sb = sandbox();
    let ws = sb.workspace(&as_pairs(&base_vault(true)));
    poison(&ws, "poison.md", "no live reference here");
    let report = json(&sb.run(&ws, &["retire", "report", "--json"]));
    assert_eq!(
        report["files_scanned"].as_u64(),
        Some(served),
        "the poisoned member is NOT in the scanned denominator: {report}"
    );
    assert_eq!(
        report["files_unserved"].as_u64(),
        Some(1),
        "the unserved population is published beside it: {report}"
    );
}

// ---------------------------------------------------------------------------
// The refusal-contract sweep — every reason word, four properties each
// ---------------------------------------------------------------------------

/// The four-property contract, asserted PER REFUSAL on its own structured
/// fields — the house `assert_refusal_contract` shape
/// (`crates/testsuite/tests/u4a2_composed_read.rs`) re-authored for this
/// surface. Where a refusal needs different properties, ADD a per-surface
/// contract assertion rather than loosening the shared one.
fn assert_contract(reason: &str, r: &Value) {
    let get = |k: &str| -> String {
        r[k].as_str()
            .unwrap_or_else(|| panic!("{reason}: no `{k}` key: {r}"))
            .to_owned()
    };
    let (subject, cause, partial, fix) = (get("subject"), get("cause"), get("partial"), get("fix"));

    assert!(!subject.is_empty(), "{reason}: names its SUBJECT: {r}");
    assert!(!cause.is_empty(), "{reason}: names its CAUSE: {r}");
    assert!(
        !partial.is_empty(),
        "{reason}: discloses the PARTIAL STATE — a reader must not have to wonder whether anything landed: {r}"
    );
    assert!(!fix.is_empty(), "{reason}: carries a FIX: {r}");
    assert!(
        fix.contains("mrd "),
        "{reason}: the fix names a RUNNABLE COMMAND, never an internal mode the caller never selected: {fix}"
    );
    assert!(
        !fix.contains("retire-") && !fix.contains("Reason::"),
        "{reason}: the fix never names the engine's own reason word back at the caller: {fix}"
    );

    let message = get("message");
    assert_eq!(
        message,
        format!("refused: {subject} {cause} {partial} Fix: {fix}"),
        "{reason}: the human sentence is assembled from exactly the four checked properties, so it cannot drift away from them"
    );
}

/// The coverage gate: every reason word is TRIGGERED by its own fixture and
/// put through [`assert_contract`]. A table rather than one test per word,
/// because coverage is a claim about the SET.
fn contract_cases() -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    vec![
        (
            "retire-block-malformed",
            vec![(
                "decisions/retire.md",
                // `id` before `version` — out of canonical order.
                "# R\n\n## H\n\n```meridian-retire\nid: x\nversion: 1\n```\n".to_owned(),
            )],
        ),
        (
            "retire-control-silent",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_text", "zzz-absent-token", true),
                ),
                ("guide.md", PROSE.to_owned()),
            ],
        ),
        (
            "retire-holding-unresolvable",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_text", "hpath", true)
                        .replace("- h: The wire node carries segments", "- h: No Such Section"),
                ),
                ("guide.md", PROSE.to_owned()),
            ],
        ),
        (
            "retire-id-ambiguous",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_text", "hpath", true),
                ),
                (
                    "decisions/again.md",
                    declaration("hpath-text", "hpath_text", "hpath", true),
                ),
                ("guide.md", PROSE.to_owned()),
            ],
        ),
        (
            "retire-marker-malformed",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_text", "hpath", true),
                ),
                (
                    "bare.md",
                    // A machine half with no visible half: no `~~…~~` on the line.
                    "# B\n\n## Body\n\nthe old name (retired: hpath-text) here\nand hpath_text\n"
                        .to_owned(),
                ),
            ],
        ),
        (
            "retire-marker-orphaned",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_text", "hpath", true),
                ),
                (
                    "stray.md",
                    "# S\n\n## Body\n\nthe ~~gone~~ new (retired: not-declared) field\nand hpath_text\n".to_owned(),
                ),
            ],
        ),
        (
            "retire-term-never-matched",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_txt", "hpath", true),
                ),
                ("guide.md", PROSE.to_owned()),
            ],
        ),
        (
            "retire-would-corrupt-engine-block",
            vec![
                (
                    "decisions/retire.md",
                    declaration("hpath-text", "hpath_text", "hpath", true),
                ),
                (
                    "locked.md",
                    "# L\n\n## Body\n\n```meridian-lock\nref: hpath_text\n```\n".to_owned(),
                ),
            ],
        ),
    ]
}

#[test]
fn every_reason_word_carries_the_four_property_contract() {
    let cases = contract_cases();
    let mut covered: Vec<String> = Vec::new();
    for (reason, files) in &cases {
        let sb = sandbox();
        let ws = sb.workspace(&as_pairs(files));
        let out = sb.run(&ws, &["retire", "report", "--json"]);
        let report = json(&out);
        let found = report["refusals"]
            .as_array()
            .expect("refusals")
            .iter()
            .find(|r| r["reason"] == *reason)
            .unwrap_or_else(|| {
                panic!(
                    "{reason}: the fixture did not trigger it — it triggered {:?}: {report}",
                    reasons(&report)
                )
            });
        assert_contract(reason, found);
        assert_eq!(code(&out), 1, "{reason}: a refusal is a finding");
        covered.push((*reason).to_owned());
    }

    // The ninth reason needs BYTES the `&str` fixture table cannot carry: a
    // non-UTF-8 hash-domain member, written after the workspace exists.
    {
        let reason = "retire-member-unserved";
        let sb = sandbox();
        let ws = sb.workspace(&as_pairs(&base_vault(true)));
        poison(&ws, "poison.md", "quiet");
        let out = sb.run(&ws, &["retire", "report", "--json"]);
        let report = json(&out);
        let found = report["refusals"]
            .as_array()
            .expect("refusals")
            .iter()
            .find(|r| r["reason"] == reason)
            .unwrap_or_else(|| {
                panic!(
                    "{reason}: the fixture did not trigger it — it triggered {:?}: {report}",
                    reasons(&report)
                )
            });
        assert_contract(reason, found);
        assert_eq!(code(&out), 1, "{reason}: a refusal is a finding");
        covered.push(reason.to_owned());
    }

    // The census reads the ENGINE, not a copy of it: `Reason::ALL` is the crate's
    // own list, kept honest at its definition site.
    let mut engine: Vec<String> = mrd::retire_cmd::Reason::ALL
        .iter()
        .map(|r| (*r).word().to_owned())
        .collect();
    engine.sort();
    covered.sort();
    assert_eq!(
        covered, engine,
        "every reason word the engine can emit has a fixture here"
    );
}

// ---------------------------------------------------------------------------
// The outside-domain line — capped prose over a complete population
// ---------------------------------------------------------------------------

/// How many excluded files the cap fixture plants under `bulk/`. Well above the
/// cap, so the remainder clause does real arithmetic.
const EXCLUDED_BULK: usize = 12;
/// The cap the human line is expected to honour — stated independently of the
/// implementation constant ON PURPOSE: a test that imports the value it checks
/// passes for any value.
const EXPECTED_SHOWN: usize = 3;

/// A base vault plus a `bulk/**` ignore rule, `EXCLUDED_BULK` files under it,
/// and one dot-segment page — the population `retire` must enumerate COMPLETELY
/// on its machine answer and name only a sample of in prose.
fn vault_with_excluded(sb: &Sandbox) -> PathBuf {
    let mut files = base_vault(true);
    files.push((
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n\nVendored copies do not move this workspace's fingerprint.\n".to_owned(),
    ));
    let ws = sb.workspace(&as_pairs(&files));
    for i in 0..EXCLUDED_BULK {
        let p = ws.join(format!("bulk/file{i:02}.md"));
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, "# bulk\n\nexcluded.\n").expect("write");
    }
    let dot = ws.join(".snapshots/2026-08-15/index.md");
    std::fs::create_dir_all(dot.parent().expect("parent")).expect("mkdir");
    std::fs::write(dot, "# noise\n\nnever served.\n").expect("write");
    ws
}

/// The human line states the FULL outside-domain count, names at most
/// `EXPECTED_SHOWN` paths with the remainder clause, and points at the key that
/// carries the rest — while `files_excluded` stays the COMPLETE population, dot
/// paths included, because this verb certifies absence (decision 0017).
/// *Mutation:* restore the uncapped `excluded.join(", ")` and the line grows
/// with the root — the 2026-08-10 3.1M-character shape.
#[test]
fn the_outside_domain_line_samples_the_paths_while_the_json_key_stays_complete() {
    let sb = sandbox();
    let ws = vault_with_excluded(&sb);

    let said = stdout(&sb.run(&ws, &["retire", "report"]));

    // Positive control FIRST: an absent line satisfies every bound below.
    assert!(
        said.contains("outside the hash domain"),
        "the outside-domain line did not fire at all, so this gate would pass \
         vacuously — the fixture is wrong, not the cap: {said}"
    );

    // The machine answer names the whole population — the count the prose must
    // state, and the completeness the cap must not touch.
    let report = json(&sb.run(&ws, &["retire", "report", "--json"]));
    let excluded: Vec<&str> = report["files_excluded"]
        .as_array()
        .expect("files_excluded")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for member in [
        "bulk/file00.md",
        "bulk/file11.md",
        ".snapshots/2026-08-15/index.md",
    ] {
        assert!(
            excluded.contains(&member),
            "certify-absence keeps the machine enumeration COMPLETE, dot paths \
             included; missing {member}: {excluded:?}"
        );
    }

    // The COUNT is the whole population and is never capped — the half that
    // keeps the exclusion non-silent.
    assert!(
        said.contains(&format!("{} markdown file(s)", excluded.len())),
        "the line must state the FULL count ({}): {said}",
        excluded.len()
    );

    // The SAMPLE is capped, and admits it.
    let rest = excluded.len() - EXPECTED_SHOWN;
    assert!(
        said.contains(&format!("and {rest} more")),
        "the line must say how many it did NOT name — a sample that does not \
         admit it is a sample reads as the whole list: {said}"
    );

    // The assertion that actually fails when the cap is removed.
    let named = excluded.iter().filter(|rel| said.contains(**rel)).count();
    assert_eq!(
        named, EXPECTED_SHOWN,
        "the line named {named} paths; the cap is {EXPECTED_SHOWN}: {said}"
    );

    // Capping prose loses nothing only if the reader is told where the rest is.
    assert!(
        said.contains("`files_excluded`"),
        "a capped line must point at the complete machine-readable list, or \
         the cap becomes the silence the enumerator clause forbids: {said}"
    );
}

/// The negative case: a population at or under the cap is named in full and
/// claims no remainder. *Mutation:* make the remainder clause unconditional and
/// this line claims "and 0 more", teaching readers to ignore the clause.
#[test]
fn a_small_outside_domain_population_is_named_in_full_with_no_remainder() {
    let sb = sandbox();
    let mut files = base_vault(true);
    files.push((
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n\nignored.\n".to_owned(),
    ));
    let ws = sb.workspace(&as_pairs(&files));
    let p = ws.join("bulk/only.md");
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, "# one\n\nexcluded.\n").expect("write");

    let said = stdout(&sb.run(&ws, &["retire", "report"]));
    assert!(
        said.contains("outside the hash domain"),
        "control: the line must fire here too: {said}"
    );
    assert!(
        said.contains("bulk/only.md"),
        "a population under the cap is named in full: {said}"
    );
    assert!(
        !said.contains(" more"),
        "there is no remainder, so the line must not claim one: {said}"
    );
}
