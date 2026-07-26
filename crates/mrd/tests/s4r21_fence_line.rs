//! **Row 21 — `mrd check` reports the CHECKOUT's fence coverage, unasked, and the
//! report cannot reach the exit.** End-to-end over the REAL binary
//! (`CARGO_BIN_EXE_mrd`), driving the shipped CLI only.
//!
//! # The defect this closes is the SILENCE, not the absence
//! `$GIT_DIR/hooks` is never a tracked path, so no clone, fetch or pull can carry
//! the fence. That is ruled to stay true: coverage is **per-checkout and opt-in,
//! permanently**, because the automatic route (a global `init.templateDir`) fences
//! every unrelated repository the operator ever clones or inits. So a fresh clone
//! is unfenced BY DESIGN — and until this unit, nothing on the `mrd check` surface
//! said so.
//!
//! # The exit code is the load-bearing claim, and it has its own arm
//! Fence state is a proposition about the **local checkout's configuration**, not
//! about the corpus's bytes or their write history. It never competed for the exit
//! code and must never reach it: colouring `check` on an unfenced checkout would
//! make governance unreachable in every fresh clone.
//! [`the_fence_line_never_reaches_the_exit_code`] reads ONE corpus twice — unfenced,
//! then fully fenced — over two corpus states, and fails if the codes ever part.
//!
//! # Three doors can disagree, and a line that reads door one is the defect rebuilt
//! The install set is `pre-commit`, `pre-merge-commit` and `pre-applypatch`. Every
//! per-door assert here **spells the three names as literals**: an arm that
//! iterated the set under test could not fail when the set shrank, which is the
//! vacuity this fleet bought the hard way.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// A sandbox whose caches are its own: a non-deployed `mrd` that registered in the
/// shared `~/.cache/meridian` would become the host's resident daemon.
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

    /// A real git repository that is also a meridian workspace, carrying a
    /// pinnable source section and a claim page — the corpus a governed `mrd pin`
    /// turns green. **Committed, and NOT fenced**: every arm installs the fence
    /// itself, or deliberately does not.
    fn corpus(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git_ok(&ws, &["init", "-q"]);
        git_ok(&ws, &["config", "user.email", "row21@example.invalid"]);
        git_ok(&ws, &["config", "user.name", "row21"]);
        write(&ws, "source.md", "# Source\n\n## Guideline\n\nthe body\n");
        write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", said(&init));
        git_ok(&ws, &["add", "-A"]);
        git_ok(&ws, &["commit", "-qm", "corpus"]);
        ws
    }

    /// A meridian workspace that is not a git repository at all — a supported
    /// state, and the one root with no door plane to read.
    fn plain(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        write(&ws, "a.md", "# A\n\nalpha\n");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", said(&init));
        ws
    }

    /// Fence the checkout through the REAL `mrd hook install`, and assert the
    /// state change on disk rather than the command's exit (R40).
    fn fence(&self, ws: &Path) {
        let out = self.run(ws, &["hook", "install"]);
        assert_eq!(out.status.code(), Some(0), "hook install: {}", said(&out));
        for door in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
            assert!(
                hooks_dir(ws).join(door).exists(),
                "the fixture IS the assert's subject: {door} must be on disk"
            );
        }
    }

    /// The governed write that gives this corpus a current baseline, so `mrd check`
    /// reads it clean and exits 0. Driven through the shipped CLI.
    fn govern(&self, ws: &Path) {
        let pin = self.run(ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
        assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
        assert!(
            std::fs::read_to_string(ws.join("claim.md"))
                .expect("claim")
                .contains("meridian-lock"),
            "R40 — the governed write landed bytes"
        );
    }
}

fn git_ok(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git runs in the test environment");
    assert!(status.success(), "git {args:?}");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs in the test environment");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Where this checkout's hooks actually live — `git`'s own answer, never a
/// `.git/hooks` guess that a linked worktree would falsify.
fn hooks_dir(ws: &Path) -> PathBuf {
    let common = PathBuf::from(git_out(ws, &["rev-parse", "--git-common-dir"]));
    if common.is_absolute() {
        common.join("hooks")
    } else {
        ws.join(common).join("hooks")
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn said(out: &Output) -> String {
    format!("{}{}", stdout(out), String::from_utf8_lossy(&out.stderr))
}

/// The `  fence:` line — the set's reading. Panics when it is missing, because
/// **the line's absence is the defect this whole unit is about** and a helper that
/// returned an empty string for it would let every arm below pass on the silence.
fn fence_line(out: &Output) -> String {
    stdout(out)
        .lines()
        .find(|l| l.starts_with("  fence: "))
        .unwrap_or_else(|| panic!("mrd check printed no fence line:\n{}", said(out)))
        .to_owned()
}

/// The `  fence doors:` line — the per-door reading, or `None` when this root has
/// no door plane. The two are DIFFERENT facts and this helper keeps them so.
fn fence_doors_line(out: &Output) -> Option<String> {
    stdout(out)
        .lines()
        .find(|l| l.starts_with("  fence doors: "))
        .map(str::to_owned)
}

/// The `fence` block off the `--json` face.
fn fence_json(out: &Output) -> serde_json::Value {
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("mrd check --json did not parse ({e}):\n{}", said(out)));
    value["fence"].clone()
}

// ── the central claim: the line is beside the verdict, never part of it ──────

/// **The card's central claim, with an arm that fails if the line ever leaks.**
///
/// ONE checkout is read twice — unfenced, then fully fenced — with nothing else
/// changed: same bytes, same journal, same corpus. The exit code must be identical
/// across the pair, and the fence WORD must differ across it. The second assert is
/// the anti-vacuity control: without it this arm would pass on a build that never
/// read the fence at all.
///
/// **Both corpus states, because a leak that only reddens hides inside a corpus
/// that already refuses.** A green corpus catches `0 → 1`; a refusing corpus
/// catches a leak that would have flipped the other way.
///
/// # The assert ORDER is part of the instrument
/// The pair is compared to ITSELF before it is compared to an expected code. A
/// leak reddens the unfenced run, so an arm that asserted the expected code first
/// would fail saying *"the fixture is not green"* — true, useless, and pointing at
/// the corpus instead of at the leak. Comparing the pair first makes the failure
/// name the thing that broke.
#[test]
fn the_fence_line_never_reaches_the_exit_code() {
    let sb = sandbox();
    // (a) the GREEN corpus — a governed pin gives it a current baseline, so this
    //     pair is where a leak that REDDENS is visible.
    // (b) the REFUSING corpus — already exits 1, so a leak that would have flipped
    //     the other way has an arm that sees it too.
    for (name, drift, expected) in [("exit-green", false, 0), ("exit-refusing", true, 1)] {
        let ws = sb.corpus(name);
        sb.govern(&ws);
        if drift {
            write(
                &ws,
                "source.md",
                "# Source\n\n## Guideline\n\nOUT OF BAND\n",
            );
        }

        let unfenced = sb.run(&ws, &["check"]);
        sb.fence(&ws);
        let fenced = sb.run(&ws, &["check"]);

        // The anti-vacuity control: without this pair of asserts the claim below
        // would pass on a build that never read a fence at all.
        assert!(
            fence_line(&unfenced).contains("fence: absent"),
            "[{name}] the control: this checkout really was unfenced: {}",
            fence_line(&unfenced)
        );
        assert!(
            fence_line(&fenced).contains("fence: installed"),
            "[{name}] the control's other half: the SAME checkout is now fenced: {}",
            fence_line(&fenced)
        );

        // THE CLAIM, asserted before any expected value: the fence state changed
        // and the exit code did not.
        assert_eq!(
            unfenced.status.code(),
            fenced.status.code(),
            "[{name}] THE CLAIM: the fence state changed and the exit code did \
             not. unfenced exited {:?}, fenced exited {:?} — the fence line has \
             leaked into the exit.\nunfenced said: {}\nfenced said: {}",
            unfenced.status.code(),
            fenced.status.code(),
            said(&unfenced),
            said(&fenced)
        );
        // And only now the fixture: the pair agrees, and it agrees on the code
        // this corpus state earns on its own.
        assert_eq!(
            fenced.status.code(),
            Some(expected),
            "[{name}] FIXTURE: this corpus state earns exit {expected}: {}",
            said(&fenced)
        );
    }
}

// ── the reading itself, state by state ──────────────────────────────────────

/// **The arm that closes the card**: a fresh clone of a fenced root reports the
/// unfenced state **without being asked** — a bare `mrd check`, no `hook status`,
/// no flag.
///
/// The mechanism is asserted beside the reading (R40): the source's hooks are on
/// disk and the clone's hook directory carries none of them, which is git's design
/// and the fact the line exists to stop being silent about.
#[test]
fn a_fresh_clone_of_a_fenced_root_says_it_is_unfenced_without_being_asked() {
    let sb = sandbox();
    let source = sb.corpus("clone-source");
    sb.govern(&source);
    git_ok(&source, &["add", "-A"]);
    git_ok(&source, &["commit", "-qm", "governed"]);
    sb.fence(&source);

    let clone = sb.tmp.path().join("clone-fresh");
    let cloned = Command::new("git")
        .args(["clone", "-q"])
        .arg(&source)
        .arg(&clone)
        .output()
        .expect("git clone");
    assert!(cloned.status.success(), "clone: {}", said(&cloned));

    // R40 — git's design, measured rather than described: the fence is not in the
    // object graph, so no clone can transport it.
    for door in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
        assert!(
            hooks_dir(&source).join(door).exists(),
            "the SOURCE carries {door} — the control that makes the clone's \
             absence mean something"
        );
        assert!(
            !hooks_dir(&clone).join(door).exists(),
            "and the clone carries no {door}: $GIT_DIR/hooks is never a tracked path"
        );
    }

    let out = sb.run(&clone, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: absent"),
        "the unasked-for reading, on a bare `mrd check`: {line}"
    );
    assert!(
        line.contains("0 of 3 doors"),
        "and it counts the doors rather than asserting one of them: {line}"
    );
    assert!(
        line.contains("never gated on"),
        "and it says outright that it is not a finding: {line}"
    );
    assert_eq!(
        fence_doors_line(&out).as_deref(),
        Some("  fence doors: pre-commit absent · pre-merge-commit absent · pre-applypatch absent"),
        "every door, spelled: {}",
        said(&out)
    );
}

/// **The doors disagree, and the line locates the disagreement.**
///
/// One door of three is unfenced. A line that read door one and called the
/// checkout fenced is the defect this whole lane is about, rebuilt — so the set's
/// word must be `installed-partial` and the per-door line must name which door.
///
/// The unfenced door is the MIDDLE one deliberately: a reader of door one alone
/// sees `installed`, and a reader of the count alone cannot say where the hole is.
#[test]
fn a_partly_fenced_checkout_is_partial_and_the_line_names_the_open_door() {
    let sb = sandbox();
    let ws = sb.corpus("partial");
    sb.fence(&ws);
    std::fs::remove_file(hooks_dir(&ws).join("pre-merge-commit")).expect("open one door");

    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: installed-partial"),
        "the set's state is not door one's state: {line}"
    );
    assert!(
        !line.contains("fence: installed —"),
        "and it must not be reported as fully fenced: {line}"
    );
    assert!(
        line.contains("2 of 3 doors"),
        "the population beside the reading: {line}"
    );
    assert!(
        line.contains("pre-merge-commit"),
        "the teaching names the open door: {line}"
    );
    assert_eq!(
        fence_doors_line(&out).as_deref(),
        Some(
            "  fence doors: pre-commit installed · pre-merge-commit absent · \
             pre-applypatch installed"
        ),
        "and the per-door line locates it exactly: {}",
        said(&out)
    );
}

/// A **foreign** hook at one door reports `foreign-hook` from `check`, matching
/// what `hook status` already reports — the reason word survives to this face
/// rather than being re-spelled here.
#[test]
fn a_foreign_hook_at_one_door_is_reported_as_foreign_from_check() {
    let sb = sandbox();
    let ws = sb.corpus("foreign");
    sb.fence(&ws);
    let foreign = hooks_dir(&ws).join("pre-applypatch");
    std::fs::write(&foreign, "#!/bin/sh\n# LEFTHOOK: do not remove\nexit 0\n").expect("write");

    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: foreign-hook"),
        "the state nothing else explains is named first: {line}"
    );
    assert!(
        line.contains("pre-applypatch"),
        "and the line says WHICH door is not this engine's: {line}"
    );
    assert_eq!(
        fence_doors_line(&out).as_deref(),
        Some(
            "  fence doors: pre-commit installed · pre-merge-commit installed · \
             pre-applypatch foreign-hook"
        ),
        "the per-door reading holds the two owned doors apart from the foreign \
         one: {}",
        said(&out)
    );
    // R40 — check REPORTS it and touches nothing.
    assert_eq!(
        std::fs::read_to_string(&foreign).expect("foreign"),
        "#!/bin/sh\n# LEFTHOOK: do not remove\nexit 0\n",
        "reporting a foreign hook may not rewrite it"
    );
    assert_eq!(
        out.status.code(),
        sb.run(&ws, &["check"]).status.code(),
        "and the reading is stable across runs"
    );
}

/// A checkout whose root is a **submodule** reports `submodule`, not a bare
/// absence: the reason word survives to the check face, and the root has **no door
/// plane at all** — so there is no per-door line to print.
///
/// The fixture is constructed and then adjudicated by git's OWN answer, never by
/// the fact that `git submodule add` succeeded.
#[test]
fn a_submodule_reports_its_reason_word_and_has_no_door_plane() {
    let sb = sandbox();
    let inner = sb.corpus("sub-inner");
    let outer = sb.corpus("sub-outer");

    let added = Command::new("git")
        .arg("-C")
        .arg(&outer)
        // `--force` only steps past an ambient ignore rule a global git config may
        // carry over a tempdir path; it changes nothing about what is built.
        .args([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "--force",
        ])
        .arg(&inner)
        .arg("vendor/inner")
        .output()
        .expect("git submodule add");
    assert!(
        added.status.success(),
        "the fixture IS the assert's subject — a real submodule: {}",
        said(&added)
    );
    let sub = outer.join("vendor").join("inner");
    assert_eq!(
        git_out(&sub, &["rev-parse", "--show-superproject-working-tree"]),
        outer
            .canonicalize()
            .expect("canonical outer")
            .to_string_lossy(),
        "R40 — git itself calls this a submodule"
    );

    let out = sb.run(&sub, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: submodule"),
        "the observed reason word, not a bare absence: {line}"
    );
    assert!(
        !line.contains("fence: absent"),
        "`absent` would say the doors were read and found empty, and they were \
         not read: {line}"
    );
    assert_eq!(
        fence_doors_line(&out),
        None,
        "a root with no reachable hook directory has no door plane to print: {}",
        said(&out)
    );
}

/// A workspace that is not a git repository at all reports `not-a-git-repo` — a
/// SUPPORTED state, named rather than passed over in silence. It too has no door
/// plane.
#[test]
fn a_workspace_with_no_repository_names_that_state_rather_than_staying_silent() {
    let sb = sandbox();
    let ws = sb.plain("no-repo");

    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: not-a-git-repo"),
        "the reason word for a root with nowhere to install: {line}"
    );
    assert!(
        line.contains("supported state"),
        "and it is not reported as a fault in the workspace: {line}"
    );
    assert_eq!(fence_doors_line(&out), None, "no repository, no doors");
}

// ── the promises the reading makes about itself ─────────────────────────────

/// **Emitting the line performs NO WRITE.** A root must not come away with an
/// `mrd-hook.lock` in its git dir as the souvenir of being looked at — the
/// property `hook.rs` § *"Why the guards run twice"* protects.
///
/// The positive control is the point: `mrd hook install` DOES take the lock and
/// does leave the file, so this probe is proven able to find one before its
/// absence is read as evidence.
#[test]
fn reporting_the_fence_takes_no_lock_and_leaves_no_souvenir() {
    let sb = sandbox();
    let source = sb.corpus("lock-source");
    sb.fence(&source);
    let common = PathBuf::from(git_out(&source, &["rev-parse", "--git-common-dir"]));
    let source_common = if common.is_absolute() {
        common
    } else {
        source.join(common)
    };
    // THE POSITIVE CONTROL — install takes the lock, so the probe below can see one.
    assert!(
        source_common.join("mrd-hook.lock").exists(),
        "the control: `mrd hook install` leaves its lock file, so a missing one \
         downstream is evidence rather than a blind probe"
    );

    let clone = sb.tmp.path().join("lock-clone");
    let cloned = Command::new("git")
        .args(["clone", "-q"])
        .arg(&source)
        .arg(&clone)
        .output()
        .expect("git clone");
    assert!(cloned.status.success(), "clone: {}", said(&cloned));
    let clone_lock = clone.join(".git").join("mrd-hook.lock");
    assert!(!clone_lock.exists(), "FIXTURE: the clone starts clean");

    let out = sb.run(&clone, &["check"]);
    assert!(
        fence_line(&out).contains("fence: absent"),
        "the run really did read the fence: {}",
        fence_line(&out)
    );
    assert!(
        !clone_lock.exists(),
        "and it left no lock file behind — a root looked at is a root untouched"
    );
}

/// **The `--json` face**, and the shape law that bit this session already: *an
/// absent field reads as "not checked"; a null does not.*
///
/// A fenceable root carries the door plane; an unfenceable one has **no `doors`
/// key at all** — never `null`, which would say the doors were read and came back
/// as nothing. The two halves are one test so the probe is proven able to see the
/// key before its absence is read as evidence.
#[test]
fn the_json_fence_block_carries_the_doors_and_omits_them_when_there_are_none() {
    let sb = sandbox();

    // ACCEPTANCE — a fenced checkout, key for key.
    let ws = sb.corpus("json-fenced");
    sb.fence(&ws);
    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(
        fence_json(&out),
        serde_json::json!({
            "state": "installed",
            "fenceable": true,
            "teaching": "this checkout is fully fenced, at the generation this engine writes",
            "engine_version": 3,
            // The claim, machine-readable: a consumer reads off this face that the
            // block did not decide the exit.
            "gates_the_exit": false,
            "doors": [
                { "name": "pre-commit", "state": "installed", "fence_version": 3 },
                { "name": "pre-merge-commit", "state": "installed", "fence_version": 3 },
                { "name": "pre-applypatch", "state": "installed", "fence_version": 3 },
            ],
            "fenced_doors": 3,
            "total_doors": 3,
        }),
        "the fenced block, key for key: {}",
        said(&out)
    );
    let block = fence_json(&out);
    assert!(
        block.as_object().expect("object").contains_key("doors"),
        "THE PROBE'S CONTROL: `contains_key` can see the key when it is there"
    );

    // REFUSAL — a root with no door plane omits the door keys entirely.
    let plain = sb.plain("json-no-repo");
    let out = sb.run(&plain, &["check", "--json"]);
    let block = fence_json(&out);
    let object = block.as_object().expect("object");
    assert_eq!(block["state"], serde_json::json!("not-a-git-repo"));
    assert_eq!(block["fenceable"], serde_json::json!(false));
    for key in ["doors", "fenced_doors", "total_doors"] {
        assert!(
            !object.contains_key(key),
            "`{key}` must be ABSENT, never null — a null says the door plane was \
             read and came back as nothing, and there is no door plane here: {block}"
        );
    }
}
