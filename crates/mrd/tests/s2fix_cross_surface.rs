//! **fixv — the CROSS-SURFACE gate.** Advisor R25's binding correction, executed:
//! *an exit criterion is verified across every surface it names, in ONE run, by
//! someone who did not write the units.* Aggregating per-unit evidence is not
//! verification of a cross-surface claim — *"per-unit proof and per-criterion
//! proof are different objects"* (R26, the re-verifier's formulation).
//!
//! Every other suite in this workspace proves one unit's surface. This one proves
//! a CRITERION: it builds ONE corpus and reads it with all the surfaces the
//! criterion names, in one run, asserting the criterion's own sentence.
//!
//! ## The engine under test
//!
//! `MRD_BIN` overrides the binary, so the same asserts run twice with no edit:
//! against `CARGO_BIN_EXE_mrd` in the workspace suite, and against the INSTALLED
//! `~/.local/bin/mrd` for the fixv evidence run. The point of the criterion is the
//! artifact the fleet actually holds, not a build of it.
//!
//! ## What each surface is allowed to prove (R26 — three DIFFERENT kinds of visibility)
//!
//! | Plane | Kind of visibility | Grain |
//! |---|---|---|
//! | `mrd walk` | user-reachable verb | **per-pin**, with reason |
//! | `mrd status` | user-reachable verb | **worst-of rollup ONLY**, never per-state |
//! | board-color | **test/example only** — no shipped verb publishes it | per-pin, via `open_board` |
//!
//! Asserting a per-state verdict out of `mrd status` would re-widen a claim R26
//! narrowed, so [`criterion_3_one_corpus_every_state_three_kinds_of_visibility`]
//! asserts the *aggregate shape* instead: one rollup, one reason word, a pin count.
//!
//! ## The fixture trap this suite is audited against
//!
//! A pin-verdict fixture whose ref is a bare `#^anchor` proves NOTHING — the
//! anchor node's span is its host line, `anchor_removals` strips exactly that
//! line, and blake3-of-empty matches in EVERY document, so decoy and target hash
//! identically and every plane reads green regardless of resolution (fix3's
//! false-gate self-catch). Every *resolving* pin here is a HEADING ref. The one
//! bare-anchor ref is the `dangling-anchor` fixture, which by construction never
//! resolves, and the R31 fixtures, which assert refusal and never-green. Where a
//! gate compares two documents, `the_two_targets_fingerprint_differently` is the
//! control that the comparison measures something.
//!
//! Isolation matters and is not decoration: `mrd read` DIALS a resident daemon, so
//! an unsandboxed drive can be served by a stale daemon built from another tree.
//! Every drive here runs under its own `XDG_CACHE_HOME`/`HOME`, and the auto-spawn
//! is pointed at nowhere, so the only daemon that can answer is one a test started
//! itself on its own socket ([`Sandbox::start_daemon`], PATH A's decorate half).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use wire::{ErrorCode, Path as WPath, PinSpec};
use wire_serve::write::{SpliceArgs, splice};

// ── the engine under test ───────────────────────────────────────────────────

/// The binary every CLI drive in this file goes through. `MRD_BIN` names the
/// INSTALLED artifact for the fixv evidence run; the workspace suite uses the
/// build under test.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

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
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            // Auto-spawn-impossible: no daemon from another tree may serve these
            // drives, so nothing but the engine under test can answer. `mrd`
            // pings before it spawns, so a daemon this sandbox started itself
            // ([`Sandbox::start_daemon`]) still answers on this socket — that is
            // the ONE resident host allowed here, and it is built from this tree.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    /// The registry cache root this sandbox's `mrd` drives resolve
    /// (`cache::cache_root` = `$XDG_CACHE_HOME/meridian`).
    fn cache_root(&self) -> PathBuf {
        self.cache_home.join("meridian")
    }

    /// Start the resident daemon IN-PROCESS, bound to this sandbox's own socket.
    ///
    /// This is `crates/registry/src/server.rs` — the host `page_decorations`'
    /// **one production caller** lives in (`server.rs:905`). A `mrd read` from
    /// this sandbox dials it through `default_socket_path()`, which resolves under
    /// the sandbox's `XDG_CACHE_HOME`, so no daemon from another tree can answer
    /// and none is left behind: dropping the handle stops it. The reaper and
    /// prewarm intervals are set past any test's lifetime — a decoration must not
    /// depend on a background thread's timing.
    #[allow(clippy::duration_suboptimal_units)]
    fn start_daemon(&self) -> registry::RunningServer {
        let reg_dir = self.cache_root().join("registry");
        std::fs::create_dir_all(&reg_dir).expect("registry dir");
        let never = Duration::from_secs(365 * 24 * 60 * 60);
        registry::RunningServer::start(registry::Config {
            socket_path: reg_dir.join("daemon.sock"),
            state_path: reg_dir.join("state.json"),
            cache_root: self.cache_root(),
            idle_threshold: never,
            reap_interval: never,
            prewarm_interval: never,
        })
        .expect("the resident daemon starts")
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    /// Run with the `put` edits channel on stdin.
    fn run_stdin(&self, cwd: &Path, args: &[&str], stdin_bytes: &str) -> Output {
        use std::io::Write as _;
        let mut child = self
            .command(cwd, args)
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
            .expect("write edits");
        child.wait_with_output().expect("wait")
    }

    /// A git-backed workspace, declared a root by `mrd init`. Git is real
    /// because the `objects:` plane and the vibe-debt gauge both ask git real
    /// questions, and it is what anchors the ladder (`git-root`).
    fn workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "fixv@example.invalid"]);
        git(&ws, &["config", "user.name", "fixv"]);
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "mrd init: {}", stderr(&init));
        ws
    }
}

fn git(dir: &Path, args: &[&str]) {
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

fn write(ws: &Path, rel: &str, body: &str) {
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

fn read(ws: &Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).expect("read")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// stdout+stderr together — a refusal rides stderr, a mint rides stdout, and the
/// asserts here care about what the operator SEES.
fn said(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}

/// The `depth N …` listing rows of a human `walk` render, trimmed.
fn walk_rows(human: &str) -> Vec<String> {
    human
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("depth "))
        .map(str::to_owned)
        .collect()
}

/// The verdict a `walk` row renders, without the ref and the rev — `green`,
/// `red content-drifted`, `grey lock-refused (…)`.
fn walk_verdict(row: &str) -> String {
    let rest = row.strip_prefix("depth ").unwrap_or(row);
    let rest = rest
        .split_once(char::is_whitespace)
        .map_or(rest, |(_, r)| r);
    // The verdict runs to the double space that separates it from the ref.
    rest.trim()
        .split("  ")
        .next()
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// The one `fp1.…` token a page carries on disk.
fn token_on_disk(ws: &Path, rel: &str) -> String {
    let text = read(ws, rel);
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .find(|w| w.starts_with("fp1.span2.b3."))
        .unwrap_or_else(|| panic!("no fp1 token in {rel}:\n{text}"))
        .to_owned()
}

/// The R4 blob hash a fixture pin carries when the test is not measuring the
/// retrieval plane. R4 makes `hash` MANDATORY — *"if hash is missing, we lost the
/// explicit target meaning"* — so there is no pin without one.
const FIXTURE_BLOB: &str = "9ae3f1deadbeef";

/// A hand-authored `meridian-lock` page — the fixture depends on the CLI's own
/// READER, never on the writer that produced the bytes.
///
/// `declared_ref` is the `page[#A/B]` convenience spelling, split into the R4
/// `object` wiki link and the `path` ARRAY at this one door — nothing downstream
/// ever sees a joined string, because R4 admits none on a row. A `#^id` fragment
/// stays a single-element array, which is what makes it the anchor arm.
///
/// NOTE FOR REVIEWERS: `version: 1` became `version: 2` across these fixtures.
/// That is the LOCK FILE schema version, not the wire protocol version.
fn locked_page(title: &str, declared_ref: &str, fingerprint: &str) -> String {
    format!(
        "# {title}\n\nclaim.\n\n{}\n",
        lock_block(&[(declared_ref, fingerprint)])
    )
}

/// The canonical R4 (`version: 2`) `meridian-lock` fence for one or more pins,
/// written by hand for the reason [`locked_page`] states. Every pin carries
/// [`FIXTURE_BLOB`]: no gate that uses this helper measures blob reachability.
fn lock_block(pins: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("```meridian-lock\nversion: 2\npins:\n");
    for (declared_ref, fingerprint) in pins {
        let (target, fragment) = match declared_ref.split_once('#') {
            Some((t, f)) => (t, f),
            None => (*declared_ref, ""),
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
        let _ = writeln!(
            out,
            "  - object: \"[[{object}]]\"\n    hash: \"{FIXTURE_BLOB}\"\n    \
             path: [{path}]\n    fingerprint: \"{fingerprint}\""
        );
    }
    out.push_str("```");
    out
}

/// Is there an `@fp` token in the file? The claim is R22/R32's exact width —
/// `@<tone>.<8 lowercase hex>` in a claim-link block-ref slot — so the probe is
/// the token grammar, never a bare `@`.
fn fp_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tone in ["green", "grey", "red"] {
        let needle = format!("@{tone}.");
        let mut rest = text;
        while let Some(at) = rest.find(&needle) {
            let tail = &rest[at + needle.len()..];
            let hex: String = tail
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .take(9)
                .collect();
            if hex.len() == 8 && !hex.chars().any(|c| c.is_ascii_uppercase()) {
                out.push(format!("@{tone}.{hex}"));
            }
            rest = &rest[at + needle.len()..];
        }
    }
    out
}

/// The sole wikilink in a render, split at its alias pipe: `(dest, label)`,
/// both without the `[[`/`]]` delimiters.
///
/// The split is what makes a decoration checkable at all: the token is only ever
/// honest in the DEST half, because the put-side strip parses the address and
/// never reads the label. One owner for the split, so no assertion here grows a
/// second spelling of where the slot is.
fn sole_link(rendered: &str) -> (String, String) {
    let node = rendered
        .split("[[")
        .nth(1)
        .and_then(|s| s.split("]]").next())
        .unwrap_or_else(|| panic!("a claim link in the render:\n{rendered}"));
    let (dest, label) = node
        .split_once('|')
        .unwrap_or_else(|| panic!("the link carries an alias label: {node}"));
    (dest.to_owned(), label.to_owned())
}

// ── the board plane — R26's TEST-VISIBLE surface, read in this same run ──────

/// One board row: the per-pin verdict projection.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BoardRow {
    src_path: String,
    to_path: String,
    to_sel: String,
    color: String,
    reason: String,
}

/// The board plane over a live workspace directory, through the SAME path
/// `crates/mrd/examples/board_colors.rs` uses — `fs::domain_snapshot` →
/// `fs::build_corpus` → `view::read_face::open_board`.
///
/// **This is not a shipped verb and this function does not pretend otherwise**
/// (R26): `open_board` has no production caller, and `input_lock` is absent from
/// `SCHEMA_SQL` so `mrd sql` cannot reach it either. Publishing the board is a
/// named stage-3 item. What this reads is exactly what a TEST can see.
fn board_rows(ws: &Path) -> Vec<BoardRow> {
    let canonical = workspace::canonicalize(ws).expect("canonicalize");
    let root = fs::WorkspaceRoot(canonical);
    let (files, _fp) = fs::domain_snapshot(&root).expect("snapshot");
    let (_index, docs) = fs::build_corpus(files).expect("build corpus");
    let conn = view::read_face::open_board(&docs).expect("open_board");
    let mut stmt = conn
        .prepare(
            "SELECT src_path, to_path, to_sel, color, reason \
             FROM board ORDER BY src_path, to_path, to_sel",
        )
        .expect("prepare");
    stmt.query_map([], |r| {
        Ok(BoardRow {
            src_path: r.get(0)?,
            to_path: r.get(1)?,
            to_sel: r.get(2)?,
            color: r.get(3)?,
            reason: r.get(4)?,
        })
    })
    .expect("query")
    .collect::<Result<Vec<_>, _>>()
    .expect("rows")
}

// ═══════════════════════════════════════════════════════════════════════════
// CRITERION 3 — one corpus, every state, three kinds of visibility
// ═══════════════════════════════════════════════════════════════════════════

/// **Criterion 3, as worded by R39.** *"A meridian-lock pin is visible with green
/// / red(drifted) / red(dangling) / grey via the reason-carrying Color model,
/// across three planes with three DIFFERENT KINDS of visibility … The three agree
/// on any pin they all see."*
///
/// The hole this closes is review finding 10: the five-states evidence was an
/// IN-PROCESS unit test, and `red(dangling)` plus both fingerprint-greys never ran
/// through the CLI at all. Here every state is built in ONE corpus and read by
/// `mrd walk`, `mrd status` and the board **in one run**, with the per-state rows
/// asserted value-for-value between the two per-pin planes.
///
/// Both dangling flavours are built, not one: `dangling-anchor` (the page resolves,
/// the anchor does not) and `selector-unresolved` (the page does not exist). The
/// criterion says "red(dangling)"; proving one spelling and naming it "dangling"
/// would be the aggregation defect in miniature.
/// The ONE corpus criterion 3 is read over. Building it is a separate function
/// from reading it, because the criterion's claim is about the READING: one
/// corpus, three surfaces, one run.
///
/// Returns the `(page, expected walk verdict)` table, so the states the corpus
/// contains and the states the planes are checked against cannot drift apart.
fn build_every_state_corpus(sb: &Sandbox, ws: &Path) -> Vec<(&'static str, &'static str)> {
    // TWO targets. One shared target would drift BOTH pins and there would be no
    // green row left to prove the planes distinguish states rather than agree by
    // rendering everything the same colour.
    write(ws, "t_ok.md", "# T\n\n## Section\n\nthe body\n");
    write(ws, "t_drift.md", "# T\n\n## Section\n\nthe body\n");
    write(ws, "green.md", "# green\n\nclaim.\n");
    write(ws, "drifted.md", "# drifted\n\nclaim.\n");
    write(ws, "control.md", "# control\n\nclaim.\n");
    git(ws, &["add", "-A"]);
    git(ws, &["commit", "-qm", "init"]);

    // Two REAL mints through the CLI — heading refs, never a bare anchor.
    for (page, target) in [
        ("green.md", "t_ok.md#T/Section"),
        ("drifted.md", "t_drift.md#T/Section"),
    ] {
        let out = sb.run(ws, &["pin", page, target]);
        assert_eq!(code(&out), 0, "pin {page}: {}", said(&out));
    }
    let pinned_drift = token_on_disk(ws, "drifted.md");

    // Drift ONLY t_drift.md, after its pin was minted.
    write(
        ws,
        "t_drift.md",
        "# T\n\n## Section\n\nthe body, now edited\n",
    );

    // ── THE FIXTURE-TRAP CONTROL ────────────────────────────────────────────
    // A third real mint over the EDITED t_drift.md. If the two documents did not
    // fingerprint differently, this token would equal the one `drifted.md` holds
    // and every red below would be a fixture artefact rather than a measurement.
    let out = sb.run(ws, &["pin", "control.md", "t_drift.md#T/Section"]);
    assert_eq!(code(&out), 0, "control pin: {}", said(&out));
    let control_token = token_on_disk(ws, "control.md");
    assert_ne!(
        control_token, pinned_drift,
        "CONTROL: the pre-edit and post-edit documents must fingerprint \
         DIFFERENTLY, or every red in this corpus proves nothing"
    );

    // The live digest of an untouched target — the payload for the grey that
    // carries a CORRECT digest under a triple this build does not implement. A
    // grey that only ever wraps a wrong digest never tests "grey never green".
    let live_digest = token_on_disk(ws, "green.md")
        .rsplit('.')
        .next()
        .expect("digest")
        .to_owned();

    // The hand-authored states. Every ref is a heading chain except the
    // dangling-anchor one, which exists precisely to NOT resolve.
    write(
        ws,
        "dangling.md",
        &locked_page("dangling", "t_ok.md#^nosuchanchor", &control_token),
    );
    write(
        ws,
        "unresolved.md",
        &locked_page("unresolved", "nosuch.md#T/Section", &control_token),
    );
    write(
        ws,
        "unverifiable.md",
        &locked_page(
            "unverifiable",
            "t_ok.md#T/Section",
            &format!("fp9.span2.b3.{live_digest}"),
        ),
    );
    write(
        ws,
        "malformed.md",
        &locked_page("malformed", "t_ok.md#T/Section", "not-a-token"),
    );
    // An unterminated quoted scalar — the lock itself refuses to parse. The
    // version is CURRENT on purpose: a stale `version: 1` block refuses for an
    // entirely different reason (the unsupported-version door), and this fixture
    // must trigger the malformed-grammar refusal the assertions below name.
    write(
        ws,
        "refused.md",
        "# refused\n\nclaim.\n\n```meridian-lock\nversion: 2\npins:\n  \
         - object: \"[[t_ok]]\n```\n",
    );

    vec![
        ("green.md", "green"),
        ("control.md", "green"),
        ("drifted.md", "red content-drifted"),
        ("dangling.md", "red dangling-anchor"),
        ("unresolved.md", "red selector-unresolved"),
        (
            "unverifiable.md",
            "grey unverifiable-fingerprint (unknown version)",
        ),
        ("malformed.md", "grey malformed-fingerprint"),
    ]
}

/// **Plane 1** — `mrd walk`, the user-reachable verb with a PER-PIN row. Returns
/// what each page actually rendered, so the caller compares planes rather than
/// re-reading prose.
fn read_walk_plane(
    sb: &Sandbox,
    ws: &Path,
    expected: &[(&'static str, &'static str)],
) -> Vec<(String, String)> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for (page, _) in expected {
        let out = sb.run(ws, &["walk", page]);
        let rows = walk_rows(&stdout(&out));
        assert_eq!(rows.len(), 1, "one pin, one row on {page}: {rows:?}");
        seen.push(((*page).to_owned(), walk_verdict(&rows[0])));
    }
    let want: Vec<(String, String)> = expected
        .iter()
        .map(|(p, v)| ((*p).to_owned(), (*v).to_owned()))
        .collect();
    assert_eq!(
        seen, want,
        "plane 1 (`mrd walk`) must render every state distinctly, through the CLI"
    );

    // `refused.md` renders its refusal WITH the cause, and names no ref — a lock
    // that cannot be parsed has no pin to point at.
    let out = sb.run(ws, &["walk", "refused.md"]);
    let rows = walk_rows(&stdout(&out));
    assert_eq!(
        rows.len(),
        1,
        "the refusal is a row, not a silence: {rows:?}"
    );
    assert!(
        rows[0].contains("grey lock-refused ("),
        "the lock refusal carries its CAUSE, never a bare class (R24): {}",
        rows[0]
    );

    // The red contract: a walk that measured a red exits 1, so a script notices.
    let out = sb.run(ws, &["walk", "drifted.md"]);
    assert_eq!(code(&out), 1, "a red walk exits 1: {}", said(&out));
    let out = sb.run(ws, &["walk", "green.md"]);
    assert_eq!(code(&out), 0, "a green walk exits 0: {}", said(&out));

    seen
}

/// **Plane 2** — `mrd status`, the user-reachable verb whose grain is a WORST-OF
/// ROLLUP and nothing finer.
///
/// R26 removed the per-state reading of this plane, so asserting a per-state
/// verdict here would test a claim the ruling deleted. What is asserted instead
/// is the AGGREGATE SHAPE: one colour, ONE reason word for a corpus carrying six,
/// and a pin count. That is the claim R26 actually left standing, encoded.
fn read_status_plane(sb: &Sandbox, ws: &Path) {
    let out = sb.run(ws, &["status"]);
    assert_eq!(code(&out), 0, "status: {}", said(&out));
    let lock_axis = stdout(&out)
        .lines()
        .find(|l| l.contains("lock "))
        .unwrap_or_default()
        .split(" · ")
        .find(|f| f.trim_start().starts_with("lock "))
        .unwrap_or_default()
        .trim()
        .to_owned();
    assert!(
        lock_axis.starts_with("lock red "),
        "worst-of: reds outrank greys and greens in one corpus — got {lock_axis:?}"
    );
    assert!(
        lock_axis.ends_with("pins]"),
        "the rollup counts the pins it summarised: {lock_axis:?}"
    );
    let reasons_named = [
        "content-drifted",
        "dangling-anchor",
        "selector-unresolved",
        "unverifiable-fingerprint",
        "malformed-fingerprint",
        "lock-refused",
    ]
    .iter()
    .filter(|r| lock_axis.contains(**r))
    .count();
    assert_eq!(
        reasons_named, 1,
        "`mrd status` is a WORST-OF ROLLUP, not a per-state verdict (R26): {lock_axis:?}"
    );
}

#[test]
fn criterion_3_one_corpus_every_state_three_kinds_of_visibility() {
    let sb = sandbox();
    let ws = sb.workspace("crit3");

    let expected = build_every_state_corpus(&sb, &ws);

    // ── PLANE 1: `mrd walk` — user-reachable verb, PER-PIN row with its reason ─
    let walk_seen = read_walk_plane(&sb, &ws, &expected);

    // ── PLANE 2: `mrd status` — user-reachable verb, WORST-OF ROLLUP ONLY ─────
    read_status_plane(&sb, &ws);

    // ── PLANE 3: board-color — TEST-VISIBLE ONLY, same corpus, same run ───────
    let board = board_rows(&ws);
    let mut board_seen: Vec<(String, String, String)> = board
        .iter()
        .map(|r| (r.src_path.clone(), r.color.clone(), r.reason.clone()))
        .collect();
    board_seen.sort();
    let mut board_want: Vec<(String, String, String)> = vec![
        ("control.md", "green", "attested"),
        ("dangling.md", "red", "dangling-anchor"),
        ("drifted.md", "red", "content-drifted"),
        ("green.md", "green", "attested"),
        ("malformed.md", "grey", "malformed-fingerprint"),
        ("refused.md", "grey", "lock-refused"),
        ("unresolved.md", "red", "selector-unresolved"),
        ("unverifiable.md", "grey", "unverifiable-fingerprint"),
    ]
    .into_iter()
    .map(|(p, c, r)| (p.to_owned(), c.to_owned(), r.to_owned()))
    .collect();
    board_want.sort();
    assert_eq!(
        board_seen, board_want,
        "plane 3 (board) must project every state per-pin over the SAME corpus"
    );

    // ── THE AGREEMENT ASSERT: walk vs board, value-for-value, per state ───────
    // The two PER-PIN planes are the two that can disagree about one pin, and
    // finding 6 / finding 17 were exactly that. `mrd status` is excluded from this
    // comparison on purpose — it has no per-pin grain to compare (R26).
    for (page, walk_label) in &walk_seen {
        let row = board
            .iter()
            .find(|r| &r.src_path == page)
            .unwrap_or_else(|| panic!("board has no row for {page}"));
        let board_label = if row.reason == "attested" {
            row.color.clone()
        } else {
            format!("{} {}", row.color, row.reason)
        };
        // walk's grey reasons carry a parenthesised detail the board's frozen
        // column list does not; compare on the stable reason word.
        let walk_word = walk_label.split(" (").next().unwrap_or(walk_label);
        assert_eq!(
            walk_word, board_label,
            "PLANE DISAGREEMENT on {page}: walk says {walk_label:?}, board says \
             {board_label:?} — the two per-pin planes must agree on any pin they \
             both see"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CRITERION 2 — the read-mint gate, normal + VIBE
// ═══════════════════════════════════════════════════════════════════════════

/// One engine session: a `ReadMintStore` held across a read call and then a pin
/// call — the H1 shape (plan §9). Two CLI processes could not model it, because a
/// session ledger is what the gate is keyed on.
fn session_read(
    root: &fs::WorkspaceRoot,
    store: &receipt::read_mint::ReadMintStore,
    actor: &str,
    rel: &str,
    selector: &str,
) {
    let doc = fs::load(root, Path::new(rel)).expect("load");
    let params = wire_serve::read::ReadParams {
        mode: Some("sections".into()),
        sections: Some(vec![wire::ReadSel::parse(selector)]),
        actor: Some(actor.to_owned()),
        ..Default::default()
    };
    wire_serve::read::composed_read(
        &doc,
        &WPath(rel.into()),
        &wire::Root("r0".into()),
        &params,
        Some(store),
        &wire_serve::read::NO_DECORATIONS,
    )
    .expect("the read serves");
}

fn vibe_pin_args(actor: &str, selector: &str, vibe: bool) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("plan.md".into()),
        actor: Some(actor.to_owned()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: wire::ReadSel::parse(selector),
            vibe: vibe.then_some(true),
        }),
    }
}

/// **Criterion 2, as worded by R39** — *"the read-mint gate is enforced on the
/// agent path, normal + vibe"*.
///
/// Review finding 23: the exit package claimed the gate holds "normal + vibe" and
/// **no vibe pin ever ran through the gate** — the vibe tests exercised the blob
/// plane with no actor, so the gate arm never executed on that path. This is that
/// missing half, and it asserts the REFUSAL (never a colour, never a blob count):
/// a fix that admitted an un-read vibe pin and merely rendered it grey would pass
/// any output-shape assertion and still mint an attestation nobody read.
///
/// The normal arm runs beside it in the same session over the same store, so the
/// two are proven to share one gate rather than two look-alike ones.
#[test]
fn criterion_2_the_gate_refuses_an_unread_vibe_pin_and_admits_a_read_one() {
    const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";
    const TARGET: &str =
        "# Guide\n\n## Goal\n\nreview before you close.\n\n## Other\n\nunrelated.\n";

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    git(dir.path(), &["init", "-q"]);
    git(
        dir.path(),
        &["config", "user.email", "fixv@example.invalid"],
    );
    git(dir.path(), &["config", "user.name", "fixv"]);
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let store = receipt::read_mint::ReadMintStore::new();
    let vibe = vibe_pin_args("agent-7", "Guide/Goal", true);

    // ── the assert that IS the claim: an un-read VIBE pin is REFUSED ──────────
    let err = splice(&root, 0, &vibe, &[], Some(&store)).expect_err("un-read vibe pin refuses");
    assert_eq!(
        err.code,
        ErrorCode::ReadMintRequired,
        "the vibe door is the SAME gate, typed the same way"
    );
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("never in your context")),
        "the refusal teaches (R24): {:?}",
        err.message
    );
    // And it refused before any byte moved — no eager blob, no promotion, no lock.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("guide.md")).expect("read"),
        TARGET,
        "a refused vibe pin leaves the TARGET byte-identical"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        PINNER,
        "a refused vibe pin leaves the PINNER byte-identical"
    );
    assert_eq!(store.len(), 0, "a refusal mints no receipt of its own");

    // A read of the WRONG selector does not open the vibe door either — the
    // receipt is selector-grained, and vibe does not soften that.
    session_read(&root, &store, "agent-7", "guide.md", "Guide/Other");
    let err = splice(&root, 0, &vibe, &[], Some(&store))
        .expect_err("a sibling section's read does not authorize this pin");
    assert_eq!(err.code, ErrorCode::ReadMintRequired);

    // ── the same request, after a COVERING read: admitted ────────────────────
    session_read(&root, &store, "agent-7", "guide.md", "Guide/Goal");
    let out = splice(&root, 0, &vibe, &[], Some(&store)).expect("the read-backed vibe pin commits");
    let wire::ResponseBody::Splice { pin, .. } = &out.body else {
        panic!("splice body");
    };
    let fact = pin
        .as_deref()
        .expect("a pin request answers with a pin fact");
    assert!(
        fact.fingerprint.starts_with("fp1.span2.b3."),
        "a real fp1 token, not a placeholder: {}",
        fact.fingerprint
    );
    let landed = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    assert!(
        landed.contains("```meridian-lock") && landed.contains(&fact.fingerprint),
        "the lock block carries the minted token:\n{landed}"
    );

    // ── and the NORMAL arm, same session, same store: one gate, not two ───────
    let normal = vibe_pin_args("agent-mallory", "Guide/Other", false);
    let err = splice(&root, 0, &normal, &[], Some(&store))
        .expect_err("a foreign actor's un-read NORMAL pin refuses identically");
    assert_eq!(err.code, ErrorCode::ReadMintRequired);
}

/// The CLI half of criterion 2 — *"verified through the CLI"* (R39). `mrd pin` is
/// the local-operator-trusted door (D16, no actor), so what the CLI proves is the
/// MINT: a real `fp1` token, a real lock block on disk, and — for `--vibe` — the
/// eager blob, counted by the vibe-debt gauge that exists to make that debt
/// visible rather than free.
#[test]
fn criterion_2_the_cli_mints_a_real_pin_normal_and_vibe() {
    let sb = sandbox();

    // normal
    let ws = sb.workspace("crit2-normal");
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nthe pinned body\n",
    );
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    let out = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(code(&out), 0, "normal pin: {}", said(&out));
    assert!(
        stdout(&out).contains("fingerprint: fp1."),
        "the CLI reports the minted token: {}",
        stdout(&out)
    );
    let claim = read(&ws, "claim.md");
    assert!(
        claim.contains("```meridian-lock")
            && claim.contains("pins:")
            && claim.contains("hash:")
            && claim.contains("fingerprint:"),
        // R4 retired the top-level `objects:` table this used to look for; the
        // blob rides the pin row as its MANDATORY `hash`, so checking `hash:`
        // and `fingerprint:` is the successor check — and a stronger one, since
        // both are now required on every row rather than optional planes.
        "a canonical lock block landed:\n{claim}"
    );
    let gauge = sb.run(&ws, &["status", "--json"]);
    let gauge: serde_json::Value = serde_json::from_str(&stdout(&gauge)).expect("status json");
    assert_eq!(
        gauge["composed"]["vibe_debt"]["blobs"], 0,
        "a NORMAL pin only COMPUTES the blob — it owes no vibe debt: {gauge}"
    );

    // vibe — pinning content that is not in any commit
    let ws = sb.workspace("crit2-vibe");
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nthe pinned body\n",
    );
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nedited after the commit\n",
    );
    let out = sb.run(
        &ws,
        &["pin", "--vibe", "claim.md", "source.md#Source/Guideline"],
    );
    assert_eq!(code(&out), 0, "vibe pin: {}", said(&out));
    assert!(
        stdout(&out).contains("blob:"),
        "the vibe pin writes the eager blob: {}",
        stdout(&out)
    );
    let gauge = sb.run(&ws, &["status", "--json"]);
    let gauge: serde_json::Value = serde_json::from_str(&stdout(&gauge)).expect("status json");
    assert_eq!(
        gauge["composed"]["vibe_debt"]["blobs"], 1,
        "the vibe-debt gauge COUNTS the eager blob — a meter, never a gate: {gauge}"
    );
    assert_eq!(
        gauge["findings"], false,
        "vibe debt is metered, not gated: {gauge}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// CRITERION 4 — the decorate → strip round trip, at document grain
// ═══════════════════════════════════════════════════════════════════════════

/// **Criterion 4, as worded by R39** — *"No WRITE introduces an `@fp` token in a
/// claim-link position on disk; a pre-existing token is left exactly as found.
/// Not 'no `@` anywhere' — code fences are deliberately not stripped."*
///
/// The ROUND TRIP end to end: a real `mrd pin` mints, the DECORATE side mints the
/// tone token into the claim link, an agent copies that decorated link back —
/// twice, which is finding 2's doubled-token shape — and the write lands stripped.
///
/// The decorate half runs through `wire_serve::read::page_decorations` +
/// `composed_read`, which is **the exact pair `crates/registry/src/server.rs:905`
/// calls** — the resident daemon is the one host that holds a corpus, so it is the
/// one host that decorates. The bare `mrd read` CLI passes `NO_DECORATIONS` by
/// design (`read.rs:309`), so the decorate half is reachable through the daemon,
/// not through a daemonless CLI drive. Stated rather than glossed: this test
/// drives production functions for the decorate half and the INSTALLED binary for
/// the strip half.
/// The DECORATE half of the round trip, through the pair the resident daemon
/// itself calls at `crates/registry/src/server.rs:905` — `page_decorations` over
/// a warm corpus, then `composed_read` with those decorations.
///
/// Returns the decorated claim link, verbatim, for the strip half to feed back.
/// The R22 assertions live here because they are assertions about DECORATING: the
/// tone rides always, it rides the block-ref slot, and the raw section face beside
/// it stays undecorated (A-K1).
fn decorate_the_way_the_daemon_does(ws: &Path, rel: &str, section: &str) -> String {
    let canonical = workspace::canonicalize(ws).expect("canonicalize");
    let root = fs::WorkspaceRoot(canonical);
    let (files, _fp) = fs::domain_snapshot(&root).expect("snapshot");
    let (index, docs) = fs::build_corpus(files).expect("build corpus");
    let decorations = wire_serve::read::page_decorations(&index, &docs, rel);
    let doc = docs.get(rel).expect("the page is in the corpus");
    let params = wire_serve::read::ReadParams {
        mode: Some("sections".into()),
        sections: Some(vec![wire::ReadSel::parse(section)]),
        ..Default::default()
    };
    let body = wire_serve::read::composed_read(
        doc,
        &WPath(rel.into()),
        &wire::Root("r0".into()),
        &params,
        None,
        &decorations,
    )
    .expect("the read serves");

    let wire::ResponseBody::Read {
        rendered_text,
        sections,
        ..
    } = &body
    else {
        panic!("read body: {body:?}");
    };

    let minted = fp_tokens(rendered_text);
    assert_eq!(
        minted.len(),
        1,
        "R22: the tone rides ALWAYS on a computed claim link — an absent marker \
         meaning either 'green' or 'not computed' is the defect s1c removed. \
         rendered:\n{rendered_text}"
    );
    assert!(
        minted[0].starts_with("@green."),
        "the pin is live, so the tone is green: {minted:?}"
    );
    let (dest, label) = sole_link(rendered_text);
    let decorated = format!("[[{dest}|{label}]]");
    assert!(
        decorated.contains(&minted[0]),
        "the token rides the BLOCK-REF slot of the link: {decorated}"
    );

    // The raw face is preserved beside it (A-K1) — `sections[].content` is what a
    // writer round-trips, and it never carries the tone.
    let raw: String = sections
        .iter()
        .flatten()
        .map(|s| s.content.clone())
        .collect();
    assert!(
        fp_tokens(&raw).is_empty(),
        "A-K1: the raw section face stays undecorated:\n{raw}"
    );

    decorated
}

#[test]
fn criterion_4_the_decorate_strip_round_trip_lands_clean_through_the_cli() {
    let sb = sandbox();
    let ws = sb.workspace("crit4-roundtrip");
    write(&ws, "guide.md", "# Guide\n\n## Goal\n\nthe goal body\n");
    write(
        &ws,
        "plan.md",
        "# Plan\n\n## Body\n\nwe rely on [[guide#^goal|the goal]].\n",
    );
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    let out = sb.run(&ws, &["pin", "plan.md", "guide.md#Guide/Goal"]);
    assert_eq!(code(&out), 0, "pin: {}", said(&out));

    // ── DECORATE (the daemon's own pair) ─────────────────────────────────────
    let decorated = decorate_the_way_the_daemon_does(&ws, "plan.md", "Plan/Body");

    // ── STRIP (the installed binary) — the agent copies the decorated link BACK,
    //    twice, and adds a fenced sample of the same shape. One write, both rules.
    let new =
        format!("we rely on {decorated} and again {decorated}.\n\n```\nsample {decorated}\n```\n");
    let edits = serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Plan"}, {"h": "Body"}]},
        "edit": {"match": {
            "old": "we rely on [[guide#^goal|the goal]].",
            "new": new,
        }}
    }]))
    .expect("edits json");
    let out = sb.run_stdin(&ws, &["put", "plan.md", "--actor", "agent-scribe"], &edits);
    assert_eq!(code(&out), 0, "put: {}", said(&out));

    let landed = read(&ws, "plan.md");
    let prose_line = landed
        .lines()
        .find(|l| l.starts_with("we rely on"))
        .expect("the prose line landed");
    assert!(
        fp_tokens(prose_line).is_empty(),
        "BOTH copies of the token are stripped — a single-pass strip would leave \
         a well-formed token behind (finding 2): {prose_line}"
    );
    assert_eq!(
        prose_line.matches("[[guide#^goal|the goal]]").count(),
        2,
        "the links survive their decoration: {prose_line}"
    );
    let fenced = landed
        .lines()
        .find(|l| l.trim_start().starts_with("sample "))
        .expect("the fenced sample landed");
    assert!(
        !fp_tokens(fenced).is_empty(),
        "R22, deliberately NOT stripped: a token-shaped string inside a fence is a \
         code SAMPLE, and stripping it would be corruption: {fenced}"
    );
}

/// **The four disk-landing paths from the blocking review, re-driven.** Findings
/// 1 (label absorption), 2 (doubled token), 13 (`create.title`) and 22 (the create
/// path's exclusions). Two are CLI-reachable and are driven through the binary;
/// two are agent-path-only and are driven through the production functions.
///
/// **Reachability, stated rather than assumed:** `plan_edits` has ZERO CLI doors —
/// all three `mrd` call sites (`pin_cmd.rs:75`, `put_cmd.rs:67`,
/// `test_cmd.rs:533`) pass `Vec::new()`. `write::create`'s CLI doors are
/// `mrd new` / `mrd unfold` (via `preset`) and the `mrd test` scenario runner.
#[test]
fn criterion_4_the_four_disk_landing_paths_leave_no_token() {
    let sb = sandbox();
    path_a_label_absorption(&sb);
    path_b_doubled_token(&sb);
    path_c_create_title();
    path_d_create_position_exclusions();
}

/// **PATH A (finding 1, P0)** — label absorption, driven through the surface that
/// actually DECORATES. The label deliberately REPEATS its own block ref: decorate
/// located the slot by a whole-node `rfind`, so the label's copy won the search,
/// absorbed the minted token, and the put-side strip — which parses the address
/// and never reads the label — could not find it again, so it reached disk. No
/// adversarial author needed: a page that cites itself is ordinary prose.
///
/// **Why this path runs a resident daemon, stated rather than assumed.**
/// `wire_serve::read::page_decorations` has exactly ONE production caller —
/// `crates/registry/src/server.rs:905`, the resident daemon, the one host that
/// holds a corpus — and the bare CLI passes `NO_DECORATIONS` by design
/// (`read_cmd.rs`). So a drive that only runs `mrd pin` **never invokes decorate
/// at all**, and stays green with finding 1 restored: a vacuous gate on a forgery
/// finding, which is the false-green class this suite exists to close. The daemon
/// is started in-process on this sandbox's own socket, and the read's `source`
/// field is asserted to be `daemon` FIRST — a degrade to the ephemeral engine
/// decorates nothing, and must fail loudly here rather than pass by proving
/// nothing.
///
/// Both halves of the round trip are asserted, because finding 1 is a defect in
/// their relationship: the mint rides the ADDRESS half (mechanism), and the
/// agent's copy-back lands with nothing token-shaped on disk (consequence).
fn path_a_label_absorption(sb: &Sandbox) {
    const PROSE: &str = "draws from [[guide#^goal|cf. guide#^goal]].";

    let ws = sb.workspace("c4-label");
    write(&ws, "guide.md", "# Guide\n\n## Goal\n\nthe goal body\n");
    write(&ws, "plan.md", &format!("# Plan\n\n## Body\n\n{PROSE}\n"));
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    let out = sb.run(&ws, &["pin", "plan.md", "guide.md#Guide/Goal"]);
    assert_eq!(code(&out), 0, "pin: {}", said(&out));

    // ── DECORATE: `mrd read` through the ONE host that decorates ─────────────
    let _daemon = sb.start_daemon();
    let out = sb.run(
        &ws,
        &["read", "plan.md", "--section", "Plan/Body", "--json"],
    );
    assert_eq!(code(&out), 0, "read: {}", said(&out));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("read json");
    assert_eq!(
        doc["source"], "daemon",
        "PATH A must DRIVE decorate or it proves nothing: the ephemeral degrade \
         passes NO_DECORATIONS, so accepting it would rebuild the vacuous gate \
         this test replaces: {doc}"
    );
    let rendered = doc["read"]["rendered_text"]
        .as_str()
        .unwrap_or_else(|| panic!("rendered_text: {doc}"));
    let (dest, label) = sole_link(rendered);
    assert_eq!(
        fp_tokens(&dest).len(),
        1,
        "the mint rides the ADDRESS half of the link, which is the half the \
         strip parses: dest={dest:?} label={label:?}"
    );
    assert!(
        fp_tokens(&label).is_empty(),
        "PATH A: a label that repeats its own block ref must not absorb the \
         mint — a token there is one the put-side strip cannot see: \
         dest={dest:?} label={label:?}"
    );

    // ── STRIP: the agent copies the decorated link back through `mrd put` ────
    let decorated = format!("[[{dest}|{label}]]");
    let edits = serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Plan"}, {"h": "Body"}]},
        "edit": {"match": {"old": PROSE, "new": format!("draws from {decorated}.")}}
    }]))
    .expect("edits json");
    let out = sb.run_stdin(&ws, &["put", "plan.md", "--actor", "agent-scribe"], &edits);
    assert_eq!(code(&out), 0, "put: {}", said(&out));
    for rel in ["plan.md", "guide.md"] {
        let text = read(&ws, rel);
        assert!(
            fp_tokens(&text).is_empty(),
            "PATH A: a token decorate put where the strip cannot see it is a \
             forged claim on disk in {rel}:\n{text}"
        );
    }
    assert!(
        read(&ws, "plan.md").contains(PROSE),
        "and the self-repeating label survives its decoration verbatim: {}",
        read(&ws, "plan.md")
    );
}

/// **PATH B (finding 2, P0)** — a DOUBLED token, through the CLI. A single-pass
/// strip turns `@a@b` into a WELL-FORMED token: a forged claim indistinguishable
/// from a mint. The assert is that nothing token-shaped survives, so "stripped to
/// something valid" fails exactly like "not stripped at all".
fn path_b_doubled_token(sb: &Sandbox) {
    let ws = sb.workspace("c4-doubled");
    write(&ws, "guide.md", "# Guide\n\n## Goal\n\nthe goal body\n");
    write(
        &ws,
        "plan.md",
        "# Plan\n\n## Body\n\ndraws from the guide.\n",
    );
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    let edits = serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Plan"}, {"h": "Body"}]},
        "edit": {"match": {
            "old": "draws from the guide.",
            "new": "draws from [[guide#^goal@green.aaaaaaaa@green.bbbbbbbb|G]].",
        }}
    }]))
    .expect("edits json");
    let out = sb.run_stdin(&ws, &["put", "plan.md", "--actor", "agent-scribe"], &edits);
    assert_eq!(code(&out), 0, "put: {}", said(&out));
    let landed = read(&ws, "plan.md");
    assert!(
        fp_tokens(&landed).is_empty(),
        "PATH B: a doubled token must not strip to a well-formed one:\n{landed}"
    );
    assert!(
        landed.contains("[[guide#^goal|G]]"),
        "the link survives, addressed by its undecorated block ref:\n{landed}"
    );
}

/// **PATH C (finding 13, P2)** — `plan_edits.create.title`, the agent path. The
/// lowering interpolates the title into HEADING bytes, and the old strip walked a
/// list of payload fields that never named it. At document grain the heading is
/// just part of the candidate.
fn path_c_create_title() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("plan.md"),
        "---\ntitle: Plan\n---\n\n# Plan\n\nbody\n",
    )
    .expect("seed");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    splice(
        &root,
        0,
        &SpliceArgs {
            id: None,
            origin: wire_serve::guard::Origin::InProcess,
            path: WPath("plan.md".into()),
            actor: Some("agent-scribe".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
            edits: Vec::new(),
            plan_edits: vec![wire::PlanEdit::Create {
                parent_hpath: "Plan".into(),
                title: "Draws from [[guide#^task1@green.19c66a18|Task One]]".into(),
                body: "body [[guide#^task1@green.19c66a18|B]]".into(),
            }],
            pin: None,
        },
        &[],
        None,
    )
    .expect("the create lowers and lands");
    let landed = std::fs::read_to_string(dir.path().join("plan.md")).expect("read");
    assert!(
        fp_tokens(&landed).is_empty(),
        "PATH C: the TITLE is heading bytes in the candidate document, and the \
         grain is the document — not a list of payload field names:\n{landed}"
    );
    assert!(
        landed.contains("## Draws from [[guide#^task1|Task One]]"),
        "the title landed, stripped, with its link intact:\n{landed}"
    );
}

/// **PATH D (finding 22, P2)** — the create path, at exactly the ruled width.
///
/// Frontmatter, HTML comments and indented code are NOT claim-link positions, for
/// the same reason a fence is not (R22): the dialect parse mints no link node
/// there, so the DECORATE side can never put a token there either. The criterion's
/// claim is about claim-link POSITIONS, and stating it wider than its proof is the
/// defect this whole loop exists to stop.
fn path_d_create_position_exclusions() {
    const TOKEN: &str = "@green.b3af12cd";

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("seed.md"), "# Seed\n").expect("seed");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let body = format!(
        "---\nsource: \"[[guide#^fm{TOKEN}|FM]]\"\n---\n\n# Born\n\n\
         live [[guide#^live{TOKEN}|L]]\n\n\
         <!-- [[guide#^cmt{TOKEN}|C]] -->\n\n\
         para\n\n    [[guide#^code{TOKEN}|D]]\n\n\
         ```text\n[[guide#^fence{TOKEN}|F]]\n```\n"
    );
    wire_serve::write::create(
        &root,
        0,
        &wire_serve::write::CreateArgs {
            id: None,
            path: WPath("born.md".into()),
            body,
            actor: Some("agent-scribe".into()),
            now: None,
            if_root: None,
            dry: false,
        },
        &[],
    )
    .expect("the birth lands");
    let landed = std::fs::read_to_string(dir.path().join("born.md")).expect("read");
    assert!(
        landed.contains("live [[guide#^live|L]]"),
        "PATH D: the ONE claim-link position is stripped:\n{landed}"
    );
    for (what, spelling) in [
        ("frontmatter", "^fm"),
        ("an HTML comment", "^cmt"),
        ("indented code", "^code"),
        ("a code fence", "^fence"),
    ] {
        assert!(
            landed.contains(&format!("[[guide#{spelling}{TOKEN}")),
            "{what} is not a claim-link position — the strip leaves it verbatim, \
             and the decorator never mints into it:\n{landed}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// THE FIX-LOOP REGRESSIONS — F4 forge, F6 dedupe, F12 refusal, in one pass
// ═══════════════════════════════════════════════════════════════════════════

/// **F4 (P1) — the gate guards the DOOR; the ARTIFACT is guarded separately.**
///
/// The reviewer's reproduction: one actor, one empty `ReadMintStore`, two requests.
/// `splice.pin` refuses `ReadMintRequired`, and then an ordinary `edits` batch
/// writes the very same `meridian-lock` bytes as page text and the verify plane
/// reads them **Green** — a claim nobody computed, by an actor with zero receipts.
///
/// The assert is on the WRITE, not on the colour (the re-verifier's rule): a fix
/// that let the forged block commit and merely rendered it grey would pass a
/// colour assertion and still have shipped the forgery. Driven through the
/// INSTALLED `mrd put`, which is a real ungated door a human can reach today.
#[test]
fn f4_the_artifact_guard_refuses_a_forged_lock_through_the_ordinary_edit_door() {
    let sb = sandbox();
    let ws = sb.workspace("f4-forge");
    write(&ws, "t.md", "# T\n\n## Section\n\nthe body\n");
    write(&ws, "claim.md", "# Claim\n\n## Body\n\ndraws from t.\n");
    write(&ws, "probe.md", "# Probe\n\n## Body\n\nx\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    // A REAL token, minted honestly on a throwaway page — the forger copies a
    // valid claim rather than inventing one, so nothing rejects it as malformed.
    let out = sb.run(&ws, &["pin", "probe.md", "t.md#T/Section"]);
    assert_eq!(code(&out), 0, "probe pin: {}", said(&out));
    let live = token_on_disk(&ws, "probe.md");

    let forged = format!(
        "draws from t.\n\n{}\n",
        lock_block(&[("t.md#T/Section", &live)])
    );
    let edits = serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Claim"}, {"h": "Body"}]},
        "edit": {"match": {"old": "draws from t.", "new": forged}}
    }]))
    .expect("edits json");
    let out = sb.run_stdin(
        &ws,
        &["put", "claim.md", "--actor", "agent-mallory"],
        &edits,
    );

    assert_ne!(
        code(&out),
        0,
        "THE CLAIM: lock bytes must not reach disk as ordinary page text — a \
         guard on a verb is not a guard on a file (R25). said:\n{}",
        said(&out)
    );
    let said = said(&out);
    assert!(
        said.contains("meridian-lock") && said.contains("without minting it"),
        "the refusal NAMES the artifact it protects: {said}"
    );
    assert!(
        said.contains("WHAT TO DO INSTEAD"),
        "R24: the refusal carries its cause AND the way forward: {said}"
    );
    let landed = read(&ws, "claim.md");
    assert!(
        !landed.contains("meridian-lock"),
        "nothing was committed:\n{landed}"
    );
    // And the un-attested page has no pin for any plane to colour.
    let out = sb.run(&ws, &["walk", "claim.md"]);
    assert!(
        walk_rows(&stdout(&out)).is_empty(),
        "no forged row reaches the walk plane: {}",
        stdout(&out)
    );
}

/// **F6 (P1) — `mrd walk` must not drop a red pin.** Two pins on ONE canonical
/// selector, one live and one drifted: walk deduped the listing, so it printed a
/// single GREEN row and exited 0 while `mrd status` rolled the same corpus up
/// `lock red content-drifted [2 pins]`. It failed in the dangerous direction, on
/// the surface most likely to be glanced at rather than read.
///
/// The assert carries both halves: the red row EXISTS and the green row is still
/// beside it. A "fix" that dropped the green instead would fail the count.
#[test]
fn f6_walk_renders_both_pins_that_share_one_selector_and_status_agrees() {
    let sb = sandbox();
    let ws = sb.workspace("f6-dedupe");
    write(&ws, "t.md", "# T\n\n## Section\n\nthe body\n");
    write(&ws, "claim.md", "# Claim\n\ndraws from t.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    let out = sb.run(&ws, &["pin", "claim.md", "t.md#T/Section"]);
    assert_eq!(code(&out), 0, "pin: {}", said(&out));
    let live = token_on_disk(&ws, "claim.md");
    let drifted = format!("fp1.span2.b3.{}", "a".repeat(64));
    assert_ne!(live, drifted, "the two pins must really differ");

    write(
        &ws,
        "claim.md",
        &format!(
            "# Claim\n\ndraws from t.\n\n{}\n",
            lock_block(&[
                ("t.md#T/Section", live.as_str()),
                ("t.md#T/Section", drifted.as_str()),
            ])
        ),
    );

    let out = sb.run(&ws, &["walk", "claim.md"]);
    let rows = walk_rows(&stdout(&out));
    assert_eq!(rows.len(), 2, "BOTH pins render, never one: {rows:?}");
    assert!(
        rows.iter().any(|r| walk_verdict(r) == "green"),
        "the green row is still there: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| walk_verdict(r) == "red content-drifted"),
        "the RED row is not hidden behind the green one: {rows:?}"
    );
    assert_eq!(
        code(&out),
        1,
        "and the walk exits 1 on its red: {}",
        said(&out)
    );

    let out = sb.run(&ws, &["status"]);
    let rollup = stdout(&out);
    assert!(
        rollup.contains("lock red content-drifted [2 pins]"),
        "the rollup plane agrees with the listing plane: {rollup}"
    );
}

/// **F12 (P2) — a REFUSED pin leaves the target byte-unchanged.** The promotion
/// used to run before the refusal, so a refused pin exited 2 having written a
/// `^slug` into a DIFFERENT file from the one the request named. Deterministic,
/// so unlike the accepted G3 orphan it never healed.
///
/// Re-driven with the R24 half beside it: the refusal must carry its CAUSE, not a
/// bare error class.
///
/// **U14 REPOINTED THE FIXTURE, not the expectation** (all-hands #2). This test
/// drove a `/`-bearing heading, whose refusal U14 RULED DEAD — an R4 `path`
/// array carries `["Guide", "A/B"]` unambiguously, so `/` no longer makes a
/// heading unrepresentable. Left alone, the old fixture still went red, but via
/// `pin_target_missing` (the sanitized `Guide/A-B` addresses nothing now)
/// instead of the representability refusal this test NAMES. Updating the
/// expected string would have left a green test permanently exercising a
/// selector typo.
///
/// So the fixture moved to the refusal that SURVIVES at the same door and in
/// the same family: a `#`-bearing heading. `#` is still a live delimiter in the
/// ingress grammars (wikilinks, `path#fragment`), nothing has ruled it
/// representable end to end, and the refusal fires at exactly the rung the
/// promotion used to precede — so the byte-identity claim below is tested on
/// the same path it always was.
#[test]
fn f12_a_refused_pin_leaves_the_target_byte_identical_and_says_why() {
    let sb = sandbox();
    let ws = sb.workspace("f12-atomic");
    write(&ws, "guide.md", "# Guide\n\n## A#B\n\nreview.\n");
    write(&ws, "plan.md", "# Plan\n\ndraws from the guide.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    let before_target = read(&ws, "guide.md");
    let before_pinner = read(&ws, "plan.md");

    let out = sb.run(&ws, &["pin", "plan.md", "guide.md#Guide/A#B"]);
    assert_ne!(
        code(&out),
        0,
        "a `#`-bearing heading is not representable: {}",
        said(&out)
    );
    let said = said(&out);
    // The REASON, precisely (all-hands #24): this must be the representability
    // refusal, not a selector miss that happens to be red.
    assert!(
        said.contains("carries a `#`") && said.contains("^id"),
        "R24: the refusal names the CAUSE and the way forward, never a bare \
         class — and it must be the `#` refusal, not a miss: {said}"
    );
    assert_eq!(
        read(&ws, "guide.md"),
        before_target,
        "the TARGET is byte-identical after a refusal"
    );
    assert_eq!(
        read(&ws, "plan.md"),
        before_pinner,
        "the PINNER is byte-identical after a refusal"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// R31 — the empty-span class, end to end on the shipped binary
// ═══════════════════════════════════════════════════════════════════════════

/// **R31/R38, re-driven end to end.** *"A fingerprint must not be able to match
/// content it does not cover."* A hash of empty bytes is a universal match, so a
/// pin over an empty normalized span can never drift — the purest false green
/// there is, in the module whose whole product is that green means green.
///
/// Every form in fix7's enumeration table is driven at the pin door on the shipped
/// binary, and **the assert is the REFUSAL, never a colour** (R31's own test
/// shape): a build that accepted a bare-anchor pin and rendered it grey would pass
/// a colour assertion and still ship a pin that cannot drift.
///
/// The control matters as much as the class: a list-item-hosted anchor MINTS,
/// because R1 removes only ` ^id` and the item text survives. A refusal that
/// swallowed the honest case too would be a false-red generator.
#[test]
fn r31_every_empty_span_form_refuses_at_the_pin_door() {
    let sb = sandbox();
    let ws = sb.workspace("r31-mint");
    // Rows 1-3: an own-line anchor mid-file, at EOF, and indented.
    write(&ws, "ownline_mid.md", "# T\n\nintro\n\n^midline\n\nmore\n");
    write(&ws, "ownline_eof.md", "# T\n\nintro\n\n^ateof\n");
    write(
        &ws,
        "ownline_ind.md",
        "# T\n\nintro\n\n  ^indented\n\nmore\n",
    );
    // Rows 4-5: the fragment-LESS whole-page ref — the form R31 did not predict.
    write(&ws, "empty.md", "");
    write(&ws, "anchors_only.md", "^a\n^b\n");
    // The control: an anchor hosted by a list item. Its span is the item LINE.
    write(&ws, "listitem.md", "# T\n\n- item text ^inline\n");
    write(&ws, "claim.md", "# Claim\n\nclaim.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    for (form, refr) in [
        ("1 own-line anchor, mid-file", "ownline_mid.md#^midline"),
        ("2 own-line anchor, at EOF", "ownline_eof.md#^ateof"),
        ("3 own-line anchor, indented", "ownline_ind.md#^indented"),
        ("4 whole-page ref over an empty file", "empty.md"),
        (
            "5 whole-page ref over an anchors-only file",
            "anchors_only.md",
        ),
    ] {
        let out = sb.run(&ws, &["pin", "claim.md", refr]);
        let said = said(&out);
        assert_ne!(
            code(&out),
            0,
            "form {form}: the pin door must REFUSE, never colour: {said}"
        );
        assert!(
            said.contains("no section addressed by") || said.contains("pin needs a SECTION"),
            "form {form}: the refusal is typed and teaches: {said}"
        );
        // Whole words only: the refusal for the whole-page forms legitimately
        // says "would REDDEN every dependent", and a substring probe would read
        // that as a verdict.
        let words: Vec<&str> = said
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        for colour in ["green", "grey", "red"] {
            assert!(
                !words.contains(&colour),
                "form {form}: a refusal is not a verdict — it must not print a \
                 colour word: {said}"
            );
        }
        assert!(
            !read(&ws, "claim.md").contains("meridian-lock"),
            "form {form}: nothing was minted"
        );
    }

    // THE CONTROL — the honest shape still mints.
    let out = sb.run(&ws, &["pin", "claim.md", "listitem.md#^inline"]);
    assert_eq!(
        code(&out),
        0,
        "CONTROL: an inline anchor whose host line survives normalization must \
         still mint — the guard closes a class, it does not blanket-refuse \
         anchors: {}",
        said(&out)
    );
}

/// The verify-side half, which R38 named the LOAD-BEARING one: the mint refusal is
/// not what closes the class, because the reachable door is a hand- or
/// tool-authored lock. A stored pin carrying the empty-span digest must NEVER read
/// green on any plane, in any document.
///
/// On the base build this exact fixture rendered `depth 1 green` on all three
/// pages and **stayed green after every target was rewritten end to end**.
#[test]
fn r31_a_stored_empty_span_pin_never_reads_green_on_any_plane() {
    let sb = sandbox();
    let ws = sb.workspace("r31-stored");
    // blake3 of zero bytes, computed through the real hasher rather than a
    // literal that could drift out of agreement with the code it stands for.
    let empty_digest = blake3::hash(b"").to_hex().to_string();
    let empty_token = format!("fp1.span2.b3.{empty_digest}");

    write(&ws, "ownline.md", "# T\n\nintro\n\n^midline\n\nmore\n");
    write(&ws, "empty.md", "");
    write(&ws, "anchors_only.md", "^a\n^b\n");
    for (page, refr) in [
        ("s_ownline", "ownline.md#^midline"),
        ("s_empty", "empty.md"),
        ("s_anchors", "anchors_only.md"),
    ] {
        write(
            &ws,
            &format!("{page}.md"),
            &locked_page(page, refr, &empty_token),
        );
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    for page in ["s_ownline.md", "s_empty.md", "s_anchors.md"] {
        let out = sb.run(&ws, &["walk", page]);
        let rows = walk_rows(&stdout(&out));
        assert_eq!(rows.len(), 1, "one stored pin, one row on {page}: {rows:?}");
        assert_ne!(
            walk_verdict(&rows[0]),
            "green",
            "a fingerprint of ZERO BYTES matches every document — it must never \
             read green on {page}: {rows:?}"
        );
    }
    // The other two planes say the same thing about the same corpus.
    let out = sb.run(&ws, &["status"]);
    assert!(
        stdout(&out).contains("lock red"),
        "the rollup plane does not report this corpus clean: {}",
        stdout(&out)
    );
    for row in board_rows(&ws) {
        assert_ne!(
            row.color, "green",
            "the board plane must not read an empty-span pin green: {row:?}"
        );
    }

    // And it stays non-green after the targets are rewritten end to end — the
    // property that makes this a PERMANENT false green rather than a stale one.
    write(
        &ws,
        "ownline.md",
        "# T\n\ncompletely different\n\n^midline\n",
    );
    write(&ws, "empty.md", "# now it has content\n");
    write(&ws, "anchors_only.md", "# T\n\nreal prose now\n");
    for page in ["s_ownline.md", "s_empty.md", "s_anchors.md"] {
        let out = sb.run(&ws, &["walk", page]);
        let rows = walk_rows(&stdout(&out));
        assert_eq!(rows.len(), 1, "{page}: {rows:?}");
        assert_ne!(
            walk_verdict(&rows[0]),
            "green",
            "{page} after rewrite: {rows:?}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// F26 — a malformed `objects:` sha is UNKNOWN, never a clean zero
// ═══════════════════════════════════════════════════════════════════════════

/// **Finding 26 (R27: IN — "a false green wearing different clothes").** A digest
/// that is not an object id was dropped before the vibe-debt gauge counted it, so a
/// corrupt retrieval plane read as a TRUE ZERO — `vibe-debt 0 blobs (0 bytes)`,
/// which is what a clean workspace reports.
///
/// The whole grey axis exists so that unverifiable never renders as verified. The
/// assert is on the JSON, because that is the shape a machine consumes and the one
/// a human render could paper over.
#[test]
fn f26_a_malformed_objects_sha_reports_unknown_never_a_clean_zero() {
    let sb = sandbox();
    let ws = sb.workspace("f26-gauge");
    write(&ws, "t.md", "# T\n\n## Section\n\nthe body\n");
    write(&ws, "claim.md", "# Claim\n\ndraws from t.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);
    let out = sb.run(&ws, &["pin", "claim.md", "t.md#T/Section"]);
    assert_eq!(code(&out), 0, "pin: {}", said(&out));

    // A clean baseline first — otherwise "unknown" could be the gauge's only mood.
    let clean = sb.run(&ws, &["status", "--json"]);
    let clean: serde_json::Value = serde_json::from_str(&stdout(&clean)).expect("status json");
    assert_eq!(
        clean["composed"]["vibe_debt"]["unknown"],
        serde_json::Value::Null
    );
    assert_eq!(clean["composed"]["vibe_debt"]["blobs"], 0);

    // Corrupt the retrieval plane: the pin's blob value is no longer an object id.
    // R4 retired the top-level `objects:` table — the hash rides the pin row it
    // was minted for — so the plane this fixture corrupts is the `hash:` field.
    let text = read(&ws, "claim.md");
    let corrupted = text
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("hash:") {
                "    hash: \"not-a-sha\"".to_owned()
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        corrupted.contains("not-a-sha"),
        "the fixture must actually corrupt the retrieval plane:\n{corrupted}"
    );
    write(&ws, "claim.md", &format!("{corrupted}\n"));

    let out = sb.run(&ws, &["status", "--json"]);
    assert_eq!(code(&out), 0, "status: {}", said(&out));
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("status json");
    let gauge = &json["composed"]["vibe_debt"];
    assert!(
        gauge["unknown"].is_string(),
        "a malformed digest lands UNKNOWN — it must not read as a true zero: {gauge}"
    );
    assert!(
        gauge["unknown"]
            .as_str()
            .is_some_and(|s| s.contains("claim.md") && s.contains("pin `t`")),
        // R24 is that the reading NAMES what it could not ask about. It used to
        // be spelled `claim.md objects.t`; with the table retired it is
        // `claim.md pin `t``. The assertion now pins the PIN as well as the
        // page, so it names the offender more precisely than before, not less.
        "the unknown NAMES what it could not ask about (R24): {gauge}"
    );
    assert!(
        gauge["label"]
            .as_str()
            .is_some_and(|s| s.starts_with("unknown")),
        "and the human render leads with the unknown, not with a zero: {gauge}"
    );
}
