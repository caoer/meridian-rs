//! The frontmatter scalar read law (wire-contract § A.6): the script plane's
//! `card.fm` serves the DECODED scalar — quote bytes stripped, double-quoted
//! escapes unescaped — so a comparison against the unquoted form arms.
//!
//! The season-1 dogfood receipts (`07-05 …/results/dogfood-season-1.md`)
//! measured the defect: `owner: "3f9a1c07"` read as length 10, `owner: ""` as
//! length 2, and every conditional keyed on a quoted scalar silently
//! evaluated false (fail-INERT). The fleet quotes frontmatter by convention,
//! so the golden scenarios G1, G4, G5, G6 and G8 all needed a quote-stripping
//! workaround (`o = o[1:-1] if o[0:1] == '"' else o`). This suite translates
//! those quoting cases into engine-surface tests: each script arms a `put`
//! exactly when the decoded comparison holds, so a regression to the raw
//! quote-bearing read renders `NoEffect` and fails the test.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome, ScriptTrace};
use serde_json::{Value, json};

/// The entry fingerprint the fake daemon reports (§4.7).
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";
/// The card's whole-file word count, as the wire toc face reports it.
const WORDS: usize = 41;

const CARD: &str = "tasks/0011-token-audit.md";

/// A fake daemon behind the one door: it records every op it is asked for and
/// answers the five this verb is allowed to speak, serving `fm` as the
/// frontmatter block of `card` (the `cat` `fm_key` answers are derived from it,
/// so the fixture and the served bytes cannot drift).
struct Fake {
    ops: Vec<String>,
    requests: Vec<Value>,
    fm: &'static str,
}

impl Fake {
    fn new(fm: &'static str) -> Self {
        Self {
            ops: Vec::new(),
            requests: Vec::new(),
            fm,
        }
    }

    fn asked(&self, op: &str) -> bool {
        self.ops.iter().any(|seen| seen == op)
    }
}

impl Door for Fake {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let op = request["op"].as_str().expect("every request names an op");
        self.ops.push(op.to_owned());
        self.requests.push(request.clone());
        Ok(match op {
            "fingerprint" => format!(r#"{{"ok":true,"body":{{"fingerprint":"{ENTRY}","seq":2}}}}"#),
            "toc" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "fingerprint": ENTRY,
                "nodes": [
                    {"kind": "frontmatter", "span": [0, 32], "node_rev": "26796ebec5d0bf1a",
                     "text_prefix_16b": "---\n", "keys": ["owner", "status", "round_7"]},
                    {"kind": "heading", "level": 1, "hpath": [{"h": "Goals"}],
                     "span": [32, 140], "content_span": [40, 140],
                     "node_rev": "a6665baff294bd04", "text_prefix_16b": "# Goals\n\nship th"},
                ],
            }})
            .to_string(),
            "read" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "root": ENTRY,
                "words_total": WORDS,
                "toc": [],
                "anchors": [],
                "rendered_text": "",
            }})
            .to_string(),
            "cat" => {
                let key = request["sec"]["fm_key"].as_str().expect("fm_key");
                let line = self
                    .fm
                    .lines()
                    .find(|l| l.starts_with(&format!("{key}:")))
                    .unwrap_or_else(|| panic!("fixture carries {key}"));
                json!({"ok": true, "body": {
                    "span": [4, 12], "node_rev": "33d5b0e1b27cb48b",
                    "content": format!("{line}\n")
                }})
                .to_string()
            }
            "splice" => {
                format!(
                    r#"{{"ok":true,"body":{{"armed":{{"edits":2}},"fingerprint_before":"{ENTRY}",
                     "fingerprint_after":"b3:c4e91d02","seq":3,"verdicts":[],
                     "receipt":{{"path":"receipts/2026-08-07.md","anchor":"r-000118"}}}}}}"#
                )
                .replace('\n', "")
            }
            other => panic!("the script entry asked for an op it must not know: {other}"),
        })
    }
}

/// Run `script` against a fake door serving `fm`, and return the trace.
fn run(fm: &'static str, script: &str) -> (ScriptTrace, Fake) {
    let mut door = Fake::new(fm);
    let argv: Vec<String> = ["--actor", "8ab41c02"].iter().map(|s| (*s).to_owned()).collect();
    let trace = attempt(&argv, script, &mut door).expect("the attempt runs");
    (trace, door)
}

/// The claim-shaped script: arm a put exactly when the decoded comparison
/// holds. A raw quote-bearing read renders the condition false, arms nothing,
/// and the trace lands `NoEffect` — the fail-INERT face the receipts measured.
const CLAIM_IF: &str = r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "3f9a1c07":
    put("tasks/0011-token-audit.md", props={"owner": "", "status": "todo"})
"#;

/// **Negative proof, read half (gate 3):** the fleet-canonical quoted id
/// `owner: "3f9a1c07"` reads back as `3f9a1c07` — the G4/G5 condition
/// (`== dead`) arms. On the unfixed base the value carries its quote bytes
/// (length 10), the condition silently evaluates false, and nothing arms.
#[test]
fn a_quoted_id_reads_back_decoded_and_arms_the_claim() {
    let (trace, door) = run(
        "---\nowner: \"3f9a1c07\"\nstatus: doing\nround_7: \"\"\n---\n\n# Goals\n\nbody\n",
        CLAIM_IF,
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "the decoded comparison must arm the put — a quote-bearing read is the \
         fail-INERT defect (trace: {:?})",
        trace.outcome
    );
    assert!(door.asked("splice"), "the claim commits through the one write op");
}

/// Season-1 G1: `owner: ""` — the fleet's canonical unowned spelling — reads
/// back as the empty string, so `== ""` arms (the golden claim-if-unowned
/// scenario).
#[test]
fn a_quoted_empty_owner_reads_back_empty_and_arms_the_claim() {
    let (trace, _) = run(
        "---\nowner: \"\"\nstatus: todo\nround_7: \"\"\n---\n\n# Goals\n\nbody\n",
        r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": "8ab41c02", "status": "doing"})
"#,
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "owner: \"\" must read back as the empty string (G1): {:?}",
        trace.outcome
    );
}

/// Season-1 G6: `round_7: ""` — the broadcast board's canonical empty — reads
/// back as the empty string, so the round-close condition arms.
#[test]
fn a_quoted_empty_round_reads_back_empty_and_arms_the_close() {
    let (trace, _) = run(
        "---\nowner: \"\"\nstatus: todo\nround_7: \"\"\n---\n\n# Goals\n\nbody\n",
        r#"
board = read("tasks/0011-token-audit.md")
if board.fm["round_7"] == "":
    put("tasks/0011-token-audit.md", props={"round_7": "closed by 8ab41c02"})
"#,
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "round_7: \"\" must read back as the empty string (G6): {:?}",
        trace.outcome
    );
}

/// The fleet's own task files carry `owner: "[[1ed98864]]"` — a quoted
/// wikilink. It reads back as the bare wikilink, so a comparison against
/// `[[1ed98864]]` arms.
#[test]
fn a_quoted_wikilink_owner_reads_back_decoded() {
    let (trace, _) = run(
        "---\nowner: \"[[1ed98864]]\"\nstatus: doing\nround_7: \"\"\n---\n\n# Goals\n\nbody\n",
        r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "[[1ed98864]]":
    put("tasks/0011-token-audit.md", props={"status": "done"})
"#,
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "owner: \"[[1ed98864]]\" must read back as [[1ed98864]]: {:?}",
        trace.outcome
    );
}

/// Control: an unquoted scalar reads back verbatim — the decode is a no-op on
/// plain values, so the existing unquoted corpus keeps working.
#[test]
fn an_unquoted_scalar_reads_back_verbatim() {
    let (trace, _) = run(
        "---\nowner: 3f9a1c07\nstatus: doing\nround_7: \"\"\n---\n\n# Goals\n\nbody\n",
        r#"
card = read("tasks/0011-token-audit.md")
if card.fm["status"] == "doing":
    put("tasks/0011-token-audit.md", props={"status": "todo"})
"#,
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "unquoted scalars must read back verbatim: {:?}",
        trace.outcome
    );
}

/// A single-quoted value reads back with its quotes stripped and `''`
/// unescaped — the decode matches the policy fm parser's string handling.
#[test]
fn a_single_quoted_value_reads_back_decoded() {
    let (trace, _) = run(
        "---\nowner: 'it''s mine'\nstatus: doing\nround_7: \"\"\n---\n\n# Goals\n\nbody\n",
        r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "it's mine":
    put("tasks/0011-token-audit.md", props={"status": "todo"})
"#,
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "single-quoted values must decode ('' → '): {:?}",
        trace.outcome
    );
}
