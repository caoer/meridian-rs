//! **U22 / H1 — lost-pin repair, end to end over a real git history.**
//!
//! The verb under test recovers a pin whose evidence is gone by walking the
//! repository's own history, and refuses to invent evidence when the walk finds
//! none. Every fixture here is a REAL git repository driven through the SHIPPED
//! binary, because the whole subject is what git recorded.
//!
//! # The shape that makes a pin recoverable — worth reading once
//! A pin's `hash` is the blob of the whole target FILE; its `fingerprint` covers
//! ONE SECTION of it. Git is content-addressed, so a file blob that is absent
//! from the store is a file whose exact bytes were never recorded. The recovery
//! is possible because the SECTION can survive in a commit whose file bytes
//! differ elsewhere: the pin's evidence is the section, and history still holds
//! it inside a different file version. That is not a corner case — an operator
//! who pins mid-edit and then keeps editing the page's other parts produces it.
//!
//! # The invariant these tests exist to hold
//! Repair rewrites the pin's `hash` and NOTHING else. A repair that reached green
//! by moving the `selector` or the `fingerprint` would be forgery, and
//! `repair_refuses_the_forgery_that_would_go_green` builds the case where that
//! forgery is AVAILABLE and proves the verb does not take it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// ── harness (the u14 pin-plane harness, same env isolation) ──────────────────

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
        self.run_with_path(cwd, args, None)
    }

    /// `path_prefix` prepends a directory to `PATH` — the seam the one-`log`,
    /// one-`cat-file` gate uses to put a logging shim in front of git.
    fn run_with_path(&self, cwd: &Path, args: &[&str], path_prefix: Option<&Path>) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env_remove("MERIDIAN_WORKSPACE")
            .env_remove("MERIDIAN_CONFIG");
        if let Some(prefix) = path_prefix {
            let existing = std::env::var("PATH").unwrap_or_default();
            cmd.env("PATH", format!("{}:{existing}", prefix.display()));
        }
        cmd.output().expect("spawn mrd")
    }

    fn git_workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git_init(&ws);
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
    git_in(dir, &["config", "user.name", "u22 fixture"]);
    git_in(dir, &["config", "user.email", "u22@fixture.invalid"]);
    git_in(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_all(dir: &Path, message: &str) {
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

/// Whether git HOLDS this object — the retrieval-plane question, asked of git
/// directly so the fixture never takes the verb's word for it.
fn git_holds(dir: &Path, oid: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["cat-file", "-e", oid])
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

fn read(ws: &Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).expect("read fixture")
}

/// The ONE pin declared in `claim.md`, read through the lock grammar's own
/// parser — never a substring match on the block's bytes.
fn the_pin(ws: &Path, page: &str) -> lock::PinEntry {
    let raw = read(ws, page);
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let found = lock::find(&doc)
        .expect("the lock block parses")
        .expect("the page carries a lock block");
    let mut pins = found.lock.pins;
    assert_eq!(pins.len(), 1, "the fixture declares exactly one pin");
    pins.remove(0)
}

// ── the fixture: a pin whose evidence history still holds ────────────────────

const INTRO_ONE: &str = "alpha intro";
const INTRO_TWO: &str = "beta intro";
const INTRO_THREE: &str = "gamma intro";
const PINNED_BODY: &str = "the pinned body";

fn source_at(intro: &str, body: &str) -> String {
    format!("# Source\n\n{intro}\n\n## Guideline\n\n{body}\n")
}

/// **The lost-but-recoverable corpus.**
///
/// 1. commit A records the page;
/// 2. the operator edits the page's INTRO and pins the guideline mid-edit — a
///    non-vibe pin hashes without `-w`, so the pinned FILE blob never enters the
///    object store;
/// 3. the operator edits the intro AGAIN and commits (commit B) — so the pinned
///    file blob is still absent, while commit B's version carries the pinned
///    SECTION byte-identically;
/// 4. the guideline itself is then rewritten and committed (commit C), which
///    turns the live claim plane red.
///
/// Result: both planes dark — the definition of LOST — with the evidence sitting
/// in commit B.
fn lost_but_recoverable(sb: &Sandbox, name: &str) -> PathBuf {
    let ws = sb.git_workspace(name);
    write(&ws, "source.md", &source_at(INTRO_ONE, PINNED_BODY));
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    commit_all(&ws, "A: the page");

    write(&ws, "source.md", &source_at(INTRO_TWO, PINNED_BODY));
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));

    // The intro moves again BEFORE the commit, so the bytes commit B records are
    // not the bytes the pin hashed — while the pinned section is untouched.
    let pinned_section = read(&ws, "source.md");
    let after_pin_intro = pinned_section.replace(INTRO_TWO, INTRO_THREE);
    write(&ws, "source.md", &after_pin_intro);
    commit_all(&ws, "B: the intro moves, the guideline does not");

    // And now the guideline itself drifts — the live claim plane goes red.
    write(
        &ws,
        "source.md",
        &after_pin_intro.replace(PINNED_BODY, "the drifted body"),
    );
    commit_all(&ws, "C: the guideline drifts");
    ws
}

/// The fixture assertion every lost-pin gate rests on: git does NOT hold the
/// pinned blob, so the retrieval plane really is dark.
fn assert_evidence_is_gone(ws: &Path, pin: &lock::PinEntry) {
    assert!(
        !git_holds(ws, &pin.hash),
        "fixture premise: the pinned blob {} must be absent from the object store",
        pin.hash
    );
}

// ── the gates ────────────────────────────────────────────────────────────────

/// **The unit's delivery.** A lost pin whose content history still holds is
/// repaired: the row's `hash` becomes a blob git actually HOLDS, and the claim —
/// object, selector, fingerprint — is byte-for-byte what it was.
#[test]
fn a_lost_pin_whose_content_history_holds_is_repaired() {
    let sb = sandbox();
    let ws = lost_but_recoverable(&sb, "recoverable");
    let before = the_pin(&ws, "claim.md");
    assert_evidence_is_gone(&ws, &before);

    let out = sb.run(&ws, &["repair"]);
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));
    assert!(
        said(&out).contains("repaired"),
        "the report names the repair: {}",
        said(&out)
    );

    let after = the_pin(&ws, "claim.md");
    assert_ne!(
        after.hash, before.hash,
        "the retrieval plane was repointed at recovered evidence"
    );
    assert!(
        git_holds(&ws, &after.hash),
        "and the evidence it now names is an object git HOLDS: {}",
        after.hash
    );

    // THE FORGERY INVARIANT, field by field.
    assert_eq!(after.object, before.object, "object is never rewritten");
    assert_eq!(
        after.selector, before.selector,
        "selector is never rewritten"
    );
    assert_eq!(
        after.fingerprint, before.fingerprint,
        "fingerprint is never rewritten — the claim is not the engine's to move"
    );
}

/// **The invariant's consequence, stated as its own gate.** The target genuinely
/// drifted, so the pin is STILL RED after a successful repair. A repair that left
/// the corpus green would have moved the claim.
#[test]
fn a_repaired_pin_is_still_red_because_the_target_really_did_drift() {
    let sb = sandbox();
    let ws = lost_but_recoverable(&sb, "still-red");
    let repair = sb.run(&ws, &["repair"]);
    assert_eq!(repair.status.code(), Some(0), "repair: {}", said(&repair));

    let walk = sb.run(&ws, &["walk", "claim.md"]);
    assert!(
        said(&walk).contains("red"),
        "the drift is still reported after the repair: {}",
        said(&walk)
    );
}

/// **`--dry` is the skip-the-final-write rehearsal (P11/D3), never a diff face.**
/// The walk runs and the recovery is reported; the page is byte-identical.
#[test]
fn dry_reports_the_recovery_and_writes_nothing() {
    let sb = sandbox();
    let ws = lost_but_recoverable(&sb, "dry");
    let before = read(&ws, "claim.md");

    let out = sb.run(&ws, &["repair", "--dry"]);
    assert_eq!(out.status.code(), Some(0), "repair --dry: {}", said(&out));
    assert!(
        said(&out).contains("repaired"),
        "the rehearsal still reports what it recovered: {}",
        said(&out)
    );
    assert_eq!(
        read(&ws, "claim.md"),
        before,
        "--dry landed no bytes on the page"
    );
}

/// **A pin whose blob git still holds is NOT lost** — it is ordinary drift with
/// its evidence intact, and this verb does not touch it. (The retrieval plane is
/// the discriminator: one plane dark is not a loss.)
#[test]
fn a_drifted_pin_whose_evidence_is_still_held_is_not_touched() {
    let sb = sandbox();
    let ws = sb.git_workspace("drift-with-evidence");
    write(&ws, "source.md", &source_at(INTRO_ONE, PINNED_BODY));
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    commit_all(&ws, "A");
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    // Commit the pinned bytes, so the pinned blob IS in the store, then drift.
    commit_all(&ws, "B: the pinned bytes are recorded");
    let recorded = the_pin(&ws, "claim.md");
    assert!(
        git_holds(&ws, &recorded.hash),
        "fixture premise: the pinned blob is held"
    );
    write(
        &ws,
        "source.md",
        &read(&ws, "source.md").replace(PINNED_BODY, "drifted"),
    );
    commit_all(&ws, "C: drift");
    let before = read(&ws, "claim.md");

    let out = sb.run(&ws, &["repair"]);
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));
    assert!(
        said(&out).contains("lost: 0"),
        "the drifted-but-held pin is not counted lost: {}",
        said(&out)
    );
    assert_eq!(read(&ws, "claim.md"), before, "and nothing was written");
}

/// **TRUE LOSS is reported and NEVER auto-fixed.** No commit in this history ever
/// carried the pinned content, so there is nothing to repair the pin WITH — the
/// verb refuses (exit 1) and the page is byte-identical.
#[test]
fn a_true_loss_is_reported_and_nothing_is_written() {
    let sb = sandbox();
    let ws = sb.git_workspace("true-loss");
    write(&ws, "source.md", &source_at(INTRO_ONE, PINNED_BODY));
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    commit_all(&ws, "A");

    // Pin content that is only ever in the working tree, then bury it.
    write(&ws, "source.md", &source_at(INTRO_ONE, "never committed"));
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    write(&ws, "source.md", &source_at(INTRO_TWO, "something else"));
    commit_all(&ws, "B: the pinned content is nowhere in history");

    let before = read(&ws, "claim.md");
    let recorded = the_pin(&ws, "claim.md");
    assert_evidence_is_gone(&ws, &recorded);

    let out = sb.run(&ws, &["repair"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a TRUE LOSS is a finding: {}",
        said(&out)
    );
    assert!(
        said(&out).contains("TRUE LOSS"),
        "and it is named as one: {}",
        said(&out)
    );
    assert_eq!(
        read(&ws, "claim.md"),
        before,
        "nothing was invented and nothing was written"
    );
}

/// **THE MUTATION-PROOF GATE.** Build the case where the forgery is AVAILABLE:
/// history holds a section at ANOTHER heading path whose bytes are exactly the
/// pinned ones, so a repair willing to move the `selector` could report green.
///
/// The test first PROVES the opportunity is real — it verifies the pinned
/// fingerprint against the other section through `classify_pin` itself and
/// asserts GREEN — and only then asserts that the verb answers TRUE LOSS and
/// leaves the row untouched. Without the first half this would be a test that
/// passes for the wrong reason.
#[test]
fn repair_refuses_the_forgery_that_would_go_green() {
    let sb = sandbox();
    let ws = sb.git_workspace("forgery");
    // Two sections, same heading text, different paths; the pin names Notes.
    // Commit A deliberately carries an OLDER Notes body: the pinned bytes must
    // exist in history under Archive's path and NOWHERE under Notes', or the
    // repair below would have a legitimate match and the gate would prove
    // nothing.
    let page = |notes: &str, archive: &str| {
        format!(
            "# Source\n\n## Notes\n\n### Guideline\n\n{notes}\n\n## Archive\n\n### Guideline\n\n\
             {archive}\n"
        )
    };
    write(&ws, "source.md", &page("the older body", "the older body"));
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    commit_all(&ws, "A");

    // Pin Notes/Guideline mid-edit so the pinned FILE blob is never recorded…
    write(&ws, "source.md", &page(PINNED_BODY, "the older body"));
    let pin = sb.run(
        &ws,
        &["pin", "claim.md", "source.md#Source/Notes/Guideline"],
    );
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    let recorded = the_pin(&ws, "claim.md");

    // …then make Archive a VERBATIM copy of the pinned subtree (the pin promoted
    // a `^slug` onto its heading, so the copy has to be taken AFTER the pin for
    // the bytes — and therefore the fingerprint — to match), and drift Notes.
    let after_pin = read(&ws, "source.md");
    let (head, _) = after_pin
        .split_once("## Archive")
        .expect("the fixture page has an Archive section");
    let notes_subtree = head
        .split_once("## Notes\n\n")
        .expect("the fixture page has a Notes section")
        .1
        .to_owned();
    let copied = format!("{head}## Archive\n\n{notes_subtree}");
    let gutted = copied.replacen(PINNED_BODY, "the notes body drifted", 1);
    write(&ws, "source.md", &gutted);
    commit_all(&ws, "B: Notes drifts, Archive keeps the bytes");

    let before = read(&ws, "claim.md");
    assert_evidence_is_gone(&ws, &recorded);

    // THE OPPORTUNITY IS REAL: the pinned fingerprint verifies GREEN against the
    // OTHER section of the committed page. A verb willing to move the selector
    // could report a repair here.
    let committed = read(&ws, "source.md");
    let doc = model::build(committed.clone(), syntax::parse(&committed));
    let forged = lock::Selector::Path(vec![
        "Source".to_owned(),
        "Archive".to_owned(),
        "Guideline".to_owned(),
    ]);
    let forged = view::walk::model_selector(&recorded.object, &forged);
    assert!(
        matches!(
            model::selector::classify_pin(&forged, &recorded.fingerprint, Some(&doc)),
            model::selector::Color::Green
        ),
        "the fixture must actually offer the forgery — the pinned token verifies \
         against Source/Archive/Guideline"
    );

    // And the verb does not take it.
    let out = sb.run(&ws, &["repair"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the available forgery is refused as a TRUE LOSS: {}",
        said(&out)
    );
    assert_eq!(
        read(&ws, "claim.md"),
        before,
        "the row is byte-identical — no selector was moved to reach green"
    );
}

/// **ONE `git log` and ONE `cat-file --batch` for the whole walk** — the law the
/// unit is built to hold, asserted the way `crates/git`'s plumbing suite asserts
/// it: a shim on `PATH` logs every argv and execs the real git.
#[test]
fn the_walk_spends_one_log_and_one_cat_file_batch() {
    let sb = sandbox();
    let ws = lost_but_recoverable(&sb, "one-log-one-batch");

    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .expect("locate git")
            .stdout,
    )
    .expect("utf-8")
    .trim()
    .to_owned();
    let shim_dir = sb.tmp.path().join("shim");
    std::fs::create_dir_all(&shim_dir).expect("shim dir");
    let log = sb.tmp.path().join("git-argv.log");
    let shim = shim_dir.join("git");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexec {real} \"$@\"\n",
            log.display()
        ),
    )
    .expect("write shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let out = sb.run_with_path(&ws, &["repair"], Some(&shim_dir));
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));

    let argv = std::fs::read_to_string(&log).expect("the shim logged");
    let logs = argv
        .lines()
        .filter(|line| line.split_whitespace().any(|word| word == "log"))
        .count();
    let batches = argv
        .lines()
        .filter(|line| line.contains("cat-file --batch") && !line.contains("--batch-check"))
        .count();
    assert_eq!(logs, 1, "exactly one `git log` for the walk:\n{argv}");
    assert_eq!(
        batches, 1,
        "exactly one `cat-file --batch` for every version read:\n{argv}"
    );
}

/// A corpus with nothing lost answers cleanly and writes nothing — the
/// population is STATED rather than left to an empty list (S3-R23(5)).
#[test]
fn a_corpus_with_nothing_lost_states_its_population() {
    let sb = sandbox();
    let ws = sb.git_workspace("nothing-lost");
    write(&ws, "source.md", &source_at(INTRO_ONE, PINNED_BODY));
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    commit_all(&ws, "A");
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    commit_all(&ws, "B");

    let out = sb.run(&ws, &["repair", "--json"]);
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(value["lost"], 0, "nothing is lost: {}", stdout(&out));
    assert!(
        value["scanned"].as_u64().expect("scanned") >= 1,
        "and the scanned population is stated, not implied: {}",
        stdout(&out)
    );
}
