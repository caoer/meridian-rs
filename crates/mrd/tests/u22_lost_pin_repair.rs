//! U22 / H1 — lost-pin repair, end to end over a real git history. The verb
//! recovers a pin whose evidence is gone by walking the repositorys own history,
//! and refuses to invent evidence when the walk finds none. Every fixture is a
//! REAL git repository driven through the SHIPPED binary, because the whole
//! subject is what git recorded.
//!
//! The shape that makes a pin recoverable: a pins `hash` is the blob of the
//! whole target FILE, while its `fingerprint` covers ONE SECTION of it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
mod common;

// ── harness (the u14 pin-plane harness, same env isolation) ──────────────────

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
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(args)
            .current_dir(cwd)
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

/// The lost-but-recoverable corpus: commit A records the page, then the operator
/// edits the pages INTRO and pins the guideline mid-edit — a non-vibe pin hashes
/// without `-w`, so the pinned FILE blob never enters the object store.
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

/// A lost pin whose content history still holds is repaired: the rows `hash`
/// becomes a blob git actually HOLDS, and the claim — object, selector,
/// fingerprint — is byte-for-byte what it was.
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

    // The forgery invariant, field by field.
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

/// The target genuinely drifted, so the pin is STILL RED after a successful
/// repair — a repair that left the corpus green would have moved the claim.
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

/// `--dry` is the skip-the-final-write rehearsal (P11/D3), never a diff face:
/// the walk runs and the recovery is reported, and the page is byte-identical.
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

/// A pin whose blob git still holds is NOT lost — it is ordinary drift with its
/// evidence intact, and this verb does not touch it.
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

/// TRUE LOSS is reported and NEVER auto-fixed: no commit in this history ever
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

/// The mutation-proof gate: history holds a section at ANOTHER heading path whose
/// bytes are exactly the pinned ones, so a repair willing to move the `selector`
/// could report green.
#[test]
fn repair_refuses_the_forgery_that_would_go_green() {
    let sb = sandbox();
    let ws = sb.git_workspace("forgery");
    // Two sections, same heading text, different paths; the pin names Notes.
    // Commit A carries an OLDER Notes body: the pinned bytes must exist in history
    // under Archives path and NOWHERE under Notes, or the repair below would have
    // a legitimate match.
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

    // …then make Archive a VERBATIM copy of the pinned subtree (the pin promoted a `^slug` onto
    // its heading, so the copy has to be taken AFTER the pin for the bytes — and therefore the
    // fingerprint — to match), and drift Notes.
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

    // The opportunity is real: the pinned fingerprint verifies GREEN against the
    // OTHER section of the committed page.
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

/// ONE `git log` and ONE `cat-file --batch` for the whole walk, asserted the way
/// `crates/git`s plumbing suite asserts it: a shim on `PATH` logs every argv and
/// execs the real git.
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

// ── the residency family: HOLDER x TARGET are INDEPENDENT axes ───────────────
//
// The per-pin TARGET ASSESSMENT is a DOOR (decisions/0045 over 0043): the pin
// row names ONE target, so repair READS it from disk — corpus residency is
// never a read admission test (§12.1, wire-contract.md:465, :813). An intact
// target means the pin is NOT LOST, so the row stays byte-identical as the
// consequence of having read it.
//
// ⭐ THE TWO GATES RUN IN OPPOSITE DIRECTIONS. `an_intact_…` demands NO write;
// `a_lost_out_of_domain_target_is_still_repaired` demands a write. A remedy
// that SKIPPED out-of-domain targets would pass the first and fail the second,
// which is exactly the reading the ruling rejected.

/// The corpus for the target axis: one in-domain target and one the hash domain
/// excludes (dot-segment), each pinned from an in-domain holder. `pin` writes
/// its `^slug` anchor into the target AFTER the commit, so both pins name a
/// working-tree blob the object store does not hold — both retrieval planes are
/// dark, and only residency of the TARGET differs.
fn target_axis(sb: &Sandbox, name: &str) -> PathBuf {
    let ws = sb.git_workspace(name);
    std::fs::create_dir_all(ws.join(".github")).expect("mkdir .github");
    write(&ws, "spec.md", "# Rule\n\nthe value is SEVEN\n");
    write(
        &ws,
        ".github/dotspec.md",
        "# Rule\n\nthe value is ELEVEN and this page is the dot-segment one\n",
    );
    write(&ws, "in.md", "# Holder in\n\nit follows the rule.\n");
    write(&ws, "out.md", "# Holder out\n\nit follows the rule.\n");
    commit_all(&ws, "A: the fixture");

    let a = sb.run(&ws, &["pin", "in.md", "spec.md#Rule"]);
    assert_eq!(a.status.code(), Some(0), "pin in-domain: {}", said(&a));
    let b = sb.run(&ws, &["pin", "out.md", ".github/dotspec.md#Rule"]);
    assert_eq!(b.status.code(), Some(0), "pin out-of-domain: {}", said(&b));
    ws
}

/// ⛔ THE DEFECT GATE. An INTACT pin whose target the hash domain excludes is
/// left BYTE-IDENTICAL, and the in-domain control of identical construction is
/// byte-identical in the same run — without that control a rewrite is
/// indistinguishable from repair doing its job.
#[test]
fn an_intact_pin_whose_target_is_out_of_domain_is_left_byte_identical() {
    let sb = sandbox();
    let ws = target_axis(&sb, "target-axis");
    // Fixture premise, asserted rather than assumed: both retrieval planes are
    // dark, so both pins are CANDIDATES for a wrongful rewrite.
    let before_in = the_pin(&ws, "in.md");
    let before_out = the_pin(&ws, "out.md");
    assert_evidence_is_gone(&ws, &before_in);
    assert_evidence_is_gone(&ws, &before_out);
    let bytes_in = read(&ws, "in.md");
    let bytes_out = read(&ws, "out.md");

    let out = sb.run(&ws, &["repair"]);
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));

    // THE ACCEPTANCE IS THE BYTES, never the exit code and never the summary.
    assert_eq!(
        read(&ws, "out.md"),
        bytes_out,
        "SUBJECT: the out-of-domain target is present and intact, so its pin is \
         not lost and the row is untouched: {}",
        said(&out)
    );
    assert_eq!(
        read(&ws, "in.md"),
        bytes_in,
        "CONTROL: the in-domain arm of identical construction is untouched too — \
         if this moved, residency was not the variable: {}",
        said(&out)
    );
    assert_eq!(
        the_pin(&ws, "out.md").hash,
        before_out.hash,
        "and the hash names the same blob it named before"
    );
}

/// ⭐ THE DISCRIMINATOR. Repair READS the named target; it does not SKIP
/// out-of-domain ones. A pin whose out-of-domain target GENUINELY drifted is
/// still lost and still repaired — a skip would leave it alone and pass the
/// gate above while quietly abandoning the verb outside the domain.
#[test]
fn a_lost_pin_whose_out_of_domain_target_drifted_is_still_repaired() {
    let sb = sandbox();
    let ws = sb.git_workspace("target-axis-drifted");
    std::fs::create_dir_all(ws.join(".github")).expect("mkdir .github");
    // The `lost_but_recoverable` shape, on a DOT-SEGMENT target: the INTRO moves
    // around the pin so no commit ever records the pinned FILE blob, while the
    // pinned SECTION survives into commit B.
    write(
        &ws,
        ".github/dotspec.md",
        &source_at(INTRO_ONE, PINNED_BODY),
    );
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    commit_all(&ws, "A: the page");

    write(
        &ws,
        ".github/dotspec.md",
        &source_at(INTRO_TWO, PINNED_BODY),
    );
    let pin = sb.run(
        &ws,
        &["pin", "claim.md", ".github/dotspec.md#Source/Guideline"],
    );
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));

    let after_pin = read(&ws, ".github/dotspec.md");
    let with_third_intro = after_pin.replace(INTRO_TWO, INTRO_THREE);
    write(&ws, ".github/dotspec.md", &with_third_intro);
    commit_all(&ws, "B: the intro moves, the guideline does not");
    write(
        &ws,
        ".github/dotspec.md",
        &with_third_intro.replace(PINNED_BODY, "the drifted body"),
    );
    commit_all(&ws, "C: the guideline drifts");

    let before = the_pin(&ws, "claim.md");
    assert_evidence_is_gone(&ws, &before);

    let out = sb.run(&ws, &["repair"]);
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));
    let after = the_pin(&ws, "claim.md");
    assert_ne!(
        after.hash,
        before.hash,
        "the out-of-domain target really is lost, so repair acted on it — a remedy \
         that SKIPPED out-of-domain targets would leave it alone here: {}",
        said(&out)
    );
    assert!(
        git_holds(&ws, &after.hash),
        "and it landed on evidence git HOLDS: {}",
        after.hash
    );
    assert_eq!(
        after.fingerprint, before.fingerprint,
        "the claim is still not the engine's to move"
    );
}

/// The receipt names WHICH PIN and WHICH TARGET per action — this verb acts on
/// many pins in one invocation, so an aggregate count cannot be correlated by a
/// caller holding its own request.
#[test]
fn every_repaired_line_names_its_pin_and_its_target() {
    let sb = sandbox();
    let ws = lost_but_recoverable(&sb, "named-subject");
    let out = sb.run(&ws, &["repair"]);
    assert_eq!(out.status.code(), Some(0), "repair: {}", said(&out));
    let text = stdout(&out);
    assert!(
        text.contains("claim.md") && text.contains("target source.md"),
        "the action line names the declaring page AND the target it acted on: {text}"
    );

    // A SECOND workspace: the run above already repaired the first one, and a
    // repaired corpus has nothing left to name.
    let fresh = lost_but_recoverable(&sb, "named-subject-json");
    let json = sb.run(&fresh, &["repair", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&json)).expect("json");
    assert_eq!(
        value["pins"][0]["target"],
        "source.md",
        "and the machine face carries the same subject: {}",
        stdout(&json)
    );
}

/// THE SECOND MEMBER of the `--json` refusal-envelope family — the lock-door leg.
///
/// The family law lives in `tests/json_refusal_family.rs`; this case lives HERE because the
/// only leg of `repair` that can carry a §8 `ErrorBody` is the lock write, and reaching it
/// needs this file's `lost_but_recoverable` corpus. Cross-referenced from that suite so the
/// enumeration stays readable from one place.
///
/// Before the fix, EVERY refusal leg of `mrd repair --json` served zero stdout bytes: the verb
/// reached `wire_serve::write::lock_write` directly and hand-composed the refusal, so it never
/// touched `engine` at all — which is why the family suite's privacy argument could not reach
/// it (`engine::refusal_fail` being private gates the doors that ASK the helper).
///
/// ⚠️ EXIT 2, NOT 1, AND THAT IS THE POINT OF THE SEPARATE FRAME EMITTER. `mrd repair` reserves
/// EXIT 1 FOR A TRUE LOSS. Routing this leg through `engine::json_refusal` — which returns the
/// findings leg — would publish the envelope and simultaneously tell a scripted caller that a
/// pin was unrecoverable when the lock door had merely refused. The frame and the exit code are
/// two judgements; `engine::json_error_frame` emits one and leaves the other to the verb.
#[test]
fn repair_serves_the_envelope_when_the_lock_door_refuses() {
    let sb = sandbox();
    let ws = lost_but_recoverable(&sb, "envelope-lock-refusal");

    // CONTROL, and it must hold in the fixed AND the unfixed tree: this corpus really does reach
    // the lock write. Without it, a zero below could mean the walk found nothing to repair —
    // a verb that never got to the door, rather than a door whose refusal has no frame.
    let dry = sb.run(&ws, &["repair", "--dry", "--json"]);
    assert_eq!(
        dry.status.code(),
        Some(0),
        "control: the rehearsal reaches the lock write and succeeds: {}",
        said(&dry)
    );
    let body: serde_json::Value =
        serde_json::from_slice(&dry.stdout).expect("control: --dry serves its JSON face");
    assert_eq!(
        body.get("repaired").and_then(serde_json::Value::as_u64),
        Some(1),
        "control: exactly one pin is recoverable, so the write leg is reachable: {body}"
    );

    // Now make the lock write fail inside the door. ⚠️ NOT by chmod-ing the PAGE: a guarded
    // write lands a temp file and renames it, and rename needs the DIRECTORY's write bit, not
    // the file's — a read-only `claim.md` is repaired successfully, which this fixture's own
    // precondition caught. The WORKSPACE DIRECTORY is the operand.
    let claim = ws.join("claim.md");
    let mut ro = std::fs::metadata(&ws).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut ro, 0o555);
    std::fs::set_permissions(&ws, ro).expect("chmod ro");

    let out = sb.run(&ws, &["repair", "--json"]);

    let mut rw = std::fs::metadata(&ws).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut rw, 0o755);
    std::fs::set_permissions(&ws, rw).expect("chmod rw");

    // PRECONDITION asserted, never assumed: as root, or on a filesystem ignoring mode bits, the
    // write succeeds and this fixture proves nothing. It must FAIL LOUD rather than skip — a
    // silent skip reports a green family over a leg nobody exercised.
    assert_ne!(
        out.status.code(),
        Some(0),
        "precondition: a read-only declaring page must make the lock door refuse; this fixture \
         cannot build its condition here and is not quietly passing: {}",
        said(&out)
    );

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !stdout.is_empty(),
        "served 0 stdout bytes — the ABSENT FRAME is the defect, and a parsing agent cannot \
         tell it from success with no output: {}",
        said(&out)
    );
    let frame: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not the JSON envelope: {e}\n{stdout}"));
    assert!(
        frame
            .get("workspace")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "the envelope names its workspace: {frame}"
    );
    assert_eq!(
        frame
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str),
        Some("io_error"),
        "the lock door's §8 code rides the frame: {frame}"
    );

    // The exit triad is NOT moved by the envelope, and this is the assertion that would have
    // caught the fix I nearly shipped.
    assert_eq!(
        out.status.code(),
        Some(2),
        "a lock-door refusal stays a TOOL failure — exit 1 in this verb means a TRUE LOSS, and \
         spelling a refusal as one would tell a script the pin is unrecoverable: {}",
        said(&out)
    );

    // The human line still names WHICH page failed: this loop writes page by page, so the page
    // name and the nothing-was-written clause are the operator's whole recovery.
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("claim.md"),
        "the refusal names the page whose repair failed: {stderr}"
    );
    assert!(
        stderr.contains("Nothing was written for that page"),
        "the refusal states what did not happen: {stderr}"
    );

    // And the HUMAN face publishes no envelope at the same leg.
    let mut ro2 = std::fs::metadata(&ws).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut ro2, 0o555);
    std::fs::set_permissions(&ws, ro2).expect("chmod ro");
    let human = sb.run(&ws, &["repair"]);
    let mut rw2 = std::fs::metadata(&ws).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut rw2, 0o755);
    std::fs::set_permissions(&ws, rw2).expect("chmod rw");
    let _ = &claim;
    assert!(
        human.stdout.is_empty(),
        "the human face says nothing on stdout; the envelope is the `--json` face's: {}",
        said(&human)
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
