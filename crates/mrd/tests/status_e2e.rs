//! End-to-end gates for `mrd status` (U3.6, the LAST leg), driving the REAL
//! binary (`CARGO_BIN_EXE_mrd`) over its process boundary against on-disk
//! workspaces. These are the merge-gate evidence: the <1s wall-time budget on a
//! 3k-doc corpus, the armed/drifted INDEX line, the forced-write violation row,
//! the composed three-axis line, and the exit triad.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

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
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A bare workspace with an `mrd init` marker so `status` resolves it.
    fn workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
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

/// Exit code as an integer (2 when the process was signalled, which never happens
/// here).
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// Write an attested INDEX page pinning `(slug, severity, armed_rev, scope)` rows
/// and each convention's `CHECK.md`. The pinned `armed_rev` is the REAL
/// `evidence_rev` of the on-disk bytes, so a row is FRESH; passing a mismatched
/// `disk_check` makes it DRIFTED (the on-disk rev ≠ the pinned rev).
fn arm_convention(
    ws: &Path,
    slug: &str,
    severity: &str,
    pinned_check: &str,
    disk_check: &str,
) -> String {
    let dir = ws.join("conventions").join(slug);
    std::fs::create_dir_all(&dir).expect("conv dir");
    std::fs::write(dir.join("CHECK.md"), disk_check).expect("check");
    let pinned_rev = policy::evidence_rev(pinned_check);
    format!(
        "- [x] **{slug}** · {severity} · `{pinned_rev}` · [[conventions/{slug}/CHECK.md]] · `{slug}/**`"
    )
}

/// Assemble a valid INDEX page (title + preamble + rows) that `parse_index_strict`
/// accepts. Verified to parse by the caller's `status` read.
fn write_index(ws: &Path, rows: &[String]) {
    let dir = ws.join("conventions");
    std::fs::create_dir_all(&dir).expect("conventions dir");
    let page = format!(
        "# Attested conventions INDEX\n\nSwept from `conventions/`.\n\n{}\n",
        rows.join("\n")
    );
    std::fs::write(dir.join("INDEX.md"), page).expect("index");
}

/// Genesis: a bare workspace with no INDEX and no git — 0 armed, genesis boundary,
/// the anchor renders unverified (never a bare `at-tip`), exit 0.
#[test]
fn status_genesis_is_clean_and_unverified() {
    let sb = sandbox();
    let ws = sb.workspace("genesis");
    let out = sb.run(&ws, &["status"]);
    let so = stdout(&out);
    assert_eq!(code(&out), 0, "clean genesis exits 0: {}", stderr(&out));
    assert!(
        so.contains("0 armed · 0 drifted · 0 forced-since-realise"),
        "genesis INDEX line: {so}"
    );
    assert!(
        so.contains("(receipts boundary: genesis)"),
        "genesis boundary: {so}"
    );
    assert!(so.contains("pin green"), "clean pin: {so}");
    assert!(so.contains("convention off"), "no armed severity: {so}");
    // W-C1: fetch-less status never renders a bare `at-tip`.
    assert!(
        so.contains("anchor unverified") || so.contains("anchor as-known"),
        "the anchor is qualified, never bare: {so}"
    );
    assert!(!so.contains("anchor at-tip\n"), "no bare at-tip line: {so}");
}

/// Armed + drifted: two armed conventions, one fresh (block), one drifted (warn) —
/// armed=2, drifted=1, the pin axis reds, the severity rolls up to block, exit 1.
#[test]
fn status_armed_and_drifted_reds_and_exits_1() {
    let sb = sandbox();
    let ws = sb.workspace("armed");
    let fresh = arm_convention(&ws, "alpha", "block", "alpha law v1\n", "alpha law v1\n");
    let drifted = arm_convention(
        &ws,
        "beta",
        "warn",
        "beta law v1\n",
        "beta law v2 DRIFTED\n",
    );
    write_index(&ws, &[fresh, drifted]);

    let out = sb.run(&ws, &["status"]);
    let so = stdout(&out);
    assert_eq!(
        code(&out),
        1,
        "a drift is a finding (exit 1): {}",
        stderr(&out)
    );
    assert!(
        so.contains("2 armed · 1 drifted"),
        "armed=2 drifted=1: {so}"
    );
    assert!(
        so.contains("pin red content-drifted"),
        "the pin axis reds: {so}"
    );
    assert!(
        so.contains("convention block"),
        "severity rolls up worst-of to block: {so}"
    );
}

/// A forced write (an `op=force` journal row) renders a violation row naming the
/// bypassed rule, counts in forced-since-realise, and exits 1.
#[test]
fn status_forced_write_renders_violation_row() {
    let sb = sandbox();
    let ws = sb.workspace("forced");
    // A hand-written reserved journal carrying one forced skip (the shape the wire
    // write door mints, U4.3): op=force + a `forced_rule=` token, both roots, an
    // anchor. status reads it frozen — it never re-evaluates.
    let journal = "# journal\n\
        - op=splice path=a.md actor=agent:a now=2026-07-23T09:00:00Z root_before=b3:0 root_after=b3:1 edits=1 ^r-000001\n\
        - op=force path=tasks/fix.md actor=agent:a now=2026-07-23T10:00:00Z root_before=b3:1 root_after=b3:2 edits=0 forced_rule=reviewer-not-owner ^r-000002\n";
    let mdir = ws.join("meridian");
    std::fs::create_dir_all(&mdir).expect("meridian dir");
    std::fs::write(mdir.join("journal.md"), journal).expect("journal");

    let out = sb.run(&ws, &["status"]);
    let so = stdout(&out);
    assert_eq!(
        code(&out),
        1,
        "a forced write is a finding (exit 1): {}",
        stderr(&out)
    );
    assert!(
        so.contains("1 forced-since-realise"),
        "forced counted (genesis boundary → all forced): {so}"
    );
    assert!(
        so.contains("violation: forced past `reviewer-not-owner`"),
        "the violation row names the bypassed rule: {so}"
    );
    assert!(
        so.contains("^r-000002"),
        "the row cites the permanent anchor: {so}"
    );
}

/// A realise receipt AFTER a forced write moves the boundary past it — the forced
/// count drops to 0 (the forced write predates the last realise apply). A second
/// forced write after the receipt counts again (over-report boundary).
#[test]
fn status_forced_since_realise_respects_the_receipts_boundary() {
    let sb = sandbox();
    let ws = sb.workspace("boundary");
    let journal = "# journal\n\
        - op=force path=old.md actor=agent:a now=2026-07-23T08:00:00Z root_before=b3:0 root_after=b3:1 edits=0 forced_rule=old-rule ^r-000001\n\
        - op=force path=new.md actor=agent:a now=2026-07-23T12:00:00Z root_before=b3:1 root_after=b3:2 edits=0 forced_rule=new-rule ^r-000002\n";
    let mdir = ws.join("meridian");
    std::fs::create_dir_all(&mdir).expect("meridian");
    std::fs::write(mdir.join("journal.md"), journal).expect("journal");
    // A realise apply at 10:00 — between the two forced writes.
    let receipts = "# realise receipts\n\
        - run {\"page\":\"p.md\",\"now\":\"2026-07-23T10:00:00Z\"} ^r-000001\n";
    let rdir = ws.join("receipts");
    std::fs::create_dir_all(&rdir).expect("receipts");
    std::fs::write(rdir.join("realise.md"), receipts).expect("receipts");

    let out = sb.run(&ws, &["status"]);
    let so = stdout(&out);
    assert!(
        so.contains("1 forced-since-realise"),
        "only the post-boundary forced write counts: {so}"
    );
    assert!(
        so.contains("(receipts boundary: since 2026-07-23T10:00:00Z)"),
        "the boundary names the realise-apply now: {so}"
    );
    assert!(
        so.contains("violation: forced past `new-rule`"),
        "the post-boundary rule shows: {so}"
    );
    assert!(
        !so.contains("old-rule"),
        "the pre-boundary forced write is excluded: {so}"
    );
}

/// The `--json` shape carries the three composed axes, the INDEX counts, the
/// boundary, and the violation rows.
#[test]
fn status_json_shape() {
    let sb = sandbox();
    let ws = sb.workspace("json");
    let fresh = arm_convention(&ws, "gamma", "block", "gamma v1\n", "gamma v1\n");
    write_index(&ws, &[fresh]);

    let out = sb.run(&ws, &["status", "--json"]);
    let so = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&so).expect("valid json");
    assert_eq!(v["index"]["armed"], 1);
    assert_eq!(v["index"]["drifted"], 0);
    assert_eq!(v["index"]["forced_since_realise"], 0);
    assert_eq!(v["index"]["boundary"]["kind"], "genesis");
    assert_eq!(v["composed"]["pin_color"], "green");
    assert_eq!(v["composed"]["convention_severity"], "block");
    assert!(
        v["composed"]["anchor"].is_string(),
        "anchor axis is rendered"
    );
    assert_eq!(v["findings"], false);
}

/// **The <1s wall-time gate (the merge budget, U3.6).** A 3k-doc corpus with a
/// handful of armed conventions: `status` reads ONE index file + O(armed)
/// `CHECK.md` re-hashes + the journal + the git refs — NEVER the 3k docs. So its
/// wall-time is independent of corpus size and stays well under the 1s hard
/// budget. The measured milliseconds are printed for the card record.
#[test]
fn status_wall_time_under_1s_on_3k_corpus() {
    let sb = sandbox();
    let ws = sb.workspace("corpus3k");

    // 3,000 ordinary docs — the corpus size status must NOT scale with.
    let docs = ws.join("docs");
    std::fs::create_dir_all(&docs).expect("docs dir");
    for i in 0..3_000u32 {
        std::fs::write(
            docs.join(format!("note-{i:04}.md")),
            format!("# Note {i}\n\nbody line for note {i}\n"),
        )
        .expect("write doc");
    }
    // A handful of armed conventions (the O(armed) work).
    let rows: Vec<String> = (0..5u32)
        .map(|i| {
            let slug = format!("conv-{i}");
            let body = format!("law {i} v1\n");
            arm_convention(&ws, &slug, "block", &body, &body)
        })
        .collect();
    write_index(&ws, &rows);

    // Warm the process/page cache with one throwaway run, then measure.
    let _ = sb.run(&ws, &["status"]);
    let start = Instant::now();
    let out = sb.run(&ws, &["status"]);
    let elapsed = start.elapsed();

    assert!(
        out.status.success() || code(&out) == 1,
        "status ran: {}",
        stderr(&out)
    );
    let so = stdout(&out);
    assert!(
        so.contains("5 armed · 0 drifted"),
        "the armed set read: {so}"
    );

    let ms = elapsed.as_millis();
    eprintln!("status wall-time on the 3k-doc corpus: {ms} ms (hard budget 1000 ms)");
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "status must be O(armed), <1s on the 3k corpus — measured {ms} ms"
    );
}

// ── S9: the meridian-lock axis, end to end over the real binary ─────────────

/// The canonical `meridian-lock` fence bytes for one pin — the exact form
/// `lock::render` emits (`crates/lock`), written by hand so this gate depends on
/// the CLI's own reader, not on the writer that produced the bytes.
fn lock_block(declared_ref: &str, fingerprint: &str) -> String {
    format!(
        "```meridian-lock\nversion: 1\npins:\n  - ref: \"{declared_ref}\"\n    fingerprint: \"{fingerprint}\"\n```"
    )
}

/// The LIVE fingerprint token of a page's document root — what a correct pin
/// holds. Computed through the engine's own mint over the same parse
/// `fs::build_corpus` runs, so the fixture cannot pin a token the reader would
/// not recompute.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root).0
}

/// The composed multi-axis line of a human `status` render (the U6.2 legend).
fn composed_line(human: &str) -> &str {
    human
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("pin "))
        .expect("the composed line")
}

/// The `composed.lock` object of a `status --json` run.
fn lock_json(sb: &Sandbox, ws: &Path) -> serde_json::Value {
    let out = sb.run(ws, &["status", "--json"]);
    assert!(
        out.status.success() || code(&out) == 1,
        "status --json ran: {}",
        stderr(&out)
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("status json");
    doc["composed"]["lock"].clone()
}

/// S9 GATE — a real `meridian-lock` pin is VISIBLE to `mrd status` on its own
/// axis, green when the pinned fingerprint still matches and red when the target
/// content moved. Before S9 `mrd status` did not read page pins at all: it
/// printed `pin green` identically whether the corpus held a correct pin, a
/// drifted one, or none.
#[test]
fn status_renders_the_meridian_lock_axis_green_then_red() {
    let sb = sandbox();
    let ws = sb.workspace("lockaxis");
    std::fs::create_dir_all(ws.join("sources")).expect("sources dir");

    let target_v1 = "# Target\n\nbody v1\n";
    std::fs::write(ws.join("sources/target.md"), target_v1).expect("write target");
    std::fs::write(
        ws.join("effect.md"),
        format!(
            "# Effect\n\ndraws from it\n\n{}\n",
            lock_block("sources/target.md", &live_fingerprint(target_v1))
        ),
    )
    .expect("write effect");

    let green = stdout(&sb.run(&ws, &["status"]));
    assert!(
        green.contains("lock green [1 pin]"),
        "a matching pin renders green on its own axis: {green}"
    );
    // The armed-set `pin` axis is untouched beside it (U6.2: never merged).
    assert!(green.contains("pin green · lock green"), "{green}");
    let j = lock_json(&sb, &ws);
    assert_eq!(j["pins"], serde_json::json!(1));
    assert_eq!(j["color"], serde_json::json!("green"));
    assert_eq!(j["reason"], serde_json::Value::Null);

    // Move the target's content — the pin now measures drift.
    std::fs::write(ws.join("sources/target.md"), "# Target\n\nbody v2 edited\n")
        .expect("rewrite target");
    let red = stdout(&sb.run(&ws, &["status"]));
    assert!(
        red.contains("lock red content-drifted [1 pin]"),
        "a drifted pin reddens: {red}"
    );
    let j = lock_json(&sb, &ws);
    assert_eq!(j["color"], serde_json::json!("red"));
    assert_eq!(j["reason"], serde_json::json!("content-drifted"));

    eprintln!("S9 composed (green): {}", composed_line(&green));
    eprintln!("S9 composed (red):   {}", composed_line(&red));
}

/// S9 GATE — a REFUSED lock block is visible as `grey lock-refused` with its
/// reason, not as a clean workspace. A corrupt lock reading as "no pins" is the
/// one answer a drift face must never give.
#[test]
fn status_renders_a_refused_lock_as_grey_not_silence() {
    let sb = sandbox();
    let ws = sb.workspace("lockrefused");
    std::fs::write(
        ws.join("effect.md"),
        "# Effect\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n",
    )
    .expect("write effect");

    let out = stdout(&sb.run(&ws, &["status"]));
    assert!(
        out.contains(
            "lock grey lock-refused (malformed at line 3: unrecognized line (canonical order: version, objects, pins)) [1 pin]"
        ),
        "a refused lock names its damage: {out}"
    );
    let j = lock_json(&sb, &ws);
    assert_eq!(j["color"], serde_json::json!("grey"));
    assert_eq!(j["reason"], serde_json::json!("lock-refused"));
    assert_eq!(j["pins"], serde_json::json!(1));
    eprintln!("S9 composed (lock-refused): {}", composed_line(&out));
}

/// S9 GATE — `status --json` ALWAYS emits the lock fields. A workspace with no
/// lock pins reports `0` and a null color, never an absent object a reader could
/// mistake for "not checked" (and never a green it did not verify).
#[test]
fn status_json_always_emits_the_lock_axis_even_with_no_pins() {
    let sb = sandbox();
    let ws = sb.workspace("nopins");
    std::fs::write(ws.join("note.md"), "# Note\n\nno lock here\n").expect("write note");

    let j = lock_json(&sb, &ws);
    assert_eq!(j["pins"], serde_json::json!(0));
    assert_eq!(j["color"], serde_json::Value::Null, "0 pins is not a color");
    assert_eq!(j["label"], serde_json::json!("none"));
    assert_eq!(j["unreadable"], serde_json::Value::Null);

    let human = stdout(&sb.run(&ws, &["status"]));
    assert!(human.contains("lock none"), "{human}");
    eprintln!("S9 composed (no pins): {}", composed_line(&human));
    eprintln!("S9 json     (no pins): {j}");
}
