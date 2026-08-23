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
//! it is not consulted (ZT 475): the designation means exactly what it meant
//! before, and no derivation reads it for `sessions`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
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
}

fn json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|e| panic!("not json ({e}): {raw}"))
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
/// nothing new. Also the `primary:` half of ZT 475 — the designation sits on a
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
