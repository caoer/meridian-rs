//! **The hook plane's gates, each proved able to say NO.**
//!
//! Every arm here drives the REAL installed fence through a REAL `git` operation
//! and reads what git did. Nothing is asserted about a hand-transcribed hook: the
//! artifact under test is the file `mrd hook install` writes, and where a fixture
//! needs bytes the installer would not write today (a fence from the future) it is
//! **derived from the installed file by editing one datum**, never typed out.
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
//! - `git merge` and `git am` landed commits past an install set of one;
//! - a fence from a newer engine reported `installed` and `install` overwrote it.
//!
//! # THE INSTRUMENT'S OWN CONTROL
//! These arms need a tree the fence is actively refusing, so anything that lands
//! landed past a fence that was trying to say no. A scratch workspace with no
//! receipt journal is that tree: `mrd check --staged` answers
//! `grey(cannot-assess)` and refuses. **That precondition is ASSERTED, never
//! assumed** — [`Fixture::refusing`] runs the verb directly and fails loudly if it
//! ever stops refusing, because a green `mrd check` would make every refusal arm
//! below pass for the wrong reason and look exactly like a working fence.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mrd::hook::{FENCED_HOOKS, FENCE_VERSION};

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
        git_ok(&fixture.ws, &["config", "user.email", "fence@example.invalid"]);
        git_ok(&fixture.ws, &["config", "user.name", "fence"]);
        fixture.write("plan.md", "# Plan\n\n## Goals\n\nalpha\n");
        let init = fixture.mrd(&["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        git_ok(&fixture.ws, &["add", "-A"]);
        git_ok(&fixture.ws, &["commit", "-qm", "corpus"]);
        fixture
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

    /// `mrd` with the ratified escape set — the CLI half of the same grammar the
    /// fence body runs.
    fn mrd_forced(&self, args: &[&str], force: &str) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MRD_HOOK_FORCE", force)
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

    fn install(&self) -> Output {
        let out = self.mrd(&["hook", "install"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "hook install on a clean repo: {}",
            said(&out)
        );
        out
    }

    /// **The instrument's control, asserted rather than assumed.** Every refusal
    /// arm below rests on this tree being one the fence is actively refusing; a
    /// green `mrd check --staged` would make all of them pass for the wrong
    /// reason, and pass silently.
    fn refusing(&self) {
        let out = self.mrd(&["check", "--staged"]);
        assert_ne!(
            out.status.code(),
            Some(0),
            "the control failed: this fixture's tree must be one `mrd check --staged` \
             REFUSES, or every arm resting on it measures nothing. Output: {}",
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

    fn status_json(&self) -> serde_json::Value {
        let out = self.mrd(&["hook", "status", "--json"]);
        serde_json::from_str(&stdout(&out)).unwrap_or_else(|e| {
            panic!("hook status --json is not json ({e}): {}", said(&out));
        })
    }
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
    fx.install();
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
    fx.install();
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
    fx.install();
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

// ── ROW 20 — the install set is a claim about coverage ───────────────────────

/// **Every door git dispatches for a commit built from a prepared index carries
/// the fence, and they carry the SAME bytes.**
///
/// The absence claim has its positive control in the same assertion: `pre-commit`
/// is checked alongside the two that were missing, so zero hits from a broken
/// fixture cannot look like a clean result.
#[test]
fn install_covers_every_door_and_one_body_serves_them_all() {
    let fx = Fixture::new("doors");
    fx.install();

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
             index, and an install set without it is a bypass"
        );
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&path).expect("stat").permissions(),
        );
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
    assert_eq!(non_sample, FENCED_HOOKS.len(), "unit: non-sample hook files");
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
    fx.install();
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
/// `installed`** — the migration every already-fenced root is in, said out loud.
#[test]
fn a_root_fenced_at_one_door_is_partial_and_install_completes_it() {
    let fx = Fixture::new("partial");
    fx.install();
    // Reduce the set to what the previous install wrote, from the installer's own
    // bytes rather than a transcription.
    for name in FENCED_HOOKS.iter().skip(1) {
        std::fs::remove_file(fx.door(name)).expect("remove");
    }

    let json = fx.status_json();
    assert_eq!(
        json["state"], "installed-partial",
        "reporting this as `installed` is the coverage claim that was false: {json}"
    );
    let teaching = json["detail"].as_str().expect("a partial set owes a teaching");
    for name in FENCED_HOOKS.iter().skip(1) {
        assert!(teaching.contains(name), "the teaching names {name}: {teaching}");
    }

    // ACCEPTANCE, same run: install completes it and SAYS it completed rather
    // than reporting an idempotent refresh that did not happen.
    let out = fx.install();
    assert!(
        stdout(&out).contains("completed"),
        "a migration is not a refresh, and reporting it as one hides the doors that \
         were open until just now: {}",
        said(&out)
    );
    assert_eq!(fx.status_json()["state"], "installed");
}

/// **Uninstall removes every door, and refuses the whole set when any one of them
/// is foreign** — naming which. A partial teardown on the strength of a decision
/// the next door reverses is the overwrite defect wearing the other sign.
#[test]
fn uninstall_clears_the_set_and_a_foreign_door_refuses_the_whole_teardown() {
    let fx = Fixture::new("teardown");
    fx.install();

    // REFUSAL — a foreign hook at the SECOND door, with an owned first door as
    // the control: the refusal must name the foreign one, not the owned one.
    let foreign = fx.door("pre-merge-commit");
    std::fs::write(&foreign, "#!/bin/sh\n# LEFTHOOK: do not remove\nexit 0\n").expect("write");
    let out = fx.mrd(&["hook", "uninstall"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    let text = said(&out);
    assert!(
        text.contains("foreign-hook") && text.contains("pre-merge-commit"),
        "a foreign hook beside an owned one must name WHICH door is foreign: {text}"
    );
    assert!(
        fx.door("pre-commit").exists(),
        "R40 — the refusal must leave the set untouched, not half torn down"
    );
    assert!(foreign.exists(), "R40 — the foreign file is still there");

    // ACCEPTANCE — with the foreign file gone, uninstall clears every door.
    std::fs::remove_file(&foreign).expect("remove");
    let out = fx.mrd(&["hook", "uninstall"]);
    assert_eq!(out.status.code(), Some(0), "uninstall: {}", said(&out));
    for name in FENCED_HOOKS {
        assert!(
            !fx.door(name).exists(),
            "R40 — uninstall exited 0 with {name} still on disk"
        );
    }
    assert_eq!(fx.status_json()["state"], "absent");
}

// ── ROWS 23 + 26 — the version line is the datum the reader reads ────────────

/// **The fence's own generation is reported, beside the generation of the engine
/// that judged it.**
///
/// `# mrd-hook-fence <n>` has been in the bytes since the plane shipped and
/// nothing parsed it: the reader decided by marker, so a fence from another
/// generation answered "installed". A verdict that does not disclose its judge
/// cannot be checked by a third party, which is the only way a skew is ever
/// caught — in a skew both participants are inside it.
#[test]
fn status_reports_the_fence_the_file_declares_and_the_engine_that_judged_it() {
    let fx = Fixture::new("versions");
    fx.install();

    let json = fx.status_json();
    assert_eq!(json["state"], "installed");
    assert_eq!(
        json["fence_version"], FENCE_VERSION,
        "the number the FILE declares: {json}"
    );
    assert_eq!(
        json["engine_version"], FENCE_VERSION,
        "and the number THIS ENGINE writes, so a third party can compare them: {json}"
    );
    let doors = json["hooks"].as_array().expect("the install set");
    assert_eq!(doors.len(), FENCED_HOOKS.len());
    for (door, name) in doors.iter().zip(FENCED_HOOKS) {
        assert_eq!(door["name"], name);
        assert_eq!(door["state"], "installed");
        assert_eq!(door["fence_version"], FENCE_VERSION);
    }
}

/// **A fence from the FUTURE is `installed-ahead`, and `install` refuses to
/// downgrade it.**
///
/// This is the skew's own scenario. An operator whose `mrd` is behind the fence
/// reads a refusal telling them to run `mrd hook install`, runs it, and the OLD
/// engine writes the OLD fence — silently restoring the worktree-reading false
/// green. The word did not exist and the guard did not either.
///
/// The fixture is the installer's own bytes with **one datum edited**, so this
/// arm speaks about the fence and not about a transcription of it.
///
/// # The anti-vacuity control, in the same run
/// `installed-ahead` could be a constant. So the same fixture is first measured
/// at the engine's own generation and must report `installed` with the install
/// accepted — the two verdicts differ only by the digit on line 2.
#[test]
fn a_fence_from_the_future_refuses_the_downgrade_and_says_which_way_the_skew_runs() {
    let fx = Fixture::new("ahead");
    fx.install();

    // CONTROL — at this engine's own generation, everything is ordinary.
    assert_eq!(fx.status_json()["state"], "installed");
    assert_eq!(fx.mrd(&["hook", "install"]).status.code(), Some(0));

    // The fence a NEWER engine would have written: the installed bytes, one datum
    // changed.
    let future = FENCE_VERSION + 7;
    for name in FENCED_HOOKS {
        let path = fx.door(name);
        let body = std::fs::read_to_string(&path).expect("read the installed fence");
        let bumped = body.replace(
            &format!("# mrd-hook-fence {FENCE_VERSION}"),
            &format!("# mrd-hook-fence {future}"),
        );
        assert_ne!(bumped, body, "the fixture must actually change the datum");
        std::fs::write(&path, &bumped).expect("write");
    }

    // REFUSAL — the report inverts.
    let json = fx.status_json();
    assert_eq!(
        json["state"], "installed-ahead",
        "the equality test collapsed `older` and `newer` into one `false` and then \
         asserted a direction it never measured: {json}"
    );
    assert_eq!(json["fence_version"], future);
    assert_eq!(json["engine_version"], FENCE_VERSION);
    let teaching = json["detail"].as_str().expect("the skew owes a teaching");
    assert!(
        teaching.contains("do NOT run `mrd hook install`"),
        "every other state's remedy is `install`; this one's is the reverse: {teaching}"
    );

    // REFUSAL — and install declines to write the older fence.
    let before = std::fs::read_to_string(fx.door("pre-commit")).expect("read");
    let out = fx.mrd(&["hook", "install"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    assert!(
        said(&out).contains("fence-ahead"),
        "the reason word names the observed state: {}",
        said(&out)
    );
    assert_eq!(
        std::fs::read_to_string(fx.door("pre-commit")).expect("read"),
        before,
        "R40 — the refusal must leave the newer fence byte-identical"
    );

    // ACCEPTANCE — a deliberate rollback stays possible through the ratified
    // escape, and is never silent about being one.
    let out = fx.mrd_forced(&["hook", "install"], "1");
    assert_eq!(out.status.code(), Some(0), "forced rollback: {}", said(&out));
    assert_eq!(fx.status_json()["fence_version"], FENCE_VERSION);
    // And an UNPARSEABLE escape is not permission — the same fail-closed law the
    // fence body's third leg runs, in its Rust spelling.
    for name in FENCED_HOOKS {
        let path = fx.door(name);
        let body = std::fs::read_to_string(&path).expect("read");
        std::fs::write(
            &path,
            body.replace(
                &format!("# mrd-hook-fence {FENCE_VERSION}"),
                &format!("# mrd-hook-fence {future}"),
            ),
        )
        .expect("write");
    }
    let out = fx.mrd_forced(&["hook", "install"], "yolo");
    assert_eq!(
        out.status.code(),
        Some(1),
        "an unreadable escape is not permission: {}",
        said(&out)
    );
}

/// **A marker-bearing fence that declares no generation refuses rather than
/// guessing** — and the report never substitutes the asking engine's number.
///
/// An undeclarable generation is not a known-old one. The plane's standing rule
/// is that an unreadable file is not an absent one; the same reading applies to
/// an unreadable datum inside a readable file.
///
/// # The fixture, and what it is NOT
/// The marker and the version live on the same line, so deleting that line makes
/// the file foreign rather than unversioned — a different state with a different
/// word, and the arm that measured it first is what found this out. The reachable
/// state is a generation that declares itself with something no `u32` parses,
/// which is what a future fence tagging itself `next` would look like to this
/// engine.
#[test]
fn an_undeclarable_generation_refuses_and_is_never_read_as_this_engines() {
    let fx = Fixture::new("unversioned");
    fx.install();
    for name in FENCED_HOOKS {
        let path = fx.door(name);
        let body = std::fs::read_to_string(&path).expect("read");
        let tagged = body.replace(
            &format!("# mrd-hook-fence {FENCE_VERSION}"),
            "# mrd-hook-fence next",
        );
        assert_ne!(tagged, body, "the fixture must actually change the datum");
        assert!(
            tagged.contains("mrd-hook-fence"),
            "the control: the marker must SURVIVE, or this measures `foreign-hook` \
             instead — which is what the first draft of this arm did"
        );
        std::fs::write(&path, tagged).expect("write");
    }

    let json = fx.status_json();
    assert_eq!(json["state"], "installed-unversioned", "{json}");
    assert!(
        json["fence_version"].is_null(),
        "a fence that cannot say what it is has not said it is current, and the \
         asking engine's number may never stand in for the file's: {json}"
    );
    assert_eq!(json["engine_version"], FENCE_VERSION);

    let out = fx.mrd(&["hook", "install"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    assert!(said(&out).contains("fence-unversioned"), "{}", said(&out));
}

/// **A fence OLDER than this engine is `installed-superseded`, and install
/// refreshes it.** The specificity control for the two arms above: a change that
/// made `installed-ahead` reachable by breaking this would have moved the defect
/// rather than closed it.
#[test]
fn an_older_fence_is_superseded_and_install_still_refreshes_it() {
    let fx = Fixture::new("superseded");
    fx.install();
    for name in FENCED_HOOKS {
        let path = fx.door(name);
        let body = std::fs::read_to_string(&path).expect("read");
        std::fs::write(
            &path,
            body.replace(
                &format!("# mrd-hook-fence {FENCE_VERSION}"),
                "# mrd-hook-fence 1",
            ),
        )
        .expect("write");
    }

    let json = fx.status_json();
    assert_eq!(json["state"], "installed-superseded", "{json}");
    assert_eq!(json["fence_version"], 1);
    assert!(
        json["detail"]
            .as_str()
            .expect("a teaching")
            .contains("refreshes it"),
        "the remedy here IS install, and the operator has to be told so: {json}"
    );

    assert_eq!(fx.mrd(&["hook", "install"]).status.code(), Some(0));
    assert_eq!(fx.status_json()["fence_version"], FENCE_VERSION);
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
