//! **F6 — the pin plane under the fence must be able to SEE the roots it judges.**
//!
//! `mrd check` and `mrd status` coloured their pins through
//! `view::walk::lock_pin_colors(docs)`, which resolves against
//! `addr::MountSet::default()` — an EMPTY mount table — and an ambient-only
//! corpus. On a BOUND root a form-3 cross-root pin therefore answered
//! `grey unmounted` when its target MATCHED, when it had DRIFTED, and when it was
//! RESTORED, while `mrd config` called the root `bound` and `mrd walk` — the one
//! caller that passed the real table — answered green / red / green over the same
//! corpus, in the same instant.
//!
//! # The finding is INSENSITIVITY, and that is what this file measures
//! A comparison gate would have passed: there is exactly one pin computer and
//! every plane agreed with it. **They agreed on a blind answer.** So the assert
//! here is not "green" and not "matches walk" alone — it is that **the axis
//! MOVES**: three states must produce three answers, because an axis whose
//! instrument cannot vary on it is unevidenced (S3-R72) and a fence reading it
//! inherits the blindness.
//!
//! # Why the lock is hand-written, and it is a finding of its own
//! **`mrd pin` cannot mint a cross-root pin**: `mrd pin claim.md
//! 'other:doc.md#Doc/Design'` is refused `bad_path` by the wire path validation
//! (measured on the deployed engine `980008813ff69586…`). The population of
//! form-3 cross-root pins is therefore written through other doors, and a fixture
//! has to do the same. **The FINGERPRINT is never invented** — it is minted by the
//! shipped `mrd pin` over byte-identical content in a scratch workspace and
//! transplanted, the "pulled corpus" shape `u14_check_pin_plane.rs` already ships
//! (a lock minted in one copy, read in another). A hand-typed fingerprint would
//! make every green here vacuous.
//!
//! Regenerable outside cargo by the same recipe:
//! `results/f6-mount-sight-repro.sh` in the session tree, engine as a parameter.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The binary every drive goes through — the real CLI, never a library call.
/// `MRD_BIN` points it at another engine, which is how the BEFORE is re-measured.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// The pinned section's bytes, in the TARGET root.
const DOC: &str = "# Doc\n\n## Design\n\nthe target root's real design note.\n";
/// The same document with the pinned section rewritten — the DRIFT.
const DOC_DRIFTED: &str = "# Doc\n\n## Design\n\nDRIFTED — rewritten in the target root.\n";

struct Sandbox {
    /// Held for its Drop: the tree is deleted when this goes out of scope.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    config: PathBuf,
    bin: PathBuf,
    /// The ambient workspace — a git repository, so the fence can be installed.
    ws: PathBuf,
    /// The MOUNTED root's directory.
    other: PathBuf,
}

const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache_home = tmp.path().join("xdg-cache");
    let bin = tmp.path().join("bin");
    let ws = tmp.path().join("ws");
    let other = tmp.path().join("other");
    for dir in [&home, &bin, &ws, &other] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    std::os::unix::fs::symlink(mrd_bin(), bin.join("mrd")).expect("link mrd onto the hook's PATH");

    // The mounted root declares its own canonical name (INV-5) — without this the
    // bind renders grey(undeclared) and every reading below is vacuous.
    std::fs::write(
        other.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n\n# Other root\n",
    )
    .expect("root declaration");
    std::fs::write(other.join("doc.md"), DOC).expect("target");

    let config = home.join("MERIDIAN.md");
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: other\npath: {}\nkind: vault\nvault: othervault\n```\n",
            other.display()
        ),
    )
    .expect("mount table");

    Sandbox {
        tmp,
        home,
        cache_home,
        config,
        bin,
        ws,
        other,
    }
}

impl Sandbox {
    fn command(&self, cwd: &Path) -> Command {
        let mut c = Command::new(mrd_bin());
        c.current_dir(cwd)
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("MERIDIAN_CONFIG", &self.config)
            .env_remove("MERIDIAN_WORKSPACE");
        c
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd).args(args).output().expect("spawn mrd")
    }

    fn run_stdin(&self, cwd: &Path, args: &[&str], stdin_bytes: &str) -> Output {
        use std::io::Write as _;
        use std::process::Stdio;
        let mut child = self
            .command(cwd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(stdin_bytes.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("wait mrd")
    }

    /// Place the fence the way `mrd skill hook`'s document says to: the one fenced
    /// block off that verb's stdout, at every door under the common dir, `chmod
    /// +x`. There is no installer — the document is the contract.
    fn place_fence(&self) {
        let out = self.run(&self.ws, &["skill", "hook"]);
        assert_eq!(out.status.code(), Some(0), "mrd skill hook: {}", said(&out));
        let doc = String::from_utf8_lossy(&out.stdout).into_owned();
        let mut lines = doc.lines();
        lines
            .by_ref()
            .find(|l| l.starts_with("```"))
            .expect("the document carries a fenced block");
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.starts_with("```") {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        let hooks = self.ws.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        for door in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
            let path = hooks.join(door);
            std::fs::write(&path, &body).expect("place the fence");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
        }
    }

    /// A real `git commit` with the fence on `PATH` — ours first, so the placed
    /// hook runs the engine under test and never a deployed one.
    fn commit(&self, message: &str) -> Output {
        Command::new("git")
            .arg("-C")
            .arg(&self.ws)
            .args(["commit", "-m", message])
            .env("PATH", format!("{}:{SYSTEM_PATH}", self.bin.display()))
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("MERIDIAN_CONFIG", &self.config)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn git commit")
    }

    /// Mint a real fingerprint for `DOC`'s pinned section through the SHIPPED pin
    /// door, over byte-identical content in a scratch workspace.
    ///
    /// The scratch workspace is a GIT repository because R4 makes a pin's `hash`
    /// mandatory — the pin door refuses outright when git cannot give it a blob
    /// oid — so a mint outside a work tree writes nothing to read a fingerprint
    /// out of.
    fn mint_fingerprint(&self) -> String {
        let mint = self.tmp.path().join("mint");
        std::fs::create_dir_all(&mint).expect("mkdir");
        std::fs::write(mint.join("doc.md"), DOC).expect("doc");
        std::fs::write(mint.join("holder.md"), "# Holder\n\nholds a pin.\n").expect("holder");
        git_ok(&mint, &["init", "-q"]);
        git_ok(&mint, &["config", "user.email", "f6@example.invalid"]);
        git_ok(&mint, &["config", "user.name", "f6"]);
        let init = self.run(&mint, &["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        let pin = self.run(&mint, &["pin", "holder.md", "doc.md#Doc/Design"]);
        assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
        let holder = std::fs::read_to_string(mint.join("holder.md")).expect("holder");
        let fp = holder
            .split_whitespace()
            .find(|t| t.contains("fp1."))
            .expect("the shipped pin door minted a fingerprint")
            .trim_matches('"')
            .to_owned();
        assert!(
            fp.starts_with("fp1."),
            "the minted token is a fingerprint: {fp}"
        );
        fp
    }

    /// The ambient workspace, carrying a hand-written form-3 CROSS-ROOT pin and one
    /// governed write so the journal plane can date the tree.
    ///
    /// Without that governed write every state below greys on the JOURNAL plane and
    /// the pin axis is masked by an unrelated refusal — the measurement would be of
    /// the wrong thing while looking identical.
    fn cross_root_corpus(&self) -> String {
        let fp = self.mint_fingerprint();
        let blob = git_out(&self.other, &["hash-object", "--", "doc.md"]);
        std::fs::write(
            self.ws.join("claim.md"),
            format!(
                "# Claim\n\nwe rely on the other root's design note.\n\n{}\n",
                lock_block("other:doc.md#Doc/Design", &blob, &fp)
            ),
        )
        .expect("claim");
        std::fs::write(
            self.ws.join("plan.md"),
            "# Plan\n\n## Goals\n\nalpha beta\n",
        )
        .expect("plan");
        git_ok(&self.ws, &["init", "-q"]);
        git_ok(&self.ws, &["config", "user.email", "f6@example.invalid"]);
        git_ok(&self.ws, &["config", "user.name", "f6"]);
        let init = self.run(&self.ws, &["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        let put = self.run_stdin(
            &self.ws,
            &["put", "plan.md"],
            "[{\"target\":{\"hpath\":[{\"h\":\"Plan\"},{\"h\":\"Goals\"}]},\
              \"edit\":{\"match\":{\"old\":\"alpha\",\"new\":\"alpha prime\"}}}]",
        );
        assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));

        // THE FIXTURE'S OWN PRECONDITION: the root is BOUND. A cross-root reading
        // over an unbound root would answer `grey unmounted` correctly, and every
        // assert below would be measuring the wrong state.
        let config = self.run(&self.ws, &["config"]);
        assert_eq!(
            config.status.code(),
            Some(0),
            "the mount table must be usable: {}",
            said(&config)
        );
        assert!(
            said(&config).contains("bound"),
            "the fixture IS the subject — `other` must be BOUND: {}",
            said(&config)
        );
        fp
    }

    /// `mrd check`'s `pins:` line, and `mrd walk`'s colour for the same ref, over
    /// the same corpus in the same instant.
    fn pin_lines(&self) -> (String, String) {
        let check = self.run(&self.ws, &["check"]);
        let pins = said(&check)
            .lines()
            .find(|l| l.trim_start().starts_with("pins:"))
            .unwrap_or("<no pins line>")
            .trim()
            .to_owned();
        let walk = self.run(&self.ws, &["walk", "claim.md"]);
        let edge = said(&walk)
            .lines()
            .find(|l| l.contains("doc.md"))
            .unwrap_or("<no walk row>")
            .trim()
            .to_owned();
        (pins, edge)
    }
}

/// **THE SENSITIVITY GATE (F6): the pin axis must MOVE on a bound root.**
///
/// MATCHED / DRIFTED / RESTORED are driven in one run over one corpus, and the
/// assert is that the three answers are not one answer. The `walk` line beside each
/// is the independent adjudicator: it always carried the real mount table, so it
/// shows what a SIGHTED instrument says about the same three states.
#[test]
fn the_pin_axis_varies_across_matched_drifted_and_restored_on_a_bound_root() {
    let sb = sandbox();
    sb.cross_root_corpus();

    let (matched, matched_walk) = sb.pin_lines();
    std::fs::write(sb.other.join("doc.md"), DOC_DRIFTED).expect("drift the target root");
    let (drifted, drifted_walk) = sb.pin_lines();
    std::fs::write(sb.other.join("doc.md"), DOC).expect("restore byte-exact");
    assert_eq!(
        std::fs::read_to_string(sb.other.join("doc.md")).expect("read back"),
        DOC,
        "the restore is byte-exact, or the third state is not the first one again"
    );
    let (restored, restored_walk) = sb.pin_lines();

    // The finding, as a single assertion: three states, three answers.
    assert_ne!(
        matched, drifted,
        "THE AXIS MUST MOVE — a pin plane that answers the same thing whether its \
         target matches or has drifted is unevidenced, and this one sits under the \
         pre-commit fence.\n  matched: {matched}\n  drifted: {drifted}"
    );
    assert_eq!(
        matched, restored,
        "and it must move BACK: a restored target reads as it did before the drift.\n  \
         matched:  {matched}\n  restored: {restored}"
    );

    // And each answer is the right one, named — not merely different from its
    // neighbour, which a random walk would also satisfy.
    assert!(
        matched.contains("green"),
        "a matched cross-root target on a bound root is GREEN: {matched}"
    );
    assert!(
        drifted.contains("content-drifted"),
        "a drifted one is RED content-drifted — the reason word, not a tone: {drifted}"
    );
    assert!(
        !matched.contains("unmounted") && !drifted.contains("unmounted"),
        "and neither is `unmounted`: the root IS bound, and saying otherwise \
         prescribes a fix already done.\n  matched: {matched}\n  drifted: {drifted}"
    );

    // The three planes agree BY CONSTRUCTION — and now on a SIGHTED answer. `walk`
    // is the adjudicator that did not move: it read the same mount table before this
    // fix and after it.
    assert!(
        matched_walk.contains("green") && restored_walk.contains("green"),
        "walk, the plane that always saw the table: {matched_walk} / {restored_walk}"
    );
    assert!(
        drifted_walk.contains("content-drifted"),
        "walk sees the drift too, and `check` now says the same thing: {drifted_walk}"
    );
}

/// **THE ACCEPTANCE (F6, gate 8): a repo holding a form-3 cross-root pin can
/// COMMIT.**
///
/// Measured on the deployed engine: `commit exit=1`, `commits 0 -> 0`, forever —
/// everything governed, the root bound, and criterion 5's acceptance arm
/// permanently unsatisfiable, because the fence's verb rolled the unseeable pin up
/// as a refusing grey. A guard that cannot be satisfied is not a guard.
#[test]
fn a_repo_holding_a_cross_root_pin_can_land_a_governed_commit() {
    let sb = sandbox();
    sb.cross_root_corpus();

    sb.place_fence();
    git_ok(&sb.ws, &["add", "-A"]);

    let before = head_count(&sb.ws);
    let commit = sb.commit("a governed commit in a repo holding a cross-root pin");
    assert!(
        commit.status.success(),
        "THE ACCEPTANCE — a bound root's matching pin may not refuse a governed \
         commit: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&sb.ws),
        before + 1,
        "R40 — the state change, not the exit: git recorded the commit"
    );
}

/// **And the fence still REFUSES when the cross-root target genuinely drifts** —
/// the other arm of the pair over the same corpus.
///
/// Without this, the acceptance above is satisfied by a plane that answers green to
/// everything, which is the same defect as answering grey to everything with the
/// sign flipped.
#[test]
fn the_fence_refuses_a_commit_whose_cross_root_target_has_drifted() {
    let sb = sandbox();
    sb.cross_root_corpus();
    sb.place_fence();

    std::fs::write(sb.other.join("doc.md"), DOC_DRIFTED).expect("drift the target root");
    git_ok(&sb.ws, &["add", "-A"]);
    let before = head_count(&sb.ws);
    let commit = sb.commit("the cross-root target moved");
    assert!(
        !commit.status.success(),
        "THE ASSERT IS THE REFUSAL — the pin's target no longer matches: {}",
        said(&commit)
    );
    assert_eq!(head_count(&sb.ws), before, "R40 — no commit was recorded");
    assert!(
        said(&commit).contains("content-drifted"),
        "and the refusal names WHAT it saw, over the root it could see: {}",
        said(&commit)
    );
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// One R4 (`version: 2`) pin, hand-written in the exact bytes `lock::render`
/// emits, so this gate depends on the CLI's own READER and not on the writer that
/// produced the row. The `page[#A/B]` convenience spelling is split into the
/// `object` wiki link and the `path` ARRAY here — R4 admits no joined string on a
/// row — and the root qualifier rides the object verbatim (`[[other:doc]]`),
/// which is what makes this pin the cross-root one the file is about.
///
/// `hash` is the git blob hash of the target file: R4 makes it MANDATORY, and
/// this file drives `mrd check`, whose anchoring plane asks git a real question
/// about it, so a placeholder would measure a different fault.
///
/// NOTE FOR REVIEWERS: `version: 1` became `version: 2`. That is the LOCK FILE
/// schema version, not the wire protocol version.
fn lock_block(declared_ref: &str, blob: &str, fingerprint: &str) -> String {
    let (target, fragment) = match declared_ref.split_once('#') {
        Some((t, f)) => (t, f),
        None => (declared_ref, ""),
    };
    let object = target.strip_suffix(".md").unwrap_or(target);
    let path = if fragment.is_empty() {
        String::new()
    } else {
        fragment
            .split('/')
            .map(|seg| format!("\"{seg}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"{blob}\"\n    path: [{path}]\n    fingerprint: \"{fingerprint}\"\n```"
    )
}

/// A fixture-setup git command whose STDOUT is the answer (`hash-object`), failing
/// loudly with git's own stderr.
fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .expect("git runs in the test environment");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs in the test environment");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// How many commits `HEAD` reaches — the state change every arm asserts (R40).
fn head_count(dir: &Path) -> usize {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("git rev-list");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// stdout+stderr together — the render rides stdout, the refusal rides stderr, and
/// what an operator SEES is the union.
fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
