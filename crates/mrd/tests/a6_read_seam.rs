//! § A.6.1 — the read half of the frontmatter scalar law, measured where the
//! dogfood receipts measured it: the script plane's `card.fm[k]`.
//!
//! **The defect this pins (dogfood season 1, finding 1, fail-INERT).** The
//! script face served the key line's SOURCE BYTES, quotes included, so
//! `card.fm["owner"] == "3f9a1c07"` was false against a file carrying
//! `owner: "3f9a1c07"`. Nothing armed, and "no effects armed" is a legitimate
//! face — the failure had no signal at all. The fleet quotes by convention, so
//! the surface degraded on production data and passed on its own fixtures.
//!
//! **Why the assertion is an OUTCOME and not a string.** A test that read `fm`
//! and compared it here would prove the test's own comparison, not the engine's
//! behaviour. These drive the real script entry through its `Door` seam and
//! assert on what the run DID: a decoded value arms the claim and reaches
//! `Committed`; a value still carrying quote bytes leaves the run at
//! `NoEffect`. That is exactly the silent face the receipts caught, so it
//! cannot pass vacuously. **Unchanged by the rewrite below.**
//!
//! **Rewritten 2026-08-08 (Amendment 1a).** `read(path)` no longer fetches
//! frontmatter with one `cat` per key; it takes the whole plane from the
//! composed read's `props[]`, which the daemon serves already decoded
//! (`wire-serve::read` `read_props`). So the decode moved OUT of the code path
//! this file exercises, and a fake serving raw key lines would now be testing
//! nothing.
//!
//! The law is therefore gated in two places instead of one, and this file keeps
//! the half it can still prove:
//!
//! - **That the daemon decodes** — `wire-serve::read::props_scalar_tests`,
//!   net-new, asserting `read_props` applies the § A.6 codec across every
//!   stored form. That gate did not exist before this change.
//! - **That the script plane carries a decoded value through and does not
//!   re-decode it** — here. The fake serves each stored form through the
//!   production codec, exactly as the daemon would, and the assertions on what
//!   the run DID are untouched.
//!
//! The fake calls `model::scalar::text` rather than hardcoding decoded strings:
//! standing in for the daemon means using the daemon's own function, not a
//! second implementation of it that could drift.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome};
use serde_json::{Value, json};

/// The entry fingerprint the fake daemon reports (§4.7).
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";

/// The one page the fake daemon serves.
const CARD: &str = "tasks/0011-token-audit.md";

/// A fake daemon serving ONE frontmatter value, verbatim as it would sit on
/// disk. Everything else it answers is fixed, so two runs differing only in
/// `owner_line` differ only in the stored quoting.
struct Fake {
    /// The bytes after `owner:` on disk — the receipts' left-hand column.
    stored: String,
    owner_line: String,
}

impl Fake {
    /// `stored` is the bytes after `owner:` on disk, exactly as the corpus
    /// carries them — the receipts' left-hand column.
    fn serving(stored: &str) -> Self {
        Self {
            stored: stored.to_owned(),
            owner_line: format!("owner:{stored}\n"),
        }
    }
}

impl Door for Fake {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let op = request["op"].as_str().expect("every request names an op");
        Ok(match op {
            "fingerprint" => format!(r#"{{"ok":true,"body":{{"fingerprint":"{ENTRY}","seq":2}}}}"#),
            "toc" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "fingerprint": ENTRY,
                "nodes": [
                    {"kind": "frontmatter", "span": [0, 32], "node_rev": "26796ebec5d0bf1a",
                     "text_prefix_16b": "---\nowner:\n", "keys": ["owner", "status"]},
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
                "words_total": 41,
                "toc": [],
                "anchors": [],
                "rendered_text": "",
                // The daemon decodes; standing in for it means using ITS
                // function, so this cannot drift from production behaviour.
                "props": [
                    {"key": "owner", "value": model::scalar::text(&self.stored),
                     "span": [4, 11], "prop_rev": "33d5b0e1"},
                    {"key": "status", "value": "todo",
                     "span": [12, 25], "prop_rev": "41f643f0"},
                ],
            }})
            .to_string(),
            "cat" => {
                let content = match request["sec"]["fm_key"].as_str() {
                    Some("owner") => self.owner_line.clone(),
                    Some("status") => "status: todo\n".to_string(),
                    _ => "## Goals\n\nship the script entry\n".to_string(),
                };
                json!({"ok": true, "body": {
                    "span": [4, 12], "node_rev": "33d5b0e1b27cb48b", "content": content
                }})
                .to_string()
            }
            "splice" => format!(
                r#"{{"ok":true,"body":{{"armed":{{"edits":1}},"fingerprint_before":"{ENTRY}","fingerprint_after":"b3:c4e91d02","seq":3,"verdicts":[]}}}}"#
            ),
            other => panic!("the script entry asked for an op it must not know: {other}"),
        })
    }
}

/// Run `src` against a daemon whose `owner` line carries `stored`, and report
/// whether the run armed anything.
fn armed(stored: &str, src: &str) -> bool {
    let mut door = Fake::serving(stored);
    let argv = ["--actor".to_owned(), "8ab41c02".to_owned()];
    let trace = attempt(&argv, src, &mut door).expect("the attempt runs");
    match trace.outcome {
        ScriptOutcome::Committed => true,
        ScriptOutcome::NoEffect => false,
        other => panic!("the run neither committed nor stayed inert: {other:?}"),
    }
}

/// Golden scenario 1's own shape, keyed on the value under test.
fn claim_if_owner_is(expected: &str) -> String {
    format!(
        r#"
card = read("{CARD}")
if card.fm["owner"] == "{expected}":
    put("{CARD}", props={{"status": "doing"}})
"#
    )
}

// ── the negative proof, read half ────────────────────────────────────────────

/// **The headline, and the run that the receipts caught failing silently.** The
/// card carries the fleet-canonical `owner: "3f9a1c07"`; the script compares
/// against the id. On the unfixed base this is `NoEffect` — the assertion below
/// reddens, and the face it reddens against is the legitimate-looking
/// "no effects armed".
#[test]
fn a_quoted_owner_arms_the_claim_it_matches() {
    assert!(
        armed(r#" "3f9a1c07""#, &claim_if_owner_is("3f9a1c07")),
        "a quoted fleet-canonical owner must compare equal to the id it carries"
    );
}

/// The control that keeps the headline honest: the decode must not make every
/// comparison true. A DIFFERENT id still arms nothing.
#[test]
fn a_quoted_owner_does_not_arm_a_claim_it_does_not_match() {
    assert!(
        !armed(r#" "3f9a1c07""#, &claim_if_owner_is("b1892b5a")),
        "decoding is not tolerance: a wrong id must still be a false condition"
    );
}

/// The season-1 read corpus, every row, at the surface it was measured on. The
/// receipts recorded these as `len` 2 / 10 / 14 against expected 0 / 8 / 12.
#[test]
fn the_season_one_read_corpus_compares_by_value() {
    for (stored, value) in [
        (r#" """#, ""),                         // len 2 → 0
        (r#" "3f9a1c07""#, "3f9a1c07"),         // len 10 → 8
        (r#" "[[1ed98864]]""#, "[[1ed98864]]"), // len 14 → 12
        (" doing", "doing"),                    // the row that always worked
    ] {
        assert!(
            armed(stored, &claim_if_owner_is(value)),
            "stored {stored:?} must reach the script as {value:?}"
        );
    }
}

/// The receipts' `len` column, asserted as itself: an agent measuring the value
/// it holds must see the value's length, not the source line's.
#[test]
fn the_length_a_script_measures_is_the_values_own() {
    let src = format!(
        r#"
card = read("{CARD}")
if len(card.fm["owner"]) == 8:
    put("{CARD}", props={{"status": "doing"}})
"#
    );
    assert!(armed(r#" "3f9a1c07""#, &src), "8 bytes, not 10");
}

/// Single quotes are corpus-legal too, and the `''` escape is the one thing
/// inside them that is not itself.
#[test]
fn single_quoted_values_decode_including_their_one_escape() {
    assert!(armed(" '3f9a1c07'", &claim_if_owner_is("3f9a1c07")));
    assert!(armed(" 'it''s'", &claim_if_owner_is("it's")));
}

/// **Malformed quoting is served verbatim, never guessed at** (§ A.6.1). A
/// reader that "helpfully" stripped the outer quotes here would hand the script
/// a value the file does not carry.
#[test]
fn malformed_quoting_reaches_the_script_unchanged() {
    assert!(armed(
        r#" "a" and "b""#,
        &claim_if_owner_is(r#"\"a\" and \"b\""#)
    ));
}

/// A plain scalar is untouched — the decode is the quoting layer and nothing
/// else, so nothing that worked before this law stops working.
#[test]
fn plain_scalars_are_untouched() {
    assert!(armed(" doing", &claim_if_owner_is("doing")));
    assert!(armed(" [a, b]", &claim_if_owner_is("[a, b]")));
}
