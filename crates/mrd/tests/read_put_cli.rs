//! U1 end-to-end gates for `mrd read` / `mrd put` — the ratified read/put
//! naming at the CLI face, driving the REAL binary over its process boundary.
//!
//! `read` gates pin the composed-read face (U4a2 leaves + v3 projection) on
//! the DEGRADE path (`MERIDIAN_DAEMON_BIN` points nowhere, so the auto-spawn
//! is spawn-impossible and the answer is deterministic in-process). `put`
//! gates pin the A-S1 obligation observable end-to-end: bytes land through
//! the production splice choke-point, a dry run lands nothing, and every
//! refusal leg exits the triad correctly (1 refused / 2 bad invocation).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

impl Sandbox {
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            // Spawn-impossible: the read path degrades in-process,
            // deterministically — no resident daemon ever starts.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    /// Run with `stdin_bytes` piped — the `put` edits channel.
    fn run_stdin(&self, cwd: &Path, args: &[&str], stdin_bytes: &str) -> Output {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(stdin_bytes.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("wait mrd")
    }

    /// A marked workspace holding the two-heading fixture doc.
    fn workspace(&self) -> PathBuf {
        self.workspace_with(DOC)
    }

    /// A marked workspace whose `doc.md` holds `body` — the round-trip gates
    /// need colliding and duplicate headings the shared fixture cannot carry.
    fn workspace_with(&self, body: &str) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("doc.md"), body).expect("doc");
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

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

/// Gate — default (toc) mode serves the rendered projection verbatim on
/// stdout: the display path the user typed, and both section titles.
#[test]
fn read_toc_serves_the_rendered_projection() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read", "doc.md"]);
    assert_eq!(code(&out), 0, "read: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("doc.md"), "display path rides: {text}");
    assert!(
        text.contains("Alpha") && text.contains("Beta"),
        "both sections listed: {text}"
    );
}

/// Gate — `--json` carries the v3-projected composed body: `file_rev` +
/// `fingerprint` (the D6 atomicity witness, `fingerprint` vocabulary — never
/// bare `root`), the toc rows with dewey ordinals, and the ephemeral source
/// label (the daemon was spawn-impossible).
#[test]
fn read_json_carries_the_v3_projected_body() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read", "doc.md", "--json"]);
    assert_eq!(code(&out), 0, "read --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    assert_eq!(v["source"], "ephemeral", "degrade path answered: {v}");
    let body = &v["read"];
    assert!(body["file_rev"].is_string(), "file_rev rides: {body}");
    assert!(
        body["fingerprint"].is_string(),
        "v3 vocabulary (fingerprint, never root): {body}"
    );
    assert!(body.get("root").is_none(), "bare root never rides: {body}");
    assert!(body["words_total"].is_u64(), "words_total rides: {body}");
    let rows = body["toc"].as_array().expect("toc rows");
    assert_eq!(rows.len(), 2, "two heading rows: {body}");
    assert_eq!(rows[0]["n"], "1", "dewey ordinal on the first row");
    assert_eq!(rows[1]["n"], "1.1", "dewey ordinal on the nested row");
    assert_eq!(rows[1]["hpath"], "Alpha/Beta", "sanitized joined hpath");
}

/// Gate — `--section` implies sections mode and answers the selected
/// section's content.
#[test]
fn read_section_selects_and_serves_content() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &["read", "doc.md", "--section", "Alpha/Beta", "--json"],
    );
    assert_eq!(code(&out), 0, "read --section: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let sections = v["read"]["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0]["sel"], "Alpha/Beta");
    assert!(
        sections[0]["content"]
            .as_str()
            .expect("content")
            .contains("four five"),
        "the section's content rides: {sections:?}"
    );
}

/// Gate — a `#FRAG` naming no section is the engine's refusal, VERBATIM, at
/// exit 1 (the finding leg — never rewritten client-side).
#[test]
fn read_frag_miss_is_the_engines_verbatim_refusal() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read", "doc.md#Nope"]);
    assert_eq!(code(&out), 1, "a refusal is the finding leg");
    assert!(
        stderr(&out).contains("no section at \"Nope\""),
        "the engine's verbatim message: {}",
        stderr(&out)
    );
}

/// Gate — `--mode toc` + `--section` is a LOUD client-side contradiction
/// (exit 2), never the wire's silent ignore.
#[test]
fn read_toc_mode_with_section_refuses_loudly() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &["read", "doc.md", "--mode", "toc", "--section", "Alpha"],
    );
    assert_eq!(code(&out), 2, "contradiction is a tool failure");
    assert!(
        stderr(&out).contains("--section conflicts with --mode toc"),
        "{}",
        stderr(&out)
    );
}

/// Gate — no PATH is a bad invocation (exit 2).
#[test]
fn read_without_path_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read"]);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("read needs a PATH"),
        "{}",
        stderr(&out)
    );
}

// ---------------------------------------------------------------------------
// read → put round-trip (fix-08): the address `read` publishes IS a `put`
// target
// ---------------------------------------------------------------------------
//
// The defect these gates close: `read` published only `hpath`, the sanitized
// joined string, and `sanitize_heading` is MANY-TO-ONE (`Scratch notes`,
// `Scratch-notes` and `Scratch/notes` all map to `Scratch-notes`). `put` takes
// the RAW segment array. So the output grammar was a lossy projection of the
// input grammar and nothing in the read output recovered the pre-image — the
// agent loop (read a document, decide an edit, write it) could not close by
// copying. `hpath_raw` carries the raw array through the read face verbatim.

/// Read `doc.md`'s toc rows as JSON.
fn toc_rows(sb: &Sandbox, ws: &Path) -> Vec<Value> {
    let out = sb.run(ws, &["read", "doc.md", "--json"]);
    assert_eq!(code(&out), 0, "read --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    v["read"]["toc"].as_array().expect("toc rows").clone()
}

/// The published `hpath_raw` of one toc row, asserted to be a real segment
/// array — the RED half of these gates: at base rev the field is absent.
fn published_address(row: &Value) -> Value {
    let raw = row.get("hpath_raw").unwrap_or(&Value::Null);
    assert!(
        raw.is_array() && !raw.as_array().expect("array").is_empty(),
        "the read face must publish hpath_raw as a raw segment array, got {raw} in row {row}"
    );
    raw.clone()
}

/// A one-edit match batch at `target_hpath` — the published address fed back
/// VERBATIM, never retyped.
fn match_at(target_hpath: &Value, old: &str, new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": target_hpath},
        "edit": {"match": {"old": old, "new": new}},
    }]))
    .expect("edits json")
}

/// Gate — the loop closes on a NESTED section: the address `read` publishes,
/// fed straight back as a `put` target, lands the write on the section that
/// was read. The ancestor `Scratch notes` survives only in `hpath_raw`; the
/// sanitized `hpath` (`Scratch-notes/Findings`) never carried it.
#[test]
fn read_published_address_round_trips_into_put() {
    let sb = sandbox();
    let ws = sb.workspace_with("# Scratch notes\n\nouter body\n\n## Findings\n\ninner body\n");
    let rows = toc_rows(&sb, &ws);
    assert_eq!(rows.len(), 2, "two heading rows: {rows:?}");
    assert_eq!(
        rows[1]["hpath"], "Scratch-notes/Findings",
        "the sanitized address still rides, unrenamed"
    );

    let addr = published_address(&rows[1]);
    assert_eq!(
        addr,
        serde_json::json!([{"h": "Scratch notes"}, {"h": "Findings"}]),
        "the raw ancestor text survives the read face"
    );

    let out = sb.run_stdin(&ws, &["put", "doc.md"], &match_at(&addr, "inner", "INNER"));
    assert_eq!(code(&out), 0, "the published address is a put target: {}", stderr(&out));
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(after.contains("INNER body"), "the write landed on the section read: {after}");
    assert!(after.contains("outer body"), "and nowhere else: {after}");
}

/// Gate — the sanitized string was never an address: split on `/` and fed back
/// as a segment array it resolves to nothing. This is the information loss the
/// round-trip gate above routes around (the refusal's WORDING belongs to the
/// separate refusal sweep, so only the leg is asserted).
#[test]
fn the_sanitized_address_does_not_round_trip() {
    let sb = sandbox();
    let ws = sb.workspace_with("# Scratch notes\n\nouter body\n\n## Findings\n\ninner body\n");
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
    let sanitized = serde_json::json!([{"h": "Scratch-notes"}, {"h": "Findings"}]);
    let out = sb.run_stdin(&ws, &["put", "doc.md"], &match_at(&sanitized, "inner", "INNER"));
    assert_eq!(code(&out), 1, "the sanitized spelling addresses nothing");
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        before,
        "and it lands no bytes"
    );
}

/// Gate — the collision case: three headings that sanitize to ONE address each
/// round-trip to their OWN section. On the read face `hpath` is `Scratch-notes`
/// for all three (and a `--section` selector silently serves the first);
/// `hpath_raw` distinguishes them byte-exactly, so each published address hits
/// exactly one section.
#[test]
fn each_collider_round_trips_to_its_own_section() {
    let sb = sandbox();
    let ws = sb.workspace_with(
        "# Scratch notes\n\nalpha body\n\n# Scratch-notes\n\nbeta body\n\n# Scratch/notes\n\ngamma body\n",
    );
    let rows = toc_rows(&sb, &ws);
    assert_eq!(rows.len(), 3, "three heading rows: {rows:?}");
    assert_eq!(
        rows.iter().map(|r| r.get("hpath").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        vec![
            Value::from("Scratch-notes"),
            Value::from("Scratch-notes"),
            Value::from("Scratch-notes")
        ],
        "one sanitized address for three sections — the many-to-one map"
    );

    let addrs: Vec<Value> = rows.iter().map(published_address).collect();
    assert_eq!(
        addrs,
        vec![
            serde_json::json!([{"h": "Scratch notes"}]),
            serde_json::json!([{"h": "Scratch-notes"}]),
            serde_json::json!([{"h": "Scratch/notes"}]),
        ],
        "three sections, three distinct published addresses"
    );

    for (addr, body) in addrs.iter().zip(["alpha", "beta", "gamma"]) {
        let out = sb.run_stdin(
            &ws,
            &["put", "doc.md"],
            &match_at(addr, &format!("{body} body"), &format!("{body} EDITED")),
        );
        assert_eq!(code(&out), 0, "collider {body}: {}", stderr(&out));
    }
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    for body in ["alpha", "beta", "gamma"] {
        assert!(after.contains(&format!("{body} EDITED")), "{body} landed: {after}");
    }
}

/// Gate — the `n` law, both halves. An occurrence index rides ONLY where the
/// raw text is ambiguous among its siblings: duplicate `# Notes` sections
/// publish `n`, and the `## Child` unique under its parent publishes none
/// (an unconditional `n` there would be a lie the resolver rejects). The
/// published `n` then selects the right duplicate.
#[test]
fn occurrence_index_rides_only_where_the_raw_text_is_ambiguous() {
    let sb = sandbox();
    let ws = sb.workspace_with(
        "# Notes\n\nfirst body\n\n# Notes\n\nsecond body\n\n## Child\n\nchild body\n",
    );
    let rows = toc_rows(&sb, &ws);
    assert_eq!(rows.len(), 3, "three heading rows: {rows:?}");
    assert_eq!(
        rows.iter().map(published_address).collect::<Vec<_>>(),
        vec![
            serde_json::json!([{"h": "Notes", "n": 1}]),
            serde_json::json!([{"h": "Notes", "n": 2}]),
            serde_json::json!([{"h": "Notes", "n": 2}, {"h": "Child"}]),
        ],
        "n on the ambiguous segment, and ONLY there — Child is unique under its parent"
    );

    let addr = published_address(&rows[1]);
    let out = sb.run_stdin(&ws, &["put", "doc.md"], &match_at(&addr, "second", "SECOND"));
    assert_eq!(code(&out), 0, "the disambiguated address writes: {}", stderr(&out));
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(after.contains("SECOND body"), "n=2 selected the second: {after}");
    assert!(after.contains("first body"), "and left the first alone: {after}");
}

/// Gate — the case that PROVES `n`-only-where-ambiguous, per leader 2702bc87.
/// A duplicate PREPENDED above the original invalidates every published
/// address to it. Under an unconditional `n` the address would read `n=1` and
/// silently retarget onto the interloper — a wrong write, no refusal. Under
/// only-where-ambiguous the address carries no `n`, so the same address now
/// refuses LOUD. Assert the refusal, and that neither section moved.
#[test]
fn a_prepended_duplicate_makes_the_published_address_refuse_loud() {
    let sb = sandbox();
    let ws = sb.workspace_with("# Notes\n\noriginal body\n");
    let addr = published_address(&toc_rows(&sb, &ws)[0]);
    assert_eq!(
        addr,
        serde_json::json!([{"h": "Notes"}]),
        "unique while unique: no occurrence index invented"
    );

    // The world changes under the held address: a second `# Notes` arrives ABOVE.
    std::fs::write(
        ws.join("doc.md"),
        "# Notes\n\ninterloper body\n\n# Notes\n\noriginal body\n",
    )
    .expect("prepend");

    let out = sb.run_stdin(&ws, &["put", "doc.md"], &match_at(&addr, "body", "EDITED"));
    assert_eq!(code(&out), 1, "the stale address must refuse, not pick: {}", stderr(&out));
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(
        after.contains("interloper body") && after.contains("original body"),
        "a refusal writes nothing — neither section moved: {after}"
    );
}

/// ACCEPTANCE CRITERION (merge owner d9419c03) — read and put must agree on
/// what `n` COUNTS. `put` resolves `n` among siblings sharing the RAW TEXT
/// (`model::resolve_hpath_node`). If the read face numbered on any other basis
/// — position among all sibling sections, document order, dewey — the
/// published `{h:"Notes",n:2}` would still be well-formed and would land on
/// the wrong section SILENTLY, at exit 0. So this asserts by CONTENT, byte for
/// byte, not by exit code.
///
/// The fixture separates the two bases: a unique `## Alpha` sits BEFORE the
/// duplicates, so "occurrence among same-text siblings" (1, 2 — correct) and
/// "position among all sibling sections" (2, 3 — wrong) disagree. Both
/// `## Notes` sections hold IDENTICAL bodies, so no match-text accident can
/// turn a wrong-sibling write into a loud refusal: it would exit 0 and change
/// the wrong bytes.
#[test]
fn the_n_carrying_address_round_trips_and_n_counts_same_text_siblings() {
    let sb = sandbox();
    let doc = "# Parent\n\n## Alpha\n\nalpha body\n\n## Notes\n\nshared body\n\n## Notes\n\nshared body\n";
    let ws = sb.workspace_with(doc);
    let rows = toc_rows(&sb, &ws);
    assert_eq!(
        rows.iter().map(published_address).collect::<Vec<_>>(),
        vec![
            serde_json::json!([{"h": "Parent"}]),
            serde_json::json!([{"h": "Parent"}, {"h": "Alpha"}]),
            serde_json::json!([{"h": "Parent"}, {"h": "Notes", "n": 1}]),
            serde_json::json!([{"h": "Parent"}, {"h": "Notes", "n": 2}]),
        ],
        "n counts occurrences among SAME-TEXT siblings (1, 2) — never position \
         among all sibling sections (which would be 2, 3 here)"
    );

    // n=1 must land on the FIRST `## Notes`, byte for byte.
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &match_at(&published_address(&rows[2]), "shared body", "first EDITED"),
    );
    assert_eq!(code(&out), 0, "n=1 writes: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        "# Parent\n\n## Alpha\n\nalpha body\n\n## Notes\n\nfirst EDITED\n\n## Notes\n\nshared body\n",
        "n=1 landed on the first same-text sibling, and only it"
    );

    // n=2 must land on the SECOND, from the address published before the edit.
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &match_at(&published_address(&rows[3]), "shared body", "second EDITED"),
    );
    assert_eq!(code(&out), 0, "n=2 writes: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        "# Parent\n\n## Alpha\n\nalpha body\n\n## Notes\n\nfirst EDITED\n\n## Notes\n\nsecond EDITED\n",
        "n=2 landed on the second same-text sibling, and only it"
    );
}

/// Gate — sections mode publishes the same round-trippable address, so the
/// read-a-section-then-write-it loop closes without a second read.
#[test]
fn sections_mode_publishes_the_round_trippable_address() {
    let sb = sandbox();
    let ws = sb.workspace_with("# Scratch notes\n\nouter body\n\n## Findings\n\ninner body\n");
    let out = sb.run(&ws, &["read", "doc.md", "--section", "Scratch-notes/Findings", "--json"]);
    assert_eq!(code(&out), 0, "read --section: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let sections = v["read"]["sections"].as_array().expect("sections").clone();
    assert_eq!(sections.len(), 1);

    let addr = published_address(&sections[0]);
    assert_eq!(
        addr,
        serde_json::json!([{"h": "Scratch notes"}, {"h": "Findings"}]),
        "the section row carries the raw array too"
    );
    let out = sb.run_stdin(&ws, &["put", "doc.md"], &match_at(&addr, "inner", "INNER"));
    assert_eq!(code(&out), 0, "the section's own address writes: {}", stderr(&out));
    assert!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back").contains("INNER body"),
        "the write landed on the section served"
    );
}

// ---------------------------------------------------------------------------
// put
// ---------------------------------------------------------------------------

/// The one-edit match batch against `Alpha/Beta`, in the wire §4.4 grammar.
fn beta_match(old: &str, new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Alpha"}, {"h": "Beta"}]},
        "edit": {"match": {"old": old, "new": new}},
    }]))
    .expect("edits json")
}

/// Gate — a match edit lands on disk through the choke-point; the human
/// summary reports the committed fingerprint.
#[test]
fn put_match_edit_commits_bytes() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &beta_match("four five", "four five six"),
    );
    assert_eq!(code(&out), 0, "put: {}", stderr(&out));
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(after.contains("four five six"), "the edit landed: {after}");
    let text = stdout(&out);
    assert!(
        text.contains("committed doc.md") && text.contains("fingerprint:"),
        "the human summary names the commit + fingerprint: {text}"
    );
}

/// Gate — `--dry` runs everything except disk: exit 0, bytes untouched.
#[test]
fn put_dry_lands_nothing() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--dry"],
        &beta_match("four five", "changed"),
    );
    assert_eq!(code(&out), 0, "dry put: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        DOC,
        "a dry run writes nothing"
    );
    assert!(stdout(&out).contains("nothing written"), "{}", stdout(&out));
}

/// Gate — `--json` carries the v3-projected splice body (`fingerprint_before`
/// / `fingerprint_after`, never bare `root_*`).
#[test]
fn put_json_speaks_the_v3_vocabulary() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--json"],
        &beta_match("four five", "four six"),
    );
    assert_eq!(code(&out), 0, "put --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let body = &v["put"];
    assert!(
        body["fingerprint_after"].is_string(),
        "fingerprint_after rides: {body}"
    );
    assert!(
        body.get("root_after").is_none(),
        "bare root_after never rides: {body}"
    );
}

/// Gate — a match with no occurrence is the engine's typed refusal at exit 1.
#[test]
fn put_no_match_is_the_finding_leg() {
    let sb = sandbox();
    let ws = sb.workspace();
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &beta_match("absent text", "anything"),
    );
    assert_eq!(code(&out), 1, "no_match refusal: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        before,
        "a refusal leaves the bytes untouched"
    );
}

/// Gate — malformed stdin (not the §4.4 grammar) is a bad invocation, exit 2.
#[test]
fn put_malformed_stdin_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(&ws, &["put", "doc.md"], "not json at all");
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("malformed edits JSON"),
        "{}",
        stderr(&out)
    );
}

/// Gate — empty stdin teaches the grammar, exit 2.
#[test]
fn put_empty_stdin_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(&ws, &["put", "doc.md"], "");
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("edits JSON on stdin"),
        "{}",
        stderr(&out)
    );
}

/// Gate — a malformed `--now` refuses at the face (the §9 format law), exit 2.
#[test]
fn put_malformed_now_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--now", "yesterday"],
        &beta_match("four five", "x"),
    );
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("--now must be RFC 3339"),
        "{}",
        stderr(&out)
    );
}

/// Gate — the USAGE text teaches both new verbs (printed on any unknown verb).
#[test]
fn usage_teaches_read_and_put() {
    let sb = sandbox();
    let out = sb.run(sb.tmp.path(), &["bogus-verb"]);
    assert_eq!(code(&out), 2);
    let usage = stderr(&out);
    assert!(
        usage.contains("mrd read <PATH>"),
        "usage teaches read:\n{usage}"
    );
    assert!(
        usage.contains("mrd put <PATH>"),
        "usage teaches put:\n{usage}"
    );
}
