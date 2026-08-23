//! U14 — `mrd check` must see the pin plane, or the fence is a false green by
//! construction. Every gate drives the real binary over its process boundary.
//!
//! The obvious corpus — pin, edit the pinned bytes in place, write — does not
//! exhibit the debt; the clone/pull shape ([`pulled_corpus`]) does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use fs::WorkspaceRoot;
use receipt::anchor::ObjectAnchor;
use std::collections::BTreeMap;
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, create};
mod common;

// ── harness ──────────────────────────────────────────────────────────────────

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
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(cwd)
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

/// Birth `path` through the production guarded-create write path — a real
/// governed write, so the corpus under test is one the engine actually made.
fn produce(root: &WorkspaceRoot, path: &str, body: &str) {
    let args = CreateArgs {
        id: None,
        path: WirePath(path.to_string()),
        body: body.to_string(),
        actor: Some("agent:u14".to_string()),
        now: None,
        if_root: None,
        dry: false,
        fields: BTreeMap::default(),
        props: BTreeMap::default(),
    };
    create(root, None, &args, &[])
        .unwrap_or_else(|e| panic!("production create {path} refused: {e:?}"));
}

const SOURCE_PINNED: &str = "# Source\n\n## Guideline\n\nthe pinned body\n";
const CLAIM: &str = "# Claim\n\nwe rely on the guideline.\n";

/// A `meridian-lock` block carrying one pin, in the canonical R4 bytes the
/// engine itself mints. The blob hash rides the pin row.
fn objects_lock(object: &str, fingerprint: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{}\"\n    path: []\n    fingerprint: \"{fingerprint}\"\n```\n",
        "a".repeat(40)
    )
}

/// The live whole-page fingerprint of `raw` — what a correct whole-page pin
/// holds, minted through the engine's own hasher over the same parse
/// `fs::build_corpus` runs.
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

/// The clone/pull shape, and it is the ordinary one: a lock is minted in one
/// working copy against the original bytes; another working copy receives
/// that lock and a source that has since moved.
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

/// Land one governed write so this corpus is decisive: anything `check`
/// refuses from here on, it refuses because of the pin plane and nothing
/// else.
fn establish_a_spotless_baseline(root: &WorkspaceRoot) {
    produce(root, "note.md", "# Note\n\ngoverned birth\n");
}

// ── the REFUSAL: a drifted pin on a SPOTLESS journal plane ───────────────────

/// The pin in `claim.md` claims content `source.md` no longer carries, and
/// nothing else on the corpus is at fault: `mrd check` must refuse.
#[test]
fn check_refuses_a_drifted_pin_with_no_write_history_plane_to_blame() {
    let sb = sandbox();
    let ws = pulled_corpus(&sb, "drifted-spotless-journal");
    let root = root_of(&ws);
    establish_a_spotless_baseline(&root);

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);

    // The control: the refusal below must come from the pin plane and nothing
    // else — there is no write-history plane, and the surface says so.
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

/// The acceptance: every write governed, the pinned content untouched, the
/// pinned blob reachable from a commit — `mrd check` must be green and exit 0.
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

/// The ordinary pin lifecycle passes through two non-durable states
/// (never-anchored, then pending-anchor) and is accepted in all of them —
/// wiring the anchoring refusal to `pending-anchor` would refuse every
/// governed commit.
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

/// The defect the fence must catch is the orphan: a `--vibe` eager blob that
/// no ref reaches and that the file no longer hashes to, so no commit of that
/// file will ever anchor it — prunable at `gc.pruneExpire`, after which the
/// pin can be verified against nothing.
///
/// The corpus isolates the anchoring finding from the content one: the
/// out-of-band edit lands outside the pinned section, so the claim plane
/// stays green while the whole-file blob moves. One defect, one word.
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

    // The control: the discriminator is the verdict class — a `cannot-assess`
    // report carries `red: false`, and only a finding sets `red: true`, so
    // the refusal is attributable to the anchoring finding.
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

/// Three planes agreeing on the verdicts over a corpus is a runtime property
/// of data — no `const` can assert it.
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

    // ── ARM 2: the pulled copy — the lock arrived with its pin, the source
    // arrived changed. All three planes must now read the same red pin. ─────
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

/// Honest degradation: a workspace with no git repository cannot be asked
/// about anchoring at all, and `check` must say so — never read the
/// unanswerable question as a clean bill. This is a `cannot-assess`, not a
/// finding.
#[test]
fn check_degrades_honestly_when_there_is_no_git_repository() {
    let sb = sandbox();
    let ws = sb.bare_workspace("no-git");
    write(&ws, "source.md", SOURCE_PINNED);
    write(&ws, "claim.md", CLAIM);
    let root = root_of(&ws);

    // `mrd pin` refuses outright where git cannot answer, so the lock is
    // written by hand, with the claim plane held green — the only thing left
    // for `check` to be unhappy about is the store it cannot ask.
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

/// A cross-root pin is skipped and stated, and the fence still passes:
/// refusing it as `grey(cannot-assess)` would make the fence refuse every
/// commit in any repo holding a cross-root pin, since R4 makes `hash`
/// mandatory on every pin.
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

    assert!(
        text.contains("anchoring scope:") && text.contains("alpha"),
        "the sight line is STATED, naming the root outside it: {text}"
    );
    assert!(
        text.contains("NOT measured here"),
        "and it says plainly that those blobs were not measured: {text}"
    );

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

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
