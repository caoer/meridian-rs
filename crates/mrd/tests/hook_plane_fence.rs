//! **The fence body's gates, each proved able to say NO — as PLACED from the
//! emitted contract.**
//!
//! Every arm here drives a real fence through a real `git` operation and reads
//! what git did. Nothing is asserted about a hand-transcribed hook: the artifact
//! under test is **the body `mrd skill hook` emits**, extracted from that verb's
//! stdout exactly as its document tells a reader to extract it and placed exactly
//! where its document says to place it ([`Fixture::place`]).
//!
//! That is what makes these design tests of the NEW contract rather than survivors
//! of the old one. The installer they used to call is deleted; the document is the
//! contract now, and a document whose body does not fence is a contract that lies.
//! `crates/mrd/tests/skill_hook_emit.rs` measures the document's CLAIMS; this file
//! measures whether following them fences a repository.
//!
//! # THE TRAP THIS FILE IS WRITTEN AGAINST
//! **A hook fix tested by running the hook and asserting exit 0 has built an
//! instrument with precisely the defect it was sent to fix.** A silently-exiting
//! broken predicate and a legitimately passing hook are indistinguishable at the
//! exit code — which IS the defect row 22 reports. So every gate here carries the
//! arm that proves it says YES and the arm that proves it says NO, in the same
//! run, and the refusal arms below fail against the pre-fix fence:
//!
//! - the force grammar's refusal spellings (`0`, `false`, `no`, `off`, `" "`) all
//!   FORCED under `[ -n "${MRD_HOOK_FORCE:-}" ]`, and the unparseable ones did too;
//! - the force path printed NOTHING, so no arm could tell a forced commit from an
//!   honest one;
//! - `git merge` and `git am` landed commits past a door set of one.
//!
//! # THE INSTRUMENT'S OWN CONTROL
//! These arms need a tree the fence is actively refusing, so anything that lands
//! landed past a fence that was trying to say no. **That precondition is ASSERTED,
//! never assumed** — [`Fixture::refusing`] runs the verb the body runs and fails
//! loudly if it ever stops refusing, because a green answer would make every
//! refusal arm below pass for the wrong reason and look exactly like a working
//! fence.
//!
//! # HOW THAT TREE IS BUILT CHANGED — and this is a REAL reduction, recorded
//! It used to be free. *A scratch workspace with no receipt journal* was already a
//! refusing tree: with no baseline, `mrd check --commit-gate` answered
//! `grey(cannot-assess)` and refused. Emptiness itself was the refusal.
//!
//! Under ZT's ruling (2026-08-03) the engine keeps no memory, the gate reads the
//! **pin plane alone**, and a workspace with no pins is vacuously clean — it exits
//! **0** (U5 corpus row 09). So five arms here began passing a control that had
//! silently become false: they type-checked perfectly and measured nothing. **That
//! was the honest signal that enforcement reduced, and the fix is not to weaken the
//! control.** [`Fixture::refusing`] still asserts a refusal; what changed is that
//! the fixture must now CONSTRUCT one — [`Fixture::drift_a_pin`] pins real content
//! and then rewrites it out of band, so the gate refuses for a reason it can
//! actually see.
//!
//! **The accounting entry:** the fence's fail-closed behaviour on a corpus that
//! claims nothing moved from IMPLICIT (every fresh workspace refused) to OPT-IN
//! (the caller asks for it with `--require-pins`).
//!
//! # WHY THE OPT-IN FLAG IS NOT USED HERE, though it exists
//! The ruling that restored fail-closed-on-empty as `mrd check --commit-gate
//! --require-pins` would rebuild this file's original mechanism exactly. It is
//! deliberately NOT used, for a reason this file already states at
//! [`Fixture::refusing`]: **the shipped fence body runs bare `mrd check
//! --commit-gate`** (`crates/mrd/src/skills/hook.md`), and *"a control on a
//! different invocation than the body runs is a control over a different verdict —
//! which is how the fence came to ship asking a permanent question in the first
//! place."* A control passing `--require-pins` would assert a refusal the real
//! fence never receives, and every `git commit` arm below drives the real fence.
//! The flag's own coverage lives with the gate, in
//! `crates/mrd/tests/s4r19_commit_gate.rs`, where the invocation under test IS the
//! flag.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mrd::hook::FENCED_HOOKS;

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// The system directories a `git commit` needs. The fixture's `bin/` is
/// prepended, so OUR `mrd` shadows any deployed one — the ordering is the
/// isolation.
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// A scratch repository that is also a meridian workspace, with an `mrd` of our
/// own on the hook's `PATH` and caches inside the sandbox.
struct Fixture {
    tmp: tempfile::TempDir,
    bin: PathBuf,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bin = tmp.path().join("bin");
        let home = tmp.path().join("home");
        let cache_home = tmp.path().join("xdg-cache");
        std::fs::create_dir_all(&bin).expect("bin");
        std::fs::create_dir_all(&home).expect("home");
        std::os::unix::fs::symlink(mrd_bin(), bin.join("mrd")).expect("link mrd onto PATH");
        let ws = tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir ws");
        let fixture = Self {
            tmp,
            bin,
            home,
            cache_home,
            ws,
        };
        git_ok(&fixture.ws, &["init", "-q", "-b", "main"]);
        git_ok(
            &fixture.ws,
            &["config", "user.email", "fence@example.invalid"],
        );
        git_ok(&fixture.ws, &["config", "user.name", "fence"]);
        fixture.write("plan.md", "# Plan\n\n## Goals\n\nalpha\n");
        fixture.write("source.md", "# Source\n\n## Guideline\n\nthe pinned body\n");
        fixture.write("claim.md", "# Claim\n\nwe rely on the guideline.\n");
        let init = fixture.mrd(&["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        git_ok(&fixture.ws, &["add", "-A"]);
        git_ok(&fixture.ws, &["commit", "-qm", "corpus"]);
        fixture.drift_a_pin();
        fixture
    }

    /// **Build the refusing tree the whole file rests on.**
    ///
    /// Pin `claim.md` to a section of `source.md` through the shipped CLI, then
    /// rewrite that section by hand — an out-of-band edit to PINNED content, which
    /// the surviving pin plane reads as `red content-drifted`. The gate the fence
    /// runs therefore refuses, and it refuses over a fact about this corpus.
    ///
    /// This function is the whole cost of the ruling, in one place. The refusal
    /// used to require nothing at all; it now requires a corpus that declares a
    /// claim and a lie told against it. Every arm below is unchanged in subject —
    /// they measure the FENCE — but none of them can run over an empty workspace
    /// any more.
    fn drift_a_pin(&self) {
        let pin = self.mrd(&["pin", "claim.md", "source.md#Source/Guideline"]);
        assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
        git_ok(&self.ws, &["add", "-A"]);
        git_ok(&self.ws, &["commit", "-qm", "pin"]);
        self.write("source.md", "# Source\n\n## Guideline\n\nOUT OF BAND\n");
        git_ok(&self.ws, &["add", "-A"]);
        assert!(
            std::fs::read_to_string(self.ws.join("source.md"))
                .expect("source")
                .contains("OUT OF BAND"),
            "R40 — the drift is on disk and staged, so it is in the interval a \
             commit records"
        );
    }

    /// **Undo [`Fixture::drift_a_pin`]** — restore the pinned section to the bytes
    /// the lock names, and stage that. The corpus becomes one with a real pin and
    /// no lie told against it.
    ///
    /// This exists for [`the_refusing_fixture_is_two_directional`], which is the
    /// condition on the whole rebuild: a control that only ever refuses has not
    /// been shown to depend on the thing it guards.
    fn heal_the_pin(&self) {
        self.write("source.md", "# Source\n\n## Guideline\n\nthe pinned body\n");
        git_ok(&self.ws, &["add", "-A"]);
    }

    fn hook_path(&self) -> String {
        format!("{}:{SYSTEM_PATH}", self.bin.display())
    }

    fn mrd(&self, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .env_remove("MRD_HOOK_FORCE")
            .output()
            .expect("spawn mrd")
    }

    /// A real git operation, run the way an operator runs it: the hook fires and
    /// our `mrd` is the only one on `PATH`.
    fn git(&self, args: &[&str], force: Option<&str>) -> Output {
        let mut c = Command::new("git");
        c.arg("-C")
            .arg(&self.ws)
            .args(args)
            .env("PATH", self.hook_path())
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .env_remove("MRD_HOOK_FORCE");
        if let Some(value) = force {
            c.env("MRD_HOOK_FORCE", value);
        }
        c.output().expect("spawn git")
    }

    /// **Place the fence the way the contract says to** — extract the one fenced
    /// block from `mrd skill hook`, write it to every door under the common dir,
    /// chmod +x. There is no installer to call: the document is the contract, and
    /// this function is a test doing exactly what its reader is told to do.
    ///
    /// It is deliberately mechanical. A helper that "knew" the body would be
    /// measuring a transcription; every byte here comes off the emitter's stdout.
    fn place(&self) {
        let doors = &FENCED_HOOKS[..];
        self.place_at(doors);
    }

    /// The same placement, over a chosen subset — for the partial-coverage arm,
    /// which needs a checkout fenced at fewer doors than the set claims.
    fn place_at(&self, doors: &[&str]) {
        let body = fence_body(&self.emit());
        let hooks = common_dir(&self.ws).join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        for name in doors {
            let path = hooks.join(name);
            std::fs::write(&path, &body).expect("place the fence");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
        }
    }

    /// The document, as an agent receives it.
    fn emit(&self) -> String {
        let out = self.mrd(&["skill", "hook"]);
        assert_eq!(out.status.code(), Some(0), "mrd skill hook: {}", said(&out));
        stdout(&out)
    }

    /// **The instrument's control, asserted rather than assumed.** Every refusal
    /// arm below rests on this tree being one the fence is actively refusing; a
    /// green answer would make all of them pass for the wrong reason, and pass
    /// silently.
    ///
    /// It asks the question THE FENCE ASKS. A control on a different invocation
    /// than the body runs is a control over a different verdict — which is how the
    /// fence came to ship asking a permanent question in the first place.
    ///
    /// **This control CAUGHT the reduction it now guards against.** When the gate
    /// stopped refusing empty workspaces, five arms in this file kept compiling and
    /// stopped measuring — and it was this assert firing that said so. It is
    /// unchanged; [`Fixture::drift_a_pin`] changed instead, because the honest fix
    /// for a control that went false is a fixture that makes it true again, never a
    /// control that asks less.
    fn refusing(&self) {
        let out = self.mrd(&["check", "--commit-gate"]);
        assert_ne!(
            out.status.code(),
            Some(0),
            "the control failed: this fixture's tree must be one `mrd check --commit-gate` \
             REFUSES, or every arm resting on it measures nothing. Output: {}",
            said(&out)
        );
        assert!(
            said(&out).contains("content-drifted"),
            "and it must refuse for the reason this fixture BUILT — a refusal from \
             somewhere else would mean the drift stopped being the thing under the \
             fence: {}",
            said(&out)
        );
    }

    fn write(&self, rel: &str, body: &str) {
        std::fs::write(self.ws.join(rel), body).expect("write");
    }

    fn dirty(&self, body: &str) {
        self.write("plan.md", body);
        git_ok(&self.ws, &["add", "-A"]);
    }

    fn door(&self, name: &str) -> PathBuf {
        common_dir(&self.ws).join("hooks").join(name)
    }

    fn head_count(&self) -> usize {
        git_out(&self.ws, &["rev-list", "--count", "HEAD"])
            .trim()
            .parse()
            .expect("a commit count")
    }

    /// The checkout's fence coverage, off the face that reports it now: `mrd
    /// check`'s `fence` block. The retired `hook status` verb was a second reader
    /// of the same doors; there is one.
    fn fence_json(&self) -> serde_json::Value {
        let out = self.mrd(&["check", "--json"]);
        let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap_or_else(|e| {
            panic!("check --json is not json ({e}): {}", said(&out));
        });
        value["fence"].clone()
    }
}

/// The fence body, extracted the way the document says to extract it: the one
/// fenced block, and it is the file. `crates/mrd/tests/skill_hook_emit.rs` holds
/// the document to there being exactly one.
fn fence_body(doc: &str) -> String {
    let mut lines = doc.lines();
    let mut body = String::new();
    lines
        .by_ref()
        .find(|l| l.starts_with("```"))
        .expect("the document carries a fenced block");
    for line in lines {
        if line.starts_with("```") {
            return body;
        }
        body.push_str(line);
        body.push('\n');
    }
    panic!("the fenced block is never closed");
}

// ── THE FIXTURE'S OWN MUTATION PROOF ─────────────────────────────────────────

/// **THE CONDITION ON THIS FILE'S REBUILD: the control varies with the world.**
///
/// Every other arm here rests on [`Fixture::refusing`], and a control that refuses
/// is worth nothing until it is also shown to PASS when nothing is wrong. That is
/// not a hypothetical worry — it is the exact defect this file just came out of.
///
/// # What the old controls actually were
/// Before the ruling, `refusing()` passed over a corpus of `plan.md` and nothing
/// else: no pin, no out-of-band write, **nothing wrong at all**. The refusal came
/// from the journal being empty, so the gate answered `grey(cannot-assess)`. The
/// five arms asserted exit 1 for a reason unrelated to what the fence guards.
/// **They were ACCIDENTALLY-refusing controls** — permanently refusing, and
/// therefore never varying with their input, which is precisely the property row 22
/// reports as the shipped fence's defect. They would have kept passing over a fence
/// that had stopped working.
///
/// So this is not a lost default being compensated for. It is an accident being
/// replaced by the thing the tests meant, and the fence ends up measured more
/// strictly than it was before.
///
/// # Both directions, one run, one fixture
/// Drifted ⇒ the gate refuses. The SAME fixture healed ⇒ the gate passes. Neither
/// half alone proves anything: the first is satisfied by a gate wired shut, the
/// second by a gate wired open, and only the pair shows the control tracks the
/// world.
#[test]
fn the_refusing_fixture_is_two_directional() {
    let fx = Fixture::new("mutation");

    // REFUSING — the drift `Fixture::new` built, which is what every other arm
    // rests on.
    let drifted = fx.mrd(&["check", "--commit-gate"]);
    assert_eq!(
        drifted.status.code(),
        Some(1),
        "the drifted corpus must refuse, or every arm in this file measures \
         nothing: {}",
        said(&drifted)
    );
    assert!(
        said(&drifted).contains("content-drifted"),
        "and for the GUARDED reason — an out-of-band edit to pinned content, not \
         an incidental absence of evidence: {}",
        said(&drifted)
    );

    // HEALED — the same corpus, the same fixture, the lie removed.
    fx.heal_the_pin();
    let healed = fx.mrd(&["check", "--commit-gate"]);
    assert_eq!(
        healed.status.code(),
        Some(0),
        "THE HALF THE OLD CONTROLS COULD NOT SHOW: with the drift repaired the gate \
         PASSES, so the refusal above is caused by the drift and not by the \
         fixture's shape: {}",
        said(&healed)
    );
    assert_ne!(
        drifted.status.code(),
        healed.status.code(),
        "the control VARIES with the world — the one property the old empty-corpus \
         control never had, because it refused over a workspace with nothing wrong \
         in it"
    );

    // AND THE FENCE FOLLOWS THE GATE, end to end through a real `git commit`. The
    // arms below prove the refusal direction; this proves a healthy world commits,
    // so the fence is not simply stuck shut.
    fx.place();
    let before = fx.head_count();
    let committed = fx.git(&["commit", "-m", "healthy world"], None);
    assert!(
        committed.status.success(),
        "a corpus with nothing wrong in it must COMMIT through the placed fence — a \
         fence that refuses everything is not a fence: {}",
        said(&committed)
    );
    assert_eq!(
        fx.head_count(),
        before + 1,
        "R40 — the commit is real, not merely a zero exit"
    );
}

// ── ROW 22 — the force value is PARSED, and the force path is RENDERED ───────

/// **The two-sided grammar, both sides in one run.**
///
/// Refusal side: every spelling an operator means as *"do NOT force"* must leave
/// the fence running, so the commit is refused. **All five of these FORCED under
/// the shipped `[ -n "${MRD_HOOK_FORCE:-}" ]`**, which reads whether a value was
/// typed and never what it says — so this half of the arm fails against the
/// pre-fix fence while the acceptance half passes unchanged.
///
/// Acceptance side: the spellings that mean force do force, in any case. Without
/// it, a fence that refused everything would pass the refusal half.
#[test]
fn the_force_value_is_parsed_and_both_sides_of_the_grammar_answer_apart() {
    let fx = Fixture::new("grammar");
    fx.place();
    fx.refusing();

    // REFUSAL ARMS — the fence runs, and this tree is one it refuses.
    for value in ["0", "false", "FALSE", "no", "off", " ", ""] {
        fx.dirty(&format!("# Plan\n\n## Goals\n\nrefusal {value:?}\n"));
        let before = fx.head_count();
        let out = fx.git(&["commit", "-m", "refusal arm"], Some(value));
        assert!(
            !out.status.success(),
            "MRD_HOOK_FORCE={value:?} means DO NOT FORCE, and the fence must run: {}",
            said(&out)
        );
        assert_eq!(
            fx.head_count(),
            before,
            "R40 — {value:?} produced a commit, so the gate opened on a value that says stop"
        );
        assert!(
            said(&out).contains("meridian fence"),
            "the refusal must be the FENCE's, not git's own: {}",
            said(&out)
        );
    }
    // The unset control, in the same run: it is not the empty string.
    fx.dirty("# Plan\n\n## Goals\n\nunset\n");
    let before = fx.head_count();
    assert!(!fx.git(&["commit", "-m", "unset"], None).status.success());
    assert_eq!(fx.head_count(), before);

    // ACCEPTANCE ARMS — the same tree, the same fence, the other verdict.
    for value in ["1", "true", "TRUE", "True", "yes", "on", " on "] {
        fx.dirty(&format!("# Plan\n\n## Goals\n\nacceptance {value:?}\n"));
        let before = fx.head_count();
        let out = fx.git(&["commit", "-m", "acceptance arm"], Some(value));
        assert!(
            out.status.success(),
            "MRD_HOOK_FORCE={value:?} is the ratified escape and must carry the commit: {}",
            said(&out)
        );
        assert_eq!(
            fx.head_count(),
            before + 1,
            "R40 — the escape must produce a commit, not merely print"
        );
    }
}

/// **The third leg: a value nobody can read is not permission.**
///
/// Under the shipped predicate every one of these FORCED, because non-emptiness
/// was the whole test. The fence fails closed instead, and names the value it
/// could not parse rather than guessing past it.
#[test]
fn an_unparseable_force_value_refuses_the_commit_and_names_itself() {
    let fx = Fixture::new("third-leg");
    fx.place();
    fx.refusing();

    for value in ["maybe", "2", "yolo", "t rue", "*"] {
        fx.dirty(&format!("# Plan\n\n## Goals\n\nunparseable {value:?}\n"));
        let before = fx.head_count();
        let out = fx.git(&["commit", "-m", "unparseable"], Some(value));
        assert!(
            !out.status.success(),
            "MRD_HOOK_FORCE={value:?} is not a decision and may not be read as one: {}",
            said(&out)
        );
        assert_eq!(fx.head_count(), before, "R40 — nothing may have committed");
        let text = said(&out);
        assert!(
            text.contains("does not parse") && text.contains(value),
            "the refusal names the value verbatim so the operator can see their own \
             typo: {text}"
        );
    }
}

/// **Rendered on the force path, and SILENT on every other path.**
///
/// The specificity half is the one that matters: a notice that fired
/// unconditionally would be no notice at all. A forced commit and an honest one
/// were indistinguishable afterwards, which is what made the escape unauditable.
#[test]
fn the_bypass_is_announced_and_only_the_bypass_is_announced() {
    let fx = Fixture::new("rendered");
    fx.place();
    fx.refusing();

    // ACCEPTANCE — a forced commit says so, on stderr, naming the value.
    fx.dirty("# Plan\n\n## Goals\n\nforced\n");
    let forced = fx.git(&["commit", "-m", "forced"], Some("yes"));
    assert!(forced.status.success(), "forced: {}", said(&forced));
    let text = said(&forced);
    assert!(
        text.contains("BYPASSED") && text.contains("NOTHING WAS CHECKED"),
        "a forced commit that printed nothing was indistinguishable from one that \
         passed the fence honestly: {text}"
    );
    assert!(
        text.contains("MRD_HOOK_FORCE=yes"),
        "the notice names the value that opened the gate: {text}"
    );

    // REFUSAL — git's own escape skips this file entirely, so the fence's notice
    // must NOT appear. This is the arm that fails if the notice is unconditional.
    fx.dirty("# Plan\n\n## Goals\n\nno-verify\n");
    let skipped = fx.git(&["commit", "--no-verify", "-m", "no-verify"], None);
    assert!(skipped.status.success(), "no-verify: {}", said(&skipped));
    assert!(
        !said(&skipped).contains("BYPASSED"),
        "the hook did not run at all, so it may not have spoken: {}",
        said(&skipped)
    );

    // REFUSAL — a fenced commit that the fence REFUSES must not claim a bypass.
    fx.dirty("# Plan\n\n## Goals\n\nfenced\n");
    let refused = fx.git(&["commit", "-m", "fenced"], Some("off"));
    assert!(!refused.status.success());
    assert!(
        !said(&refused).contains("BYPASSED"),
        "the fence ran and refused; announcing a bypass here would be a record of an \
         intent the operator never expressed: {}",
        said(&refused)
    );
}

// ── ROW 20 — the door set is a claim about coverage ──────────────────────────

/// **Following the document's placement instruction leaves every door git
/// dispatches for a commit built from a prepared index carrying the fence, and
/// carrying the SAME bytes — executable.**
///
/// The mode bit is the half a reader most easily drops: a hook git cannot execute
/// is a hook git SKIPS, silently, so a placement that forgot the `chmod` produces
/// a checkout that reports `installed` and fences nothing.
///
/// The absence claim has its positive control in the same assertion: `pre-commit`
/// is checked alongside the two that were missing, so zero hits from a broken
/// fixture cannot look like a clean result.
#[test]
fn placing_covers_every_door_and_one_body_serves_them_all() {
    let fx = Fixture::new("doors");
    fx.place();

    // NAMED, not read off the constant under test. An arm parameterised by the
    // set it is measuring cannot fail when that set shrinks — measured: with
    // `FENCED_HOOKS` cut back to `["pre-commit"]` the `for name in FENCED_HOOKS`
    // form below still passed, and only these three literals caught it.
    for name in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
        assert!(
            fx.door(name).exists(),
            "{name} is a door git dispatches for a commit built from a prepared index"
        );
    }

    let mut bodies = Vec::new();
    for name in FENCED_HOOKS {
        let path = fx.door(name);
        assert!(
            path.exists(),
            "{name} is a door git dispatches for a commit it builds from a prepared \
             index, and a door set without it is a bypass"
        );
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "{name}: mode {mode:o}");
        bodies.push(std::fs::read_to_string(&path).expect("read"));
    }
    assert!(
        bodies.windows(2).all(|w| w[0] == w[1]),
        "one body serves three doors — a per-door body would be three fences to keep \
         in step and three generations to reconcile"
    );

    // The count, from the disk rather than from the engine's own report.
    let non_sample = std::fs::read_dir(common_dir(&fx.ws).join("hooks"))
        .expect("hooks dir")
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().ends_with(".sample"))
        .count();
    assert_eq!(
        non_sample,
        FENCED_HOOKS.len(),
        "unit: non-sample hook files"
    );
}

/// **`git merge` and `git am` are refused; `git cherry-pick` LANDS, as declared.**
///
/// The merge and the patch are the doors that closed: both landed commits past
/// the shipped install set with the fence printing nothing. The cherry-pick is
/// the declared limit, asserted so it cannot regress into a silent surprise and
/// so a future reader sees it was chosen rather than missed.
#[test]
fn the_closed_doors_refuse_and_the_declared_open_one_still_lands() {
    let fx = Fixture::new("merge");
    fx.place();
    fx.refusing();

    // A side branch, built past the fence with git's own escape so the setup does
    // not depend on the thing under test.
    git_ok(&fx.ws, &["checkout", "-q", "-b", "side"]);
    fx.write("side.md", "# Side\n\nside work\n");
    git_ok(&fx.ws, &["add", "-A"]);
    let out = fx.git(&["commit", "--no-verify", "-m", "side"], None);
    assert!(out.status.success(), "setup commit: {}", said(&out));
    let side_sha = git_out(&fx.ws, &["rev-parse", "HEAD"]).trim().to_owned();
    git_ok(&fx.ws, &["checkout", "-q", "main"]);

    // REFUSAL — the merge commit git would build from a prepared index.
    let before = fx.head_count();
    let merged = fx.git(&["merge", "--no-ff", "-m", "merge side", "side"], None);
    assert!(
        !merged.status.success(),
        "a merge commit landed past the fence — this is the shipped defect: {}",
        said(&merged)
    );
    assert_eq!(
        fx.head_count(),
        before,
        "R40 — HEAD moved, so the merge commit landed whatever the exit said"
    );
    assert!(
        said(&merged).contains("meridian fence"),
        "the refusal is the fence's, and it printed nothing at all before: {}",
        said(&merged)
    );
    git_ok(&fx.ws, &["merge", "--abort"]);

    // REFUSAL — `git am`, through pre-applypatch.
    let patch = fx.tmp.path().join("side.patch");
    std::fs::write(
        &patch,
        git_out(&fx.ws, &["format-patch", "--stdout", "-1", &side_sha]),
    )
    .expect("write patch");
    let before = fx.head_count();
    let applied = fx.git(&["am", &patch.display().to_string()], None);
    assert!(
        !applied.status.success(),
        "`git am` landed a commit past the fence: {}",
        said(&applied)
    );
    assert_eq!(fx.head_count(), before, "R40 — HEAD must not have moved");
    let _ = fx.git(&["am", "--abort"], None);

    // ACCEPTANCE OF THE DECLARED LIMIT — cherry-pick dispatches no veto-capable
    // hook that can read the index, and this states it out loud. If a future
    // change closes this door, this arm fails and the declaration gets rewritten
    // deliberately instead of drifting.
    let before = fx.head_count();
    let picked = fx.git(&["cherry-pick", &side_sha], None);
    assert!(
        picked.status.success(),
        "the declared limit: cherry-pick is not a path git offers a veto on: {}",
        said(&picked)
    );
    assert_eq!(fx.head_count(), before + 1, "and it produced a commit");
}

/// **A root carrying only `pre-commit` reports the PARTIAL state, not
/// `installed`** — and it is a real bypass, proved by a real `git merge` landing
/// through the door nobody placed.
///
/// The reporting half alone would be satisfied by a word; the merge is what makes
/// `installed-partial` mean something. Completing the set closes it, in the same
/// run, so the arm cannot pass by refusing everything.
#[test]
fn a_root_fenced_at_one_door_is_partial_and_a_merge_walks_through_the_others() {
    let fx = Fixture::new("partial");
    fx.place_at(&["pre-commit"]);
    fx.refusing();

    let json = fx.fence_json();
    assert_eq!(
        json["state"], "installed-partial",
        "reporting this as `installed` is the coverage claim that was false: {json}"
    );
    let teaching = json["teaching"]
        .as_str()
        .expect("a partial set owes a teaching");
    for name in FENCED_HOOKS.iter().skip(1) {
        assert!(
            teaching.contains(name),
            "the teaching names {name}: {teaching}"
        );
    }

    // THE BYPASS IS REAL, not merely reported. A side branch merged in dispatches
    // `pre-merge-commit`, and there is nothing standing in that door.
    git_ok(&fx.ws, &["checkout", "-q", "-b", "side"]);
    fx.write("side.md", "# Side\n\nside work\n");
    git_ok(&fx.ws, &["add", "-A"]);
    let out = fx.git(&["commit", "--no-verify", "-m", "side"], None);
    assert!(out.status.success(), "setup commit: {}", said(&out));
    git_ok(&fx.ws, &["checkout", "-q", "main"]);

    let before = fx.head_count();
    let merged = fx.git(&["merge", "--no-ff", "-m", "merge side", "side"], None);
    // `> before` rather than `+ 1`: a --no-ff merge makes the side branch's own
    // commits reachable too, so the count grows by more than the merge commit.
    assert!(
        merged.status.success() && fx.head_count() > before,
        "the open door is a BYPASS, and this arm exists to show it is not a wording: {}",
        said(&merged)
    );

    // ACCEPTANCE, same run: placing at every door closes it, and the same merge
    // shape is refused. Without this half the arm passes on a fence that refuses
    // nothing at all.
    fx.place();
    assert_eq!(fx.fence_json()["state"], "installed");
    git_ok(&fx.ws, &["checkout", "-q", "side"]);
    fx.write("side.md", "# Side\n\nmore side work\n");
    git_ok(&fx.ws, &["add", "-A"]);
    let out = fx.git(&["commit", "--no-verify", "-m", "side 2"], None);
    assert!(out.status.success(), "setup commit: {}", said(&out));
    git_ok(&fx.ws, &["checkout", "-q", "main"]);

    let before = fx.head_count();
    let merged = fx.git(&["merge", "--no-ff", "-m", "merge side 2", "side"], None);
    assert!(
        !merged.status.success() && fx.head_count() == before,
        "the completed set must refuse the merge the partial one let through: {}",
        said(&merged)
    );
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn git_ok(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?}: {}", said(&out));
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?}: {}", said(&out));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn common_dir(dir: &Path) -> PathBuf {
    let raw = git_out(dir, &["rev-parse", "--git-common-dir"]);
    let path = PathBuf::from(raw.trim());
    if path.is_absolute() {
        path
    } else {
        dir.join(path)
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Everything the process said, both streams — a refusal read off one stream is
/// half an observation.
fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
