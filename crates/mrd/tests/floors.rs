//! U4.4 floor rules — the full enumerated suite, gated across the two surviving
//! `mrd test` tiers, plus the refusals whose TEXT no tier can assert and the
//! genesis reading the retired scenario tier used to own.
//!
//! The six floor rules live under `tests/floors/rules/<id>.md` — one PAGE each,
//! tag-registered and identified by frontmatter `id:`. This file:
//!
//! - **corpus tier** — drives `mrd test --corpus` over the six committed specs
//!   (`tests/floors/corpus/specs/*.md`): fire-where-expected + zero dead rules,
//!   exit 0 each.
//! - **genesis** — the reader-side semantics of a NEVER-ARMED workspace: absent
//!   artifact + absent marker is unarmed, the door is a bit-for-bit no-op, and
//!   the write is still journaled and still grey.
//! - **history tier** — builds a fresh temp git workspace, seeds the
//!   `reviewer-not-owner` floor PAGE + a journal, and drives `mrd test --history`:
//!   the owner-self-close history row is a would-refuse item, declared in a
//!   GOLDEN list.
//! - **message-naming** — loads the `claim-cas` and `verdict-reviewer-bind`
//!   floors through the SAME `policy` registration + loader the door uses and
//!   asserts the refusal NAMES the winner (gate 2) and the bound reviewer
//!   (gate 6).
//!
//! # What retired here, and where it went (ruling D3)
//! The scenario tier is gone: its fixture space was `conventions/<slug>/scenarios/`,
//! addressed by a folder name that is no longer an identity. Its two suite-level
//! scenarios were carried rather than dropped — `bounce-approve-lands` into the
//! `close-verdict` corpus spec plus a byte-exact unit test on the production
//! writer, and `first-arm-genesis-grey` REWRITTEN here against the artifact +
//! marker pair, which is what its subject (`conventions/INDEX.md`'s birth)
//! became. Its 14 per-floor scenarios and 6 base files are redundant against the
//! corpus specs and the governed tree they run over.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use wire_serve::write::{CreateArgs, create};

// ── shared helpers ────────────────────────────────────────────────────────────

/// The `mrd` binary under test.
fn mrd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
}

/// `tests/floors/…` under the crate manifest dir.
fn floors(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/floors")
        .join(sub)
}

/// Run `mrd <args…>`, returning `(exit_code, stdout)`.
fn run(args: &[&str]) -> (i32, String) {
    let out: Output = mrd().args(args).output().expect("spawn mrd");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

// ── tier 2: the corpus expected-fire manifest ─────────────────────────────────

/// Every floor rule's corpus spec is fire-where-expected with zero dead rules —
/// exit 0. This is the `--corpus` tier recording for all six floors.
#[test]
fn corpus_specs_all_fire_where_expected() {
    for id in [
        "reviewer-not-owner",
        "claim-cas",
        "close-verdict",
        "decoy-close",
        "verdict-reviewer-bind",
        "meta-convention",
    ] {
        let spec = floors(&format!("corpus/specs/{id}.md"));
        let (code, stdout) = run(&["test", "--corpus", spec.to_str().unwrap(), "--json"]);
        let report: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{id}: corpus JSON did not parse ({e}); stdout={stdout}"));
        assert_eq!(code, 0, "{id}: corpus spec must exit 0 (report={report})");
        assert_eq!(report["summary"]["mismatches"], 0, "{id}: no fire mismatch");
        assert_eq!(report["summary"]["dead_rules"], 0, "{id}: no dead rule");
        assert_eq!(report["summary"]["errors"], 0, "{id}: no eval error");
        assert_eq!(
            report["rule"], id,
            "{id}: the report is keyed on the page's `id:`, not on a filename"
        );
    }
}

/// The `close-verdict` spec carries the BOUNCE cases ported from the retired
/// scenario tier: a re-decision written through the same `put at:upsert` is
/// admitted by the floor rather than refused as a duplicate.
#[test]
fn the_bounce_re_decision_is_admitted_by_the_close_verdict_floor() {
    let spec = floors("corpus/specs/close-verdict.md");
    let (code, stdout) = run(&["test", "--corpus", spec.to_str().unwrap(), "--json"]);
    let report: Value = serde_json::from_str(&stdout).expect("corpus JSON parses");
    assert_eq!(code, 0, "the bounce cases must pass (report={report})");
    let case = |name: &str| -> Value {
        report["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("case `{name}` not in the report"))
            .clone()
    };
    for name in ["bounce-reject", "bounce-approve-lands"] {
        let row = case(name);
        assert_eq!(row["outcome"], "pass", "{name} lands: {row}");
        assert_eq!(row["in_scope"], true, "{name} really ran the floor: {row}");
        assert_eq!(row["matched"], true, "{name}: {row}");
    }
}

// ── genesis: the never-armed reading (rewritten from first-arm-genesis-grey) ───

/// The bytes a genesis write lands.
const GENESIS_PAGE: &str = "# Genesis\n\nthe first write on a never-armed workspace.\n";

/// Gate 7, REWRITTEN — the genesis epoch, read from the artifact + marker pair.
///
/// The retired scenario's subject was the birth of `conventions/INDEX.md`, a file
/// this cutover deletes. The BEHAVIOUR it guarded is unchanged, so it is
/// re-expressed against what replaced that file: `meridian/armed-rules.md` plus
/// the `meridian/attested` marker. On a workspace where BOTH are absent —
///
/// - the resolved law is **never-armed**, not an empty armed set and not a fault;
/// - the door is therefore a bit-for-bit no-op, so an ordinary write LANDS;
/// - it is still JOURNALED (ungated is not unrecorded);
/// - and it carries NO enforcement verdict — GREY is the ABSENCE of a green
///   verdict, never a token.
///
/// Scope, as ruled: this is the READER side only. The ACT of first arming is the
/// writer's test and is owed at the ARM disk edge; nothing here writes either
/// half of the pair.
#[test]
fn the_genesis_epoch_is_unarmed_ungated_journaled_and_grey() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    // The subject: BOTH halves absent. Asserted rather than assumed, because the
    // whole reading below is a claim about this exact pair.
    assert!(
        !dir.path().join(fs::domain::ARMED_RULES_PATH).exists(),
        "the genesis workspace has no armed-rules artifact"
    );
    assert!(
        !dir.path().join(fs::domain::ATTESTED_MARKER_PATH).exists(),
        "and has never been armed"
    );

    // READER side — the once-armed pivot reads the MARKER, and it says never.
    let law = wire_serve::armed_disk::resolve_at(&root, "tasks/genesis.md");
    assert!(
        law.never_armed(),
        "absent marker ⇒ never armed, so the gate is the bit-for-bit no-op"
    );
    assert!(law.rules().is_empty(), "nothing is armed at genesis");
    assert!(
        law.faults().is_empty(),
        "an absent artifact on a NEVER-armed workspace is genesis, not a fault: {:?}",
        law.faults()
    );

    // DOOR side — the write lands, ungated.
    let args = CreateArgs {
        id: None,
        path: wire::Path("tasks/genesis.md".to_owned()),
        body: GENESIS_PAGE.to_owned(),
        actor: Some("worker-a".to_owned()),
        now: None,
        if_root: None,
        dry: false,
    };
    let out = create(&root, 0, &args, &[]).expect("the genesis write lands ungated");

    // GREY on the enforcement axis: no green verdict, and no token standing in
    // for one. Grey is what the ABSENCE renders as.
    assert!(
        out.verdicts.is_empty(),
        "a never-armed write carries no enforcement verdict: {:?}",
        out.verdicts
    );

    // PRESENT: ungated is not unrecorded.
    let journal = std::fs::read_to_string(dir.path().join(fs::domain::RESERVED_JOURNAL_PATH))
        .unwrap_or_default();
    assert!(
        journal.contains("op=create") && journal.contains("tasks/genesis.md"),
        "the genesis write is journaled: {journal}"
    );

    // And the bytes really landed.
    let landed =
        std::fs::read_to_string(dir.path().join("tasks/genesis.md")).expect("bytes landed");
    assert!(landed.contains("Genesis"), "the genesis bytes are on disk");
}

/// The control that makes the genesis reading load-bearing: the pivot is the
/// MARKER, not the artifact. Plant the marker alone and the SAME absent artifact
/// stops being genesis and becomes a fault — otherwise "no artifact ⇒ unarmed"
/// would be a silent disarm anyone could perform with `rm`.
#[test]
fn the_marker_alone_turns_an_absent_artifact_into_a_fault() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    std::fs::create_dir_all(dir.path().join("meridian")).unwrap();
    std::fs::write(dir.path().join(fs::domain::ATTESTED_MARKER_PATH), "").unwrap();

    let law = wire_serve::armed_disk::resolve_at(&root, "tasks/genesis.md");
    assert!(
        !law.never_armed(),
        "the marker says this workspace HAS been armed"
    );
    assert!(
        !law.faults().is_empty(),
        "a missing artifact on an armed workspace is a fault, never a disarm"
    );
    assert!(
        law.refusing().next().is_some(),
        "and a whole-artifact fault refuses"
    );
}

// ── message-naming: gates 2 and 6 (the corpus tier records the rule, not text) ─

/// Load a floor rule PAGE through the SAME registration + loader the door uses.
fn floor_rule(id: &str) -> policy::Rule {
    let path = floors(&format!("rules/{id}.md"));
    let bytes = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{id} must be readable at {}: {e}", path.display()));
    let registration = policy::register_page(policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: &format!("rules/{id}.md"),
        bytes: &bytes,
    })
    .unwrap_or_else(|e| panic!("{id} must register: {e}"))
    .unwrap_or_else(|| panic!("{id} must carry a rules/* tag"));
    assert_eq!(
        registration.id().as_str(),
        id,
        "the page's frontmatter id is its identity"
    );
    policy::load_rule(&registration, &bytes, policy::CheckLimits::default())
        .unwrap_or_else(|e| panic!("{id} must load: {e}"))
}

/// Build a path-stamped `model::Document` (the fixture doc-building path).
fn doc(path: &str, md: &str) -> model::Document {
    let mut d = model::build(md.to_string(), syntax::parse(md));
    if let model::NodeKind::Document { path: p, .. } = &mut d.root.kind {
        *p = path.to_string();
    }
    d
}

/// Load a floor rule by id and run its `check_change` over the change derived
/// from `before`→`after` as `actor` (splice, no evidence edges).
fn refusals(
    id: &str,
    path: &str,
    before_md: &str,
    after_md: &str,
    actor: &str,
) -> Vec<policy::Refusal> {
    let rule = floor_rule(id);
    let before = doc(path, before_md);
    let after = doc(path, after_md);
    let change = policy::derive_change(
        &before,
        &after,
        &[],
        policy::Invocation {
            op: policy::ChangeOp::Splice,
            actor: Some(actor),
            force: false,
        },
        &[],
        &|_: &str| None,
    );
    rule.check_change(&change)
        .expect("check_change evaluates")
        .refusals
}

/// Gate 2 — a contested claim's refusal NAMES the winner (the current holder),
/// and cites the surviving corpus CASE that shows the legal path.
#[test]
fn claim_cas_refusal_names_the_winner() {
    let before = "---\nstatus: open\nowner: worker-a\n---\n# T\n\nx\n";
    let after = "---\nstatus: open\nowner: worker-b\n---\n# T\n\nx\n";
    let r = refusals("claim-cas", "tasks/t.md", before, after, "worker-b");
    assert_eq!(r.len(), 1, "a contested claim fires exactly once");
    assert!(
        r[0].message.contains("worker-a"),
        "the loser's refusal must name the winner (worker-a): {}",
        r[0].message
    );
    assert_eq!(r[0].passing_scenario, "uncontested-claim");
}

/// Gate 6 — a Verdict naming a reviewer other than the closing actor is refused,
/// the refusal NAMES the unbound reviewer, and it cites the surviving case.
#[test]
fn verdict_bind_refusal_names_the_reviewer() {
    let before = "---\nreviewer: carol\noutcome: pending\n---\n# V\n\nx\n";
    let after = "---\nreviewer: carol\noutcome: approve\n---\n# V\n\nx\n";
    let r = refusals(
        "verdict-reviewer-bind",
        "verdicts/v.md",
        before,
        after,
        "dave",
    );
    assert_eq!(r.len(), 1, "an unbound Verdict fires exactly once");
    assert!(
        r[0].message.contains("carol") && r[0].message.contains("dave"),
        "the refusal must name the named reviewer (carol) and the closing actor (dave): {}",
        r[0].message
    );
    assert_eq!(r[0].passing_scenario, "bound-verdict");
}

/// Ruling D, asserted rather than reviewed: **no floor refusal cites a page that
/// is absent from the tree.** Every `passing =` used to be a
/// `scenarios/<name>.md` path into the tier that retired, so after the deletion
/// each of those citations named a file nobody could open — a refusal whose legal
/// path teaches nothing.
///
/// The check is deliberately shaped as "no citation is path-like" rather than
/// "every cited file exists": the ruling did not move the pages, it changed WHAT
/// a citation is. A citation is a corpus CASE id now, and the case-existence half
/// is enforced where cases live — a citation naming no case is reported DEAD by
/// `corpus_specs_all_fire_where_expected`, and a case expecting no citation is a
/// fire mismatch in the same run.
#[test]
fn no_floor_refusal_cites_a_page_that_is_not_in_the_tree() {
    let ids = [
        "reviewer-not-owner",
        "claim-cas",
        "close-verdict",
        "decoy-close",
        "verdict-reviewer-bind",
        "meta-convention",
    ];
    let mut seen = 0usize;
    for id in ids {
        let page = std::fs::read_to_string(floors(&format!("rules/{id}.md"))).expect("floor page");
        for line in page.lines() {
            let Some(rest) = line.trim().strip_prefix("passing = \"") else {
                continue;
            };
            let citation = rest.trim_end_matches("\",").trim_end_matches('"');
            seen += 1;
            // Path-shaped means a separator or a markdown extension in ANY case —
            // a citation is a case-sensitive id, but a `.MD` that slipped past
            // would name a page just as surely as a `.md`.
            assert!(
                !citation.contains('/') && !citation.to_ascii_lowercase().ends_with(".md"),
                "{id} cites `{citation}` — a path-shaped citation names a page, and the pages it \
                 used to name are gone; cite the surviving corpus case id instead"
            );
        }
    }
    assert_eq!(
        seen, 8,
        "all eight floor citations were inspected (six rules, meta-convention carries three)"
    );
}

// ── tier 3: the history run over a real temp git workspace ─────────────────────

/// The workspace-relative page the history tier calibrates.
const HISTORY_RULE_PAGE: &str = "rules/reviewer-not-owner.md";

/// Run a git command in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Write `body` to `dir/rel`, creating parents.
fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Commit the whole working tree.
fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// One journal row (matches `receipt::journal::parse_rows`: `- op= path= actor=
/// root_before= root_after= … ^r-NNNNNN`).
fn row(op: &str, path: &str, actor: &str, seq: u64, rb: &str, ra: &str, extra: &str) -> String {
    format!(
        "- op={op} path={path} actor={actor} root_before={rb} root_after={ra} {extra} ^r-{seq:06}"
    )
}

const FIX_OPEN: &str =
    "---\ntype: task\nstatus: open\nowner: worker-a\n---\n\n# Fix parser\n\nbody\n";
const FIX_CLOSED: &str =
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Fix parser\n\nbody\n";
const FIX_NOTE: &str =
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Fix parser\n\nbody\n\n- reviewed\n";

/// Seed a temp git workspace with the reviewer-not-owner FLOOR page + a journal
/// whose C1 row is an owner-self-close (would-refuse), C2 a reviewer edit (passes).
fn seeded_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    let ws = dir.path();

    git(ws, &["init", "-q", "-b", "main"]);
    git(ws, &["config", "user.email", "test@meridian.local"]);
    git(ws, &["config", "user.name", "mrd-test"]);

    // The floor rule as a PAGE in the workspace (the real U4.4 law).
    let page = std::fs::read_to_string(floors("rules/reviewer-not-owner.md")).unwrap();
    write(ws, HISTORY_RULE_PAGE, &page);

    // C0 — bob creates fix-parser (owner worker-a) + r-000001 (create).
    write(ws, "tasks/fix-parser.md", FIX_OPEN);
    let j1 = format!(
        "# Receipt journal\n\n{}\n",
        row(
            "create",
            "tasks/fix-parser.md",
            "worker-b",
            1,
            "b3:0",
            "b3:1",
            "before=absent after=aaaa edits=0"
        )
    );
    write(ws, "meridian/journal.md", &j1);
    commit(ws, "C0 create fix-parser");

    // C1 — worker-a (the OWNER) closes her own task + r-000002 (splice) → would-refuse.
    write(ws, "tasks/fix-parser.md", FIX_CLOSED);
    let j2 = format!(
        "{j1}{}\n",
        row(
            "splice",
            "tasks/fix-parser.md",
            "worker-a",
            2,
            "b3:1",
            "b3:2",
            "edits=1 status:put:aaaa->bbbb"
        )
    );
    write(ws, "meridian/journal.md", &j2);
    commit(ws, "C1 owner self-close");

    // C2 — reviewer-b edits + r-000003 (splice) → passes (reviewer != owner).
    write(ws, "tasks/fix-parser.md", FIX_NOTE);
    let j3 = format!(
        "{j2}{}\n",
        row(
            "splice",
            "tasks/fix-parser.md",
            "reviewer-b",
            3,
            "b3:2",
            "b3:3",
            "edits=1 Fix parser:put:bbbb->cccc"
        )
    );
    write(ws, "meridian/journal.md", &j3);
    commit(ws, "C2 reviewer edit");

    dir
}

/// Run `mrd test --history <ws> --rule rules/reviewer-not-owner.md --json`.
fn run_history(ws: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec![
        "test",
        "--history",
        ws.to_str().unwrap(),
        "--rule",
        HISTORY_RULE_PAGE,
        "--json",
    ];
    args.extend_from_slice(extra);
    let out = mrd().args(&args).output().expect("spawn mrd");
    let code = out.status.code().unwrap_or(-1);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "history JSON did not parse ({e}); stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    });
    (code, report)
}

/// The history tier reconstructs the journal, fires the owner-self-close row as
/// an UNDECLARED would-refuse (exit 1), then a GOLDEN declaration flips it to a
/// declared exception (exit 0). This is the `--history` tier recording.
#[test]
fn history_owner_self_close_is_a_would_refuse_then_declared() {
    let dir = seeded_workspace();
    let ws = dir.path();

    // Undeclared: the owner self-close row is a would-refuse finding (exit 1).
    let (code, report) = run_history(ws, &[]);
    assert_eq!(
        code, 1,
        "an undeclared would-refuse is exit 1 (report={report})"
    );
    assert_eq!(
        report["rule"], "reviewer-not-owner",
        "the report is keyed on the page's id"
    );
    assert_eq!(report["rule_page"], HISTORY_RULE_PAGE);
    assert_eq!(
        report["summary"]["undeclared"], 1,
        "one undeclared would-refuse"
    );
    assert_eq!(report["fidelity"]["full_bytes"], 2, "C1+C2 are full-bytes");
    assert_eq!(
        report["fidelity"]["structural"], 1,
        "C0 create is structural"
    );

    // Declare it in the GOLDEN list — the rule page's `.golden.md` SIBLING, which
    // is where a page-shaped law keeps its exceptions now that it has no folder.
    let golden = "---\nrule: rules/reviewer-not-owner.md\n---\n\n# Golden list\n\n\
        - item=r-000002 reason=\"legacy owner self-close predates the reviewer-not-owner floor\"\n";
    write(ws, "rules/reviewer-not-owner.golden.md", golden);
    let (code, report) = run_history(ws, &[]);
    assert_eq!(
        code, 0,
        "a declared would-refuse is exit 0 (report={report})"
    );
    assert_eq!(report["summary"]["undeclared"], 0, "nothing undeclared");
    assert_eq!(report["summary"]["declared"], 1, "one declared exception");
}
