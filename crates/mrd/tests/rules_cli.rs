//! `mrd rules` end-to-end gates — the effective-rules print verb (registration ruling § 7),
//! driven through the REAL binary over its process boundary. Every gate here is a claim about
//! the SURFACE an operator reads, so none of it is asserted through a library call: the fixture
//! is a workspace on disk plus a user scope anchored by a `MERIDIAN.md`, and the measurement is
//! the CLIs own stdout and exit code. Two fixture disciplines, both deliberate 1. **The armed
//! artifact is MINTED, never hand-typed.** `policy::armed::arm` resolves through the landed
//! resolver and pins each winners real page rev.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// A rule page: the registration tag, the id, and a body that makes the bytes
/// (and so the rev) unique per page.
fn rule_page(kind: &str, id: &str, body: &str) -> String {
    format!("---\ntags: [type/rule, rules/{kind}]\nid: {id}\n---\n\n# {id}\n\n{body}\n")
}

struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
}

impl Sandbox {
    fn write(&self, rel: &str, bytes: &str) {
        let path = self.ws.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    fn write_home(&self, rel: &str, bytes: &str) {
        let path = self.home.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .output()
            .expect("spawn mrd")
    }

    /// stdout, with a non-zero exit's stderr attached to the panic message.
    fn stdout(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "mrd {args:?} exited {:?}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    /// Every file under the workspace as `(relative path, bytes)` — the read-only witness. Wider
    /// than the hash domain on purpose: a verb that wrote a marker, a lock, or a dotfile would
    /// escape a domain-only compare.
    fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        collect(&self.ws, &self.ws, &mut out);
        out
    }

    /// The production hash-domain fold over the workspace — the same root the
    /// engine's own guards compare.
    fn merkle_root(&self) -> String {
        let root = fs::WorkspaceRoot(self.ws.clone());
        let (_files, folded) = fs::domain_snapshot(&root).expect("snapshot");
        format!("{folded:?}")
    }
}

fn collect(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if entry.file_type().expect("file_type").is_dir() {
            collect(base, &path, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .expect("under the base")
                .to_string_lossy()
                .to_string();
            out.insert(rel, std::fs::read(&path).expect("read"));
        }
    }
}

/// A sandbox whose workspace is a declared meridian root and whose HOME carries
/// a `MERIDIAN.md` anchor (so the user rung is declared) but no `rules/` yet.
fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache_home = tmp.path().join("xdg-cache");
    let ws = tmp.path().join("ws");
    for dir in [&home, &cache_home, &ws] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    let sandbox = Sandbox {
        tmp,
        home,
        cache_home,
        ws,
    };
    sandbox.write(
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# The workspace\n",
    );
    sandbox.write_home(
        "MERIDIAN.md",
        "---\ntype: meridian-config\nversion: 1\n---\n\n# This machine\n",
    );
    sandbox
}

/// The populated fixture: one id overridden across all three ladder layers, one id at the root
/// only, one id in each of two SIBLING sessions, one collision on a single chain, and one page
/// that fails to register.
fn populated() -> Sandbox {
    let s = sandbox();
    // `task.notify` at all three layers.
    s.write_home(
        "rules/user-notify.md",
        &rule_page("hook", "task.notify", "the user-space default"),
    );
    s.write(
        "notify.md",
        &rule_page("hook", "task.notify", "the workspace-root rule"),
    );
    s.write(
        "sessions/s1/notify.md",
        &rule_page("hook", "task.notify", "s1 overrides"),
    );
    // A sibling session carrying the SAME id — no conflict under § 3 narrowing.
    s.write(
        "sessions/s2/notify.md",
        &rule_page("hook", "task.notify", "s2 overrides too"),
    );
    // An id that exists at the workspace root ONLY, so an inheriting folder has
    // a winner with nothing beneath it.
    s.write(
        "root-only.md",
        &rule_page("check", "root.only", "the root's own law"),
    );
    // A collision: two pages, same id, same scope, ONE chain.
    s.write("policy/a.md", &rule_page("hook", "collide.here", "page a"));
    s.write("policy/b.md", &rule_page("hook", "collide.here", "page b"));
    s
}

/// The bytes of a page that offers itself to registration and is refused: a
/// registration tag with no `id:`.
const REFUSED_PAGE: &str = "---\ntags: [rules/hook]\n---\n\n# no id\n";

/// [`populated`] plus a refused rule page at `policy/broken.md`. It is a separate fixture
/// because a refusal is a finding on its OWN CHAIN: this page mounts at `policy`, so it reddens
/// `policy` and everything beneath it (§ 3 "Refusal scoping", Keeping it out of [`populated`]
/// is what lets the other gates measure their own subject instead of this one.
///
///
fn populated_with_a_refusal() -> Sandbox {
    let s = populated();
    s.write("policy/broken.md", REFUSED_PAGE);
    s
}

// ── the default view ──────────────────────────────────────────────────────────

/// **P1** — a folder with no rule at-or-above it prints an empty effective set
/// and exits 0. An empty population is a legitimate answer, not a failure.
#[test]
fn a_folder_with_no_rules_prints_an_empty_set_and_exits_zero() {
    let s = sandbox();
    s.write("notes/plan.md", "# plan\n");
    let out = s.run(&["rules", "notes"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "an empty set is clean: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("rules at notes"), "{stdout}");
    assert!(stdout.contains("(no rules in effect)"), "{stdout}");
}

/// **P1b** — the verb with NO path defaults to the cwd and answers there.
#[test]
fn no_path_defaults_to_the_cwd() {
    let s = populated();
    let stdout = String::from_utf8_lossy(&s.run(&["rules"]).stdout).to_string();
    assert!(stdout.starts_with("rules at .\n"), "{stdout}");
    // At the workspace root the root-mounted pages govern, and the deeper
    // session pages are not on this chain at all.
    assert!(stdout.contains("winner    notify.md"), "{stdout}");
    assert!(
        !stdout.contains("sessions/s1/notify.md"),
        "a deeper page does not govern its parent: {stdout}"
    );
}

/// **P2** — a folder inheriting a workspace-root rule prints the root page as
/// winner with nothing shadowed beneath it.
#[test]
fn an_inherited_root_rule_has_a_winner_and_no_shadow() {
    let s = populated();
    let stdout = s.stdout(&["rules", "sessions/s1"]);
    let block = block_for(&stdout, "root.only");
    assert_eq!(
        block,
        vec![
            "  root.only  armed=-",
            "      winner    root-only.md  rev=REV  scope=workspace:0  kinds=check"
        ],
        "inherited, unshadowed, and unarmed"
    );
}

/// **P3 + P4** — an overridden id prints the winner FIRST and every page it shadows beneath it,
/// in ladder order across all three rungs. The assert is on the shadowed entries PRESENCE, not
/// merely on the winner being right.
#[test]
fn an_override_prints_the_winner_first_then_the_pages_it_shadows() {
    let s = populated();
    let stdout = s.stdout(&["rules", "sessions/s1"]);
    let block = block_for(&stdout, "task.notify");
    assert_eq!(
        block.len(),
        4,
        "the id line plus a THREE-layer chain: {block:#?}"
    );
    assert_eq!(
        block[1],
        "      winner    sessions/s1/notify.md  rev=REV  scope=workspace:2  kinds=hook"
    );
    assert_eq!(
        block[2], "      shadowed  notify.md  rev=REV  scope=workspace:0  kinds=hook",
        "the workspace-root page it shadows is VISIBLE, not collapsed"
    );
    // The user rung. Its depth digit is deliberately not asserted: this pages mount scope moves
    // when the mount-law lift lands on the discovery surface (`rules/` → parent), and this gate is
    // about the LAYER being present in the chain, in last position, not about its depth.
    //
    assert!(
        block[3].starts_with("      shadowed  rules/user-notify.md  rev=")
            && block[3].contains("scope=user:"),
        "the user-space page is the outermost rung of the chain: {}",
        block[3]
    );
}

/// **P6** — two SIBLING sessions carrying one id are no conflict: each chain
/// resolves to its own page, and neither renders a collision.
#[test]
fn sibling_scopes_carrying_one_id_do_not_collide() {
    let s = populated();
    for session in ["s1", "s2"] {
        let stdout = s.stdout(&["rules", &format!("sessions/{session}")]);
        assert!(
            !stdout.contains("REFUSED"),
            "sibling same-id pages are the normal template-copy case: {stdout}"
        );
        let block = block_for(&stdout, "task.notify");
        assert_eq!(
            block[1],
            format!(
                "      winner    sessions/{session}/notify.md  rev=REV  scope=workspace:2  kinds=hook"
            ),
            "each sibling governs its own subtree"
        );
        assert!(
            !stdout.contains(&format!(
                "sessions/{}/notify.md",
                if session == "s1" { "s2" } else { "s1" }
            )),
            "the other sibling is not even a candidate here: {stdout}"
        );
    }
}

/// **P5** — a collision at one scope on ONE chain is reported AS a collision, naming every tied
/// page, with no arbitrary winner and no omission. Every other id on the chain still resolves,
/// and the verb exits 1.
#[test]
fn a_collision_on_one_chain_is_reported_and_gates_the_exit() {
    let s = populated();
    let out = s.run(&["rules", "policy"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(1), "a collision is a finding");
    let block = block_for(&stdout, "collide.here");
    assert_eq!(
        block[0],
        "  collide.here  REFUSED collision at scope=workspace:1 — this id resolves to nothing"
    );
    let pages: Vec<&str> = block[1..].iter().map(|line| line.trim()).collect();
    assert_eq!(
        pages.len(),
        2,
        "both tied pages, neither dropped: {pages:?}"
    );
    assert!(pages[0].starts_with("tied      policy/a.md"), "{pages:?}");
    assert!(pages[1].starts_with("tied      policy/b.md"), "{pages:?}");
    assert!(
        !block[0].contains("armed="),
        "a collided id resolves to nothing, so it has no armed cell: {}",
        block[0]
    );
    // Every other id is unaffected.
    assert!(
        block_for(&stdout, "root.only")[1].contains("winner    root-only.md"),
        "{stdout}"
    );
}

/// **P13** — a page that failed to register is REPORTED, naming the page and the reason. A rule
/// that silently failed to register is a rule that silently stopped being enforced, so it is a
/// finding.
#[test]
fn a_refused_rule_page_is_reported_with_its_reason() {
    let s = populated_with_a_refusal();
    let out = s.run(&["rules", "policy"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(1),
        "a refused rule page is a finding"
    );
    assert!(stdout.contains("refused:"), "{stdout}");
    assert!(
        stdout.contains("policy/broken.md") && stdout.contains("id:"),
        "the refusal names the page and the reason: {stdout}"
    );
}

/// **P13, scoped (§ 3 "Refusal scoping", — a refused rule page reddens its OWN chain and no
/// other. Four paths, one fixture: | queried path | on `policy/broken.md`s chain? | exit |
/// |---|---|---| | `policy` (its mount) | yes | 1 | | `policy/deeper` (beneath it) | yes | 1 |
/// | `sessions/s2` (a sibling scope) | no |
///
///
///
///
///
///
///
///
///
///
#[test]
fn a_refused_rule_page_reddens_its_own_chain_and_no_other() {
    let s = populated_with_a_refusal();
    s.write("policy/deeper/note.md", "# under the refusal's mount\n");

    for at in ["policy", "policy/deeper"] {
        let out = s.run(&["rules", at]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{at} is on the refusal's chain: {stdout}"
        );
        assert!(
            stdout.contains("policy/broken.md"),
            "and it is NAMED at {at}: {stdout}"
        );
    }

    for at in ["sessions/s2", "."] {
        let out = s.run(&["rules", at]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{at} is off the refusal's chain, so a broken page in `policy/` is not \
             its finding: {stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !stdout.contains("broken.md"),
            "and it is not even printed at {at}: {stdout}"
        );
    }
}

/// **The all-refusals-always invariant, held against the scoping.** A refusal a sibling scoped
/// query cannot see is STILL in the corpus-wide walks report — the same feed the ARM act and
/// the cutover sweep read.
///
///
///
///
///
#[test]
fn the_corpus_wide_walk_reports_a_refusal_the_sibling_query_does_not() {
    let s = populated();
    s.write("sessions/s1/broken.md", REFUSED_PAGE);

    let out = s.run(&["rules", "sessions/s2"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the sibling scoped query is clean: {stdout}"
    );
    assert!(!stdout.contains("broken.md"), "{stdout}");

    let refused = walk_refusals(&s);
    assert_eq!(
        refused,
        vec!["sessions/s1/broken.md".to_owned()],
        "and the walk over the very same workspace still reports it, always"
    );
}

/// Every refusal a corpus-wide walk over `s` encounters, page-ascending — the
/// UN-narrowed index, which is what the ARM act and any sweep read.
fn walk_refusals(s: &Sandbox) -> Vec<String> {
    let root = fs::WorkspaceRoot(s.ws.clone());
    let (files, _) = fs::domain_snapshot(&root).expect("snapshot");
    let text: Vec<(String, String)> = files
        .into_iter()
        .filter_map(|(page, bytes)| String::from_utf8(bytes).ok().map(|b| (page, b)))
        .collect();
    let index = policy::RuleIndex::discover(text.iter().map(|(page, bytes)| policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page,
        bytes,
    }));
    let mut pages: Vec<String> = index
        .refused()
        .iter()
        .map(|r| r.page().to_owned())
        .collect();
    pages.sort();
    pages
}

/// **The mount law reaches refusals too.** A refused page in a `<scope>/rules/` layout folder
/// is lifted to `<scope>` exactly as a registered one is, so it reddens the scope its author
/// filed it to govern — not merely the folder it is kept in. Measured at the CLI, where the
/// lift has to have survived `policy`s `RegisterError` and the verbs narrowing.
///
#[test]
fn a_refusal_in_a_layout_folder_reddens_its_lifted_scope() {
    let s = sandbox();
    s.write("demo/rules/broken.md", REFUSED_PAGE);
    s.write("demo/tasks/card.md", "---\ntype: task\n---\n\n# a card\n");
    s.write("other/note.md", "# elsewhere\n");

    for at in ["demo", "demo/tasks"] {
        let out = s.run(&["rules", at]);
        assert_eq!(
            out.status.code(),
            Some(1),
            "the lifted mount governs {at}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("demo/rules/broken.md"),
            "named at {at}"
        );
    }
    for at in [".", "other"] {
        let out = s.run(&["rules", at]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "the lift is ONE level, so {at} is not under `demo`: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

/// **The meridian-rs self-test — the measurement that started this card.** Before the § 3
/// "Refusal scoping" amendment, `mrd rules` on meridian-rs itself exited 1 from *every* path,
/// naming `crates/testsuite/data/meridian-md/refusals/frontmatter-unparseable.md` — a
/// deliberately malformed `MERIDIAN.md` schema fixture. Two independent things were wrong and
/// both are fixed here, so both halves are measured: 1. **The repo is clean, measured on the
/// REAL repo.
///
///
///
///
///
///
///
///
///
///
///
///
///
///
#[test]
fn meridian_rs_itself_is_clean_while_a_refusal_still_reddens_its_own_subtree() {
    // ── half 1: the real repo ────────────────────────────────────────────────
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/mrd is two levels under the repo root")
        .to_path_buf();
    assert!(
        repo.join("crates/testsuite/data/meridian-md/refusals/frontmatter-unparseable.md")
            .is_file(),
        "the malformed fixture is still ON DISK and still tested by the schema pack — \
         it left the hash domain, it was not deleted"
    );

    let neutral = tempfile::tempdir().expect("tempdir");
    let out = Command::new(mrd_bin())
        .args(["rules"])
        .current_dir(&repo)
        .env("HOME", neutral.path())
        .env("XDG_CACHE_HOME", neutral.path())
        .env_remove("MERIDIAN_CONFIG")
        .env_remove("MERIDIAN_WORKSPACE")
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .output()
        .expect("spawn mrd");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(0),
        "meridian-rs itself carries no finding: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !stdout.contains("frontmatter-unparseable"),
        "and the fixture is not even printed — it is outside the hash domain, so no \
         walk of this repo encounters it: {stdout}"
    );

    // ── half 2: the same shape, deliberately mounted in a sandbox ────────────
    let s = sandbox();
    s.write("data/refusals/frontmatter-unparseable.md", REFUSED_PAGE);
    s.write(
        "data/corpus/well-formed.md",
        "---\ntype: note\n---\n\n# ok\n",
    );

    let at_refusals = s.run(&["rules", "data/refusals"]);
    let printed = String::from_utf8_lossy(&at_refusals.stdout).to_string();
    assert_eq!(
        at_refusals.status.code(),
        Some(1),
        "a query AT the refusals subtree still reddens — narrowing scopes the \
         refusal, it does not silence it: {printed}"
    );
    assert!(
        printed.contains("data/refusals/frontmatter-unparseable.md"),
        "and it names the page: {printed}"
    );

    for at in [".", "data/corpus"] {
        let out = s.run(&["rules", at]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{at} is off the refusal's chain: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    assert_eq!(
        walk_refusals(&s),
        vec!["data/refusals/frontmatter-unparseable.md".to_owned()],
        "and the corpus-wide walk over that sandbox reports it ALWAYS — which is \
         exactly why the repo half had to be solved by leaving the domain, not by \
         narrowing"
    );
}

// ── the single-layer views ────────────────────────────────────────────────────

/// **P7** — `--workspace` prints the workspace-root layer ALONE: no session-tree
/// page, no user-space page.
#[test]
fn the_workspace_flag_prints_one_layer() {
    let s = populated();
    let out = s.run(&["rules", "--workspace"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.starts_with("rules at the workspace-root layer\n"),
        "{stdout}"
    );
    assert!(stdout.contains("winner    notify.md"), "{stdout}");
    assert!(stdout.contains("winner    root-only.md"), "{stdout}");
    for absent in [
        "sessions/s1/notify.md",
        "sessions/s2/notify.md",
        "rules/user-notify.md",
    ] {
        assert!(
            !stdout.contains(absent),
            "{absent} is not in the workspace-root layer: {stdout}"
        );
    }
    assert!(
        !stdout.contains("scope=user:"),
        "no user rung in this view: {stdout}"
    );
}

/// **P8** — `--user` prints the user layer alone.
#[test]
fn the_user_flag_prints_one_layer() {
    let s = populated();
    s.write_home(
        "rules/user-only.md",
        &rule_page("check", "user.only", "a user-space law"),
    );
    let out = s.run(&["rules", "--user"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.starts_with("rules at the user layer\n"), "{stdout}");
    assert!(stdout.contains("rules/user-notify.md"), "{stdout}");
    assert!(stdout.contains("rules/user-only.md"), "{stdout}");
    assert!(
        !stdout.contains("scope=workspace:"),
        "no workspace rung in this view: {stdout}"
    );
    assert!(
        stdout.contains("user-scope ") && stdout.contains("MERIDIAN.md"),
        "the view names the scope it read and the anchor that declared it: {stdout}"
    );
}

/// **P15** — the mount law, proven at the CLI surface rather than in the resolver. A page whose
/// immediate container is named `rules` mounts at that folders PARENT, so a workspaces rules
/// kept in `<root>/rules/` govern the whole workspace instead of only that folder.
///
///
///
///
///
///
///
///
#[test]
fn a_layout_folder_page_renders_its_lifted_mount_scope() {
    let s = sandbox();
    s.write(
        "rules/kept-here.md",
        &rule_page("check", "kept.here", "kept in the layout folder"),
    );
    s.write(
        "rules/deeper/filed-deeper.md",
        &rule_page("check", "filed.deeper", "filed deliberately deeper"),
    );
    // The verb answers about a real place, so the queried folder must exist.
    s.write("tasks/card.md", "---\ntype: task\n---\n\n# a card\n");

    let stdout = s.stdout(&["rules", "tasks"]);
    assert_eq!(
        block_for(&stdout, "kept.here"),
        vec![
            "  kept.here  armed=-",
            // `workspace:0`, NOT `workspace:1` — the `rules/` container lifted.
            "      winner    rules/kept-here.md  rev=REV  scope=workspace:0  kinds=check"
        ],
        "a layout-folder page governs the workspace, and says so: {stdout}"
    );
    assert!(
        !stdout.contains("filed.deeper"),
        "the lift is one level and never recursive, so a page under rules/deeper/ \
         is out of play at tasks/: {stdout}"
    );
}

/// **P8, the anchor arm** — with no `MERIDIAN.md` there is no user scope, so the user layer is
/// EMPTY and the output NAMES the absent anchor. The fixture holds a `rules/` tree that a
/// widened walk would have found.
#[test]
fn an_absent_anchor_yields_an_empty_user_layer_that_says_why() {
    let s = populated();
    std::fs::remove_file(s.home.join("MERIDIAN.md")).expect("drop the anchor");
    let stdout = s.stdout(&["rules", "--user"]);
    assert!(stdout.contains("(no rules in effect)"), "{stdout}");
    assert!(
        stdout.contains("user-scope none  (no anchor at"),
        "the empty is explained, never silent: {stdout}"
    );
    assert!(
        !stdout.contains("user-notify.md"),
        "no anchor ⇒ nothing under HOME is a candidate: {stdout}"
    );
    // …and the default view loses exactly the user rung, keeping the rest.
    let effective = s.stdout(&["rules", "sessions/s1"]);
    let block = block_for(&effective, "task.notify");
    assert_eq!(
        block.len(),
        3,
        "two workspace rungs, no user rung: {block:#?}"
    );
}

// ── the armed column ──────────────────────────────────────────────────────────

/// Mint a real ARM artifact over the fixtures own bytes and write it into the workspace.
/// `requests` is `(arm root, id, mode)`; every winners rev is pinned by `policy::armed::arm`
/// through the landed resolver.
fn arm(s: &Sandbox, requests: &[(&str, &str, &str)]) {
    let root = fs::WorkspaceRoot(s.ws.clone());
    let (files, _) = fs::domain_snapshot(&root).expect("snapshot");
    let text: Vec<(String, String)> = files
        .into_iter()
        .map(|(page, bytes)| (page, String::from_utf8(bytes).expect("utf-8")))
        .collect();
    let index = policy::RuleIndex::discover(text.iter().map(|(page, bytes)| policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page,
        bytes,
    }));

    let mut artifact: Option<policy::armed::ArmedArtifact> = None;
    for (arm_root, id, mode) in requests {
        let root = policy::armed::ArmRoot::parse(arm_root).expect("a legal root");
        // The rev the "reviewer" attests is the winner's own, read back through
        // the same resolver the arm act will use — never a literal.
        let resolved = index.narrowed_to(root.as_str()).resolve();
        let attested_rev = resolved
            .get(id)
            .expect("the id resolves at this root")
            .winner()
            .rev()
            .to_owned();
        let act = policy::armed::arm(
            &index,
            &root,
            vec![policy::armed::ArmRequest {
                id: policy::RuleId::parse(id).expect("a legal id"),
                mode: policy::armed::Mode::parse(mode).expect("a legal mode"),
                attested_rev,
            }],
        )
        .expect("the arm act");
        match artifact.as_mut() {
            None => artifact = Some(act),
            Some(held) => held.merge(act).expect("merge"),
        }
    }
    let page = artifact.expect("at least one arm").render();
    s.write(policy::armed::ARMED_RULES_PATH, &page);
}

/// **P9** — with no artifact every armed cell reads `-` and the header names the
/// absent artifact: registered here, armed nowhere.
#[test]
fn without_an_artifact_every_row_is_registered_and_unarmed() {
    let s = populated();
    let stdout = s.stdout(&["rules", "sessions/s1"]);
    assert!(
        stdout.contains("armed-set  none  (meridian/armed-rules.md absent)"),
        "{stdout}"
    );
    assert!(stdout.contains("  task.notify  armed=-"), "{stdout}");
}

/// **P9 + P10** — the armed join is `(id, arm root)` narrowed to PATH: an id armed at a SIBLING
/// root reads `-` here, and where an inner and an outer arm both contain the path the DEEPEST
/// one is the one rendered. The two arms carry DIFFERENT modes on purpose. Equal modes would
/// render the same cell whichever row won, so the assertion would pass without the selection
/// law working — a control has to be able to fail.
///
///
#[test]
fn the_armed_cell_joins_on_id_and_arm_root_and_the_deepest_arm_wins() {
    let s = populated();
    arm(
        &s,
        &[
            ("", "task.notify", "off"),
            ("sessions/s1", "task.notify", "armed"),
        ],
    );

    // At s1: two arms contain this path, the deeper one governs.
    let at_s1 = s.stdout(&["rules", "sessions/s1"]);
    assert!(
        at_s1.contains("  task.notify  armed=armed"),
        "the INNER arm governs at s1: {at_s1}"
    );
    // At the sibling s2: only the workspace arm contains this path, and it
    // pinned the ROOT page — which is not s2's winner, so the divergence shows.
    let at_s2 = s.stdout(&["rules", "sessions/s2"]);
    assert!(
        at_s2.contains("  task.notify  armed=off@notify.md"),
        "the OUTER arm governs at s2, and it pinned another page: {at_s2}"
    );
    // An id armed nowhere stays `-` while its neighbour is armed — the join is
    // per-id, not per-file.
    assert!(at_s1.contains("  root.only  armed=-"), "{at_s1}");
}

/// **P11** — an armed row whose pinned page has drifted renders red rather than clean, and the
/// drift gates the exit. The freeze is the point: discovery moves, the arm does not.
///
#[test]
fn a_drifted_pin_reddens_the_armed_cell_and_gates_the_exit() {
    let s = populated();
    arm(&s, &[("sessions/s1", "task.notify", "armed")]);
    // Edit the pinned page AFTER arming.
    s.write(
        "sessions/s1/notify.md",
        &rule_page("hook", "task.notify", "s1 overrides, edited after the arm"),
    );

    let out = s.run(&["rules", "sessions/s1"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(1), "a red armed row is a finding");
    assert!(
        stdout.contains("  task.notify  armed=armed(drifted)"),
        "the rev the arm pinned no longer stands: {stdout}"
    );
}

/// A corrupt artifact NEVER reads as "nothing armed": the verb says the armed set
/// is unreadable, keeps printing the registration view, and exits 1.
#[test]
fn a_corrupt_artifact_is_unreadable_never_silently_unarmed() {
    let s = populated();
    s.write(
        policy::armed::ARMED_RULES_PATH,
        "# Not the armed set\n\nnothing here parses as the § 4 table.\n",
    );
    let out = s.run(&["rules", "sessions/s1"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("armed-set  UNREADABLE"), "{stdout}");
    assert!(
        stdout.contains("winner    sessions/s1/notify.md"),
        "the registration view still prints: {stdout}"
    );
}

// ── read-only, and one resolver ───────────────────────────────────────────────

/// **P12 — the read-only proof, asserted directly.** Every view of the verb runs over a
/// populated workspace with a minted armed artifact, and afterwards the hash-domain merkle root
/// AND the full file tree (paths and bytes, wider than the domain) are unchanged. Nothing is
/// armed, no receipt is minted, no marker appears.
///
#[test]
fn every_view_leaves_the_workspace_bit_for_bit_unchanged() {
    let s = populated();
    arm(&s, &[("sessions/s1", "task.notify", "armed")]);

    let tree_before = s.tree();
    let root_before = s.merkle_root();

    for args in [
        vec!["rules"],
        vec!["rules", "sessions/s1"],
        vec!["rules", "policy"],
        vec!["rules", "--workspace"],
        vec!["rules", "--user"],
        vec!["rules", "sessions/s1", "--json"],
    ] {
        let out = s.run(&args);
        assert!(
            out.status.code() != Some(2),
            "mrd {args:?} was a bad invocation: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    assert_eq!(s.merkle_root(), root_before, "the hash domain never moved");
    let tree_after = s.tree();
    assert_eq!(
        tree_after.keys().collect::<Vec<_>>(),
        tree_before.keys().collect::<Vec<_>>(),
        "no file appeared or vanished"
    );
    assert!(tree_after == tree_before, "no file's bytes changed");
    assert!(
        !s.ws.join("meridian/journal.md").exists(),
        "no receipt journal was minted"
    );
    assert!(
        !s.ws.join(policy::ATTESTED_MARKER_PATH).exists(),
        "nothing armed: the once-armed marker was never written"
    );
}

/// **P14 — one resolver, structurally.** The verbs own source carries no override law: no scope
/// or depth comparison, no id grouping, no ordering of candidates.
///
///
///
///
///
///
#[test]
fn the_cli_layer_holds_no_second_resolver() {
    let source = include_str!("../src/rules_cmd.rs");
    // The body, without the module documentation, so prose describing the law
    // cannot satisfy or break a claim about code.
    let body = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    // `.verify_at(` replaced a hand-composed `.select_at(` + `.verify(` here (C3 gate finding
    // F-4): selection-then-verification has two wrong orders, so the composition is `policy`s
    // single one and the CLI calls it. `.select_at(` survives in the list because the armed CELL
    // still needs the selected rows themselves to render mode and pinned page — reading rows is
    // not composing the law.
    //
    for called in [
        "RuleIndex::discover",
        ".narrowed_to(",
        ".resolve()",
        ".select_at(",
        ".verify_at(",
        ".chain()",
    ] {
        assert!(body.contains(called), "the verb must call {called}");
    }
    for forbidden in [
        // The whole-artifact health report applies NO selection, so composing it
        // with `select_at` by hand is the second composition F-4 removed.
        ".verify(pages)",
        ".verify(&",
        "sort_by",
        "sort_unstable",
        "max_by",
        "min_by",
        ".depth() >",
        ".depth() <",
        "scope() >",
        "scope() <",
        "scope().cmp",
        "depth().cmp",
    ] {
        assert!(
            !body.contains(forbidden),
            "`{forbidden}` in the CLI layer is a second resolver — the fix belongs in `policy`"
        );
    }
}

/// `--json` carries the same law the human render does: the chain with its
/// roles, the armed cell, and the single-layer view's name.
#[test]
fn the_json_face_carries_the_chain_and_the_armed_cell() {
    let s = populated();
    arm(&s, &[("sessions/s1", "task.notify", "armed")]);
    let stdout = s.stdout(&["rules", "sessions/s1", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    let rules = value["rules"]["rules"].as_array().expect("rules array");
    let notify = rules
        .iter()
        .find(|row| row["id"] == "task.notify")
        .expect("the id");
    assert_eq!(notify["state"], "resolved");
    assert_eq!(notify["armed"]["mode"], "armed");
    assert_eq!(notify["armed"]["redness"], serde_json::Value::Null);
    let chain = notify["chain"].as_array().expect("chain");
    assert_eq!(chain[0]["role"], "winner");
    assert_eq!(chain[0]["page"], "sessions/s1/notify.md");
    assert_eq!(chain[1]["role"], "shadowed");
    assert_eq!(chain[1]["page"], "notify.md");
    assert_eq!(chain[2]["layer"], "user");
    assert_eq!(value["rules"]["view"], "effective");
    assert_eq!(value["rules"]["armed_set"]["state"], "present");
}

/// A PATH outside the workspace is exit 2, never a quiet fall back to the root's law — which
/// would answer a question about a folder the operator never named.
#[test]
fn a_path_outside_the_workspace_is_a_bad_invocation() {
    let s = populated();
    let out = s.run(&["rules", "/etc"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("outside the workspace"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A PATH that is not on disk is refused, and the RETIRED `mrd rules replay` form (decision 8)
/// refuses through that same arm now that the `rules` namespace belongs to this verb. Measured
/// end-to-end because the regression it guards is a silent success: `mrd rules replay` printing
/// an empty rule set.
#[test]
fn a_path_that_is_not_on_disk_is_refused_and_so_is_the_retired_form() {
    let s = populated();
    for args in [
        vec!["rules", "sessions/typo"],
        vec!["rules", "replay"],
        vec!["rules", "replay", "--rules", "x", "--snapshots", "y"],
    ] {
        let out = s.run(&args);
        assert_eq!(out.status.code(), Some(2), "mrd {args:?} must refuse");
        assert!(
            out.stdout.is_empty(),
            "mrd {args:?} printed a rule set: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

/// The verb is in `mrd help`, so the authoritative surface names it.
#[test]
fn the_verb_is_documented_in_the_cli_surface() {
    let s = sandbox();
    let stdout = s.stdout(&["help"]);
    assert!(stdout.contains("mrd rules [PATH]"), "{stdout}");
    assert!(stdout.contains("--workspace"), "{stdout}");
}

// ── reading the render ────────────────────────────────────────────────────────

/// One ids block: its own line plus the indented chain beneath it, with every 16-hex rev
/// replaced by `REV` so a fixture edit does not have to restate a hash the engine computed.
///
fn block_for(stdout: &str, id: &str) -> Vec<String> {
    let mut block = Vec::new();
    let mut inside = false;
    for line in stdout.lines() {
        let is_chain_line = line.starts_with("      ");
        if inside && !is_chain_line {
            break;
        }
        if inside {
            block.push(mask_revs(line));
            continue;
        }
        if line.starts_with(&format!("  {id}  ")) {
            inside = true;
            block.push(mask_revs(line));
        }
    }
    assert!(!block.is_empty(), "no block for {id} in:\n{stdout}");
    block
}

/// Replace every `rev=<16 hex>` with `rev=REV`.
fn mask_revs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(cut) = rest.find("rev=") {
        out.push_str(&rest[..cut + 4]);
        let tail = &rest[cut + 4..];
        let hex: String = tail.chars().take_while(char::is_ascii_hexdigit).collect();
        assert_eq!(hex.len(), 16, "a page rev is 16 hex: {line}");
        out.push_str("REV");
        rest = &tail[hex.len()..];
    }
    out.push_str(rest);
    out
}
