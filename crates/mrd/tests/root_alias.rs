//! The root alias (`meridian-md-schema.md` § 5.1b, `address-grammar.md` § 4.6a):
//! ONE constant a skill can spell — `sessions:` — that any machine's mount table
//! maps to whatever that machine calls the root.
//!
//! The four acceptance checks of the accepted hook-support design § 2.1, each
//! run through the real binary against a real `MERIDIAN.md`:
//!
//! 1. an aliased mount resolves `sessions:`, and the row names the mount while
//!    the canonical `ref:` echo carries the NAME, never the alias;
//! 2. a mount NAMED `sessions` resolves `sessions:` with no alias line — a name
//!    is its own alias, and there is no special case to get wrong;
//! 3. a table with neither refuses, naming the missing root and teaching the one
//!    line that would make the constant resolve;
//! 4. an alias shadowing any name refuses the WHOLE table — nothing partially
//!    loaded.
//!
//! `primary:` is deliberately absent from every fixture but the one that proves
//! it is not consulted: the designation means exactly what it meant
//! before, and no derivation reads it for `sessions`.

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
    /// The directory every fixture mounts, whatever it names it.
    tree: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

/// A sandbox whose `~/MERIDIAN.md` is `blocks`, and whose mounted tree declares
/// itself as `declared_name` (INV-5 — without the root's own declaration the
/// bind renders `grey(undeclared)` and every acceptance arm goes vacuous).
fn sandbox(declared_name: &str, blocks: impl Fn(&Path) -> String) -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let tree = tmp.path().join("the-tree");
    for d in [&home, &ws, &tree] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    // Anchor the ambient workspace: an unanchored tree refuses at exit 2, and
    // these arms are about root resolution, not the resolution tier.
    std::fs::create_dir_all(ws.join(".git")).expect(".git");
    std::fs::write(
        tree.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {declared_name}\n---\n\n# Root\n"),
    )
    .expect("root declaration");
    std::fs::write(tree.join("x.md"), "# X\n\nthe target page.\n").expect("target");

    let config = home.join("MERIDIAN.md");
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n{}",
            blocks(&tree)
        ),
    )
    .expect("config");

    let cache_home = tmp.path().join("xdg-cache");
    // The engine canonicalizes at bind (`address-grammar.md` § 8), and on macOS
    // the temp dir is reached through a `/private` symlink — so the fixture must
    // hold the canonical spelling or every landed-path assertion compares the
    // symlink against its target.
    let tree = std::fs::canonicalize(&tree).expect("canonical tree");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        config,
        tree,
    }
}

impl Sandbox {
    fn run(&self, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(&self.ws)
            .env("MERIDIAN_CONFIG", &self.config)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    fn stdout(&self, args: &[&str]) -> String {
        let out = self.run(args);
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// stdout+stderr together — for the write faces, whose human receipt and
    /// refusal do not land on one stream.
    fn both(&self, args: &[&str]) -> String {
        let out = self.run(args);
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }

    /// A `put` batch fed on stdin. Wire-origin puts demand fingerprint-or-force;
    /// these arms are about the SPELLING that comes back, not the guard.
    fn put(&self, args: &[&str], edits: &str) -> Output {
        let mut owned: Vec<&str> = args.to_vec();
        if !owned.contains(&"--force") {
            owned.push("--force");
        }
        let mut child = common::mrd_command(&self.home, &self.cache_home)
            .args(&owned)
            .current_dir(&self.ws)
            .env("MERIDIAN_CONFIG", &self.config)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, edits.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }
}

fn json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|e| panic!("not json ({e}): {raw}"))
}

/// The substring trap this whole file must not fall into, as a helper.
///
/// `field-notes-sessions` ENDS IN `sessions`, so `haystack.contains("sessions:")`
/// is true for BOTH spellings and asserts nothing. Every negative assertion here
/// anchors on a boundary the alias would have to occupy: the `[[` of a lock
/// object, the start of a receipt's path word, a `"` in JSON.
fn assert_no_alias_spelling(haystack: &str, alias: &str, what: &str) {
    for boundary in ["[[", "\"", " ", "(", "\n"] {
        let needle = format!("{boundary}{alias}:");
        assert!(
            !haystack.contains(&needle),
            "{what}: the alias `{alias}:` appears at a `{boundary}` boundary — an alias is a \
             lookup spelling and reaches no stored byte or echoed line (§ 4.6a):\n{haystack}",
        );
    }
}

/// Check 1 — an aliased mount answers the constant, and every canonical echo
/// still prints the MOUNT's name.
///
/// This is the whole point of the field: `field-notes-sessions` is what THIS
/// machine calls the tree, `sessions:` is what a skill is allowed to hard-code,
/// and nothing the engine writes back learns the alias.
#[test]
fn an_aliased_mount_answers_the_constant_and_echoes_the_name() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\nalias: sessions\n```\n",
            tree.display()
        )
    });

    let out = sb.run(&["resolve", "sessions:x.md", "--json"]);
    assert!(
        out.status.success(),
        "the alias must resolve: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = json(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(
        value["root"], "field-notes-sessions",
        "the row names the MOUNT"
    );
    assert_eq!(value["alias"], "sessions", "and which spelling landed it");
    assert_eq!(
        value["ref"], "field-notes-sessions:x.md",
        "the canonical ref echo carries the name — never the alias (§ 4.6a)",
    );
    assert_eq!(
        value["path"],
        sb.tree.join("x.md").display().to_string(),
        "and it lands in the aliased mount's tree",
    );

    // The human face carries the same two facts, in the design's own shape.
    let human = sb.stdout(&["resolve", "sessions:x.md"]);
    assert!(
        human.contains("root: field-notes-sessions (alias sessions)"),
        "the human row must name the mount and the alias that reached it:\n{human}",
    );
    assert!(
        human.contains("ref: field-notes-sessions:x.md"),
        "the canonical echo is the name:\n{human}",
    );

    // The mount's own name never stops working: an alias ADDS a spelling.
    let by_name = json(&sb.stdout(&["resolve", "field-notes-sessions:x.md", "--json"]));
    assert_eq!(by_name["root"], "field-notes-sessions");
    assert_eq!(
        by_name["alias"],
        serde_json::Value::Null,
        "no alias was spelled, so none is reported",
    );
    assert_eq!(
        by_name["path"], value["path"],
        "both spellings land in one tree"
    );
}

/// Check 2 — a mount NAMED `sessions` resolves `sessions:` with no alias line.
///
/// The no-special-case half of § 5.1b: name-first-then-alias means a name is its
/// own alias, so the machine that already calls its root `sessions` writes
/// nothing new. Also the `primary:` half of the law — the designation sits on a
/// DIFFERENT mount here, and nothing derives `sessions` from it.
#[test]
fn a_name_is_its_own_alias_and_primary_is_not_consulted() {
    let sb = sandbox("sessions", |tree| {
        format!(
            "```meridian-mount\nname: sessions\npath: {}\nvault: sessions-vault\n```\n\n\
             ```meridian-mount\nname: elsewhere\npath: {}\nprimary: true\n```\n",
            tree.display(),
            tree.parent().expect("parent").join("project").display(),
        )
    });

    let value = json(&sb.stdout(&["resolve", "sessions:x.md", "--json"]));
    assert_eq!(value["root"], "sessions");
    assert_eq!(
        value["alias"],
        serde_json::Value::Null,
        "no alias line was needed, so none is reported",
    );
    assert_eq!(value["path"], sb.tree.join("x.md").display().to_string());
    assert_eq!(
        value["primary"], false,
        "the primary designation sits on ANOTHER mount and is not consulted for `sessions` (ZT 475)",
    );
}

/// Check 3 — a table with no mount named or aliased `sessions` refuses, and the
/// refusal teaches the ONE line that would make the constant resolve.
///
/// "Declare the mount" alone is the WRONG remedy here: the tree is already
/// mounted, under another name. A refusal that only said that would send a
/// reader to add a second mount for a tree they have.
#[test]
fn a_table_with_neither_refuses_and_teaches_the_alias_line() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\n```\n",
            tree.display()
        )
    });

    let out = sb.run(&["resolve", "sessions:x.md"]);
    assert!(!out.status.success(), "an unknown root must refuse");
    assert_eq!(
        out.status.code(),
        Some(1),
        "an address-plane refusal is an ANSWER about this machine's topology, exit 1",
    );
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("names root `sessions`, which this machine does not bind"),
        "the refusal must name the missing root:\n{text}",
    );
    assert!(
        text.contains("declare `alias: sessions` on the mount that holds that tree"),
        "the refusal must teach the alias line, verbatim:\n{text}",
    );
    assert!(
        text.contains("field-notes-sessions"),
        "and enumerate what DOES bind, so a reader can pick the mount to alias:\n{text}",
    );
    assert!(
        text.contains(
            "Or scaffold the implicit default: mkdir -p ~/.local/share/ucc/sessions \
             && mrd init ~/.local/share/ucc/sessions --name sessions."
        ),
        "the refusal must teach the §5.1c scaffold beside the alias line:\n{text}",
    );
}

/// Check 5 — the implicit default (schema §5.1c): a machine that maps NOTHING
/// resolves `sessions:` anyway once `$HOME/.local/share/ucc/sessions` is
/// scaffolded, and both config faces mark the row implicit.
#[test]
fn a_scaffolded_default_answers_the_constant_with_no_declaration_at_all() {
    // The declared world knows only `field-notes` — nothing claims `sessions`.
    let sb = sandbox("field-notes", |tree| {
        format!(
            "```meridian-mount\nname: field-notes\npath: {}\n```\n",
            tree.display()
        )
    });

    // Scaffold the default tree in the sandbox HOME, exactly as the refusal
    // teaches (mkdir + declaration; `mrd init` writes the same §4 bytes).
    let sessions = sb.home.join(".local/share/ucc/sessions");
    std::fs::create_dir_all(&sessions).expect("scaffold the default tree");
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Root\n",
    )
    .expect("default declaration");
    std::fs::write(sessions.join("x.md"), "# X\n\nthe target page.\n").expect("target");
    let canonical_sessions = std::fs::canonicalize(&sessions).expect("canonical default tree");

    let out = sb.run(&["resolve", "sessions:x.md", "--json"]);
    assert!(
        out.status.success(),
        "the implicit default must resolve the constant: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = json(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(value["root"], "sessions", "the default's own name");
    assert_eq!(
        value["alias"],
        serde_json::Value::Null,
        "the name answered, no alias was involved",
    );
    assert_eq!(
        value["path"],
        canonical_sessions.join("x.md").display().to_string(),
        "and it lands in the default tree",
    );

    // Both config faces carry the provenance the wire deliberately does not.
    let config = json(&sb.stdout(&["config", "--json"]));
    let mounts = config["mounts"].as_array().expect("mounts array");
    let row = mounts
        .iter()
        .find(|m| m["name"] == "sessions")
        .expect("the default row is served");
    assert_eq!(row["implicit"], true, "--json marks the defaulted row");
    assert_eq!(row["state"], "bound");
    let declared = mounts
        .iter()
        .find(|m| m["name"] == "field-notes")
        .expect("the declared row");
    assert_eq!(
        declared["implicit"], false,
        "a declared row never reads implicit"
    );

    let human = sb.stdout(&["config"]);
    assert!(
        human.contains("(implicit default)"),
        "the human face marks the defaulted row:\n{human}",
    );
}

/// Check 4 — an alias equal to any mount's name refuses the WHOLE table.
///
/// Not a warning and not a dropped row: `NO_PARTIAL_LOAD_CLAUSE` means no mount
/// table loads at all, so a shadowed name can never be resolved half-right. The
/// shadowed name is declared AFTER the shadowing alias on purpose — the half an
/// incremental, block-at-a-time check would miss.
#[test]
fn an_alias_shadowing_a_name_refuses_the_whole_table() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\nalias: notes\n```\n\n\
             ```meridian-mount\nname: notes\npath: {}\n```\n",
            tree.display(),
            tree.parent().expect("parent").join("project").display(),
        )
    });

    let out = sb.run(&["config"]);
    assert!(!out.status.success(), "a shadowing alias must refuse");
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("alias: notes") && text.contains("`name` of a meridian-mount block"),
        "the refusal must name the collision:\n{text}",
    );
    assert!(
        text.contains("No mount table was loaded; the config is not partially applied."),
        "a shadow refuses the whole table, never a dropped row:\n{text}",
    );

    // And no door serves a partially-loaded table: even the mount that does NOT
    // collide is unreachable while the file is broken.
    let resolve = sb.run(&["resolve", "field-notes-sessions:x.md"]);
    assert!(
        !resolve.status.success(),
        "nothing resolves off a table that refused to load",
    );
}

/// The `alias` column is on both config faces (§ 2.1's `roots`/`mounts` row), and
/// prints only where one is declared: absence is the majority row, so a marker
/// on every line would cost every reader to state nothing.
#[test]
fn the_config_faces_publish_the_alias_column() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\nalias: sessions\n```\n",
            tree.display()
        )
    });

    let human = sb.stdout(&["config"]);
    assert!(
        human.contains("alias:sessions"),
        "the human mount row publishes the alias:\n{human}",
    );

    let value = json(&sb.stdout(&["config", "--json"]));
    assert_eq!(value["mounts"][0]["alias"], "sessions");
    assert_eq!(
        value["mounts"][0]["name"], "field-notes-sessions",
        "the name leg is untouched by the alias leg",
    );
}

/// F5 (a) — a cross-root PIN through an alias stores the mount's NAME in the
/// `meridian-lock` object, and echoes it back.
///
/// This is the sharpest form of the canonical-spelling law, because a lock
/// object is PORTABLE SHARED CONTENT: the bytes travel to machines whose tables
/// never heard of this alias, and a `[[sessions:…]]` object there resolves to
/// nothing. `mrd pin` writes the lock through the daemon splice, so this arm
/// exercises the real write path, not a projection of it.
#[test]
fn a_cross_root_pin_stores_the_mount_name_never_the_alias() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\nalias: sessions\n```\n\n\
             ```meridian-mount\nname: pinner-root\npath: {}\nvault: pinner-vault\n```\n",
            tree.display(),
            tree.parent().expect("parent").join("project").display(),
        )
    });
    // The PINNING page lives in the ambient workspace; the TARGET is reached by
    // the alias — the pin-cross-root shape, the only one that can leak.
    std::fs::write(
        sb.ws.join("pinner.md"),
        "---\ntype: note\n---\n\n# Pinner\n\n## Notes\n\nseed\n",
    )
    .expect("seed the pinning page");
    std::fs::write(
        sb.tree.join("target.md"),
        "# Target\n\n## Design\n\nthe pinned section.\n",
    )
    .expect("seed the target");
    // An R4 pin's hash IS the target's meaning, and it comes from git — so the
    // target root must be a real work tree. `--vibe` writes the blob into the
    // object store so the pin is retrievable before any commit references it.
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&sb.tree)
            .output()
            .expect("git")
            .status
            .success();
        assert!(ok, "git {args:?} failed in the target tree");
    };
    git(&["init", "--quiet"]);

    // The selector is a ROOT-ANCHORED heading path — it starts at the file's top
    // heading, so `Target/Design`, never bare `Design`.
    let out = sb.run(&[
        "pin",
        "pinner.md",
        "sessions:target.md#Target/Design",
        "--vibe",
    ]);
    let said =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the alias must pin: {said}");

    let lock = std::fs::read_to_string(sb.ws.join("pinner.md")).expect("read back the lock");
    assert!(
        lock.contains("[[field-notes-sessions:target"),
        "the lock object carries the MOUNT's name — these bytes are portable:\n{lock}",
    );
    assert_no_alias_spelling(&lock, "sessions", "the stored meridian-lock");
    assert_no_alias_spelling(&said, "sessions", "the pin receipt");
}

/// F5 (b) — a `put` through an alias echoes the mount's NAME in its receipt.
///
/// The receipt is a line a human or an agent acts on afterwards, often on
/// another machine; naming a root by this machine's private lookup spelling
/// sends them somewhere that does not exist. `--json` is asserted beside the
/// human face because the two used to disagree, and one clean face hid the other.
#[test]
fn a_put_through_an_alias_echoes_the_mount_name() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\nalias: sessions\n```\n",
            tree.display()
        )
    });
    std::fs::write(
        sb.tree.join("page.md"),
        "---\ntype: note\n---\n\n# Page\n\n## Notes\n\nseed\n",
    )
    .expect("seed");

    let edits = r#"[{"target":{"hpath":[{"h":"Page"},{"h":"Notes"}]},"edit":{"match":{"old":"seed","new":"landed"}}}]"#;
    let out = sb.put(&["put", "sessions:page.md"], edits);
    let said =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the alias must write: {said}");
    assert!(
        said.contains("committed field-notes-sessions:page.md"),
        "the human receipt names the MOUNT, rooted as the caller wrote rooted:\n{said}",
    );
    assert_no_alias_spelling(&said, "sessions", "the put receipt");

    // It really wrote, through the alias, to the aliased tree.
    let landed = std::fs::read_to_string(sb.tree.join("page.md")).expect("read back");
    assert!(
        landed.contains("landed"),
        "the edit reached the tree:\n{landed}"
    );

    // The --json face, which was already clean, must stay clean AND agree.
    let out = sb.put(
        &["put", "sessions:page.md", "--json"],
        &edits
            .replace("seed", "landed")
            .replace("landed\",\"new\":\"landed", "landed\",\"new\":\"again"),
    );
    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "the json face writes too: {raw}");
    assert_no_alias_spelling(&raw, "sessions", "the put --json receipt");
}

/// F3 — the pin door's unbound-root refusal teaches the ALIAS line too.
///
/// An agent spelling the agreed constant at a write door, on a machine that
/// mounts that very tree under its own name, needs ONE alias line. "Declare the
/// mount in the target root's own MERIDIAN.md and bind it" alone is the wrong
/// remedy — it sends them to duplicate a root they already have.
#[test]
fn the_pin_door_teaches_the_alias_line_on_an_unbound_root() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\n```\n",
            tree.display()
        )
    });
    std::fs::write(
        sb.ws.join("pinner.md"),
        "---\ntype: note\n---\n\n# Pinner\n\n## Notes\n\nseed\n",
    )
    .expect("seed");

    let said = sb.both(&["pin", "pinner.md", "sessions:target.md#Design"]);
    assert!(
        said.contains("declare `alias: sessions` on the mount that holds that tree"),
        "the pin door carries the alias teaching, like the read doors:\n{said}",
    );
    assert!(
        said.contains("field-notes-sessions"),
        "and enumerates what DOES bind, so a reader can pick the mount to alias:\n{said}",
    );
}

/// D1/D2/D3 — the remaining doors that echo a resolved rooted ref: `rm`'s
/// deletion receipt, `walk`'s root (both faces), and `pin`'s pinning-PAGE half.
///
/// One law, one helper, one arm: every door that echoes a rooted ref echoes the
/// mount's NAME. These three were missed when `canonical_ref` was introduced
/// because they hold the same `replace(rel)` shape the fixed doors held — which
/// is the argument for a shared seam and against per-door judgement.
#[test]
fn every_echoing_door_names_the_mount_not_the_alias() {
    let sb = sandbox("field-notes-sessions", |tree| {
        format!(
            "```meridian-mount\nname: field-notes-sessions\npath: {}\nvault: field-notes-sessions\nalias: sessions\n```\n",
            tree.display()
        )
    });
    for (page, body) in [
        ("doomed.md", "---\ntype: note\n---\n\n# Doomed\n\nbytes.\n"),
        ("walked.md", "---\ntype: note\n---\n\n# Walked\n\nbytes.\n"),
        (
            "pinner.md",
            "---\ntype: note\n---\n\n# Pinner\n\n## Notes\n\nseed\n",
        ),
        (
            "target.md",
            "# Target\n\n## Design\n\nthe pinned section.\n",
        ),
    ] {
        std::fs::write(sb.tree.join(page), body).expect("seed");
    }

    // D2 — walk, both faces. The json `root` field is the sharp one: a consumer
    // parses it and has nothing to recover from.
    let raw = sb.stdout(&["walk", "sessions:walked.md", "--json"]);
    let value = json(&raw);
    assert_eq!(
        value["walk"]["root"], "field-notes-sessions:walked.md",
        "walk.root is a structured field and carries the MOUNT's name:\n{raw}",
    );
    assert_no_alias_spelling(&raw, "sessions", "the walk --json report");
    assert_no_alias_spelling(
        &sb.both(&["walk", "sessions:walked.md"]),
        "sessions",
        "the walk human header",
    );

    // D3 — pin's pinning-PAGE half, the line whose target half is already
    // canonical. Both ends of one sentence, one vocabulary.
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&sb.tree)
                .output()
                .expect("git")
                .status
                .success(),
            "git {args:?} failed",
        );
    };
    git(&["init", "--quiet"]);
    let said = sb.both(&[
        "pin",
        "sessions:pinner.md",
        "target.md#Target/Design",
        "--vibe",
    ]);
    assert!(
        said.contains("into field-notes-sessions:pinner.md"),
        "the pinning page is named by the MOUNT:\n{said}",
    );
    assert_no_alias_spelling(&said, "sessions", "the pin receipt's page half");

    // D1 — rm, last: it deletes the page it names.
    // Remove-what-you-read: `rm` demands the file rev from every origin and has
    // no `--force` — deletion is the one write with no recovery. So the arm does
    // what a caller does: read the page through the alias, then remove it
    // through the alias with the rev that read served.
    let read = json(&sb.stdout(&["read", "sessions:doomed.md", "--json"]));
    let rev = read["read"]["file_rev"]
        .as_str()
        .unwrap_or_else(|| panic!("the read must serve a file_rev: {read}"))
        .to_owned();
    let said = sb.both(&["rm", "sessions:doomed.md", "--rev", &rev]);
    assert!(
        said.contains("removed field-notes-sessions:doomed.md"),
        "a DELETION receipt names the MOUNT — it is the only record of what went:\n{said}",
    );
    assert_no_alias_spelling(&said, "sessions", "the rm receipt");
    assert!(
        !sb.tree.join("doomed.md").exists(),
        "and it really removed the file, through the alias",
    );
}
