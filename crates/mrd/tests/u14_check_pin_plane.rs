//! **U14 — `mrd check` must SEE THE PIN PLANE, or the fence is a false green by construction.**
//! Every gate here drives the REAL binary (`CARGO_BIN_EXE_mrd`) over its process boundary.
//! Amendment — the journal plane this file argues against is GONE Everything below narrates the
//! pin planes blind spot in terms of the journal plane that could not see it. That framing is
//! history: ZT ruled the engine keeps no memory (*"History is pin to git when we lock. Anything
//! between locks is not history."*), so the journal plane was deleted whole. The files SUBJECT
//! is unchanged and its refusals still gate — what changed is that the isolation it used to
//! establish by counting journal rows is now structural. Read the journal-plane prose below as
//! the argument for why this test exists, not as a description of what `check` reads today.
//! What is left after u14i and U32, stated so a green cannot be misread (S3-R58) u14i
//! downgraded the *claim* (a baseline it cannot date is `grey(cannot-assess)`, never green) and
//! U32 gave the governed splice its journal row. Together they make a green `mrd check` mean
//! *"baseline provable AND nothing the JOURNAL plane can see"* — **a true statement about the
//! journal plane that stays silent about the pin plane.** The silence is the debt, and it is
//! this units. The corpus, and why the OBVIOUS one proves nothing *Pin, then edit the pinned
//! bytes in place, then write* does **not** exhibit the debt, and measuring that was the first
//! thing this unit did.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use fs::WorkspaceRoot;
use receipt::anchor::ObjectAnchor;
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, create};

// ── harness ──────────────────────────────────────────────────────────────────

/// The binary under test. `MRD_BIN` names another artifact (the fixv convention), so the SAME
/// asserts can be pointed at a pre-change build to show them redden.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
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
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env_remove("MERIDIAN_WORKSPACE")
            .env_remove("MERIDIAN_CONFIG")
            .output()
            .expect("spawn mrd")
    }

    /// A git-backed, `mrd init`-marked workspace. Git is real because the pin
    /// plane asks git real questions about the pinned blob.
    fn git_workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git_init(&ws);
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", said(&init));
        ws
    }

    /// A workspace with no git repository behind it at all — the honest-degradation
    /// fixture.
    fn bare_workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", said(&init));
        ws
    }
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("LC_ALL", "C")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "fixture `git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_owned()
}

fn git_init(dir: &Path) {
    git_in(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
    git_in(dir, &["config", "user.name", "u14 fixture"]);
    git_in(dir, &["config", "user.email", "u14@fixture.invalid"]);
    git_in(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_all(dir: &Path, message: &str) {
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

/// Whether a git query SUCCEEDS — for the predicate forms (`cat-file -e`) whose
/// answer is the exit status and whose failure is not a fixture fault.
fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run git")
        .status
        .success()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// stdout+stderr together — the render rides stdout, the refusal message rides
/// stderr, and "is there a green anywhere" cares about what the operator SEES.
fn said(out: &Output) -> String {
    format!(
        "{}{}",
        stdout(out),
        String::from_utf8_lossy(&out.stderr).into_owned()
    )
}

fn write(ws: &Path, rel: &str, body: &str) {
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

fn root_of(ws: &Path) -> WorkspaceRoot {
    WorkspaceRoot(workspace::canonicalize(ws).expect("canonicalize"))
}

/// Birth `path` through the PRODUCTION guarded-create write path — a real governed write, so
/// the corpus under test is one the engine actually made. (It was the **baseline refresher**
/// when a journal rows `root_after` re-anchored the baseline; there is no baseline plane to
/// refresh now.)
fn produce(root: &WorkspaceRoot, path: &str, body: &str) {
    let args = CreateArgs {
        id: None,
        path: WirePath(path.to_string()),
        body: body.to_string(),
        actor: Some("agent:u14".to_string()),
        now: None,
        if_root: None,
        dry: false,
    };
    create(root, None, &args, &[])
        .unwrap_or_else(|e| panic!("production create {path} refused: {e:?}"));
}

const SOURCE_PINNED: &str = "# Source\n\n## Guideline\n\nthe pinned body\n";
const CLAIM: &str = "# Claim\n\nwe rely on the guideline.\n";

/// A `meridian-lock` block carrying ONE pin, in the canonical R4 bytes the engine itself mints
/// (quoted scalars, `version: 2`). R4 retired the top-level `objects:` table and moved the blob
/// hash onto the pin row, so the RETRIEVAL plane can no longer be put in front of `check` on
/// its own — a hash arrives only as some pins `hash`.
///
///
///
///
///
///
///
///
///
///
///
///
fn objects_lock(object: &str, fingerprint: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{}\"\n    path: []\n    fingerprint: \"{fingerprint}\"\n```\n",
        "a".repeat(40)
    )
}

/// The LIVE whole-page fingerprint of `raw` — what a CORRECT whole-page pin holds, minted
/// through the engines own hasher over the same parse `fs::build_corpus` runs, so a fixture
/// cannot pin a token the reader would not recompute.
///
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

/// The pinned corpus every gate below starts from: `source.md` + `claim.md`
/// committed, then one governed `mrd pin` through the shipped CLI.
fn pinned_corpus(sb: &Sandbox, name: &str, vibe: bool) -> PathBuf {
    let ws = sb.git_workspace(name);
    write(&ws, "source.md", SOURCE_PINNED);
    write(&ws, "claim.md", CLAIM);
    if !vibe {
        commit_all(&ws, "init");
    }
    let mut args = vec!["pin", "claim.md", "source.md#Source/Guideline"];
    if vibe {
        args.push("--vibe");
    }
    let pin = sb.run(&ws, &args);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    assert!(
        std::fs::read_to_string(ws.join("claim.md"))
            .expect("claim")
            .contains("meridian-lock"),
        "R40 — the governed pin actually landed its lock block"
    );
    ws
}

/// **The corpus that leaves the journal plane with NOTHING to report while a pin is red — the
/// clone/pull shape, and it is the ordinary one.** A lock is minted in one working copy against
/// the original bytes; another working copy receives that lock **and a source that has since
/// moved**.
///
///
///
///
///
///
///
///
///
fn pulled_corpus(sb: &Sandbox, name: &str) -> PathBuf {
    let minted = pinned_corpus(sb, &format!("{name}-origin"), false);
    let original = std::fs::read_to_string(minted.join("source.md")).expect("minted source");

    let ws = sb.git_workspace(name);
    std::fs::copy(minted.join("claim.md"), ws.join("claim.md")).expect("the lock arrives verbatim");
    write(
        &ws,
        "source.md",
        &original.replace("the pinned body", "OUT OF BAND"),
    );
    commit_all(&ws, "pulled");
    assert!(
        std::fs::read_to_string(ws.join("claim.md"))
            .expect("claim")
            .contains("meridian-lock"),
        "R40 — the pin arrived with the pull"
    );
    assert!(
        std::fs::read_to_string(ws.join("source.md"))
            .expect("source")
            .contains("OUT OF BAND"),
        "R40 — and the source it pins arrived CHANGED"
    );
    ws
}

/// Land ONE governed write so this corpus is decisive: anything `check` refuses from here on,
/// it refuses because of the pin plane and nothing else. **That isolation used to be
/// ESTABLISHED and is now STRUCTURAL.** This function counted journal rows (0 on a fresh clone,
/// 1 after the write) to prove the journal plane was spotless and could not be the cause of a
/// refusal.
///
///
///
///
///
///
///
///
///
fn establish_a_spotless_baseline(root: &WorkspaceRoot) {
    produce(root, "note.md", "# Note\n\ngoverned birth\n");
}

// ── the REFUSAL: a drifted pin on a SPOTLESS journal plane ───────────────────

/// **The debt S3-R58 names, measured on this tree.** The journal plane has nothing whatever to
/// report — one row, current baseline, no chain to break — and the pin in `claim.md` claims
/// content `source.md` no longer carries. `mrd check` must refuse.
///
///
///
///
///
///
///
///
///
///
///
///
///
#[test]
fn check_refuses_a_drifted_pin_with_no_write_history_plane_to_blame() {
    let sb = sandbox();
    let ws = pulled_corpus(&sb, "drifted-spotless-journal");
    let root = root_of(&ws);
    establish_a_spotless_baseline(&root);

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);

    // The CONTROL: the refusal below must come from the pin plane and nothing else. This used to
    // be established — the render said `chain: green` and `foreign_edit: none`, so the journal
    // plane demonstrably had no complaint. It is STRUCTURAL now: there is no write-history plane,
    // and the surface says so in its own words. A verb that assesses no write history cannot
    // refuse over one.
    //
    assert!(
        text.contains(check::WRITE_HISTORY_NOT_ASSESSED),
        "no write-history plane exists to be the thing refusing, and the surface \
         states it: {text}"
    );
    assert!(
        !text.contains(check::GREY_CANNOT_ASSESS),
        "and nothing here is unassessable, so the refusal below is a VERDICT: {text}"
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "THE ASSERT IS THE REFUSAL (R26): an out-of-band rewrite of pinned content \
         must make `mrd check` exit non-zero, or the fence fences nothing: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a finding rides exit 1 — the triad stays CLOSED, no fourth code (S3-R6): {text}"
    );
    assert!(
        text.contains("content-drifted"),
        "and it cites its why in the walk plane's own reason word: {text}"
    );

    // `--json` carries the same verdict on the other face (S3-R6).
    let js = sb.run(&ws, &["check", "--json"]);
    assert_eq!(js.status.code(), Some(1), "json refuses too: {}", said(&js));
    let value: serde_json::Value = serde_json::from_slice(&js.stdout).expect("json");
    assert_eq!(
        value["red"],
        serde_json::json!(true),
        "a drifted pin is a lie, not an absence of evidence: {value}"
    );
    assert!(
        serde_json::to_string(&value["pins"])
            .expect("pins")
            .contains("content-drifted"),
        "the reason word is distinct on the --json face too: {value}"
    );
}

// ── the ACCEPTANCE: a fully governed, fully anchored corpus ──────────────────

/// **S3-R8(c), on its own corpus.** Every write governed, the pinned content untouched, the
/// pinned blob reachable from a commit. `mrd check` must be GREEN and exit 0.
///
///
///
///
///
///
#[test]
fn check_accepts_a_fully_governed_anchored_corpus() {
    let sb = sandbox();
    let ws = pinned_corpus(&sb, "all-governed", false);
    // Commit the pin so the pinned blob is reachable from a ref: anchored, the one
    // durable state. R40 — assert the state, not the command.
    commit_all(&ws, "pin");

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert!(
        !text.contains(check::GREY_CANNOT_ASSESS),
        "nothing here is unassessable: every write was governed and git can be \
         asked: {text}"
    );
    assert!(
        text.contains(&format!("1 {}", ObjectAnchor::Anchored.word())),
        "the pinned blob is reachable from a commit — the durable state, and the \
         count is what says so (S3-R23(5): the reading carries its population): {text}"
    );
    assert!(
        !text.contains("ORPHANED"),
        "nothing here is held by nothing: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "THE ACCEPTANCE (S3-R8(c)): a fully governed, fully anchored corpus is \
         ACCEPTED, or the guard blocks everything: {text}"
    );
    assert!(
        text.contains("pins: green"),
        "and the pin plane says so in words, so a reader can tell an assessed \
         green from a silence: {text}"
    );
}

/// **The acceptance arm that nearly did not exist, and the measurement that forced it (S3-R8(c)
/// again, from the other side).** Wiring the anchoring refusal to `pending-anchor` — the state
/// criterion 5 names — reddened all three legs of `u35_journaled_doors.rs`, which drive a REAL
/// `git commit` through the RATIFIED fence over FULLY GOVERNED corpora.
///
///
///
///
///
///
///
///
///
#[test]
fn the_ordinary_pin_lifecycle_passes_through_two_non_durable_states_and_is_accepted_in_all_of_them()
{
    let sb = sandbox();
    let ws = pinned_corpus(&sb, "lifecycle", false);

    // (1) After `mrd pin`, before `git add`: a non-vibe pin hashes WITHOUT `-w`,
    //     so the recorded blob is in no object database at all.
    let oid = std::fs::read_to_string(ws.join("claim.md"))
        .expect("claim")
        .split('"')
        .find(|t| t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .expect("the lock records a blob oid")
        .to_string();
    assert!(
        !git_ok(&ws, &["cat-file", "-e", &oid]),
        "R40 — state: the object is absent, i.e. {}",
        ObjectAnchor::NeverAnchored.word()
    );
    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "and check ACCEPTS it: this is where every operator stands right after a \
         pin, and refusing here refuses the ordinary workflow: {}",
        said(&out)
    );

    // (2) Staged — exactly where a pre-commit hook runs. `git add` writes the blob, so it is
    // present and no ref reaches it: pending-anchor, by construction.
    git_in(&ws, &["add", "-A"]);
    assert!(
        git_ok(&ws, &["cat-file", "-e", &oid]),
        "R40 — state: `git add` wrote the blob into the object database"
    );
    assert!(
        git_in(&ws, &["rev-list", "--objects", "--all"])
            .lines()
            .all(|line| !line.starts_with(&oid)),
        "R40 — and no ref reaches it yet: that is {}, and it is the state EVERY \
         pre-commit hook sees",
        ObjectAnchor::PendingAnchor.word()
    );
    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert!(
        text.contains(&format!("1 {}", ObjectAnchor::PendingAnchor.word())),
        "the three-state reading SEES it — GAP A closed, the verb is no longer \
         blind: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "AND ACCEPTS it. A refusal here refuses every governed commit there has \
         ever been — measured against the ratified fence in u35_journaled_doors: {text}"
    );

    // (3) Committed: anchored, the one durable state.
    git_in(&ws, &["commit", "-q", "--no-verify", "-m", "pin"]);
    let out = sb.run(&ws, &["check"]);
    assert_eq!(out.status.code(), Some(0), "still accepted: {}", said(&out));
    assert!(
        said(&out).contains(&format!("1 {}", ObjectAnchor::Anchored.word())),
        "and now durable: {}",
        said(&out)
    );
}

// ── GAP A: the pending-anchor refusal, the fence's SECOND job ────────────────

/// **GAP A.** The ratified fence refuses a *pending-anchor* commit, and **no
/// journal row will ever carry that fact** — the anchoring state is git's, not the
/// receipt journal's.
///
/// The defect the fence must catch is the ORPHAN: a `--vibe` eager blob that no ref
/// reaches **and that the file no longer hashes to**, so no commit of that file
/// will ever anchor it. It is prunable at `gc.pruneExpire` and then the pin can be
/// verified against nothing.
///
/// # The corpus isolates the anchoring finding from the content one
/// The out-of-band edit lands in `## Notes`, **outside** the pinned `## Guideline`
/// section: the pin's fingerprint covers the section, so the CLAIM plane stays
/// green, while the `objects:` blob is whole-FILE and moves. One defect, one word —
/// a refusal that also cried `content-drifted` here would be a second wrong answer.
#[test]
fn check_refuses_an_orphaned_pinned_blob() {
    let sb = sandbox();
    let ws = sb.git_workspace("orphaned-blob");
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nthe pinned body\n\n## Notes\n\nnot pinned.\n",
    );
    write(&ws, "claim.md", CLAIM);
    let pin = sb.run(
        &ws,
        &["pin", "claim.md", "source.md#Source/Guideline", "--vibe"],
    );
    assert_eq!(pin.status.code(), Some(0), "pin --vibe: {}", said(&pin));

    let recorded = git_in(&ws, &["hash-object", "--", "source.md"]);
    assert_eq!(
        git_in(&ws, &["cat-file", "-t", &recorded]),
        "blob",
        "R40 — the eager --vibe write put the blob in the object database"
    );

    // The out-of-band edit, OUTSIDE the pinned section.
    write(
        &ws,
        "source.md",
        &std::fs::read_to_string(ws.join("source.md"))
            .expect("source")
            .replace("not pinned.", "not pinned, and edited out of band.\n"),
    );
    let live = git_in(&ws, &["hash-object", "--", "source.md"]);
    assert_ne!(
        live, recorded,
        "R40 — the state this gate turns on: the file no longer hashes to the \
         recorded blob, so no commit of it will ever anchor that blob"
    );
    assert!(
        git_in(&ws, &["rev-list", "--objects", "--all"])
            .lines()
            .all(|line| !line.starts_with(&recorded)),
        "and no ref reaches the recorded blob either — it is held by nothing"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert_ne!(
        out.status.code(),
        Some(0),
        "THE ASSERT IS THE REFUSAL: a pin whose evidence is held by nothing and \
         will be anchored by nothing must not pass the fence's verb: {text}"
    );
    assert_eq!(out.status.code(), Some(1), "on the finding leg: {text}");

    // **THE CONTROL, and it is not "the journal plane is silent" — it cannot be.** Orphaning the
    // blob means editing the file, which moves the tree root, which leaves the journal baseline
    // stale: the journal plane is grey here and there is no honest corpus where it is not. So the
    // discriminator is the VERDICT CLASS, which grey cannot reach: a `cannot-assess` report
    // carries `red: false`, and only a FINDING sets `red: true`. The exit alone would be satisfied
    // by the journals grey and would prove nothing about this unit.
    //
    let js = sb.run(&ws, &["check", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&js.stdout).expect("json");
    assert_eq!(
        value["red"],
        serde_json::json!(true),
        "the anchoring finding is what makes this RED — the journal plane's grey \
         could never produce this, so the refusal is attributable: {value}"
    );
    assert_eq!(
        value["pins"]["anchoring"]["orphaned"]
            .as_array()
            .expect("orphaned")
            .len(),
        1,
        "and it is attributable to exactly one orphaned blob: {value}"
    );
    assert!(
        text.contains(ObjectAnchor::PendingAnchor.word()) && text.contains("ORPHANED"),
        "citing its why in the ONE spelling of that state — taken from \
         `receipt::anchor`, never re-spelled here (S3-R49: the word already \
         existed, so it is REUSED): {text}"
    );
    assert!(
        !text.contains("content-drifted"),
        "and it does NOT accuse the content plane: the pinned SECTION is \
         untouched, so a drift word here would be a second wrong answer: {text}"
    );
    assert!(
        text.contains("pins: green"),
        "the claim plane says green, and it is right to: {text}"
    );
}

// ── the three planes over ONE corpus, in ONE run ─────────────────────────────

/// **The door form (S3-R60: this is a RUN, not a `const`).** *Three planes agreeing on the
/// VERDICTS over a corpus* is a runtime property of data — no `const` can assert it, and trying
/// would produce the weakened middle S3-R23(4) forbids.
///
///
///
///
///
#[test]
fn walk_status_and_check_agree_on_one_corpus_in_one_run() {
    let sb = sandbox();
    let ws = pinned_corpus(&sb, "three-planes", false);
    commit_all(&ws, "pin");

    // ── ARM 1: the governed corpus. All three planes see a green pin. ───────
    let walk = sb.run(&ws, &["walk", "claim.md"]);
    let status = sb.run(&ws, &["status"]);
    let check = sb.run(&ws, &["check"]);
    assert_eq!(
        walk.status.code(),
        Some(0),
        "walk is green on the governed corpus: {}",
        said(&walk)
    );
    assert!(
        !stdout(&walk).contains("red "),
        "walk sees no red pin: {}",
        stdout(&walk)
    );
    assert!(
        stdout(&status).contains("lock green"),
        "status's lock axis is green: {}",
        stdout(&status)
    );
    assert_eq!(
        check.status.code(),
        Some(0),
        "and check agrees — it is the third plane now, not a spectator: {}",
        said(&check)
    );

    // ── ARM 2: the pulled copy — the lock arrived with its pin, the source arrived changed, one
    // governed write leaves the journal plane SPOTLESS. All three planes must now read the same
    // red pin. ────────────────────
    let ws = pulled_corpus(&sb, "three-planes-pulled");
    establish_a_spotless_baseline(&root_of(&ws));

    let walk = sb.run(&ws, &["walk", "claim.md"]);
    let status = sb.run(&ws, &["status"]);
    let check = sb.run(&ws, &["check"]);

    assert!(
        stdout(&walk).contains("red content-drifted"),
        "PLANE 1 — walk, per-pin: {}",
        stdout(&walk)
    );
    assert_eq!(walk.status.code(), Some(1), "walk reddens");
    assert!(
        stdout(&status).contains("lock red content-drifted"),
        "PLANE 2 — status, worst-of rollup: {}",
        stdout(&status)
    );
    assert_eq!(
        status.status.code(),
        Some(0),
        "R12: `mrd status`'s exit triad does NOT change — the rollup is a reading, \
         not a gate: {}",
        said(&status)
    );
    let text = said(&check);
    assert!(
        text.contains("content-drifted"),
        "PLANE 3 — check, the fence's verb, on the SAME corpus in the SAME run: {text}"
    );
    assert_eq!(
        check.status.code(),
        Some(1),
        "and check REFUSES, which is the whole point of it being the third plane: {text}"
    );
}

// ── honest degradation (gate 5) ──────────────────────────────────────────────

/// **Honest degradation, typed.** A workspace with no git repository behind it cannot be asked
/// about anchoring at all. `check` must say so — never read the unanswerable question as a
/// clean bill. The refusal rides the same closed triad with the ruled reason word: this is a
/// `cannot-assess`, not a finding, and the word is the one that already exists.
///
///
#[test]
fn check_degrades_honestly_when_there_is_no_git_repository() {
    let sb = sandbox();
    let ws = sb.bare_workspace("no-git");
    write(&ws, "source.md", SOURCE_PINNED);
    write(&ws, "claim.md", CLAIM);
    let root = root_of(&ws);

    // R4 makes a pins `hash` mandatory, so `mrd pin` REFUSES outright where git cannot answer —
    // which is exactly here. To put a blob sha in front of a missing repository the lock is
    // therefore written by hand, with the CLAIM plane held GREEN (the live whole-page token) so
    // the only thing left for `check` to be unhappy about is the store it cannot ask.
    //
    write(
        &ws,
        "claim.md",
        &format!(
            "{CLAIM}\n{}",
            objects_lock("source", &live_fingerprint(SOURCE_PINNED))
        ),
    );
    produce(&root, "note.md", "# Note\n\ngoverned birth\n");
    assert!(
        !ws.join(".git").exists(),
        "R40 — the state this gate turns on: there is no repository here"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert_ne!(
        out.status.code(),
        Some(0),
        "an unanswerable anchoring question is never a clean bill: {text}"
    );
    assert!(
        text.contains(check::GREY_CANNOT_ASSESS),
        "and it names itself with the RULED reason word, taken from its one \
         source (S3-R6): {text}"
    );
    assert!(
        !text.contains(ObjectAnchor::Anchored.word()),
        "it may not borrow the word the answerable path earns: {text}"
    );
}

/// **THE DISCLOSURE PIN — a cross-root pin is SKIPPED AND STATED, and the fence still passes**
/// (ruling End-to-end, through the real binary. This tests subject was replaced by a ruling,
/// not repaired It was `check_cannot_ask_another_roots_object_store_and_says_so` and it
/// asserted the opposite: exit non-zero, `grey(cannot-assess)`. That refusal was correct on its
/// own terms and unchanged since v1 — but R4 made `hash` mandatory on every pin, so what used
/// to reach it (cross-root `objects:` rows, which nobody wrote) became EVERY cross-root pin,
/// and **the fence began refusing every commit in any repo holding one**.
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
#[test]
fn a_cross_root_pin_is_skipped_and_disclosed_and_does_not_refuse_the_fence() {
    let sb = sandbox();
    let ws = sb.git_workspace("rooted-key");
    write(&ws, "source.md", SOURCE_PINNED);
    write(
        &ws,
        "claim.md",
        &format!(
            "{CLAIM}\n{}",
            objects_lock("alpha:source", &live_fingerprint(SOURCE_PINNED))
        ),
    );
    commit_all(&ws, "init");
    let root = root_of(&ws);
    produce(&root, "note.md", "# Note\n\ngoverned birth\n");

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);

    // THE DISCLOSURE — the human face names the population it did not measure.
    assert!(
        text.contains("anchoring scope:") && text.contains("alpha"),
        "the sight line is STATED, naming the root outside it: {text}"
    );
    assert!(
        text.contains("NOT measured here"),
        "and it says plainly that those blobs were not measured: {text}"
    );

    // AND THE ANCHORING PLANE DID NOT GREY ON IT — the scoping is not a refusal.
    assert!(
        !text.contains(check::GREY_CANNOT_ASSESS),
        "a question outside this gate's jurisdiction is not `cannot-assess`: {text}"
    );

    // The `--json` face carries the same narrowing, machine-readable.
    let json_out = sb.run(&ws, &["check", "--json"]);
    let doc: serde_json::Value =
        serde_json::from_str(&stdout(&json_out)).expect("check --json parses");
    let scope = &doc["pins"]["anchoring_out_of_jurisdiction"];
    assert_eq!(scope["count"], 1, "the machine face counts it too: {doc}");
    assert!(
        scope["refs"][0]
            .as_str()
            .is_some_and(|r| r.contains("alpha") && r.contains("claim.md")),
        "count alone cannot be acted on — the refs name WHICH pins: {doc}"
    );
}
