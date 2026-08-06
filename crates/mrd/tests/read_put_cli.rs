//! U1 end-to-end gates for `mrd read` / `mrd put` — the ratified read/put naming at the CLI
//! face, driving the real binary over its process boundary.

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
    assert_eq!(
        rows[1]["hpath"],
        serde_json::json!([{"h": "Alpha"}, {"h": "Beta"}]),
        "the published address is the SEGMENT array (U14), not a joined string"
    );
}

/// Gate — `--section` IS the section read and answers the selected
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
    assert_eq!(
        sections[0]["sel"],
        serde_json::json!({"hpath": [{"h": "Alpha"}, {"h": "Beta"}]}),
        "U14: the selector is echoed in the caller's own TAGGED grammar, so a \
         caller pairing responses to requests compares structure to structure"
    );
    assert!(
        sections[0]["content"]
            .as_str()
            .expect("content")
            .contains("four five"),
        "the section's content rides: {sections:?}"
    );
}

// ---------------------------------------------------------------------------
// read face v2 (ZT 2026-08-04) — the dogfood G2/G3/G4/G8 gates
// ---------------------------------------------------------------------------

/// A page whose deep heading has a title distinct from every spelling of its PATH: the
/// ancestors are long, and `Slash/Title Here` sanitizes to something else again, so "the row
/// prints its raw title" and "the row prints its full address" cannot both pass.
const DEEP: &str = "# Architecture and its discontents\n\nintro words\n\n\
    ## Storage layer considered as a whole\n\nlayer words\n\n\
    ### Slash/Title Here\n\nleaf words\n";

/// Gate G2 — the human toc row carries the leaf's raw title and not its ancestors.
#[test]
fn the_human_toc_prints_raw_titles_and_never_re_types_ancestors() {
    let sb = sandbox();
    let ws = sb.workspace_with(DEEP);
    let out = sb.run(&ws, &["read", "doc.md"]);
    assert_eq!(code(&out), 0, "read: {}", stderr(&out));
    let text = stdout(&out);

    assert!(
        text.contains("\n  1.1.1,3,Slash/Title Here,"),
        "the deep row is its dewey, its depth and its RAW title — the raw \
         spelling `put` takes, not the sanitized `Slash-Title-Here`. The \
         ordinal is bare here and quoted at depth 2 because the encoder quotes \
         exactly when bare would lie, and `1.1.1` decodes as no number: {text}"
    );
    assert!(
        !text.contains("discontents/"),
        "no row re-types an ancestor path (G2): {text}"
    );
    assert!(
        text.lines().all(|l| l.chars().count() <= 120),
        "every row fits a terminal: {text}"
    );
}

/// Gate G8 — the human toc prints the fingerprint, and it is the value `put --if-fingerprint`
/// accepts: the token round-trips.
#[test]
fn the_fingerprint_the_human_toc_prints_is_the_guard_the_put_takes() {
    let sb = sandbox();
    let ws = sb.workspace();
    let text = stdout(&sb.run(&ws, &["read", "doc.md"]));
    let fp = text
        .lines()
        .find_map(|l| l.strip_prefix("fp: "))
        .expect("the human toc prints fp")
        .trim_matches('"')
        .to_owned();
    assert!(fp.starts_with("b3:"), "a real fingerprint token: {fp}");

    let edits = match_at(
        &serde_json::json!([{"h": "Alpha"}]),
        "one two three",
        "one two four",
    );
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--dry", "--if-fingerprint", &fp],
        &edits,
    );
    assert_eq!(
        code(&out),
        0,
        "the guard admits the fp the read printed: {}",
        stderr(&out)
    );
}

/// Gate G3/G4 — a toc read's `--json` serves the structured rows alone: an agent reading JSON
/// wants the rows, not a render of them. Where a body was requested, `rendered_text` is prose
/// and still rides.
#[test]
fn json_serves_the_map_once_and_the_prose_only_when_a_body_was_asked_for() {
    let sb = sandbox();
    let ws = sb.workspace();

    let out = sb.run(&ws, &["read", "doc.md", "--json"]);
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let body = &v["read"];
    assert!(body["toc"].is_array(), "the structured map rides: {body}");
    assert!(
        body.get("rendered_text").is_none(),
        "and is not repeated as a render of itself (G4): {body}"
    );

    let out = sb.run(
        &ws,
        &["read", "doc.md", "--section", "Alpha/Beta", "--json"],
    );
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let rendered = v["read"]["rendered_text"]
        .as_str()
        .expect("a body was requested, so the prose rides");
    assert!(
        rendered.contains("four five"),
        "and it is the PROSE, not the map (G3): {rendered}"
    );
}

/// Gate G2 (sections) — a body opens with its `n`, not with its full address. The marker is
/// the dewey ordinal — and that ordinal is a `--section` selector, so the line that opens a
/// body also says how to ask for it again.
#[test]
fn a_section_body_opens_with_its_ordinal_not_its_full_address() {
    let sb = sandbox();
    let ws = sb.workspace_with(DEEP);
    let text = stdout(&sb.run(&ws, &["read", "doc.md", "--section", "1.1.1"]));
    assert!(
        text.contains("\n== 1.1.1 ==\n"),
        "the body's marker is its ordinal: {text}"
    );
    assert!(
        !text.contains("== Architecture"),
        "and never the mega full-address banner (G2): {text}"
    );
    assert!(
        text.contains("leaf words"),
        "the body itself is served verbatim: {text}"
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

/// Gate — A5: `--mode` is retired, so it is an unknown flag (exit 2), not a quietly-accepted
/// word. The selector alone says which face the caller wants, and a stale invocation learns
/// that here rather than getting a toc it did not ask for.
#[test]
fn read_mode_flag_is_retired_and_unknown() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &["read", "doc.md", "--mode", "toc", "--section", "Alpha"],
    );
    assert_eq!(code(&out), 2, "a retired flag is a tool failure");
    assert!(
        stderr(&out).contains("unknown flag: --mode"),
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
// read → put round-trip: the address `read` publishes IS a `put` target
// ---------------------------------------------------------------------------
//
// `sanitize_heading` is many-to-one (`Scratch notes`, `Scratch-notes` and
// `Scratch/notes` all map to `Scratch-notes`), so a sanitized string can never
// recover its pre-image. The read face publishes one address — the raw segment
// array — and it is the grammar `put` accepts. These gates assert that shape.

/// Read `doc.md`'s toc rows as JSON.
fn toc_rows(sb: &Sandbox, ws: &Path) -> Vec<Value> {
    let out = sb.run(ws, &["read", "doc.md", "--json"]);
    assert_eq!(code(&out), 0, "read --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    v["read"]["toc"].as_array().expect("toc rows").clone()
}

/// The published address of one toc row, asserted to be a real segment array (U14: `hpath` is
/// the raw array).
fn published_address(row: &Value) -> Value {
    let raw = row.get("hpath").unwrap_or(&Value::Null);
    assert!(
        raw.is_array() && !raw.as_array().expect("array").is_empty(),
        "the read face must publish hpath as a raw segment array, got {raw} in row {row}"
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

/// Gate — the loop closes on a NESTED section: the address `read` publishes, fed straight back
/// as a `put` target, lands the write on the section that was read. The ancestor's raw text
/// `Scratch notes` rides the published address verbatim; the sanitized spelling
/// (`Scratch-notes/Findings`) that used to occupy `hpath` never carried it, and is no longer
/// published at all.
#[test]
fn read_published_address_round_trips_into_put() {
    let sb = sandbox();
    let ws = sb.workspace_with("# Scratch notes\n\nouter body\n\n## Findings\n\ninner body\n");
    let rows = toc_rows(&sb, &ws);
    assert_eq!(rows.len(), 2, "two heading rows: {rows:?}");
    assert_eq!(
        rows[1]["hpath"],
        serde_json::json!([{"h": "Scratch notes"}, {"h": "Findings"}]),
        "the published address carries the ancestor's RAW text (U14) — the \
         sanitized `Scratch-notes/Findings` spelling is gone from this face"
    );

    let addr = published_address(&rows[1]);
    assert_eq!(
        addr,
        serde_json::json!([{"h": "Scratch notes"}, {"h": "Findings"}]),
        "the raw ancestor text survives the read face"
    );

    let out = sb.run_stdin(&ws, &["put", "doc.md"], &match_at(&addr, "inner", "INNER"));
    assert_eq!(
        code(&out),
        0,
        "the published address is a put target: {}",
        stderr(&out)
    );
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(
        after.contains("INNER body"),
        "the write landed on the section read: {after}"
    );
    assert!(after.contains("outer body"), "and nowhere else: {after}");
}

/// Gate — the sanitized string was never an address: split on `/` and fed back as a segment
/// array it resolves to nothing. This is the information loss the round-trip gate above routes
/// around (the refusal's WORDING belongs to the separate refusal sweep, so only the leg is
/// asserted).
#[test]
fn the_sanitized_address_does_not_round_trip() {
    let sb = sandbox();
    let ws = sb.workspace_with("# Scratch notes\n\nouter body\n\n## Findings\n\ninner body\n");
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
    let sanitized = serde_json::json!([{"h": "Scratch-notes"}, {"h": "Findings"}]);
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &match_at(&sanitized, "inner", "INNER"),
    );
    assert_eq!(code(&out), 1, "the sanitized spelling addresses nothing");
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        before,
        "and it lands no bytes"
    );
}

/// Gate — the collision case: three headings that sanitize to one address each round-trip to
/// their own section.
#[test]
fn each_collider_round_trips_to_its_own_section() {
    let sb = sandbox();
    let ws = sb.workspace_with(
        "# Scratch notes\n\nalpha body\n\n# Scratch-notes\n\nbeta body\n\n# Scratch/notes\n\ngamma body\n",
    );
    let rows = toc_rows(&sb, &ws);
    assert_eq!(rows.len(), 3, "three heading rows: {rows:?}");
    let published: Vec<Value> = rows
        .iter()
        .map(|r| r.get("hpath").cloned().unwrap_or(Value::Null))
        .collect();
    for (i, a) in published.iter().enumerate() {
        for (j, b) in published.iter().enumerate().skip(i + 1) {
            assert_ne!(
                a, b,
                "rows {i} and {j} publish the SAME address — the sanitized \
                 many-to-one collision is back on the read face (U14)"
            );
        }
    }

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
        assert!(
            after.contains(&format!("{body} EDITED")),
            "{body} landed: {after}"
        );
    }
}

/// Gate — the `n` law, both halves. An occurrence index rides ONLY where the raw text is
/// ambiguous among its siblings: duplicate `Notes` sections publish `n`, and the `Child` unique
/// under its parent publishes none (an unconditional `n` there would be a lie the resolver
/// rejects). The published `n` then selects the right duplicate.
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
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &match_at(&addr, "second", "SECOND"),
    );
    assert_eq!(
        code(&out),
        0,
        "the disambiguated address writes: {}",
        stderr(&out)
    );
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(
        after.contains("SECOND body"),
        "n=2 selected the second: {after}"
    );
    assert!(
        after.contains("first body"),
        "and left the first alone: {after}"
    );
}

/// Gate — the case that proves `n`-only-where-ambiguous: a duplicate prepended above the
/// original invalidates every published address to it. Under an unconditional `n` the address
/// would read `n=1` and silently retarget onto the interloper — a wrong write, no refusal.
/// Under only-where-ambiguous it carries no `n`, so the same address now refuses loud.
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
    assert_eq!(
        code(&out),
        1,
        "the stale address must refuse, not pick: {}",
        stderr(&out)
    );
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(
        after.contains("interloper body") && after.contains("original body"),
        "a refusal writes nothing — neither section moved: {after}"
    );
}

/// Read and put must agree on what `n` counts. `put` resolves `n` among siblings sharing the
/// raw text (`model::resolve_hpath_node`); numbered on any other basis the published
/// `{h:"Notes",n:2}` would still be well-formed and land on the wrong section silently.
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
    let out = sb.run(
        &ws,
        &[
            "read",
            "doc.md",
            "--section",
            // U14 / D2: `--section` takes the RAW heading text — what you read
            // is what you type.
            "Scratch notes/Findings",
            "--json",
        ],
    );
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
    assert_eq!(
        code(&out),
        0,
        "the section's own address writes: {}",
        stderr(&out)
    );
    assert!(
        std::fs::read_to_string(ws.join("doc.md"))
            .expect("read back")
            .contains("INNER body"),
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

/// Gate — D3: `--dry` SHOWS the change. The unified diff runs from the file's current bytes to
/// the candidate the rehearsal built, so a caller decides from what WOULD land rather than from
/// a count of edits.
#[test]
fn put_dry_shows_the_diff() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--dry"],
        &beta_match("four five", "four FIVE six"),
    );
    assert_eq!(code(&out), 0, "dry put: {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("--- current") && text.contains("+++ candidate"),
        "the sides are named for what they ARE — never a/ and b/ files: {text}"
    );
    assert!(
        text.contains("-four five") && text.contains("+four FIVE six"),
        "both sides of the change ride: {text}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        DOC,
        "showing the diff is still a rehearsal — nothing lands"
    );
}

/// Gate — D3: `--dry --json` carries the same diff as a FIELD, so a machine
/// caller reads it out of the frame instead of scraping stdout.
#[test]
fn put_dry_json_carries_the_diff_as_a_field() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--dry", "--json"],
        &beta_match("four five", "four FIVE six"),
    );
    assert_eq!(code(&out), 0, "dry put --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let diff = v["diff"].as_str().expect("diff field");
    assert!(
        diff.contains("+four FIVE six"),
        "the diff is the field's value: {diff}"
    );
}

/// Gate — D3: `--validate` is the silent check. A rehearsal that passes says nothing and
/// answers with exit 0 alone; a line of reassurance is what would stop it being a silent check.
#[test]
fn put_validate_is_silent_on_a_pass() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--validate"],
        &beta_match("four five", "changed"),
    );
    assert_eq!(code(&out), 0, "validate: {}", stderr(&out));
    assert_eq!(stdout(&out), "", "a passing check says nothing");
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        DOC,
        "--validate is a rehearsal: nothing lands"
    );
}

/// Gate — D3: a finding under `--validate` is NOT silent. The engine's verbatim refusal rides
/// stderr at exit 1, the same body `--dry` would get — the two faces differ on a PASS and
/// nowhere else.
#[test]
fn put_validate_findings_exit_nonzero_with_the_refusal_body() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--validate"],
        &beta_match("nothing matches this", "x"),
    );
    assert_eq!(code(&out), 1, "a finding is the findings leg");
    assert_eq!(stdout(&out), "", "the refusal rides stderr, not stdout");
    assert!(
        stderr(&out).contains("no_match") || stderr(&out).contains("occurrence"),
        "the engine's verbatim refusal: {}",
        stderr(&out)
    );
}

/// Gate — D3: `--dry` and `--validate` are the two faces of one rehearsal, so asking for both
/// is a contradiction (exit 2), never a silent precedence rule a caller has to learn.
#[test]
fn put_dry_and_validate_together_refuse_loudly() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--dry", "--validate"],
        &beta_match("four five", "changed"),
    );
    assert_eq!(code(&out), 2, "contradiction is a tool failure");
    assert!(
        stderr(&out).contains("--dry and --validate"),
        "{}",
        stderr(&out)
    );
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

/// Gate (G9) — the §4.4 REQUEST object on stdin is refused, and the refusal names the mistake
/// instead of leaving a caller with "expected a sequence" and the doc that misled them.
#[test]
fn the_wire_request_envelope_on_stdin_is_refused_by_name() {
    let sb = sandbox();
    let ws = sb.workspace();
    let envelope = r#"{"id":42,"op":"splice","path":"doc.md","edits":[
        {"target":{"hpath":[{"h":"Alpha"}]},
         "edit":{"match":{"old":"one","new":"ONE"}}}]}"#;
    let out = sb.run_stdin(&ws, &["put", "doc.md", "--validate"], envelope);
    assert_eq!(code(&out), 2, "a bad invocation, not a refusal");
    let err = stderr(&out);
    assert!(err.contains("BARE edits ARRAY"), "{err}");
    assert!(err.contains("\"edits\""), "{err}");

    // The shape it points at is the shape that works — asserted here so the
    // hint can never teach a grammar the door does not accept.
    let bare = r#"[{"target":{"hpath":[{"h":"Alpha"}]},
                    "edit":{"match":{"old":"one","new":"ONE"}}}]"#;
    let ok = sb.run_stdin(&ws, &["put", "doc.md", "--validate"], bare);
    assert_eq!(code(&ok), 0, "{}", stderr(&ok));
}

/// Gate (G9) — an object with no `edits` key gets the decoder's own message and NOTHING added:
/// the hint fires on the diagnosed case only, so it can never mis-name some other malformed
/// input.
#[test]
fn a_non_envelope_object_gets_no_envelope_hint() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(&ws, &["put", "doc.md", "--validate"], r#"{"nope":1}"#);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("malformed edits JSON on stdin"), "{err}");
    assert!(!err.contains("BARE edits ARRAY"), "{err}");
}

// ---------------------------------------------------------------------------
// the refuse surface: what a refusal tells a person at a terminal
// ---------------------------------------------------------------------------

/// Gate (G7) — a `cas_mismatch` refusal tells the caller to apply the `diff` extra and resend
/// with `new_fingerprint`. On the human face those were wire fields nobody could see, so the
/// no-re-read shortcut was unreachable exactly where it was offered. Both now PRINT under the
/// message.
#[test]
fn cas_mismatch_prints_the_extras_its_message_names() {
    let sb = sandbox();
    let ws = sb.workspace();
    let stale = serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Alpha"}, {"h": "Beta"}]},
        "edit": {"match": {"old": "four five", "new": "six"}},
        "if_node_rev": "b3b:0000000000000000000000000000000000000000000000000000000000000000",
    }]))
    .expect("edits json");
    let out = sb.run_stdin(&ws, &["put", "doc.md", "--dry"], &stale);
    assert_eq!(code(&out), 1, "cas_mismatch refuses: {}", stderr(&out));
    let text = stderr(&out);
    assert!(
        text.contains("new_fingerprint: "),
        "the token to resend with is printable:\n{text}"
    );
    assert!(
        text.contains("diff (apply this to your copy):") || text.contains("new_content ("),
        "the rung's own extra is printable:\n{text}"
    );
}

/// Gate (G6) — a `root_mismatch` was a bare `expected …, actual …` line: no sentence, no
/// nothing-was-written clause, no fix. It now reads like the read face's exemplary refusal, and
/// its fix line carries the fingerprint the guard wants (G8, partially: the value is reachable
/// without `--json`).
#[test]
fn root_mismatch_names_the_failure_and_gives_a_fix() {
    let sb = sandbox();
    let ws = sb.workspace();
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
    let out = sb.run_stdin(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3b:0000000000000000000000000000000000000000000000000000000000000000",
        ],
        &beta_match("four five", "six"),
    );
    assert_eq!(code(&out), 1, "root_mismatch refuses: {}", stderr(&out));
    let text = stderr(&out);
    assert!(text.contains("root_mismatch"), "names the failure:\n{text}");
    assert!(
        text.contains("No edit was applied"),
        "states what did NOT happen:\n{text}"
    );
    assert!(
        text.contains("Fix: re-run with `--if-fingerprint b3"),
        "the fix line carries the fingerprint the guard wants:\n{text}"
    );
    assert!(
        text.contains("pinned:") && text.contains("current:"),
        "both tokens stay printable:\n{text}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        before,
        "the refusal's own claim: nothing was written"
    );
}

/// Gate — the malformed-stdin refusal claims nothing was written, and that claim is true for
/// a structural reason: `read_stdin_edits` runs before the workspace is resolved and before
/// any splice, so the exit happens with zero engine contact. Nothing but this gate holds that
/// ordering in place. Pinned across the malformed family, not one spelling, because the
/// clause is on all of them.
#[test]
fn a_malformed_stdin_refusal_leaves_the_document_byte_unchanged() {
    for stdin in [
        "not json at all",
        r#"{"id":"1","edits":[{"target":{"hpath":[{"h":"Alpha"}]}}]}"#,
        // A truncated array and a stray comma: malformed in a way that has
        // nothing to do with an envelope.
        r#"[{"target":{"hpath":[{"h":"Alpha"}]}}"#,
        r#"[{"target":{"hpath":[{"h":"Alpha"}]}},]"#,
    ] {
        let sb = sandbox();
        let ws = sb.workspace();
        let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
        let out = sb.run_stdin(&ws, &["put", "doc.md"], stdin);
        assert_eq!(
            code(&out),
            2,
            "bad invocation for {stdin}: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("Nothing was parsed and nothing was written"),
            "every malformed case states it for {stdin}:\n{}",
            stderr(&out)
        );
        assert_eq!(
            std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
            before,
            "the refusal's own claim, enforced, for {stdin}"
        );
    }
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

/// Gate (G9) — `mrd put --help` states the stdin shape itself, rather than delegating the whole
/// question to a wire section whose worked example shows a request object. The citation stays;
/// what changed is that the help no longer needs the reader to notice which part of §4.4 it
/// meant.
#[test]
fn put_help_states_the_bare_array_stdin_shape() {
    let sb = sandbox();
    let out = sb.run(sb.tmp.path(), &["put", "--help"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let page = stdout(&out);
    assert!(page.contains("BARE JSON"), "{page}");
    assert!(page.contains("§4.4"), "the wire citation stays:\n{page}");
    assert!(
        !page.contains("\"edits\":"),
        "the help must not show the envelope it refuses:\n{page}"
    );
}
