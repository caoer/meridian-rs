//! Row 21 — `mrd check` reports the checkout's fence coverage, unasked, and
//! the report cannot reach the exit. End-to-end over the real binary, driving
//! the shipped CLI only.
//!
//! `$GIT_DIR/hooks` is never a tracked path, so a fresh clone is unfenced by
//! design; coverage is per-checkout and opt-in, permanently. The defect this
//! closes is the silence about that state, not the state itself.
//!
//! Fence state is a proposition about the local checkout's configuration, so
//! it never reaches the exit code: colouring `check` on an unfenced checkout
//! would make governance unreachable in every fresh clone.
//!
//! The first line composes a count (coverage — doors carrying this engine's
//! marker, at any generation) with the set word (currency); arms that read
//! the count pin its whole clause. The install set is `pre-commit`,
//! `pre-merge-commit`, `pre-applypatch`, and every per-door assert spells the
//! three names as literals, so an arm cannot go vacuous when the set shrinks.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use mrd::hook::FENCE_VERSION;

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

    /// The same drive with the workspace pinned by `MERIDIAN_WORKSPACE` —
    /// tier 1 of the discovery ladder, the only rung that can seat a
    /// workspace root below the worktree top-level.
    fn run_at(&self, cwd: &Path, workspace: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_WORKSPACE", workspace)
            .output()
            .expect("spawn mrd")
    }

    /// A real git repository that is also a meridian workspace, with a
    /// pinnable source section and a claim page. Committed, and not fenced:
    /// every arm installs the fence itself, or deliberately does not.
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

    /// Fence the checkout the way `mrd skill hook`'s document says to — the
    /// body off that verb's stdout, written to every door, `chmod +x` — and
    /// assert the state change on disk rather than the command's exit.
    fn fence(&self, ws: &Path) {
        let out = self.run(ws, &["skill", "hook"]);
        assert_eq!(out.status.code(), Some(0), "mrd skill hook: {}", said(&out));
        let body = fence_body(&stdout(&out));
        let hooks = hooks_dir(ws);
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        for door in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
            let path = hooks.join(door);
            std::fs::write(&path, &body).expect("place the fence");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
            assert!(
                path.exists(),
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
    String::from_utf8_lossy(&git_try(dir, args).stdout)
        .trim()
        .to_owned()
}

/// One git call whose FAILURE is the interesting outcome — the seam
/// [`a_root_git_cannot_answer_for_is_named_rather_than_read_as_a_bare_absence`] needs,
/// where [`git_out`] would hand back an empty string and hide it.
fn git_try(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs in the test environment")
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

/// The fence body, extracted the way `mrd skill hook`'s document says to: the
/// one fenced block, and it is the file.
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

/// The marker line's opening. What follows the generation is prose, so both
/// helpers below take the first token after this prefix, never the rest.
const MARKER_PREFIX: &str = "# mrd-hook-fence ";

/// The generation ONE placed door declares, read off the file itself — the
/// fixture's own ground truth, never `mrd`'s reading of it.
fn declared_version(ws: &Path, door: &str) -> Option<u32> {
    std::fs::read_to_string(hooks_dir(ws).join(door))
        .expect("read an installed door")
        .lines()
        .find_map(|line| line.strip_prefix(MARKER_PREFIX))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

/// Restate one door's declared generation, leaving the placed fence otherwise
/// byte-for-byte. The marker line is found, never this engine's number
/// spelled.
fn retag_door(ws: &Path, door: &str, generation: &str) {
    let path = hooks_dir(ws).join(door);
    let body = std::fs::read_to_string(&path).expect("read an installed door");
    let mut rewritten = String::new();
    for line in body.lines() {
        match line.strip_prefix(MARKER_PREFIX) {
            Some(rest) => {
                let tail = rest.split_once(' ').map_or("", |(_generation, tail)| tail);
                rewritten.push_str(MARKER_PREFIX);
                rewritten.push_str(generation);
                if !tail.is_empty() {
                    rewritten.push(' ');
                    rewritten.push_str(tail);
                }
            }
            None => rewritten.push_str(line),
        }
        rewritten.push('\n');
    }
    std::fs::write(&path, rewritten).expect("write the retagged door");
}

/// Whether one door still carries this engine's ownership marker. The control
/// that separates a RETAGGED door from a foreign one.
fn carries_marker(ws: &Path, door: &str) -> bool {
    std::fs::read_to_string(hooks_dir(ws).join(door))
        .expect("read an installed door")
        .contains("mrd-hook-fence")
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

/// The ` fence:` line — the set's reading. Panics when missing: the line's
/// absence is the defect this unit is about, and an empty string would let
/// every arm pass on the silence.
fn fence_line(out: &Output) -> String {
    stdout(out)
        .lines()
        .find(|l| l.starts_with("  fence: "))
        .unwrap_or_else(|| panic!("mrd check printed no fence line:\n{}", said(out)))
        .to_owned()
}

/// The count clause of the ` fence:` line — everything between the set word
/// and the teaching. Lifted out rather than substring-matched: the clause may
/// state the coverage it measured and not the currency the set word states.
fn fence_count_clause(out: &Output) -> String {
    let line = fence_line(out);
    line.split_once(" — ")
        .and_then(|(_word, rest)| rest.split_once(';'))
        .map_or_else(
            || panic!("the fence line carries no count clause: {line}"),
            |(clause, _teaching)| clause.to_owned(),
        )
}

/// The ` fence doors:` line — the per-door reading, or `None` when this root has no door plane.
/// The two are DIFFERENT facts and this helper keeps them so.
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

/// The central claim: one checkout read twice — unfenced, then fully fenced —
/// with nothing else changed. The exit code must be identical across the pair
/// and the fence word must differ across it; the second assert is the
/// anti-vacuity control.
#[test]
fn the_fence_line_never_reaches_the_exit_code() {
    let sb = sandbox();
    // (a) the green corpus: a leak that reddens is visible. (b) the refusing
    // corpus: already exits 1, so a leak the other way is visible too.
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

/// A fresh clone of a fenced root reports the unfenced state without being
/// asked — a bare `mrd check`, no `hook status`, no flag. The mechanism is
/// asserted beside the reading: the source's hooks are on disk and the
/// clone's hook directory carries none of them.
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
    assert_eq!(
        fence_count_clause(&out),
        "0 of 3 doors carry this engine's fence marker, at any generation",
        "and it counts the doors rather than asserting one of them — over the whole \
         clause, so the count cannot regain the currency claim it does not \
         measure: {line}"
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

/// The doors disagree, and the line locates the disagreement: one door of
/// three is unfenced, and the set's word must not read as fully fenced.
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
    assert_eq!(
        fence_count_clause(&out),
        "2 of 3 doors carry this engine's fence marker, at any generation",
        "the population beside the reading, pinned as a whole clause: it states the \
         coverage it measured and leaves currency to the word: {line}"
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

/// A version-skewed door is `installed-superseded` on this face too, and the
/// per-door line says which door is stale — an operator told they are current
/// has no reason to refresh. The skewed door is the middle one, so a render
/// that reads door one alone would call the checkout current.
#[test]
fn a_version_skewed_door_is_superseded_from_check_and_the_line_names_it() {
    let sb = sandbox();
    let ws = sb.corpus("superseded");
    sb.fence(&ws);
    retag_door(&ws, "pre-merge-commit", "1");

    // The positive control, read off the files before any render is
    // inspected: the fixture really does produce exactly one skewed door.
    let skewed = declared_version(&ws, "pre-merge-commit");
    assert_eq!(
        skewed,
        Some(1),
        "the fixture IS the assert's subject: pre-merge-commit must declare an \
         older generation on disk"
    );
    for door in ["pre-commit", "pre-applypatch"] {
        let current = declared_version(&ws, door);
        assert!(
            current > skewed,
            "the control's other half — {door} must still declare the generation \
             this engine writes, or there is no SKEW to report: {current:?} vs \
             {skewed:?}"
        );
    }
    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: installed-superseded"),
        "the set's word carries the skew: {line}"
    );
    assert!(
        !line.contains("fence: installed —"),
        "THE HAZARD, stated as an assert: a stale fence reported as current is \
         the silent wrong answer this lane exists to kill: {line}"
    );
    assert_eq!(
        fence_count_clause(&out),
        "3 of 3 doors carry this engine's fence marker, at any generation",
        "THE CLAUSE-AXIS CLAIM: the count says COVERAGE and names the generation \
         it is blind to; the currency is the set word's claim, standing beside it. \
         A clause that reverts to `carry this engine's fence` asserts the currency \
         this count never measured — false as written on exactly this checkout, \
         and false next to the word contradicting it: {line}"
    );
    assert_eq!(
        fence_doors_line(&out).as_deref(),
        Some(
            "  fence doors: pre-commit installed · pre-merge-commit \
             installed-superseded · pre-applypatch installed"
        ),
        "THE CLAIM: the per-door line spells the stale door's own word, and a \
         render that flattened it to `installed` fails here: {}",
        said(&out)
    );

    // The same skew on the `--json` face — the machine reader of this surface is
    // exactly the consumer a "current" answer would mislead.
    let out = sb.run(&ws, &["check", "--json"]);
    let block = fence_json(&out);
    assert_eq!(block["state"], serde_json::json!("installed-superseded"));
    assert_eq!(
        block["doors"],
        serde_json::json!([
            { "name": "pre-commit", "state": "installed", "fence_version": FENCE_VERSION },
            { "name": "pre-merge-commit", "state": "installed-superseded", "fence_version": 1 },
            { "name": "pre-applypatch", "state": "installed", "fence_version": FENCE_VERSION },
        ]),
        "each door's own state and its own declared generation, never the asking \
         engine's: {}",
        said(&out)
    );

    // R40 — check reports the skew and refreshes nothing; a status face that
    // re-placed the fence would be a writer.
    assert_eq!(
        declared_version(&ws, "pre-merge-commit"),
        Some(1),
        "reading a superseded fence may not rewrite it"
    );
}

/// The two remaining door states. `installed-ahead` is the one word whose
/// remedy inverts — every other word means "re-place the fence"; that one
/// means "do not: the `mrd` answering is the one that is behind". A render
/// that flattened it to `installed` would route an operator into downgrading
/// their own fence, so the inverted remedy is asserted, not described.
#[test]
fn the_ahead_and_unversioned_doors_each_keep_their_own_word_on_the_line() {
    let sb = sandbox();
    let ws = sb.corpus("ahead-unversioned");
    sb.fence(&ws);
    retag_door(&ws, "pre-merge-commit", "999");
    retag_door(&ws, "pre-applypatch", "next");

    // THE POSITIVE CONTROL, off the files, before any render is inspected.
    let current = declared_version(&ws, "pre-commit");
    let ahead = declared_version(&ws, "pre-merge-commit");
    assert!(
        ahead > current && current.is_some(),
        "the fixture IS the assert's subject: pre-merge-commit must declare a \
         generation ABOVE the untouched door's, or there is nothing ahead to \
         report — {ahead:?} vs {current:?}"
    );
    assert_eq!(
        declared_version(&ws, "pre-applypatch"),
        None,
        "and pre-applypatch must declare a generation no `u32` parses"
    );
    assert!(
        carries_marker(&ws, "pre-applypatch"),
        "the control that first draft got wrong — the MARKER must survive the \
         retag, or this measures `foreign-hook` instead of `installed-unversioned`"
    );
    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: installed-ahead"),
        "the word whose remedy inverts is named ahead of every other: {line}"
    );
    assert!(
        line.contains("do NOT re-place"),
        "THE REASON THIS ARM EXISTS: the remedy INVERTS here, and a reader sent \
         to re-place from `mrd skill hook` by this line would downgrade their own \
         fence: {line}"
    );

    // THE CLAIM. The set word carried `installed-ahead` and could carry nothing else;
    // `installed-unversioned` reaches the operator through THIS line only.
    assert_eq!(
        fence_doors_line(&out).as_deref(),
        Some(
            "  fence doors: pre-commit installed · pre-merge-commit \
             installed-ahead · pre-applypatch installed-unversioned"
        ),
        "each door keeps its OWN word, and a render that flattened either to \
         `installed` fails here: {}",
        said(&out)
    );

    // The same two words on the `--json` face, each beside the generation the
    // file declares — `null` is the file's own answer.
    let out = sb.run(&ws, &["check", "--json"]);
    let block = fence_json(&out);
    assert_eq!(block["state"], serde_json::json!("installed-ahead"));
    assert_eq!(
        block["doors"],
        serde_json::json!([
            { "name": "pre-commit", "state": "installed", "fence_version": FENCE_VERSION },
            { "name": "pre-merge-commit", "state": "installed-ahead", "fence_version": 999 },
            { "name": "pre-applypatch", "state": "installed-unversioned", "fence_version": null },
        ]),
        "each door's own state and its own declared generation: {}",
        said(&out)
    );

    // R40 — check reports both states and rewrites neither; refreshing an
    // ahead door would be the downgrade itself.
    assert_eq!(
        declared_version(&ws, "pre-merge-commit"),
        ahead,
        "reading an ahead fence may not replace it with this engine's older one"
    );
    assert!(
        carries_marker(&ws, "pre-applypatch"),
        "and the unversioned door is left exactly as it was found"
    );
}

/// `installed-unversioned` as the set word. The arm above carries an ahead
/// door beside its unversioned one, and precedence gives the set word to
/// `installed-ahead` — so this fixture keeps the set word reachable: one
/// unversioned door, and nothing above it in precedence to shadow it.
#[test]
fn an_unversioned_door_alone_carries_the_set_word_rather_than_flattening_to_installed() {
    let sb = sandbox();
    let ws = sb.corpus("unversioned-set-word");
    sb.fence(&ws);
    retag_door(&ws, "pre-merge-commit", "next");

    // THE POSITIVE CONTROL, off the files, before any render is inspected.
    assert_eq!(
        declared_version(&ws, "pre-merge-commit"),
        None,
        "the fixture IS the assert's subject: pre-merge-commit must declare a \
         generation no `u32` parses"
    );
    assert!(
        carries_marker(&ws, "pre-merge-commit"),
        "and the MARKER must survive the retag, or this measures `foreign-hook` \
         instead of `installed-unversioned`"
    );
    // The precedence control: the two untouched doors declare the same
    // generation, so nothing outranks `installed-unversioned`. Compared rather
    // than spelled — a `FENCE_VERSION` bump must not silently no-op this.
    let untouched = declared_version(&ws, "pre-commit");
    assert!(
        untouched.is_some() && untouched == declared_version(&ws, "pre-applypatch"),
        "the two untouched doors must declare one generation between them, or a \
         higher-precedence word takes the set: {untouched:?} vs {:?}",
        declared_version(&ws, "pre-applypatch")
    );
    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: installed-unversioned"),
        "THE CLAIM: the SET word carries the undeclarable generation: {line}"
    );
    assert!(
        !line.contains("fence: installed —"),
        "THE HAZARD: a fence whose currency cannot be judged, reported as \
         current, is the silent wrong answer this lane exists to kill — and the \
         operator is never told why the document then says to refuse: {line}"
    );
    assert_eq!(
        fence_count_clause(&out),
        "3 of 3 doors carry this engine's fence marker, at any generation",
        "coverage counts an undeclarable door like any other; the currency is the \
         set word's claim, standing beside it: {line}"
    );
    assert!(
        line.contains("pre-merge-commit"),
        "the teaching names the door whose generation cannot be read: {line}"
    );
    assert_eq!(
        fence_doors_line(&out).as_deref(),
        Some(
            "  fence doors: pre-commit installed · pre-merge-commit \
             installed-unversioned · pre-applypatch installed"
        ),
        "and no door outranks it — the per-door line spells all three, so an \
         `installed-ahead` or `foreign-hook` sneaking into the fixture fails \
         here rather than silently shadowing the word under test: {}",
        said(&out)
    );

    // The same word on the `--json` face — its `state` key mirrors the SET word
    // and nothing else, so this face has no per-door line to fall back on.
    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(
        fence_json(&out)["state"],
        serde_json::json!("installed-unversioned"),
        "{}",
        said(&out)
    );

    // R40 — check REPORTS the undeclarable generation and rewrites nothing.
    assert_eq!(declared_version(&ws, "pre-merge-commit"), None);
    assert!(carries_marker(&ws, "pre-merge-commit"));
}

/// A checkout whose root is a submodule reports `submodule`, not a bare
/// absence, and has no door plane at all — no per-door line to print. The
/// fixture is adjudicated by git's own answer.
#[test]
fn a_submodule_reports_its_reason_word_and_has_no_door_plane() {
    let sb = sandbox();
    let inner = sb.corpus("sub-inner");
    let outer = sb.corpus("sub-outer");

    let added = Command::new("git")
        .arg("-C")
        .arg(&outer)
        // `--force` only steps past an ambient ignore rule a global git config may carry over a
        // tempdir path; it changes nothing about what is built.
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

/// A workspace that is not a git repository at all reports `not-a-git-repo` —
/// a supported state, named rather than passed over. No door plane.
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

/// `core.hooksPath` is set, so this checkout's own hook directory is dead —
/// and the line says so instead of reporting a bare absence, whose teaching
/// would place a file git never runs.
#[test]
fn a_redirected_hooks_path_is_named_on_the_check_face_rather_than_read_as_unfenced() {
    let sb = sandbox();
    let ws = sb.corpus("hooks-path-check");
    let elsewhere = sb.tmp.path().join("their-checkout").join(".githooks");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");
    std::fs::write(elsewhere.join("pre-commit"), "#!/bin/sh\nexit 0\n").expect("their hook");
    git_ok(
        &ws,
        &["config", "core.hooksPath", &elsewhere.display().to_string()],
    );

    // THE POSITIVE CONTROL — git's OWN answer, never the fact that `git config`
    // exited 0: this root really does send git's hook lookup elsewhere.
    assert_eq!(
        git_out(&ws, &["config", "--get", "core.hooksPath"]),
        elsewhere.display().to_string(),
        "the fixture IS the assert's subject: git must report the redirect"
    );
    // The control's other half, and the reason `absent` is TEMPTING here: this
    // root's own hook directory genuinely carries no fence.
    assert!(
        !hooks_dir(&ws).join("pre-commit").exists(),
        "the redirected root's own hooks directory is empty, which is exactly \
         what a render that ignored core.hooksPath would report as `absent`"
    );

    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: hooks-path-redirected"),
        "the observed reason word, not a bare absence: {line}"
    );
    assert!(
        !line.contains("fence: absent"),
        "THE HAZARD: `absent` teaches `mrd skill hook`, and on this root that \
         writes a file git will never run — a checkout that reads as fenced and \
         gates nothing: {line}"
    );
    assert!(
        line.contains(&elsewhere.display().to_string()),
        "and the line names WHERE git actually looks, which is what the operator \
         needs to act: {line}"
    );
    assert!(
        line.contains("another checkout's hook directory"),
        "the stronger true thing: that path already carries a pre-commit, so \
         installing would write into a repository this operator does not own: \
         {line}"
    );
    assert_eq!(
        fence_doors_line(&out),
        None,
        "the doors were never read, so there is no door plane to print: {}",
        said(&out)
    );

    let out = sb.run(&ws, &["check", "--json"]);
    let block = fence_json(&out);
    assert_eq!(block["state"], serde_json::json!("hooks-path-redirected"));
    assert_eq!(block["fenceable"], serde_json::json!(false));
    for key in ["doors", "fenced_doors", "total_doors"] {
        assert!(
            !block.as_object().expect("object").contains_key(key),
            "`{key}` must be ABSENT, never null — the doors were not read: {block}"
        );
    }
}

/// The meridian workspace root sits below the worktree top-level, and the
/// line names both roots instead of reporting a bare absence. The fixture
/// needs tier 1: the ladder is `MERIDIAN_WORKSPACE` → nearest `.git` → cwd,
/// and tier 2 can only ever answer the top-level itself.
#[test]
fn a_workspace_below_the_worktree_top_level_names_both_roots_rather_than_reading_unfenced() {
    let sb = sandbox();
    let outer = sb.corpus("nested-outer");
    let nested = outer.join("nested");
    std::fs::create_dir_all(&nested).expect("mkdir nested");
    write(&nested, "a.md", "# A\n\nalpha\n");
    let init = sb.run_at(&nested, &nested, &["init"]);
    assert!(
        init.status.success(),
        "init the nested root: {}",
        said(&init)
    );

    // THE POSITIVE CONTROL — git's OWN answer: the two directories really are
    // different, and the top-level really is the outer one.
    let top_level = PathBuf::from(git_out(&nested, &["rev-parse", "--show-toplevel"]));
    let canonical_outer = outer.canonicalize().expect("canonical outer");
    let canonical_nested = nested.canonicalize().expect("canonical nested");
    assert_eq!(
        top_level, canonical_outer,
        "the fixture IS the assert's subject: git calls the OUTER directory this \
         worktree's top-level"
    );
    assert_ne!(
        canonical_nested, canonical_outer,
        "and the workspace under test is a different directory from it, or there \
         is no mismatch to report"
    );

    let out = sb.run_at(&nested, &nested, &["check"]);
    // The control's other half: the run really did resolve to the NESTED root.
    // Without it every assert below could be reading the outer workspace.
    let header = stdout(&out)
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_owned();
    assert!(
        header.ends_with(&canonical_nested.display().to_string()),
        "the env override really seated the workspace at the nested root: \
         {header}"
    );

    let line = fence_line(&out);
    assert!(
        line.contains("fence: workspace-not-toplevel"),
        "the observed reason word, not a bare absence: {line}"
    );
    assert!(
        !line.contains("fence: absent"),
        "THE HAZARD: `absent` teaches `mrd skill hook` HERE, and a fence \
         placed for a nested workspace covers the whole top-level instead: \
         {line}"
    );
    assert!(
        line.contains(&canonical_nested.display().to_string())
            && line.contains(&canonical_outer.display().to_string()),
        "and the line names BOTH roots — the one asked about and the one git \
         answers with — because naming only one leaves the operator guessing \
         which directory the fence would cover: {line}"
    );
    assert!(
        line.contains("Ask from"),
        "with the remedy pointing at the top-level rather than at this root: \
         {line}"
    );
    assert_eq!(
        fence_doors_line(&out),
        None,
        "the doors were never read, so there is no door plane to print: {}",
        said(&out)
    );

    let out = sb.run_at(&nested, &nested, &["check", "--json"]);
    let block = fence_json(&out);
    assert_eq!(block["state"], serde_json::json!("workspace-not-toplevel"));
    assert_eq!(block["fenceable"], serde_json::json!(false));
}

/// Git ran and refused, so the fence state is unknown — and the line says so
/// rather than either neighbour: `absent` claims the doors were read and
/// found empty; `not-a-git-repo` claims there is nowhere to install at all.
#[test]
fn a_root_git_cannot_answer_for_is_named_rather_than_read_as_a_bare_absence() {
    let sb = sandbox();
    let ws = sb.corpus("cannot-ask");
    let config = ws.join(".git").join("config");

    // THE POSITIVE CONTROL — before the corruption, git answers this question,
    // so a failure after it is evidence rather than a blind probe.
    assert!(
        git_try(&ws, &["rev-parse", "--git-common-dir"])
            .status
            .success(),
        "the control: git can answer for this root before the fixture breaks it"
    );
    let sound = std::fs::read_to_string(&config).expect("read .git/config");
    std::fs::write(&config, format!("{sound}[core\n")).expect("write .git/config");
    // The control's other half: git itself, not this test, says it can no longer
    // answer.
    let refused = git_try(&ws, &["rev-parse", "--git-common-dir"]);
    assert!(
        !refused.status.success(),
        "the fixture IS the assert's subject: git must refuse to answer for this \
         root now"
    );

    let out = sb.run(&ws, &["check"]);
    let line = fence_line(&out);
    assert!(
        line.contains("fence: cannot-ask-git"),
        "the observed reason word: the engine could not ask, and says so: {line}"
    );
    assert!(
        !line.contains("fence: absent"),
        "THE HAZARD: `absent` claims the doors WERE read and came back empty. \
         They were never read, and this root may be fully fenced: {line}"
    );
    assert!(
        !line.contains("fence: not-a-git-repo"),
        "and not the other neighbour either — that word means there is nowhere \
         to install, a supported state an operator is meant to accept, and this \
         root has a repository whose configuration merely cannot be read: {line}"
    );
    assert!(
        line.contains("cannot determine the hook directory")
            && line.contains("git rev-parse --git-common-dir"),
        "the refusal carries WHAT failed, verbatim — a reader who is not told \
         which question went unanswered cannot fix it: {line}"
    );
    assert_eq!(
        fence_doors_line(&out),
        None,
        "the doors were never read, so there is no door plane to print: {}",
        said(&out)
    );

    let out = sb.run(&ws, &["check", "--json"]);
    let block = fence_json(&out);
    assert_eq!(block["state"], serde_json::json!("cannot-ask-git"));
    assert_eq!(block["fenceable"], serde_json::json!(false));

    // R40 — check reports the unreadable config and repairs nothing.
    assert_eq!(
        std::fs::read_to_string(&config).expect("read .git/config"),
        format!("{sound}[core\n"),
        "reading a root git cannot answer for may not rewrite its config"
    );
}

// ── the promises the reading makes about itself ─────────────────────────────

/// Emitting the line performs no write: a root looked at is a root untouched.
#[test]
fn reporting_the_fence_writes_nothing_into_the_git_directory() {
    let sb = sandbox();
    let ws = sb.corpus("souvenir");
    sb.fence(&ws);
    let git_dir = hooks_dir(&ws)
        .parent()
        .expect("the git dir holds hooks/")
        .to_path_buf();

    // THE POSITIVE CONTROL — the probe can see a git dir change, so its silence
    // downstream is evidence rather than a blind assert.
    let baseline = git_dir_snapshot(&git_dir);
    std::fs::write(git_dir.join("mrd-control-probe"), "x").expect("write the probe file");
    assert_ne!(
        git_dir_snapshot(&git_dir),
        baseline,
        "the control failed: this snapshot cannot see a new file in the git dir, so \
         its stability below would mean nothing"
    );
    std::fs::remove_file(git_dir.join("mrd-control-probe")).expect("remove the probe file");
    let before = git_dir_snapshot(&git_dir);
    assert_eq!(before, baseline, "the control cleaned up after itself");

    let out = sb.run(&ws, &["check"]);
    assert!(
        fence_line(&out).contains("fence: installed"),
        "the run really did read the fence: {}",
        fence_line(&out)
    );
    assert_eq!(
        git_dir_snapshot(&git_dir),
        before,
        "reading the doors changed the git directory — a root looked at is a root \
         untouched, and there is no writer left in this engine that may leave a \
         souvenir"
    );

    // The same claim on the other side: an unfenced checkout must not acquire
    // anything either.
    let clone = sb.tmp.path().join("souvenir-clone");
    let cloned = Command::new("git")
        .args(["clone", "-q"])
        .arg(&ws)
        .arg(&clone)
        .output()
        .expect("git clone");
    assert!(cloned.status.success(), "clone: {}", said(&cloned));
    let clone_git = clone.join(".git");
    let before = git_dir_snapshot(&clone_git);
    let out = sb.run(&clone, &["check"]);
    assert!(
        fence_line(&out).contains("fence: absent"),
        "the clone really did read as unfenced: {}",
        fence_line(&out)
    );
    assert_eq!(
        git_dir_snapshot(&clone_git),
        before,
        "an unfenced checkout comes away byte-identical from being looked at"
    );
}

/// Every path under a git directory, with its size — enough to notice a file
/// appearing, vanishing, or being rewritten, and cheap enough to take twice.
fn git_dir_snapshot(dir: &Path) -> Vec<(PathBuf, u64)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => walk(&path, out),
                Ok(meta) => out.push((path, meta.len())),
                Err(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// The `--json` face: an absent field reads as "not checked"; a null does
/// not. A fenceable root carries the door plane; an unfenceable one has no
/// `doors` key at all. Both halves in one test, so the probe is proven able
/// to see the key before its absence is read as evidence.
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
            "engine_version": FENCE_VERSION,
            // The claim, machine-readable: a consumer reads off this face that the
            // block did not decide the exit.
            "gates_the_exit": false,
            "doors": [
                { "name": "pre-commit", "state": "installed", "fence_version": FENCE_VERSION },
                { "name": "pre-merge-commit", "state": "installed", "fence_version": FENCE_VERSION },
                { "name": "pre-applypatch", "state": "installed", "fence_version": FENCE_VERSION },
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
