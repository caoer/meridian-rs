//! End-to-end gates for `mrd status`, driving the real binary
//! (`CARGO_BIN_EXE_mrd`) over its process boundary against on-disk workspaces.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
mod common;

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

    /// A workspace declared a root by `mrd init`. Unanchored unless the caller
    /// plants a `.git`, so `status` resolves it as the cwd default (`ephemeral`)
    /// — which is a resolution, just not an anchored one.
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

/// A minimal loadable CHECK rule page, varied by `marker` so two versions hash
/// differently. `paths:` keeps the law off every page these fixtures write — the
/// measurement is the armed-set READ, never a gate.
fn rule_page(id: &str, marker: &str) -> String {
    format!(
        "---\ntags: [type/rule, rules/check]\nid: {id}\npaths:\n  - {id}/**\n---\n\n\
         # {id} {marker}\n\n```starlark\ndef check_change(change):\n    pass\n```\n"
    )
}

/// Arm rules into `meridian/armed-rules.md` and plant the once-armed marker.
///
/// Each entry is `(id, pinned bytes, on-disk bytes, mode)`. The artifact is MINTED
/// by `policy::armed::arm` through the landed resolver — never hand-typed, because
/// an artifact a fixture wrote by hand is an artifact the arming act never
/// approved, and `status` would then be reading a page production could not
/// produce. Arming pins `page_rev(pinned)`; writing DIFFERENT bytes to disk
/// afterwards is exactly what evidence drift is.
///
/// BOTH files, always: the artifact alone leaves a workspace the marker says is
/// unarmed, and the marker alone is a missing-artifact fault.
fn arm_rules(ws: &Path, rules: &[(&str, String, String, policy::armed::Mode)]) {
    let pages: Vec<(String, &String)> = rules
        .iter()
        .map(|(id, pinned, _, _)| (format!("rules/{id}.md"), pinned))
        .collect();
    for ((page, pinned), (_, _, on_disk, _)) in pages.iter().zip(rules) {
        let full = ws.join(page);
        std::fs::create_dir_all(full.parent().expect("rules dir")).expect("rules dir");
        // Pinned bytes first so the rev the ARM act reads is the attested one …
        std::fs::write(&full, pinned.as_str()).expect("rule page");
        let _ = on_disk;
    }
    let index = policy::RuleIndex::discover(pages.iter().map(|(page, bytes)| policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page,
        bytes,
    }));
    // The act loads every firing winner. Hand it the PINNED bytes — the ones on
    // disk right now and the ones the index resolved. The live bytes replace
    // them below, which is the only honest way to build a drifted row.
    let source: std::collections::BTreeMap<String, String> = pages
        .iter()
        .map(|(page, pinned)| (page.clone(), (*pinned).clone()))
        .collect();
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        rules
            .iter()
            .map(|(id, pinned, _, mode)| policy::armed::ArmRequest {
                id: policy::RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: policy::page_rev(pinned),
            }),
        &source,
        policy::CheckLimits::default(),
    )
    .expect("the fixture arms at each page's live rev");
    let artifact_path = ws.join(fs::domain::ARMED_RULES_PATH);
    std::fs::create_dir_all(artifact_path.parent().expect("meridian dir")).expect("meridian dir");
    std::fs::write(artifact_path, artifact.render()).expect("artifact");
    std::fs::write(ws.join(fs::domain::ATTESTED_MARKER_PATH), "").expect("once-armed marker");
    // … then the live bytes, which may differ. A row whose page moved under its
    // pin is DRIFTED, and that is the only way to build one honestly.
    for ((page, _), (_, _, on_disk, _)) in pages.iter().zip(rules) {
        std::fs::write(ws.join(page), on_disk.as_str()).expect("rule page on disk");
    }
}

/// Genesis: a bare workspace with NEITHER the armed-rules artifact NOR the
/// once-armed marker, and no git — 0 armed, genesis boundary, the anchor renders
/// unverified (never a bare `at-tip`), exit 0. The absent artifact is genesis and
/// NOT a fault precisely because the marker is absent too.
#[test]
fn status_genesis_is_clean_and_unverified() {
    let sb = sandbox();
    let ws = sb.workspace("genesis");
    let out = sb.run(&ws, &["status"]);
    let so = stdout(&out);
    assert_eq!(code(&out), 0, "clean genesis exits 0: {}", stderr(&out));
    assert!(
        so.contains("  rules: 0 armed · 0 drifted · forced-since-realise: not-tracked"),
        "genesis rules line, labelled `rules:` (ZT ruling 5 — the report \
         states the rules facts, not the artifact's name): {so}"
    );
    assert!(!so.contains("armed-rules:"), "the old label is gone: {so}");
    assert!(so.contains("pin green"), "clean pin: {so}");
    assert!(so.contains("armed off"), "no armed mode: {so}");
    // W-C1: fetch-less status never renders a bare `at-tip`.
    assert!(
        so.contains("anchor unverified") || so.contains("anchor as-known"),
        "the anchor is qualified, never bare: {so}"
    );
    assert!(!so.contains("anchor at-tip\n"), "no bare at-tip line: {so}");
}

/// `status` names how it resolved the workspace it judged, never the path
/// alone. Both cases are asserted in one test because the discriminating fact
/// is that the word changes with the situation: a git-anchored tree reads
/// `git-root`, a gitless `mrd init`-declared one reads `declared` — a
/// hardcoded word would pass one half and fail the other.
#[test]
fn status_names_the_tier_that_resolved_the_workspace() {
    let sb = sandbox();

    let bare = sb.workspace("unanchored");
    let out = sb.run(&bare, &["status"]);
    let so = stdout(&out);
    let canon_bare = std::fs::canonicalize(&bare).unwrap();
    assert!(
        so.contains(&format!(
            "status  {} (declared)",
            canon_bare.to_string_lossy()
        )),
        "the header names the root AND how it resolved: {so}"
    );

    let anchored = sb.workspace("anchored");
    std::fs::create_dir_all(anchored.join(".git")).expect("git anchor");
    let canon_anchored = std::fs::canonicalize(&anchored).unwrap();
    let out = sb.run(&anchored, &["status"]);
    let so = stdout(&out);
    assert!(
        so.contains(&format!(
            "status  {} (git-root)",
            canon_anchored.to_string_lossy()
        )),
        "the word tracks the situation: {so}"
    );

    // `--json` carries the same word, so a machine reader is not left to parse
    // the human line for it.
    let out = sb.run(&anchored, &["status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(v["source"], "git-root", "--json carries the provenance");
    assert_eq!(v["workspace"], canon_anchored.to_string_lossy().as_ref());
}

/// Armed + drifted: two armed rules, one fresh (block), one drifted (warn) —
/// armed=2, drifted=1, the pin axis reds, the mode rolls up to block, exit 1.
#[test]
fn status_armed_and_drifted_reds_and_exits_1() {
    let sb = sandbox();
    let ws = sb.workspace("armed");
    arm_rules(
        &ws,
        &[
            (
                "alpha",
                rule_page("alpha", "v1"),
                rule_page("alpha", "v1"),
                policy::armed::Mode::Block,
            ),
            (
                "beta",
                rule_page("beta", "v1"),
                rule_page("beta", "v2-DRIFTED"),
                policy::armed::Mode::Warn,
            ),
        ],
    );

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
        so.contains("armed block"),
        "the mode rolls up worst-of to block: {so}"
    );
}

/// The journal is inert, and this pins it: a hand-written page in the retired
/// journal's exact shape — `op=force` rows with `forced_rule=` tokens and
/// anchors, plus a realise receipt to bound them by — moves nothing on this
/// surface. The engine keeps no memory by design.
#[test]
fn a_journal_shaped_page_is_inert_and_the_disclosure_is_rendered() {
    let sb = sandbox();
    let ws = sb.workspace("inert-journal");

    // Everything the old surface fed on, present and maximal: two forced rows either side of a
    // realise apply, so a counter that came back would have to choose between 1 and 2 and would
    // trip this test whichever it chose.
    let journal = "# journal\n\
        - op=force path=old.md actor=agent:a now=2026-07-23T08:00:00Z root_before=b3:0 root_after=b3:1 edits=0 forced_rule=old-rule ^r-000001\n\
        - op=force path=new.md actor=agent:a now=2026-07-23T12:00:00Z root_before=b3:1 root_after=b3:2 edits=0 forced_rule=new-rule ^r-000002\n";
    let mdir = ws.join("meridian");
    std::fs::create_dir_all(&mdir).expect("meridian dir");
    std::fs::write(mdir.join("journal.md"), journal).expect("journal");
    let receipts = "# realise receipts\n\
        - run {\"page\":\"p.md\",\"now\":\"2026-07-23T10:00:00Z\"} ^r-000001\n";
    let rdir = ws.join("receipts");
    std::fs::create_dir_all(&rdir).expect("receipts");
    std::fs::write(rdir.join("realise.md"), receipts).expect("receipts");

    let out = sb.run(&ws, &["status"]);
    let so = stdout(&out);

    // Nothing observes a forced write, so a clean workspace is clean.
    assert_eq!(
        code(&out),
        0,
        "a journal-shaped page is not a finding: {so}{}",
        stderr(&out)
    );

    // The count, the boundary and the violation rows are all gone.
    assert!(
        !so.contains("forced-since-realise: 1") && !so.contains("forced-since-realise: 2"),
        "the count is not back: {so}"
    );
    assert!(
        !so.contains("violation: forced past"),
        "no violation row is rendered from a journal page: {so}"
    );
    assert!(
        !so.contains("old-rule") && !so.contains("new-rule"),
        "no `forced_rule=` token reaches the surface: {so}"
    );
    assert!(
        !so.contains("^r-000002"),
        "no journal anchor is cited: {so}"
    );
    assert!(
        !so.contains("receipts boundary"),
        "the realise ledger no longer bounds anything on this surface: {so}"
    );

    // And the disclosure is present — silence is not compliance.
    assert!(
        so.contains("forced-since-realise: not-tracked"),
        "the axis is disclosed, not dropped: {so}"
    );
    // The WHY rides `--json` and `--help`, never the human line (report-voice
    // law: the report states what is, teaching is on demand).
    assert!(
        !so.contains("the engine keeps no memory"),
        "the human line carries no mechanism footnote: {so}"
    );
}

/// The `--json` shape carries the three composed axes, the armed-rules counts,
/// the boundary, and the violation rows.
#[test]
fn status_json_shape() {
    let sb = sandbox();
    let ws = sb.workspace("json");
    arm_rules(
        &ws,
        &[(
            "gamma",
            rule_page("gamma", "v1"),
            rule_page("gamma", "v1"),
            policy::armed::Mode::Block,
        )],
    );

    let out = sb.run(&ws, &["status", "--json"]);
    let so = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&so).expect("valid json");
    assert_eq!(v["armed_rules"]["armed"], 1);
    assert_eq!(v["armed_rules"]["drifted"], 0);
    // The disclosure object, and the ABSENCE of what it replaced. A `0` would read as "checked,
    // none found" and a `boundary` would bound a count that no longer exists — both are claims
    // about a property nothing observes.
    let forced = &v["armed_rules"]["forced_since_realise"];
    assert_eq!(forced["tracked"], serde_json::json!(false));
    assert_eq!(forced["state"], serde_json::json!("not-tracked"));
    assert!(
        forced["why"].as_str().is_some_and(|w| w.contains("git")),
        "the reason names where the answer lives now: {forced}"
    );
    assert!(
        !forced.is_number(),
        "the key must not be a count again: {forced}"
    );
    assert!(
        v["armed_rules"].get("boundary").is_none(),
        "the receipts boundary is removed, not zeroed: {v}"
    );
    assert!(
        v.get("violations").is_none(),
        "an empty violations array would read as 'looked, found none': {v}"
    );
    assert_eq!(v["composed"]["pin_color"], "green");
    assert_eq!(v["composed"]["armed_mode"], "block");
    assert!(
        v["composed"]["anchor"].is_string(),
        "anchor axis is rendered"
    );
    assert_eq!(v["findings"], false);
}

// ── S9: the meridian-lock axis, end to end over the real binary ─────────────

/// The canonical `meridian-lock` fence bytes for one pin — the exact form `lock::render` emits
/// (`crates/lock`), written by hand so this gate depends on the CLI's own reader, not on the
/// writer that produced the bytes.
fn lock_block(declared_ref: &str, fingerprint: &str) -> String {
    lock_block_pinning(declared_ref, FIXTURE_BLOB, fingerprint)
}

/// The blob hash a fixture pin carries when the test is not measuring the
/// retrieval plane (R4 makes `hash` mandatory on every pin row).
const FIXTURE_BLOB: &str = "9ae3f1deadbeef";

/// One R4 pin, hand-written in the exact bytes `lock::render` emits, so this
/// gate depends on the CLI's own reader and not on the writer that produced
/// them. `declared_ref` is a `page[A/B]` convenience spelling, split into the
/// `object` and the `path` array here — R4 admits no joined string on the row.
fn lock_block_pinning(declared_ref: &str, blob: &str, fingerprint: &str) -> String {
    let (target, fragment) = match declared_ref.split_once('#') {
        Some((t, f)) => (t, f),
        None => (declared_ref, ""),
    };
    let object = target.strip_suffix(".md").unwrap_or(target);
    let path = if fragment.is_empty() {
        String::new()
    } else {
        fragment
            .split('/')
            .map(|seg| format!("\"{seg}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{blob}\"\n    path: [{path}]\n    fingerprint: \"{fingerprint}\"\n```"
    )
}

/// The live fingerprint token of a page's document root — what a correct pin
/// holds. Computed through the engine's own mint over the same parse
/// `fs::build_corpus` runs, so the fixture cannot pin a token the reader
/// would not recompute.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
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

/// S9 gate — a real `meridian-lock` pin is visible to `mrd status` on its own
/// axis: green when the pinned fingerprint still matches, red when the target
/// content moved.
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

/// S9 GATE — a REFUSED lock block is visible as `grey lock-refused` with its reason, not as a
/// clean workspace. A corrupt lock reading as "no pins" is the one answer a drift face must
/// never give.
#[test]
fn status_renders_a_refused_lock_as_grey_not_silence() {
    let sb = sandbox();
    let ws = sb.workspace("lockrefused");
    std::fs::write(
        ws.join("effect.md"),
        "# Effect\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n",
    )
    .expect("write effect");

    let out = stdout(&sb.run(&ws, &["status"]));
    assert!(
        out.contains(
            "lock grey lock-refused (malformed at line 3: unrecognized line (canonical order: version, pins)) [1 pin]"
        ),
        "a refused lock names its damage: {out}"
    );
    let j = lock_json(&sb, &ws);
    assert_eq!(j["color"], serde_json::json!("grey"));
    assert_eq!(j["reason"], serde_json::json!("lock-refused"));
    assert_eq!(j["pins"], serde_json::json!(1));
    eprintln!("S9 composed (lock-refused): {}", composed_line(&out));
}

// ── S11: the vibe-debt gauge, end to end over the real binary + real git ────

/// Run `git` in `ws` with a fixed identity, and fail loudly with git's own
/// stderr — a fixture that silently skipped its git setup would prove nothing.
fn git(ws: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(ws)
        .args([
            "-c",
            "user.name=S11 fixture",
            "-c",
            "user.email=s11@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The lock fence bytes for one pinned blob — the retrieval plane the gauge
/// measures, written by hand so this gate depends on the CLI's own reader.
/// The blob rides the pin row as its `hash`; `key` is the pin's `object`.
fn lock_block_with_object(key: &str, blob_sha: &str) -> String {
    lock_block_pinning(key, blob_sha, &format!("fp1.span2.b3.{}", "0".repeat(64)))
}

/// The `composed.vibe_debt` object of a `status --json` run.
fn vibe_json(sb: &Sandbox, ws: &Path) -> serde_json::Value {
    let out = sb.run(ws, &["status", "--json"]);
    assert!(
        out.status.success() || code(&out) == 1,
        "status --json ran: {}",
        stderr(&out)
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("status json");
    doc["composed"]["vibe_debt"].clone()
}

/// S11 gate 1 — a never-committed vibe blob increments the gauge in both
/// count and bytes. The fixture reproduces `--vibe`'s eager write exactly
/// (`git hash-object -w`): the blob is in the object database, reachable from
/// no ref, and the lock references it.
#[test]
fn status_vibe_debt_counts_a_never_committed_blob() {
    let sb = sandbox();
    let ws = sb.workspace("vibedebt");
    git(&ws, &["init"]);

    let vibe = "# Vibe\n\nnever committed\n";
    std::fs::write(ws.join("vibe.md"), vibe).expect("write vibe");
    // The eager write S7's `--vibe` performs: the blob exists before any commit.
    let oid = git(&ws, &["hash-object", "-w", "--", "vibe.md"]);
    std::fs::write(
        ws.join("effect.md"),
        format!(
            "# Effect\n\ndraws from it\n\n{}\n",
            lock_block_with_object("vibe.md", &oid)
        ),
    )
    .expect("write effect");

    let out = sb.run(&ws, &["status"]);
    let human = stdout(&out);
    let expected = format!("vibe-debt 1 blob ({} bytes)", vibe.len());
    assert!(
        human.contains(&expected),
        "an unreachable vibe blob is debt ({expected}): {human}"
    );
    // METER, NOT A GATE: debt alone never turns the exit triad.
    assert_eq!(code(&out), 0, "debt is not a finding: {}", stderr(&out));

    let j = vibe_json(&sb, &ws);
    assert_eq!(j["blobs"], serde_json::json!(1));
    assert_eq!(j["bytes"], serde_json::json!(vibe.len()));
    assert_eq!(j["unknown"], serde_json::Value::Null);
    eprintln!("S11 composed (1 blob owed): {}", composed_line(&human));
    eprintln!("S11 json     (1 blob owed): {j}");
}

/// S11 gate 2 — a committed blob contributes nothing. Same lock, same sha:
/// the only change is that a commit now reaches the object, which is exactly
/// what paying the debt means.
#[test]
fn status_vibe_debt_ignores_a_committed_blob() {
    let sb = sandbox();
    let ws = sb.workspace("vibepaid");
    git(&ws, &["init"]);

    let vibe = "# Vibe\n\ncommitted after all\n";
    std::fs::write(ws.join("vibe.md"), vibe).expect("write vibe");
    let oid = git(&ws, &["hash-object", "-w", "--", "vibe.md"]);
    std::fs::write(
        ws.join("effect.md"),
        format!(
            "# Effect\n\ndraws from it\n\n{}\n",
            lock_block_with_object("vibe.md", &oid)
        ),
    )
    .expect("write effect");

    // Before the commit: owed.
    let owed = stdout(&sb.run(&ws, &["status"]));
    assert!(
        owed.contains(&format!("vibe-debt 1 blob ({} bytes)", vibe.len())),
        "the unpaid reading, for contrast: {owed}"
    );

    git(&ws, &["add", "vibe.md"]);
    git(&ws, &["commit", "-m", "anchor the vibe blob"]);
    // The SAME oid — committing anchors the blob, it does not re-hash it.
    assert_eq!(
        git(&ws, &["rev-parse", "HEAD:vibe.md"]),
        oid,
        "the committed blob is the pinned one"
    );

    let paid = stdout(&sb.run(&ws, &["status"]));
    assert!(
        paid.contains("vibe-debt 0 blobs (0 bytes)"),
        "an anchored blob is not debt: {paid}"
    );
    let j = vibe_json(&sb, &ws);
    assert_eq!(j["blobs"], serde_json::json!(0));
    assert_eq!(j["bytes"], serde_json::json!(0));
    eprintln!("S11 composed (paid): {}", composed_line(&paid));
}

/// S11 GATE 3 — ZERO RENDERS. A corpus that owes nothing still shows the gauge on the human
/// line and still emits `0` in `--json`: a gauge that hides at zero is not a gauge, and an
/// absent field reads as "not measured".
#[test]
fn status_vibe_debt_zero_renders_and_json_emits_it() {
    let sb = sandbox();
    let ws = sb.workspace("vibezero");
    std::fs::write(ws.join("note.md"), "# Note\n\nno lock here\n").expect("write note");

    let human = stdout(&sb.run(&ws, &["status"]));
    assert!(
        human.contains("vibe-debt 0 blobs (0 bytes)"),
        "zero holds its segment: {human}"
    );
    let j = vibe_json(&sb, &ws);
    assert_eq!(j["blobs"], serde_json::json!(0));
    assert_eq!(j["bytes"], serde_json::json!(0));
    assert_eq!(j["label"], serde_json::json!("0 blobs (0 bytes)"));
    assert_eq!(
        j["unknown"],
        serde_json::Value::Null,
        "nothing referenced is a MEASURED zero, not an unknown"
    );
    eprintln!("S11 composed (zero): {}", composed_line(&human));
    eprintln!("S11 json     (zero): {j}");
}

/// S11 — the gauge degrades honestly. A lock referencing a blob in a workspace that is not a
/// git repository reports `unknown` with git's own wording, never a `0` a reader would take for
/// clean.
#[test]
fn status_vibe_debt_outside_a_repository_is_unknown_not_zero() {
    let sb = sandbox();
    let ws = sb.workspace("vibenorepo");
    std::fs::write(
        ws.join("effect.md"),
        format!(
            "# Effect\n\n{}\n",
            lock_block_with_object("vibe.md", &"a".repeat(40))
        ),
    )
    .expect("write effect");

    let human = stdout(&sb.run(&ws, &["status"]));
    assert!(
        human.contains("vibe-debt unknown (not a git repository"),
        "unmeasurable says so: {human}"
    );
    let j = vibe_json(&sb, &ws);
    assert_eq!(j["blobs"], serde_json::json!(0));
    assert!(
        j["unknown"]
            .as_str()
            .is_some_and(|d| d.contains("not a git repository")),
        "the reason is carried: {j}"
    );
    eprintln!("S11 composed (unknown): {}", composed_line(&human));
}

/// S9 GATE — `status --json` ALWAYS emits the lock fields. A workspace with no lock pins
/// reports `0` and a null color, never an absent object a reader could mistake for "not
/// checked" (and never a green it did not verify).
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

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
