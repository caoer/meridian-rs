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
        common::reap_daemon(&self.home, &self.cache_home);
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
    // Anchor the ambient workspace: an unanchored tree now refuses (outside a
    // declared workspace, exit 2) — these arms test cross-root behavior, not
    // the resolution tier.
    std::fs::create_dir_all(ws.join(".git")).expect(".git");
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
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(args)
            .current_dir(&self.ws)
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

/// **Criterion 5 — a hand-edited stored URI is served AS WRITTEN on read-back and flagged in the
/// answer header**, never resolved to something plausible and never a refusal of the call. The
/// acceptance half is the canonical URI in the same fixture, which reads back.
#[test]
fn a_hand_edited_stored_uri_is_served_as_written_and_flagged_on_read_back() {
    let sb = sandbox(true);
    // Hand-written, non-canonical: `adv-uri` is the legacy action spelling. It names a vault this
    // machine BINDS, so it is the engine's to judge — and the engine does not guess.
    let hand_edited =
        "[x](obsidian://adv-uri?vault=field-notes-sessions&filepath=notes.md&heading=Design)";
    sb.seed("page.md", &format!("# Page\n\n## Body\n\n{hand_edited}\n"));
    let out = sb.run(&["read", "page.md", "--section", "Page/Body"]);
    assert_eq!(
        code(&out),
        0,
        "a governed URI that is not canonical is served, not refused: {}",
        stderr(&out),
    );
    let text = stdout(&out);
    assert!(
        text.contains(&format!("\n{hand_edited}\n")),
        "the link is served byte-identical to disk, untranslated:\n{text}",
    );
    assert_eq!(
        text.matches("a stored cross-root link is left as written")
            .count(),
        1,
        "one notice in the header, naming the link:\n{text}",
    );
    assert!(
        text.contains("line 5: a stored cross-root link is left as written"),
        "the notice names the link's line in the file:\n{text}",
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

/// A hand-written `obsidian://open` link whose `file` parameter holds a literal `=` (the
/// `year=…/month=…` session layout) is not canon-encoded. A section read serves it at rc 0, byte-
/// identical to disk, with one header notice; the toc read serves too, and the two faces agree on
/// the section's rev.
#[test]
fn a_non_canonical_file_parameter_reads_at_rc_0_on_both_faces() {
    let sb = sandbox(true);
    let link = "[x](obsidian://open?vault=field-notes-sessions&file=year=2026/month=10/foo.md)";
    sb.seed(
        "page.md",
        &format!("# Page\n\n## Body\n\nsee {link} here\n\n## Other\n\nplain\n"),
    );

    let section = sb.run(&["read", "page.md", "--section", "Page/Body", "--json"]);
    assert_eq!(
        code(&section),
        0,
        "a section holding a non-canonical link is served: {}",
        stderr(&section),
    );
    let section: serde_json::Value = serde_json::from_str(&stdout(&section)).expect("json");
    let read = &section["read"];
    assert_eq!(
        read["sections"][0]["content"].as_str().expect("content"),
        format!("## Body\n\nsee {link} here\n\n"),
        "the raw plane is the bytes on disk",
    );
    let rendered = read["rendered_text"].as_str().expect("rendered_text");
    assert!(
        rendered.contains(link),
        "the rendered face leaves it as written:\n{rendered}"
    );
    let notice = read["notice"].as_str().expect("a notice");
    assert!(
        notice.starts_with("line 5: a stored cross-root link is left as written")
            && notice.contains("`file` parameter is not canon-encoded"),
        "the notice names the line and the decode's words: {notice}",
    );
    assert!(
        rendered.contains("notice: "),
        "the human face carries it in the header:\n{rendered}",
    );

    let other = sb.run(&["read", "page.md", "--section", "Page/Other", "--json"]);
    assert_eq!(code(&other), 0, "{}", stderr(&other));
    let other: serde_json::Value = serde_json::from_str(&stdout(&other)).expect("json");
    assert!(
        other["read"]["notice"].is_null(),
        "a section that does not hold the link carries no notice: {other}",
    );

    let toc = sb.run(&["read", "page.md", "--json"]);
    assert_eq!(code(&toc), 0, "the toc read serves: {}", stderr(&toc));
    let toc: serde_json::Value = serde_json::from_str(&stdout(&toc)).expect("json");
    let body_rev = toc["read"]["toc"]
        .as_array()
        .expect("toc rows")
        .iter()
        .find(|r| r["title"] == "Body")
        .map(|r| r["sec_rev"].clone())
        .expect("a Body row");
    assert_eq!(
        body_rev, read["sections"][0]["sec_rev"],
        "the toc read and the section read agree on the section",
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
