//! The fence body's gates, each proved able to say no — as placed from the emitted contract.
//!
//! Every arm drives a real fence through a real `git` operation and reads what git did. The
//! artifact under test is the body `mrd skill hook` emits, extracted from that verb's stdout
//! exactly as its document tells a reader to and placed exactly where it says
//! ([`Fixture::place`]). `crates/mrd/tests/skill_hook_emit.rs` measures the document's
//! claims; this file measures whether following them fences a repository.
//!
//! A hook fix tested by running the hook and asserting exit 0 has built an instrument with
//! precisely the defect it was sent to fix: a silently-exiting broken predicate and a
//! legitimately passing hook are indistinguishable at the exit code. So every gate carries
//! the arm that proves it says yes and the arm that proves it says no, in the same run.
//!
//! The refusing tree is asserted, never assumed — [`Fixture::refusing`] runs the verb the
//! body runs and fails loudly if it ever stops refusing. The gate reads the pin plane alone
//! and a workspace with no pins is vacuously clean, so the fixture constructs a refusal:
//! [`Fixture::drift_a_pin`] pins real content and rewrites it out of band. The opt-in
//! `--require-pins` flag is deliberately not used here: the shipped fence body runs bare
//! `mrd check --commit-gate`, and a control on a different invocation is a control over a
//! different verdict (the flag's coverage lives in `crates/mrd/tests/s4r19_commit_gate.rs`).

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mrd::hook::FENCED_HOOKS;

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// The system directories a `git commit` needs. The fixtures `bin/` is prepended, so our `mrd`
/// shadows any deployed one — the ordering is the isolation.
mod system_path;
use system_path::system_path;
mod common;

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

    /// Build the refusing tree the whole file rests on. Pin `claim.md` to a section of
    /// `source.md` through the shipped CLI, then rewrite that section by hand — an out-of-band
    /// edit to pinned content, which the pin plane reads as `red content-drifted`. The gate
    /// the fence runs therefore refuses, and it refuses over a fact about this corpus.
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

    /// Undo [`Fixture::drift_a_pin`] — restore the pinned section to the bytes the lock names,
    /// and stage that. Exists for [`the_refusing_fixture_is_two_directional`]: a control that
    /// only ever refuses has not been shown to depend on the thing it guards.
    fn heal_the_pin(&self) {
        self.write("source.md", "# Source\n\n## Guideline\n\nthe pinned body\n");
        git_ok(&self.ws, &["add", "-A"]);
    }

    fn hook_path(&self) -> String {
        format!("{}:{}", self.bin.display(), system_path())
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

    /// The instruments control, asserted rather than assumed. Every refusal arm below rests on
    /// this tree being one the fence is actively refusing; a green answer would make all of
    /// them pass for the wrong reason, and pass silently. It asks the question the fence asks.
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

    /// The checkouts fence coverage, off the face that reports it: `mrd check`s `fence` block.
    fn fence_json(&self) -> serde_json::Value {
        let out = self.mrd(&["check", "--json"]);
        let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap_or_else(|e| {
            panic!("check --json is not json ({e}): {}", said(&out));
        });
        value["fence"].clone()
    }
}

/// The fence body, extracted the way the document says to extract it: the one fenced block, and
/// it is the file. `crates/mrd/tests/skill_hook_emit.rs` holds the document to there being
/// exactly one.
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

/// The condition on this file: the control varies with the world. Every other arm rests on
/// [`Fixture::refusing`], and a control that refuses is worth nothing until it is also shown
/// to pass when nothing is wrong.
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

    // AND THE FENCE FOLLOWS THE GATE, end to end through a real `git commit`. The arms below prove
    // the refusal direction; this proves a healthy world commits, so the fence is not simply stuck
    // shut.
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

/// The two-sided grammar, both sides in one run. Refusal side: every spelling an operator
/// means as "do NOT force" must leave the fence running, so the commit is refused.
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

/// The third leg: a value nobody can read is not permission. The fence fails closed and names
/// the value it could not parse rather than guessing past it.
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

/// Rendered on the force path, and silent on every other path. The specificity half is the
/// one that matters: a notice that fired unconditionally would be no notice at all.
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

    // REFUSAL — git's own escape skips this file entirely, so the fence's notice must NOT appear.
    // This is the arm that fails if the notice is unconditional.
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

/// Following the documents placement instruction leaves every door git dispatches for a
/// commit built from a prepared index carrying the fence — the same bytes — executable.
#[test]
fn placing_covers_every_door_and_one_body_serves_them_all() {
    let fx = Fixture::new("doors");
    fx.place();

    // Named literals, not read off the constant under test: an arm parameterised by the set
    // it is measuring cannot fail when that set shrinks.
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

/// `git merge` and `git am` are refused; `git cherry-pick` lands, as declared. The
/// cherry-pick is the declared limit, asserted so it cannot regress into a silent surprise
/// and so a future reader sees it was chosen rather than missed.
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

    // ACCEPTANCE OF THE DECLARED LIMIT — cherry-pick dispatches no veto-capable hook that can
    // read the index. If a future change closes this door, this arm fails and the declaration
    // gets rewritten deliberately instead of drifting.
    let before = fx.head_count();
    let picked = fx.git(&["cherry-pick", &side_sha], None);
    assert!(
        picked.status.success(),
        "the declared limit: cherry-pick is not a path git offers a veto on: {}",
        said(&picked)
    );
    assert_eq!(fx.head_count(), before + 1, "and it produced a commit");
}

/// A root carrying only `pre-commit` reports the partial state, not `installed` — and it is
/// a real bypass, proved by a real `git merge` landing through the door nobody placed.
/// Completing the set closes it, in the same run, so the arm cannot pass by refusing
/// everything.
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
    // `> before` rather than `+ 1`: a --no-ff merge makes the side branch's own commits reachable
    // too, so the count grows by more than the merge commit.
    assert!(
        merged.status.success() && fx.head_count() > before,
        "the open door is a BYPASS, and this arm exists to show it is not a wording: {}",
        said(&merged)
    );

    // ACCEPTANCE, same run: placing at every door closes it, and the same merge shape is
    // refused. Without this half the arm passes on a fence that refuses nothing at all.
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

impl Drop for Fixture {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
