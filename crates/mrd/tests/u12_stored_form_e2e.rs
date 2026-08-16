//! U12 — criterion 4, both halves, driven through the real `mrd put` /
//! `mrd read`. Round-trip identity alone is satisfied by never translating at
//! all, so every gate asserts what the stored bytes are, not merely that they
//! survive a round trip.

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
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.cache_home);
    }
}

/// A `MERIDIAN.md` mount table binding `sessions` to `dir` under the Obsidian
/// vault name `field-notes-sessions`. The vault name deliberately differs from
/// the canonical root name — if they were equal, the vault-name assertions
/// would also pass for an implementation that stored the root name.
fn config_raw(sessions: Option<&Path>) -> String {
    use std::fmt::Write as _;
    let mut raw = "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n".to_string();
    if let Some(dir) = sessions {
        let _ = write!(
            raw,
            "```meridian-mount\nname: sessions\npath: {}\nvault: field-notes-sessions\n```\n",
            dir.display()
        );
    }
    raw
}

fn sandbox(bind_sessions: bool) -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    for d in [&home, &ws, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    // The mounted root declares its own canonical name (INV-5) — without this
    // the bind renders grey(undeclared) and every acceptance below is vacuous.
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

    let config = home.join("MERIDIAN.md");
    std::fs::write(
        &config,
        config_raw(bind_sessions.then_some(sessions.as_path())),
    )
    .expect("config");

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
        let mut owned: Vec<&str> = args.to_vec();
        if !owned.iter().any(|a| *a == "--force") {
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
            use std::io::Write as _;
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(edits.as_bytes())
                .expect("write edits");
        }
        child.wait_with_output().expect("wait mrd")
    }

    /// Seed the ambient workspace with `raw` at `page`, as a git-free workspace
    /// the engine treats as its root.
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

const PAGE: &str = "---\nroot: SESSION.md\ntitle: the def\n---\n\n# Page\n\n## Body\n\nseed\n";

// ---------------------------------------------------------------------------
// Criterion 4 — the POSITIVE half
// ---------------------------------------------------------------------------

/// **The stored form IS an `obsidian://` URI carrying the VAULT NAME** — not the
/// agent-plane `root:` spelling, and not a device-local vault id.
///
/// Measured on the BYTES ON DISK, through the production put door.
#[test]
fn a_cross_root_ref_stores_as_an_obsidian_uri_carrying_the_vault_name() {
    let sb = sandbox(true);
    sb.seed("page.md", PAGE);
    let out = sb.put(
        &["put", "page.md"],
        &put_section("see [[sessions:notes.md#Design]]\n"),
    );
    assert_eq!(code(&out), 0, "put refused: {}", stderr(&out));

    let disk = sb.read_back("page.md");
    assert!(
        disk.contains(
            "[sessions:notes.md#Design](obsidian://advanced-uri\
             ?vault=field-notes-sessions&filepath=notes.md&heading=Design)"
        ),
        "the stored form must be the canonical URI:\n{disk}",
    );
    assert!(
        !disk.contains("[[sessions:notes.md#Design]]"),
        "no agent-plane wikilink survives on disk:\n{disk}",
    );
    assert!(
        disk.contains("vault=field-notes-sessions") && !disk.contains("vault=sessions"),
        "the URI carries the VAULT name, never the canonical ROOT name:\n{disk}",
    );
}

/// **Round-trip identity**, through the two production verbs: what `mrd read`
/// renders is the agent-plane spelling the put was given, byte for byte.
#[test]
fn the_agent_plane_form_round_trips_through_the_stored_form() {
    let sb = sandbox(true);
    for agent in [
        "[[sessions:notes.md]]",
        "[[sessions:notes.md#Design]]",
        "[[sessions:notes.md#^claim-1]]",
        "[[sessions:notes.md#Design|the design note]]",
    ] {
        sb.seed("page.md", PAGE);
        let body = format!("see {agent} here\n");
        let out = sb.put(&["put", "page.md"], &put_section(&body));
        assert_eq!(code(&out), 0, "put refused for {agent}: {}", stderr(&out));

        let disk = sb.read_back("page.md");
        assert!(
            disk.contains("obsidian://"),
            "{agent} must reach disk as a stored URI:\n{disk}",
        );
        let read = sb.run(&["read", "page.md", "--section", "Page/Body"]);
        assert_eq!(code(&read), 0, "read refused: {}", stderr(&read));
        assert!(
            stdout(&read).contains(agent),
            "the rendered face must read back {agent} byte-identically:\n{}",
            stdout(&read),
        );
    }
}

// ---------------------------------------------------------------------------
// The positional bound — frontmatter is NOT an address position
// ---------------------------------------------------------------------------

/// `root: SESSION.md` in frontmatter survives a write byte-identically in
/// that span, and the pin covering it stays valid — `root:` is a live YAML
/// key in the shipped preset/def grammar, not an address position.
#[test]
fn frontmatter_root_survives_byte_identically_and_its_rev_does_not_move() {
    let sb = sandbox(true);
    sb.seed("page.md", PAGE);

    let before = sb.read_back("page.md");
    let fm_before = before
        .split("---")
        .nth(1)
        .expect("frontmatter span")
        .to_string();
    // The rev of the frontmatter node BEFORE — the value a pin covering that
    // span would have recorded.
    let rev_before = fm_rev(&before);

    let out = sb.put(
        &["put", "page.md"],
        &put_section("see [[sessions:notes.md#Design]]\n"),
    );
    assert_eq!(code(&out), 0, "put refused: {}", stderr(&out));

    let after = sb.read_back("page.md");
    let fm_after = after
        .split("---")
        .nth(1)
        .expect("frontmatter span")
        .to_string();
    assert_eq!(
        fm_after, fm_before,
        "the frontmatter span must be byte-identical — `root:` is not an address position",
    );
    assert!(
        fm_after.contains("root: SESSION.md"),
        "and the key is still there, unchanged: {fm_after}",
    );
    assert_eq!(
        fm_rev(&after),
        rev_before,
        "a pin covering the frontmatter stays valid: its node rev did not move",
    );
    assert!(
        after.contains("obsidian://"),
        "ACCEPTANCE: the body link DID translate in the same write, so the \
         untouched assertion above is not satisfied by a transform that does \
         nothing:\n{after}",
    );
}

/// The frontmatter node's rev, computed by the engine's own parse — never
/// guessed from bytes.
fn fm_rev(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    doc.root
        .children
        .iter()
        .find(|n| matches!(n.kind, model::NodeKind::Frontmatter { .. }))
        .map(|n| n.node_rev.0.clone())
        .expect("the fixture carries frontmatter")
}

// ---------------------------------------------------------------------------
// The refusals — each with the acceptance half beside it
// ---------------------------------------------------------------------------

/// An unmounted root has no vault name, so it has no stored form: the write
/// refuses and teaches the declaration. The acceptance half is the bound root
/// in the same fixture, which lands.
#[test]
fn an_unmounted_root_refuses_the_write_and_teaches_the_declaration() {
    let sb = sandbox(true);
    sb.seed("page.md", PAGE);
    let out = sb.put(
        &["put", "page.md"],
        &put_section("see [[assets:media/logo.png]]\n"),
    );
    assert_ne!(
        code(&out),
        0,
        "an unmounted root must refuse: {}",
        stdout(&out)
    );
    let msg = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        msg.contains("assets") && msg.contains("MERIDIAN.md"),
        "the refusal names the missing root and teaches the fix: {msg}",
    );
    assert_eq!(
        sb.read_back("page.md"),
        PAGE,
        "a refused write leaves the file byte-unchanged",
    );

    // ACCEPTANCE, same fixture, same door: the bound root lands.
    let ok = sb.put(
        &["put", "page.md"],
        &put_section("see [[sessions:notes.md]]\n"),
    );
    assert_eq!(code(&ok), 0, "the bound root must land: {}", stderr(&ok));
    assert!(sb.read_back("page.md").contains("obsidian://"));
}

/// The ordinary corpus is untouched — the control that keeps this transform
/// from crying wolf. An external URL parses as an address with root `https`,
/// so an unbounded transform would refuse every write carrying one.
#[test]
fn ordinary_links_and_ambient_refs_are_untouched() {
    let sb = sandbox(true);
    sb.seed("page.md", PAGE);
    let body = "[[ambient.md]] and [ext](https://example.com) and [m](mailto:a@b.example)\n\
                and `root:code.md` in a span\n";
    let out = sb.put(&["put", "page.md"], &put_section(body));
    assert_eq!(
        code(&out),
        0,
        "the ordinary corpus must land: {}",
        stderr(&out)
    );
    let disk = sb.read_back("page.md");
    assert!(disk.contains("[[ambient.md]]"), "{disk}");
    assert!(disk.contains("[ext](https://example.com)"), "{disk}");
    assert!(disk.contains("[m](mailto:a@b.example)"), "{disk}");
    assert!(disk.contains("`root:code.md`"), "{disk}");
    assert!(
        !disk.contains("obsidian://"),
        "nothing here has a stored form, so nothing was minted:\n{disk}",
    );
}

/// No machine binds anything — the state every machine starts in. A
/// cross-root ref refuses (there is no vault name to store) and the ordinary
/// corpus is unaffected, which keeps this from bricking every single-root
/// workspace.
#[test]
fn an_absent_mount_table_refuses_cross_root_and_leaves_everything_else_alone() {
    let sb = sandbox(false);
    sb.seed("page.md", PAGE);
    let refused = sb.put(
        &["put", "page.md"],
        &put_section("see [[sessions:notes.md]]\n"),
    );
    assert_ne!(code(&refused), 0, "no table, no vault name, no stored form");
    assert_eq!(sb.read_back("page.md"), PAGE, "byte-unchanged");

    let ok = sb.put(
        &["put", "page.md"],
        &put_section("ordinary [[ambient.md]]\n"),
    );
    assert_eq!(
        code(&ok),
        0,
        "an ordinary write on a machine with no mount table is untouched: {}",
        stderr(&ok),
    );
}

/// **Criterion 5 — a hand-edited stored URI fails LOUDLY on read-back** rather than resolving
/// to something plausible. The acceptance half is the canonical URI in the same fixture, which
/// reads back.
#[test]
fn a_hand_edited_stored_uri_fails_loudly_on_read_back() {
    let sb = sandbox(true);
    // Hand-written, non-canonical: `adv-uri` is the legacy action spelling and `%2F` is a path
    // separator the canon writes literally. Both name a vault this machine BINDS, so both are the
    // engine's to judge.
    sb.seed(
        "page.md",
        "# Page\n\n## Body\n\n\
         [x](obsidian://adv-uri?vault=field-notes-sessions&filepath=notes.md&heading=Design)\n",
    );
    let out = sb.run(&["read", "page.md", "--section", "Page/Body"]);
    assert_ne!(
        code(&out),
        0,
        "a governed URI that is not canonical must refuse: {}",
        stdout(&out),
    );

    // ACCEPTANCE: the canonical spelling reads back, in the same fixture.
    sb.seed(
        "page.md",
        "# Page\n\n## Body\n\n\
         [x](obsidian://advanced-uri?vault=field-notes-sessions&filepath=notes.md&heading=Design)\n",
    );
    let ok = sb.run(&["read", "page.md", "--section", "Page/Body"]);
    assert_eq!(code(&ok), 0, "the canonical form reads: {}", stderr(&ok));
    assert!(
        stdout(&ok).contains("[[sessions:notes.md#Design|x]]"),
        "and it reads back as the agent-plane spelling:\n{}",
        stdout(&ok),
    );

    // And a URI naming a vault this machine does NOT bind is left verbatim —
    // an ordinary link a human wrote, not the engine's to claim.
    sb.seed(
        "page.md",
        "# Page\n\n## Body\n\n[x](obsidian://adv-uri?vault=someone-else&filepath=a.md&block=c)\n",
    );
    let foreign = sb.run(&["read", "page.md", "--section", "Page/Body"]);
    assert_eq!(
        code(&foreign),
        0,
        "a foreign vault is not ours to refuse: {}",
        stderr(&foreign),
    );
}

/// A retained cross-root address — one the document already carried, in a
/// section this write does not touch — is not rewritten: that would move the
/// fingerprint of a node this write does not own, reddening unrelated pins.
/// The same rule the `@fp` strip follows.
#[test]
fn a_retained_agent_plane_address_is_not_this_writes_to_move() {
    let sb = sandbox(true);
    // Seeded out of band, as a pre-U12 page would be.
    let seeded = "# Page\n\n## Keep\n\nold [[sessions:notes.md]] here\n\n## Body\n\nseed\n";
    sb.seed("page.md", seeded);
    let out = sb.put(&["put", "page.md"], &put_section("fresh text\n"));
    assert_eq!(
        code(&out),
        0,
        "the untouched section must not block: {}",
        stderr(&out)
    );
    let disk = sb.read_back("page.md");
    assert!(
        disk.contains("old [[sessions:notes.md]] here"),
        "a retained address is left exactly as it was:\n{disk}",
    );
    assert!(disk.contains("fresh text"), "and the edit landed:\n{disk}");
}
