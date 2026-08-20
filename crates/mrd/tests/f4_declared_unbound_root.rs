//! F4 — a root declared in `MERIDIAN.md` but not bound on this machine: the ordinary laptop
//! case, `~/MERIDIAN.md` declaring a root that is not checked out here. `bind_one` renders it
//! `MountState::PathUnseeable` (never an error — failing there would brick every machine that
//! does not hold all declared roots), so `projection()` files it under `MountSet::unreachable`
//! and `is_bound` is false.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    /// Load-bearing: the tree is deleted when this drops.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    ws: PathBuf,
    config: PathBuf,
    /// The path `notes` is declared at and which deliberately does NOT exist.
    absent: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.cache_home);
    }
}

/// A mount table declaring TWO roots: `sessions`, bound to a real directory, and
/// `notes`, declared at a path that does not exist.
///
/// Both arms live in ONE table on purpose. A fixture carrying only the unbound
/// root would let a fix that refuses every cross-root write pass every refusal
/// gate below (S3-R8(c)).
fn config_raw(sessions: &Path, absent: &Path) -> String {
    format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
         ```meridian-mount\nname: sessions\npath: {}\nvault: field-notes-sessions\n```\n\n\
         ```meridian-mount\nname: notes\npath: {}\nvault: field-notes-notes\n```\n",
        sessions.display(),
        absent.display(),
    )
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    for d in [&home, &ws, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    // Anchor the ambient workspace: an unanchored tree now refuses (outside a
    // declared workspace, exit 2) — these arms test declared-unbound roots,
    // not the resolution tier.
    std::fs::create_dir_all(ws.join(".git")).expect(".git");
    // The bound root declares its own canonical name (INV-5) — without this the
    // bind renders grey(undeclared) and the acceptance half is vacuous.
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("root declaration");
    std::fs::write(
        sessions.join("notes.md"),
        "# Notes\n\n## Design\n\nthe sessions root's design note.\n",
    )
    .expect("target");

    // Declared, and deliberately never created — the machine that does not hold
    // this root.
    let absent = tmp.path().join("absent-notes-root");
    assert!(
        !absent.exists(),
        "the fixture's whole point is that this path does not exist",
    );

    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, config_raw(&sessions, &absent)).expect("config");

    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        config,
        absent,
    }
}

impl Sandbox {
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_CONFIG", &self.config)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("spawn mrd")
    }

    fn put(&self, args: &[&str], edits: &str) -> Output {
        // Wire-origin put demands fingerprint-or-force. These arms test
        // declared-but-unbound roots, not the guard.
        let mut owned: Vec<&str> = args.to_vec();
        if !owned.contains(&"--force") {
            owned.push("--force");
        }
        let mut child = self
            .command(&owned)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        {
            common::feed_stdin(&mut child, edits.as_bytes());
        }
        child.wait_with_output().expect("wait mrd")
    }

    fn seed(&self, page: &str, raw: &str) {
        std::fs::write(self.ws.join(page), raw).expect("seed");
    }

    fn read_back(&self, page: &str) -> String {
        std::fs::read_to_string(self.ws.join(page)).expect("read back")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// A one-edit batch replacing the whole `Body` section.
fn put_section(new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Page"}, {"h": "Body"}]},
        "edit": {"put": {"at": "content", "text": new}},
    }]))
    .expect("edits json")
}

const PAGE: &str = "# Page\n\n## Body\n\nseed\n";

// ---------------------------------------------------------------------------
// The fixture's own control — the state under test is real
// ---------------------------------------------------------------------------

/// The fixture asserts its own premise: `mrd config` must report `notes` as declared and
/// unreachable, distinctly from an undeclared root — otherwise every gate below is measuring
/// some other state and passing for the wrong reason.
#[test]
fn the_fixture_really_declares_an_unreachable_root() {
    let sb = sandbox();
    let out = sb.run(&["config"]);
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.contains("notes"),
        "the table must carry the declared-but-unbound root:\n{text}",
    );
    assert!(
        text.contains(&sb.absent.display().to_string()),
        "and name the path a reader must go and check:\n{text}",
    );
    assert!(
        text.contains("sessions"),
        "the bound root is in the same table — both arms, one fixture:\n{text}",
    );
}

// ---------------------------------------------------------------------------
// F4 — THE DEFECT, at both positions
// ---------------------------------------------------------------------------

/// The finding: a markdown link naming a declared-but-unbound root reaches disk in its
/// agent-plane spelling, untranslated and unguarded, at exit 0. `root:` is the agent plane's
/// spelling and is unresolvable garbage to Obsidian on disk (criterion 4).
#[test]
fn a_markdown_link_to_a_declared_unbound_root_must_not_reach_disk_raw() {
    let sb = sandbox();
    sb.seed("page.md", PAGE);
    let out = sb.put(&["put", "page.md"], &put_section("see [x](notes:a.md)\n"));

    let disk = sb.read_back("page.md");
    assert!(
        !disk.contains("(notes:a.md)"),
        "F4: the agent-plane `root:` spelling reached disk untranslated and \
         unguarded at exit {} — a link no reader can follow:\n{disk}",
        code(&out),
    );
    assert_ne!(
        code(&out),
        0,
        "an address with no stored form must REFUSE the write, never land it raw",
    );
    assert_eq!(
        sb.read_back("page.md"),
        PAGE,
        "a refused write leaves the file byte-unchanged",
    );
}

/// The refusal must teach the right thing — it names the path to check, not a declaration
/// that already exists: the mount entry is already correct, so prescribing "declare it in
/// `~/MERIDIAN.md`" spends the user's trust and points at nothing (S3-R43/S3-R50).
#[test]
fn the_refusal_names_the_path_never_a_declaration_already_made() {
    let sb = sandbox();
    sb.seed("page.md", PAGE);
    let out = sb.put(&["put", "page.md"], &put_section("see [x](notes:a.md)\n"));
    let msg = format!("{}{}", stdout(&out), stderr(&out));

    assert!(
        msg.contains(&sb.absent.display().to_string()),
        "the refusal must name the declared PATH a reader must go and check:\n{msg}",
    );
    assert!(
        !msg.contains("declare it in"),
        "and must NOT prescribe a declaration that is already done:\n{msg}",
    );
}

/// The asymmetry, stated as a gate: position 1 (wikilink) already refuses this root; position
/// 2 (markdown link) did not. Both arms are asserted so a future edit cannot fix one and
/// silently regress the other.
#[test]
fn both_positions_refuse_a_declared_unbound_root_identically() {
    for body in ["see [[notes:a.md]]\n", "see [x](notes:a.md)\n"] {
        let sb = sandbox();
        sb.seed("page.md", PAGE);
        let out = sb.put(&["put", "page.md"], &put_section(body));
        assert_ne!(
            code(&out),
            0,
            "both address positions must refuse a declared-but-unbound root; \
             {body:?} did not: {}",
            stdout(&out),
        );
        assert_eq!(
            sb.read_back("page.md"),
            PAGE,
            "and neither may land bytes: {body:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// BOTH ARMS — the fix must not buy the refusal with the ordinary corpus
// ---------------------------------------------------------------------------

/// **ACCEPTANCE 1 — the bound root still translates**, in the same fixture and
/// the same table that carries the unreachable one.
#[test]
fn a_bound_root_still_translates_at_both_positions() {
    let sb = sandbox();

    sb.seed("page.md", PAGE);
    let wiki = sb.put(
        &["put", "page.md"],
        &put_section("see [[sessions:notes.md#Design]]\n"),
    );
    assert_eq!(
        code(&wiki),
        0,
        "the bound root must land: {}",
        stderr(&wiki)
    );
    let disk = sb.read_back("page.md");
    assert!(
        disk.contains(
            "[sessions:notes.md#Design](obsidian://advanced-uri\
             ?vault=field-notes-sessions&filepath=notes.md&heading=Design)"
        ),
        "the canonical stored URI, carrying the VAULT name:\n{disk}",
    );

    sb.seed("page.md", PAGE);
    let md = sb.put(
        &["put", "page.md"],
        &put_section("see [the design](sessions:notes.md#Design)\n"),
    );
    assert_eq!(
        code(&md),
        0,
        "a markdown link to a BOUND root must still translate: {}",
        stderr(&md),
    );
    let disk = sb.read_back("page.md");
    assert!(
        disk.contains("obsidian://advanced-uri?vault=field-notes-sessions"),
        "position 2 on a bound root still mints the stored form:\n{disk}",
    );
    assert!(
        !disk.contains("(sessions:notes.md#Design)"),
        "and no agent-plane spelling survives:\n{disk}",
    );
}

/// Acceptance 2 — the ordinary corpus is untouched: an external URL parses as an address with
/// root `https`, so a naive refusal scan would cry wolf on the majority case.
#[test]
fn ordinary_external_links_are_still_untouched() {
    let sb = sandbox();
    sb.seed("page.md", PAGE);
    let body = "[[ambient.md]] and [ext](https://example.com) and [m](mailto:a@b.example)\n\
                and [rel](./sibling.md) and `notes:code.md` in a span\n";
    let out = sb.put(&["put", "page.md"], &put_section(body));
    assert_eq!(
        code(&out),
        0,
        "the ordinary corpus must land — a fix that refuses external links is \
         an instrument that cries wolf on the majority case: {}",
        stderr(&out),
    );
    let disk = sb.read_back("page.md");
    assert!(disk.contains("[ext](https://example.com)"), "{disk}");
    assert!(disk.contains("[m](mailto:a@b.example)"), "{disk}");
    assert!(disk.contains("[rel](./sibling.md)"), "{disk}");
    assert!(disk.contains("[[ambient.md]]"), "{disk}");
    assert!(disk.contains("`notes:code.md`"), "{disk}");
    assert!(
        !disk.contains("obsidian://"),
        "nothing here has a stored form, so nothing was minted:\n{disk}",
    );
}

/// **ACCEPTANCE 3 — a document with no agent-plane occupant writes clean**, on a machine whose
/// table carries an unreachable root. The mere PRESENCE of a declared-but-unbound root may not
/// disturb ordinary single-root traffic.
#[test]
fn a_document_with_no_cross_root_position_writes_clean() {
    let sb = sandbox();
    sb.seed("page.md", PAGE);
    let out = sb.put(
        &["put", "page.md"],
        &put_section("just ordinary prose, no addresses at all\n"),
    );
    assert_eq!(
        code(&out),
        0,
        "the ordinary write must land: {}",
        stderr(&out)
    );
    assert!(
        sb.read_back("page.md").contains("just ordinary prose"),
        "and the edit landed",
    );
}

/// Acceptance 4 — a retained address is still not this write's to move: a
/// declared-but-unbound address in a section this write does not touch must not turn every
/// unrelated edit into a refusal, or any page that ever mentioned an unreachable root becomes
/// permanently unwritable.
#[test]
fn a_retained_unbound_address_does_not_block_an_unrelated_edit() {
    let sb = sandbox();
    let seeded = "# Page\n\n## Keep\n\nold [x](notes:a.md) here\n\n## Body\n\nseed\n";
    sb.seed("page.md", seeded);
    let out = sb.put(&["put", "page.md"], &put_section("fresh text\n"));
    assert_eq!(
        code(&out),
        0,
        "an untouched section carrying an unreachable address must not block an \
         unrelated edit: {}",
        stderr(&out),
    );
    let disk = sb.read_back("page.md");
    assert!(
        disk.contains("old [x](notes:a.md) here"),
        "the retained address is left exactly as it was:\n{disk}",
    );
    assert!(disk.contains("fresh text"), "and the edit landed:\n{disk}");
}
