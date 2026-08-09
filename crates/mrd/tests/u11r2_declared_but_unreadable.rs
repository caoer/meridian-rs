//! U11 round 2 (S3-R43) — a declared root this machine cannot read must not
//! be called "not bound". The teaching for an *undeclared* root is "declare
//! it in `~/MERIDIAN.md`"; on a root that is declared and merely unreadable
//! that sentence is false and the prescribed fix is already done. This file
//! is the CLI half of the redden.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    /// Load-bearing: the tree dies with this. Read by nothing.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    ws: PathBuf,
    config: PathBuf,
}

/// A `MERIDIAN.md` DECLARING `sessions` at `path` — whether or not `path` exists.
fn config_raw(path: &Path) -> String {
    format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# Roots\n\n\
         ```meridian-mount\nname: sessions\npath: {}\nkind: vault\nvault: sessions\n```\n",
        path.display()
    )
}

/// One R4 pin per object, hand-written in the exact bytes `lock::render`
/// emits, so these gates depend on the CLI's own reader, never on the writer.
///
/// These fixtures pin cross-root targets whose roots are unreadable on
/// purpose; the fingerprints are deliberately fake, because the subject is
/// the root grey, not the content compare.
fn lock_block(objects: &[&str]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("```meridian-lock\nversion: 2\npins:\n");
    for object in objects {
        let _ = write!(
            out,
            "  - object: \"[[{object}]]\"\n    hash: \"9ae3f1deadbeef\"\n    \
             path: []\n    fingerprint: \"fp1.span2.b3.{}\"\n",
            "0".repeat(64)
        );
    }
    out.push_str("```");
    out
}

/// `create_it` decides whether the declared directory actually exists; the
/// declaration is identical either way, making the pair a controlled
/// comparison.
fn sandbox(create_it: bool) -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    for d in [&home, &ws] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    if create_it {
        std::fs::create_dir_all(&sessions).expect("sessions");
        std::fs::write(
            sessions.join("MERIDIAN.md"),
            "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions\n",
        )
        .expect("declaration");
        std::fs::write(sessions.join("notes.md"), "# Notes\n\nreadable.\n").expect("notes");
    }

    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, config_raw(&sessions)).expect("config");
    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        config,
    }
}

impl Sandbox {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_CONFIG", &self.config)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    fn claim(&self) {
        std::fs::write(
            self.ws.join("claim.md"),
            format!("# Claim\n\n{}\n", lock_block(&["sessions:notes"])),
        )
        .expect("claim");
        let init = self.run(&["init"]);
        assert!(init.status.success(), "init failed");
    }

    /// `(reason, detail)` of the one walk entry.
    fn walk_reason(&self) -> (Output, String, String) {
        let out = self.run(&["walk", "claim.md", "--json"]);
        let v: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("walk json ({e}): {out:?}"));
        let entries = v["walk"]["entries"].as_array().expect("entries").clone();
        assert_eq!(entries.len(), 1, "one pin, one entry: {entries:?}");
        (
            out.clone(),
            entries[0]["reason"].as_str().unwrap_or_default().to_owned(),
            entries[0]["detail"].as_str().unwrap_or_default().to_owned(),
        )
    }
}

/// A root declared in `MERIDIAN.md` whose path cannot be read must not be
/// reported with the undeclared root's reason word, and its detail must name
/// the path rather than prescribe a declaration that already exists.
#[test]
fn a_declared_but_unreadable_root_is_not_reported_as_undeclared() {
    let sb = sandbox(false); // declared, but the directory does not exist
    sb.claim();

    // The independent surface, untouched by U11, gets this right.
    let cfg = String::from_utf8_lossy(&sb.run(&["config"]).stdout).into_owned();
    assert!(
        cfg.contains("grey(path-unseeable)"),
        "precondition: the MOUNT plane already classifies this state correctly, \
         so any disagreement is the ADDRESS plane's defect: {cfg}",
    );

    let (out, reason, detail) = sb.walk_reason();
    assert_ne!(
        reason,
        "unmounted",
        "a root the file DECLARES must not be reported as one this machine does \
         not bind — the undeclared teaching prescribes an action already taken: \
         {}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert_eq!(
        reason,
        addr::PATH_UNSEEABLE_REASON_WORD,
        "S3-R49 — the word is REUSED from the mount plane and taken BARE here: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert!(
        detail.contains("sessions") && detail.contains("No such file or directory"),
        "the refusal names the PATH that could not be read, and why — never the \
         mount entry, which is already correct: {detail}",
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "grey still REFUSES on exit 1 — no fourth exit code (S3-R6)",
    );
}

/// The acceptance half: the same declaration, with the directory actually
/// present, must still bind and resolve — without this, a build that reported
/// every root as unreadable would pass the gate above.
#[test]
fn the_same_declaration_with_a_readable_path_still_binds_and_resolves() {
    let sb = sandbox(true); // identical MERIDIAN.md — the path exists
    sb.claim();

    let cfg = String::from_utf8_lossy(&sb.run(&["config"]).stdout).into_owned();
    assert!(
        cfg.contains("bound"),
        "the control must actually bind, or the comparison is not controlled: {cfg}",
    );

    let (out, reason, _detail) = sb.walk_reason();
    assert_ne!(
        reason,
        "unmounted",
        "a readable declared root is not unmounted: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    // The pin's rev is deliberately wrong, so the TRUE verdict here is a measured drift — proving
    // the ref resolved into the mounted root and was actually compared, rather than being
    // short-circuited by any grey.
    assert_eq!(
        reason,
        "content-drifted",
        "bound + readable resolves and renders its TRUE verdict — the green/red \
         path is what stops grey becoming the universal answer: {}",
        String::from_utf8_lossy(&out.stdout),
    );
}

/// The third cause: declared, `Bound` per the mount table, but the corpus
/// could not be built. It must not borrow the undeclared root's word either.
/// Constructed by making the root a file where a directory is expected.
#[test]
fn a_declared_root_whose_corpus_cannot_be_built_is_not_reported_as_undeclared() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    for d in [&home, &ws] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    // A regular FILE standing where the mount path points.
    let not_a_dir = tmp.path().join("sessions");
    std::fs::write(&not_a_dir, "I am a file, not a root\n").expect("file");

    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, config_raw(&not_a_dir)).expect("config");
    let sb = Sandbox {
        tmp,
        cache_home: home.join("../xdg-cache"),
        home,
        ws,
        config,
    };
    sb.claim();

    let (out, reason, _detail) = sb.walk_reason();
    assert_ne!(
        reason,
        "unmounted",
        "a declared root whose corpus cannot be built is not an UNDECLARED root: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert_eq!(reason, addr::PATH_UNSEEABLE_REASON_WORD);
}

/// Both root greys distinct in one run: one `MERIDIAN.md` declares `sessions`
/// at an unreadable path and says nothing about `assets`; one page pins a ref
/// into each.
#[test]
fn both_root_greys_are_distinct_in_one_run_and_both_refuse() {
    let sb = sandbox(false); // `sessions` declared, path absent; `assets` undeclared
    std::fs::write(
        sb.ws.join("claim.md"),
        format!(
            "# Claim\n\n{}\n",
            lock_block(&["sessions:notes", "assets:logo"])
        ),
    )
    .expect("claim");
    assert!(sb.run(&["init"]).status.success());

    let out = sb.run(&["walk", "claim.md", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).expect("walk json");
    let entries = v["walk"]["entries"].as_array().expect("entries").clone();
    assert_eq!(entries.len(), 2, "two pins, two entries: {entries:?}");

    let word_for = |sel: &str| -> (String, String) {
        let e = entries
            .iter()
            .find(|e| e["selector"].as_str() == Some(sel))
            .unwrap_or_else(|| panic!("no entry for {sel}: {entries:?}"));
        (
            e["reason"].as_str().unwrap_or_default().to_owned(),
            e["teaching"].as_str().unwrap_or_default().to_owned(),
        )
    };

    let (declared_word, declared_teaching) = word_for("sessions:notes.md");
    let (undeclared_word, undeclared_teaching) = word_for("assets:logo.md");

    assert_eq!(declared_word, addr::PATH_UNSEEABLE_REASON_WORD);
    assert_eq!(undeclared_word, "unmounted");
    assert_ne!(
        declared_word, undeclared_word,
        "TWO causes, TWO words — collapsing them is the defect round 2 exists to fix",
    );

    // Each teaching prescribes ITS OWN fix, and never the other's.
    assert!(
        declared_teaching.contains("is declared, but the path it binds could not be read")
            && !declared_teaching.contains("Fix: declare"),
        "the DECLARED root's teaching names the path and never re-prescribes the \
         declaration: {declared_teaching}",
    );
    assert!(
        undeclared_teaching.contains("Fix: declare 'assets' in ~/MERIDIAN.md"),
        "the UNDECLARED root's teaching IS the declaration: {undeclared_teaching}",
    );

    assert_eq!(
        out.status.code(),
        Some(1),
        "both greys REFUSE, on the one exit code — no fourth (S3-R6)",
    );

    // The human line carries the same distinction — not only `--json`.
    let human = String::from_utf8_lossy(&sb.run(&["walk", "claim.md"]).stdout).into_owned();
    assert!(human.contains("grey unmounted"), "human line: {human}");
    assert!(
        human.contains(&format!("grey {}", addr::PATH_UNSEEABLE_REASON_WORD)),
        "human line carries the second word too: {human}",
    );
}

/// The pinning gate — pins the string production emits, on the surface
/// production renders it, never a `const` nothing calls.
#[test]
fn the_emitted_teaching_is_pinned_verbatim_on_the_surface_that_renders_it() {
    let sb = sandbox(false);
    std::fs::write(
        sb.ws.join("claim.md"),
        format!("# Claim\n\n{}\n", lock_block(&["assets:logo"])),
    )
    .expect("claim");
    assert!(sb.run(&["init"]).status.success());

    let human = String::from_utf8_lossy(&sb.run(&["walk", "claim.md"]).stdout).into_owned();
    let emitted = human
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("grey(unmounted):"))
        .unwrap_or_else(|| panic!("no teaching line was EMITTED at all: {human}"));

    assert_eq!(
        emitted,
        "grey(unmounted): root 'assets' is not mounted — the address \
         'assets:logo.md' names a root this machine does not bind. Not red: \
         nothing drifted, you just cannot see from here. Refs to mounted roots \
         remain served. Fix: declare 'assets' in ~/MERIDIAN.md as a mount entry \
         (name / path / kind); see [[address-grammar]].",
        "the teaching a user READS is pinned verbatim — not a const nothing renders",
    );
}

/// The pinning gate for the second word, same rule. The path is a tempdir, so
/// it is substituted rather than hardcoded — every other byte is pinned,
/// including the clause that must not appear.
#[test]
fn the_emitted_path_unseeable_teaching_is_pinned_verbatim() {
    let sb = sandbox(false);
    sb.claim();

    let human = String::from_utf8_lossy(&sb.run(&["walk", "claim.md"]).stdout).into_owned();
    let emitted = human
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("grey(path-unseeable):"))
        .unwrap_or_else(|| panic!("no path-unseeable teaching was EMITTED: {human}"));

    // The declared path, as `MERIDIAN.md` spells it.
    let declared = sb
        .config
        .parent()
        .expect("home")
        .parent()
        .expect("tmp")
        .join("sessions");
    let expected = format!(
        "grey(path-unseeable): root 'sessions' is declared, but the path it binds \
         could not be read: {} (No such file or directory (os error 2)). Not red: \
         nothing drifted, you just cannot see from here. Refs to readable roots \
         remain served. Fix: check that {} exists and is readable, or change the \
         mount entry's path; see [[address-grammar]].",
        declared.display(),
        declared.display(),
    );
    assert_eq!(emitted, expected, "the emitted teaching is pinned verbatim");
    assert!(
        !emitted.contains("Fix: declare"),
        "and it never prescribes the declaration — the root IS declared",
    );
}
