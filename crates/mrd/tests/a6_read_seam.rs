//! § A.6.1 — the read half of the frontmatter scalar law, measured where the
//! dogfood receipts measured it: the script plane's `card["fm"][k]`.
//!
//! **The defect this pins (dogfood season 1, finding 1, fail-INERT).** The
//! script face served the key line's SOURCE BYTES, quotes included, so
//! `card["fm"]["owner"] == "3f9a1c07"` was false against a file carrying
//! `owner: "3f9a1c07"`. Nothing armed, and "no effects armed" is a legitimate
//! face — the failure had no signal at all. The fleet quotes by convention, so
//! the surface degraded on production data and passed on its own fixtures.
//!
//! **Why the assertion is an OUTCOME and not a string.** A test that read `fm`
//! and compared it here would prove the test's own comparison, not the engine's
//! behaviour. These drive the real script entry and assert on what the run DID:
//! a decoded value arms the claim and reaches `Committed`; a value still
//! carrying quote bytes leaves the run at `NoEffect`. That is exactly the silent
//! face the receipts caught, so it cannot pass vacuously. **Unchanged by either
//! rewrite below.**
//!
//! **Rewritten 2026-08-08 (Amendment 1a).** `read(path)` no longer fetches
//! frontmatter with one `cat` per key; it takes the whole plane from the
//! composed read's `props[]`, which the daemon serves already decoded. So the
//! decode moved out of the code path this file exercised, and the fake was
//! changed to serve each stored form through the production codec.
//!
//! ⭐ **Rewritten again 2026-08-23 (card
//! `script-door-commit-premise-world-grain-vs-touch-set`) — and this one is a
//! PROMOTION.** `mrd script` became one lane: the whole attempt is the wire
//! `script` op, so this process no longer performs the read, and a fake door
//! standing in for the daemon can no longer place a `props[]` where the
//! evaluation would see it. The rows here would have had nothing left to
//! measure.
//!
//! So they run against a LIVE daemon over REAL FILES instead. That is strictly
//! stronger than what they replaced, and in the direction the season-1 finding
//! came from: the old fake called `model::scalar::text` to stand in for the
//! daemon — one function of the chain — while this drives the whole chain the
//! corpus actually has (the frontmatter parse, `read_props`' § A.6 codec, the
//! kernel's comparison), on bytes as they sit on disk. A decode that worked in
//! the codec and broke in the parse was invisible before and is not now.
//!
//! The other half of the law keeps its own gate:
//! `wire-serve::read::props_scalar_tests` asserts `read_props` applies the § A.6
//! codec across every stored form.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome};
use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The one page each fixture serves.
const CARD: &str = "tasks/0011-token-audit.md";

// ── the live daemon ──────────────────────────────────────────────────────────

/// A real `RunningServer` over a corpus of one card, whose `owner` line carries
/// the stored form under test.
///
/// Struct fields drop in declaration order: `server` (stop → drain) MUST precede
/// `_tmp`, else the workspace vanishes under the builder — the class-2 flake
/// (pipelines 1098/1101).
struct Fixture {
    server: RunningServer,
    ws: PathBuf,
    _tmp: TempDir,
}

impl Fixture {
    /// `stored` is the bytes AFTER `owner:` on disk — the receipts' left-hand
    /// column, leading space included.
    fn serving(stored: &str) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let ws = tmp.path().join("ws");
        let card = ws.join(CARD);
        std::fs::create_dir_all(card.parent().expect("a parent")).expect("mkdir");
        std::fs::write(
            &card,
            format!("---\nowner:{stored}\nstatus: todo\n---\n\n# Goals\n\nship the script entry\n"),
        )
        .expect("seed the card");
        let server = RunningServer::start(config(&tmp)).expect("the daemon starts");
        Self {
            server,
            ws,
            _tmp: tmp,
        }
    }
}

/// A real daemon config: the reaper never evicts a warm engine mid-test, and the
/// idle-exit clock is the test's.
#[allow(clippy::duration_suboptimal_units)]
fn config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    config
}

/// The production NDJSON dialogue, with the `corpus_warming` retry the cold
/// gate's `recovery: retry` contract asks for.
struct LiveDoor {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl LiveDoor {
    fn open(socket: &Path, ws: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("dial the daemon");
        let mut door = Self {
            writer: stream.try_clone().expect("clone"),
            reader: BufReader::new(stream),
        };
        let hello = door
            .call(&json!({
                "op": "hello", "proto": 1, "contract": "v3",
                "workspace": ws.to_str().expect("utf-8 workspace"),
            }))
            .expect("the handshake");
        assert_eq!(
            serde_json::from_str::<Value>(&hello).expect("a frame")["ok"],
            json!(true),
            "the daemon binds the workspace: {hello}"
        );
        door
    }
}

impl Door for LiveDoor {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let started = std::time::Instant::now();
        loop {
            let mut line = serde_json::to_string(request)?;
            line.push('\n');
            self.writer.write_all(line.as_bytes())?;
            self.writer.flush()?;
            let mut response = String::new();
            self.reader.read_line(&mut response)?;
            if let Ok(frame) = serde_json::from_str::<Value>(&response)
                && frame["ok"] != json!(true)
                && frame["error"]["code"] == json!("corpus_warming")
            {
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "corpus_warming persisted past 30s; last: {response}"
                );
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            return Ok(response);
        }
    }
}

/// Run `src` against a daemon whose card's `owner` line carries `stored`, and
/// report whether the run armed anything.
fn armed(stored: &str, src: &str) -> bool {
    let fixture = Fixture::serving(stored);
    let mut door = LiveDoor::open(fixture.server.socket_path(), &fixture.ws);
    let argv = ["--actor".to_owned(), "8ab41c02".to_owned()];
    let trace = attempt(&argv, src, &mut door).expect("the attempt runs");
    match trace.outcome {
        ScriptOutcome::Committed => true,
        ScriptOutcome::NoEffect => false,
        other => panic!(
            "the run neither committed nor stayed inert: {other:?} — {:?}",
            trace.fault
        ),
    }
}

/// Golden scenario 1's own shape, keyed on the value under test.
fn claim_if_owner_is(expected: &str) -> String {
    format!(
        r#"
card = read("{CARD}")
if card["fm"]["owner"] == "{expected}":
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
if len(card["fm"]["owner"]) == 8:
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
