//! **U30 — the stage-3 CROSS-SURFACE gate.** Mirrors stage 2s `fixv`
//! (`s2fix_cross_surface.rs`): *an exit criterion is verified across every surface it names, in
//! ONE run, by someone who did not write the units.* > **Per-unit proof and per-criterion proof
//! are DIFFERENT OBJECTS.** Fifteen > units each proving their own surface is not a proof of a
//! criterion that > names three. Stage 2s gate was withdrawn for exactly that substitution.
//! Every other suite in this workspace proves one units surface. This one proves a CRITERION:
//! ONE corpus, every surface the criterion names, one run, asserting the criterions own
//! ratified sentence. The engine under test `MRD_BIN` overrides the binary, so the same asserts
//! run twice with no edit: against `CARGO_BIN_EXE_mrd` in the workspace suite (criterion 7),
//! and against the **INSTALLED** artifact for the U30 evidence run (condition 2). The point of
//! a criterion is the artifact the fleet actually holds, not a build of it.
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
//!
//!

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

// ── the engine under test ───────────────────────────────────────────────────

/// The binary every drive in this file goes through. `MRD_BIN` names the INSTALLED artifact for
/// the U30 evidence run; the workspace suite uses the build under test.
///
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// The TRUE target, in the `notes` root.
const TARGET: &str = "# Notes\n\n## Design\n\nthe notes root's real design note.\n";
/// The DECOY: same basename, ambient root, DIFFERENT bytes. This is the file a
/// pre-U11 engine resolves `notes:notes.md` onto (FINDING 03).
const DECOY: &str = "# Notes\n\n## Design\n\nAMBIENT ROOT FILE — the wrong document.\n";

/// The canonical name of the vault root, and the vault name it carries. **They
/// differ, and that difference is criterion 2's whole non-vacuity argument.**
const VAULT_ROOT: &str = "notes";
const VAULT_NAME: &str = "ZT wiki";
/// The `git-folder` root — bound, and with no vault by construction.
const FOLDER_ROOT: &str = "archive";

struct Corpus {
    /// Load-bearing: the tree is deleted when this drops, so it is held for its
    /// Drop and read by nothing.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    /// The ambient workspace — where the decoy lives and the claims are written.
    ws: PathBuf,
    /// The vault-kind root's directory.
    notes: PathBuf,
    /// The `git-folder`-kind root's directory.
    archive: PathBuf,
    /// A real root that is NEVER bound — criterion 3's unmounted half.
    sessions: PathBuf,
    /// `$HOME/MERIDIAN.md`, the default rung's file.
    config: PathBuf,
}

/// Write a root's own self-declaration (INV-5). Without it the bind renders
/// `grey(undeclared)` and every acceptance below is vacuous.
fn declare(root: &Path, name: &str) {
    std::fs::write(
        root.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
    )
    .expect("root declaration");
}

/// The mount table, parameterised on the one axis criterion 2s sensitivity proof needs: whether
/// the vault name COLLIDES with the canonical name. `vault_name` is the corpus-side
/// falsification knob (S3-R72). Passing [`VAULT_ROOT`] here produces the *"all three spellings
/// collide"* fixture the criterion says **proves nothing** — and the assertion that rejects it
/// is the demonstrated-sensitive instrument for the translation axis.
///
///
fn table(notes: &Path, archive: Option<&Path>, vault_name: &str) -> String {
    use std::fmt::Write as _;
    let mut raw = "---\ntype: meridian-config\nversion: 1\n---\n\n# The U30 corpus\n\n".to_string();
    let _ = write!(
        raw,
        "```meridian-mount\nname: {VAULT_ROOT}\npath: {}\nkind: vault\nvault: {vault_name}\n```\n",
        notes.display()
    );
    if let Some(dir) = archive {
        let _ = write!(
            raw,
            "```meridian-mount\nname: {FOLDER_ROOT}\npath: {}\nkind: git-folder\n```\n",
            dir.display()
        );
    }
    raw
}

fn corpus() -> Corpus {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let notes = tmp.path().join("notes");
    let archive = tmp.path().join("archive");
    let sessions = tmp.path().join("sessions");
    for d in [&home, &ws, &notes, &archive, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    declare(&notes, VAULT_ROOT);
    declare(&archive, FOLDER_ROOT);
    declare(&sessions, "sessions");
    std::fs::write(notes.join("notes.md"), TARGET).expect("target");
    std::fs::write(ws.join("notes.md"), DECOY).expect("decoy");

    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, table(&notes, Some(&archive), VAULT_NAME)).expect("config");
    let cache_home = tmp.path().join("xdg-cache");

    Corpus {
        tmp,
        home,
        cache_home,
        ws,
        notes,
        archive,
        sessions,
        config,
    }
}

impl Corpus {
    /// A drive with `MERIDIAN_CONFIG` **SET** — the override rung.
    fn command(&self, args: &[&str]) -> Command {
        let mut c = self.bare(args);
        c.env("MERIDIAN_CONFIG", &self.config);
        c
    }

    /// A drive with `MERIDIAN_CONFIG` **REMOVED** — the default rung. Identical in every other
    /// respect, which is what makes criterion 1 a differential over exactly one variable.
    ///
    fn bare(&self, args: &[&str]) -> Command {
        let mut c = Command::new(mrd_bin());
        c.args(args)
            .current_dir(&self.ws)
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            // The producer-side isolation: this engine may not become the host's
            // resident daemon.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            // The bridge period's variables are cleared so the bridge section
            // does not depend on who ran this.
            .env_remove("CCC_LLM_WIKI_PATH")
            .env_remove("CCC_LLM_WIKI_REPOS_ROOT");
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("spawn mrd")
    }

    fn run_bare(&self, args: &[&str]) -> Output {
        self.bare(args).output().expect("spawn mrd")
    }

    fn put(&self, args: &[&str], edits: &str) -> Output {
        let mut child = self
            .command(args)
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

    /// Rewrite the mount table in place. The corpus tree is untouched — only the table moves, so
    /// the table alone carries any verdict change (S3-R28(c): when two facts are cited as one risk,
    /// measure which one carries it).
    fn rebind(&self, archive: bool, vault_name: &str) {
        std::fs::write(
            &self.config,
            table(
                &self.notes,
                archive.then_some(self.archive.as_path()),
                vault_name,
            ),
        )
        .expect("rewrite the table");
    }

    /// Write `claim.md` in the ambient workspace pinning `ref` at `rev`.
    fn claim(&self, reference: &str, rev: &str) {
        let claim = format!(
            "# Claim\n\ndraws from a mounted root.\n\n{}\n",
            lock_block(reference, rev)
        );
        std::fs::write(self.ws.join("claim.md"), claim).expect("claim");
        let init = self.run(&["init"]);
        assert!(init.status.success(), "init: {}", said(&init));
    }

    /// The one walk entry, as `(output, color, reason, detail, selector)`.
    fn walk_entry(&self) -> (Output, String, String, String, String) {
        let out = self.run(&["walk", "claim.md", "--json"]);
        let v: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("walk json ({e}): {}", text(&out)));
        let entries = v["walk"]["entries"].as_array().expect("entries").to_owned();
        assert_eq!(entries.len(), 1, "one pin, one entry: {entries:?}");
        let e = &entries[0];
        (
            out.clone(),
            str_of(e, "color"),
            str_of(e, "reason"),
            str_of(e, "detail"),
            str_of(e, "selector"),
        )
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn said(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// The live document-root rev of `raw` — the same parse the binary runs, so an The live
/// fingerprint token of a pages document root — what a CORRECT R4 pin holds, minted through the
/// engines own mint over the same parse the corpus builder runs.
///
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

/// One whole-body R4 pin, hand-written in the exact bytes `lock::render` emits — so these gates
/// depend on the CLIs own READER, never on the writer that made them. `object` keeps its
/// `root:` prefix: the canonical agent-plane spelling. NOTE FOR REVIEWERS: `version: 2` is the
/// LOCK FILE schema version, not the wire protocol version.
///
///
fn lock_block(object: &str, fingerprint: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"9ae3f1deadbeef\"\n    path: []\n    fingerprint: \"{fingerprint}\"\n```"
    )
}

/// The `mrd config` human row for `name`, whitespace-collapsed.
fn mount_row(out: &str, name: &str) -> String {
    out.lines()
        .map(str::trim_end)
        .find(|l| l.trim_start().starts_with(&format!("{name}  ")))
        .unwrap_or_else(|| panic!("no mount row for `{name}` in:\n{out}"))
        .trim()
        .to_string()
}

/// The `--json` mount object for `name`.
fn mount_json(v: &Value, name: &str) -> Value {
    v["mounts"]
        .as_array()
        .expect("mounts array")
        .iter()
        .find(|m| m["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no mount `{name}` in {v}"))
        .clone()
}

// ═══════════════════════════════════════════════════════════════════════════ CRITERION 1
// — Advisor S3-R33, verbatim "`mrd config` publishes the resolved configuration CHAIN, not
// merely its endpoint. Two environments differing ONLY in `MERIDIAN_CONFIG` produce
// DISTINGUISHABLE output — human line and `--json` alike — naming which origin supplied
// the config. VERIFIED DIFFERENTIALLY: run both spellings, set and unset, and diff them.
// BYTE-IDENTICAL OUTPUT ACROSS TWO ENVIRONMENTS THAT DIFFER IN THE CHAIN IS A FAILURE, NOT
// A PASS — it proves the surface publishes a constant rather than a resolution."
// ═══════════════════════════════════════════════════════════════════════════
//
//
//

/// **Criterion 1, as ONE object across every surface it names.** Four properties, one corpus,
/// one run: the differential (human **and** `--json`), the malformed refusal with **no partial
/// mount table**, the absent file leaving single-root behaviour unchanged, and the configs own
/// **rev**.
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
fn criterion_1_the_chain_is_published_and_the_two_spellings_are_distinguishable() {
    let c = corpus();

    // ── (a) THE DIFFERENTIAL, human face ────────────────────────────────────
    let set = c.run(&["config"]);
    let unset = c.run_bare(&["config"]);
    assert_eq!(code(&set), 0, "set: {}", said(&set));
    assert_eq!(code(&unset), 0, "unset: {}", said(&unset));
    let (human_set, human_unset) = (text(&set), text(&unset));

    assert_ne!(
        human_set, human_unset,
        "BYTE-IDENTICAL OUTPUT ACROSS TWO ENVIRONMENTS THAT DIFFER IN THE CHAIN IS A \
         FAILURE, NOT A PASS — it proves the surface publishes a constant rather than a \
         resolution.\n--- set ---\n{human_set}--- unset ---\n{human_unset}",
    );
    // The endpoint AGREES — which is what makes the line above a statement about
    // the chain rather than about the path.
    let endpoint = c.config.display().to_string();
    assert!(
        human_set.contains(&endpoint) && human_unset.contains(&endpoint),
        "both spellings resolve the SAME endpoint, so only the origin distinguishes them",
    );
    assert!(
        human_set.contains("origin:MERIDIAN_CONFIG"),
        "the override rung NAMES itself: {human_set}",
    );
    assert!(
        human_unset.contains("origin:$HOME/MERIDIAN.md"),
        "the default rung NAMES itself: {human_unset}",
    );

    // ── (b) THE DIFFERENTIAL, `--json` face ─────────────────────────────────
    let jset = c.run(&["config", "--json"]);
    let junset = c.run_bare(&["config", "--json"]);
    let (raw_set, raw_unset) = (text(&jset), text(&junset));
    assert_ne!(
        raw_set, raw_unset,
        "`--json` ALIKE — a machine face that publishes a constant is the same defect \
         one plane over:\n{raw_set}\n{raw_unset}",
    );
    let vset: Value = serde_json::from_str(&raw_set).expect("json set");
    let vunset: Value = serde_json::from_str(&raw_unset).expect("json unset");
    assert_eq!(vset["origin"].as_str(), Some("MERIDIAN_CONFIG"));
    assert_eq!(vunset["origin"].as_str(), Some("$HOME/MERIDIAN.md"));
    assert_eq!(
        vset["path"], vunset["path"],
        "the machine face agrees on the endpoint too — the origin is the only difference",
    );

    // ── (c) THE FILE CARRIES A REV ──────────────────────────────────────────
    assert!(
        human_set.contains("file_rev:"),
        "the config carries a rev like any page: {human_set}",
    );
    assert!(
        vset["file_rev"].as_str().is_some_and(|r| !r.is_empty()),
        "and states it in `--json`: {vset}",
    );

    // ── (d) MALFORMED → TEACHING REFUSAL, NO PARTIAL MOUNT TABLE ────────────
    let good = std::fs::read_to_string(&c.config).expect("read config");
    std::fs::write(
        &c.config,
        "---\ntype: meridian-config\nversion: 99\n---\n\n# bad\n",
    )
    .expect("malform");
    let bad = c.run(&["config"]);
    assert_eq!(
        code(&bad),
        1,
        "a refused config rides exit 1: {}",
        text(&bad)
    );
    let refusal = format!("{}{}", text(&bad), said(&bad));
    assert!(
        refusal.contains("version"),
        "the refusal TEACHES — it names what is broken: {refusal}",
    );
    assert!(
        !refusal.contains("mounts ("),
        "NO PARTIAL MOUNT TABLE: a refused config publishes nothing it could not \
         vouch for:\n{refusal}",
    );

    // ── (e) ABSENT → UNCHANGED SINGLE-ROOT BEHAVIOUR, and absent is NOT an error
    std::fs::remove_file(&c.config).expect("remove config");
    let absent = c.run_bare(&["config"]);
    assert_eq!(
        code(&absent),
        0,
        "absent is NOT an error — every machine starts here: {}",
        said(&absent),
    );
    assert!(
        text(&absent).contains("no config file — single-root behaviour, unchanged"),
        "and it SAYS so, rather than leaving a reader to read an empty table as a \
         failure: {}",
        text(&absent),
    );
    std::fs::write(&c.config, good).expect("restore");
}

// ═══════════════════════════════════════════════════════════════════════════ CRITERION 2
// — Advisor S3-R35 (amends S3-R33), verbatim "`mrd config` publishes the three-way
// translation — canonical name / vault name / path — for every mounted root, such that
// each of the three is recoverable from the output WHERE IT EXISTS. Verified over a corpus
// containing at least one root whose vault name DIFFERS from its canonical name, so the
// mapping is proven rather than coincidentally identical. A fixture in which all three
// spellings collide proves nothing about a translation. A ROOT WITH NO VAULT (A GIT-FOLDER
// ENTRY) IS A PARTIAL CELL BY DESIGN: THE CRITERION REQUIRES THE OUTPUT TO SAY SO — AN
// ABSENT VAULT RENDERED AS ABSENT, NEVER SILENTLY OMITTED — AND THE CORPUS MUST INCLUDE AT
// LEAST ONE SUCH ROOT, so the translation is not proven only where it is easiest."
// ═══════════════════════════════════════════════════════════════════════════
//
//
//
//

/// Does this output prove a TRANSLATION rather than a coincidence? **Extracted as a named
/// predicate because it is the instrument S3-R72 asks for.
///
///
///
///
///
///
fn translation_is_proven(v: &Value) -> bool {
    let m = mount_json(v, VAULT_ROOT);
    let (Some(name), Some(vault)) = (m["name"].as_str(), m["vault"].as_str()) else {
        return false;
    };
    name != vault && m["path"].as_str().is_some()
}

/// **Criterion 2, as ONE object across every surface it names.** Both faces, both root KINDS,
/// and all four legs the criterion carries: the three-way map where it exists, the absent vault
/// **stated**, the declared-vs-bound mismatch **failing loud**, and an absent declaration
/// rendering **grey**.
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
fn criterion_2_the_three_way_translation_is_recoverable_where_it_exists() {
    let c = corpus();

    let human = c.run(&["config"]);
    let json = c.run(&["config", "--json"]);
    assert_eq!(
        code(&human),
        0,
        "EVERY root is BOUND on this corpus, vault-less one included: {}",
        text(&human),
    );
    assert_eq!(code(&json), 0, "and the machine face agrees");
    let out = text(&human);
    let v: Value = serde_json::from_str(&text(&json)).expect("json");

    // ── (a) ALL THREE LEGS, WHERE THEY EXIST — the vault-kind root ───────────
    let row = mount_row(&out, VAULT_ROOT);
    let m = mount_json(&v, VAULT_ROOT);
    assert!(
        row.contains(VAULT_ROOT)
            && row.contains(&c.notes.display().to_string())
            && row.contains(&format!("vault:{VAULT_NAME}")),
        "canonical name, path, and vault name are ALL recoverable from the human \
         line: {row}",
    );
    assert_eq!(m["name"].as_str(), Some(VAULT_ROOT));
    assert_eq!(m["vault"].as_str(), Some(VAULT_NAME));
    assert!(
        m["path"].as_str().is_some(),
        "and all three from `--json`: {m}",
    );
    assert!(row.ends_with("bound"), "a translated root is bound: {row}");

    // ── (b) THE NON-COINCIDENCE, read from the OUTPUT and not from the fixture
    assert!(
        translation_is_proven(&v),
        "the vault name must DIFFER from the canonical name — a fixture in which all \
         three spellings collide proves nothing about a translation: {m}",
    );

    // ── (c) THE PARTIAL CELL, STATED — the git-folder root ───────────────────
    let folder_row = mount_row(&out, FOLDER_ROOT);
    assert!(
        folder_row.ends_with("vault:(none)  bound"),
        "AN ABSENT VAULT RENDERED AS ABSENT, NEVER SILENTLY OMITTED — a blank cell is \
         byte-identical to a dropped value, and the reader is the one asked to \
         guess: {folder_row}",
    );
    let folder = mount_json(&v, FOLDER_ROOT);
    assert!(
        folder.get("vault").is_some() && folder["vault"].is_null(),
        "the machine face states the same fact as `null` at a PRESENT key — an absent \
         key would be the silent omission the criterion rejects: {folder}",
    );
    assert_eq!(
        folder["kind"].as_str(),
        Some("git-folder"),
        "and it names the kind that makes the cell partial BY DESIGN: {folder}",
    );

    // ── (d) DECLARED-VS-BOUND MISMATCH FAILS LOUD ───────────────────────────
    declare(&c.notes, "renamed-behind-the-table");
    let loud = c.run(&["config"]);
    assert_eq!(code(&loud), 1, "a mismatch FAILS LOUD: {}", text(&loud));
    let msg = format!("{}{}", text(&loud), said(&loud));
    assert!(
        msg.contains("declares itself") && msg.contains("renamed-behind-the-table"),
        "and it names BOTH spellings, because a silent pick would make stored links \
         mean different things on different machines: {msg}",
    );
    assert!(
        !msg.contains("mounts ("),
        "a bind refusal publishes NO partial mount table: {msg}",
    );

    // ── (e) AN ABSENT DECLARATION RENDERS GREY ──────────────────────────────
    std::fs::remove_file(c.notes.join("MERIDIAN.md")).expect("undeclare");
    let grey = c.run(&["config"]);
    assert_eq!(
        code(&grey),
        1,
        "grey REFUSES on exit 1, exactly as red does — there is no fourth exit code \
         (S3-R6): {}",
        text(&grey),
    );
    let grey_out = text(&grey);
    assert!(
        mount_row(&grey_out, VAULT_ROOT).ends_with("grey(undeclared)"),
        "an absent self-declaration renders GREY, with its own reason word: {grey_out}",
    );
    // The ACCEPTANCE half in the same breath (S3-R8(c)): the OTHER root is still bound, so this is
    // a per-root grey and not a build that greys everything.
    assert!(
        mount_row(&grey_out, FOLDER_ROOT).ends_with("vault:(none)  bound"),
        "the untouched root stays BOUND — a guard proven only by what it blocks is \
         indistinguishable from a guard that blocks everything: {grey_out}",
    );
    declare(&c.notes, VAULT_ROOT);
}

/// **The S3-R72 sensitivity proof for criterion 2s TRANSLATION axis, run in this same process
/// against this same binary.** S3-R64s redden ancestor `1e89a61a` reddens criterion 2s
/// *differential* half but is **GREEN BY CONSTRUCTION on the translation half** — the mount
/// table
///
///
///
///
///
#[test]
fn criterion_2_the_translation_assertion_is_sensitive_corpus_side() {
    let c = corpus();

    // The corpus criterion 2 REQUIRES: vault name ≠ canonical name.
    let proven: Value = serde_json::from_str(&text(&c.run(&["config", "--json"]))).expect("json");
    assert!(
        translation_is_proven(&proven),
        "the required corpus must pass the predicate",
    );

    // The corpus criterion 2 REJECTS: all three spellings collide. Same binary,
    // same tree, one value changed in the mount table.
    c.rebind(true, VAULT_ROOT);
    let collided: Value = serde_json::from_str(&text(&c.run(&["config", "--json"]))).expect("json");
    assert!(
        !translation_is_proven(&collided),
        "THE INSTRUMENT MUST BE ABLE TO SAY NO. A colliding fixture that satisfies the \
         translation assertion means the assertion measures existence, not \
         translation — and the criterion's own sentence rejects it: {collided}",
    );

    // And the ABSENCE-MARKER assertion varies on its own axis too: drop the git-folder root and
    // the row it reads is gone. A population that can empty is a coverage arm that can silently
    // stop meaning anything (S3-R37).
    c.rebind(false, VAULT_NAME);
    let dropped = text(&c.run(&["config"]));
    assert!(
        !dropped.contains(FOLDER_ROOT),
        "with the partial-cell root unbound there is no row to assert — which is why \
         the corpus REQUIREMENT is part of the criterion and not a convenience: \
         {dropped}",
    );
    assert!(
        dropped.contains("mounts (1):"),
        "and the count moves with it, so the emptying is visible: {dropped}",
    );
}

// ═══════════════════════════════════════════════════════════════════════════ CRITERION 3 —
// the resolved BYTES come from the TARGET root
// ═══════════════════════════════════════════════════════════════════════════

/// **Criterion 3, as ONE object: all three verdicts on ONE corpus in ONE run.** Green when
/// matching, `red content-drifted` when not, **grey with a teaching refusal** when unmounted —
/// cited from **`mrd walk`**, the per-item surface §9.1 cleared, never from `mrd read`.
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
fn criterion_3_the_resolved_bytes_come_from_the_target_root_and_all_verdicts_render() {
    let c = corpus();
    c.claim(&format!("{VAULT_ROOT}:notes"), &live_fingerprint(TARGET));

    // ── (a) GREEN — mounted and matching ────────────────────────────────────
    let (out, color, reason, _d, selector) = c.walk_entry();
    assert_eq!(
        color,
        "green",
        "a cross-root ref to a MOUNTED root renders its true verdict: {}",
        text(&out),
    );
    assert!(reason.is_empty(), "green carries no reason word: {reason}");
    assert_eq!(
        selector,
        format!("{VAULT_ROOT}:notes.md"),
        "the listing names the ROOT-QUALIFIED address, never the bare in-root path",
    );
    assert_eq!(code(&out), 0, "a green walk exits 0");

    // ── (b) THE CONTROL FIRST — edit the AMBIENT DECOY. If the engine resolved onto the ambient
    // file, THIS is the edit that would move the verdict.
    std::fs::write(
        c.ws.join("notes.md"),
        "# Notes\n\n## Design\n\nDECOY EDITED.\n",
    )
    .expect("edit the decoy");
    let (_o, after_decoy, _r, _d, _s) = c.walk_entry();
    assert_eq!(
        after_decoy, "green",
        "editing the AMBIENT root's same-basename file must NOT move a cross-root \
         verdict — if it does, the bytes are coming from the wrong root (FINDING 03)",
    );

    // ── (c) NOW edit the file IN THE TARGET ROOT ─────────────────────────────
    std::fs::write(
        c.notes.join("notes.md"),
        "# Notes\n\n## Design\n\nthe notes root's design note, EDITED.\n",
    )
    .expect("edit the target");
    let (out, color, reason, _d, _s) = c.walk_entry();
    assert_eq!(
        color, "red",
        "editing the TARGET root's file moves the verdict — the bytes came from there",
    );
    assert_eq!(
        reason,
        "content-drifted",
        "a mounted root's drifted pin is RED content-drifted — a measured \
         disagreement, never grey: {}",
        text(&out),
    );
    assert_eq!(code(&out), 1, "a red edge is a finding — exit 1");

    // ── (d) THE UNMOUNTED HALF — grey with a teaching refusal ──────────────── Restore the bytes
    // first, so the ONLY thing that changes from green to grey is the mount table.
    //
    std::fs::write(c.notes.join("notes.md"), TARGET).expect("restore the target");
    let (_o, restored, _r, _d, _s) = c.walk_entry();
    assert_eq!(restored, "green", "green again — the corpus is back at (a)");

    // `sessions` is a real root with a real declaration that is NEVER bound.
    std::fs::write(c.sessions.join("notes.md"), TARGET).expect("target in the unmounted root");
    c.claim("sessions:notes", &live_fingerprint(TARGET));
    let (out, color, reason, detail, _s) = c.walk_entry();
    assert_eq!(
        color,
        "grey",
        "a ref to an UNMOUNTED root renders GREY — never red, never file_not_found: {}",
        text(&out),
    );
    assert!(
        !reason.is_empty() && reason != "content-drifted",
        "with its OWN reason word, distinct from the drift word (S3-R6/S3-R43): {reason}",
    );
    assert!(
        detail.contains("sessions"),
        "and a TEACHING refusal naming the missing mount — a refusal that does not \
         name the root teaches nothing: {detail}",
    );
    assert_eq!(
        code(&out),
        1,
        "grey rides exit 1, exactly as red does (S3-R6)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════ CRITERION 4
// — the stored form IS an `obsidian://` URI carrying the VAULT NAME
// ═══════════════════════════════════════════════════════════════════════════

fn put_body(new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Page"}, {"h": "Body"}]},
        "edit": {"put": {"at": "content", "text": new}},
    }]))
    .expect("edits json")
}

const PAGE: &str = "---\nroot: SESSION.md\ntitle: the def\n---\n\n# Page\n\n## Body\n\nseed\n";

/// **Criterion 4, as ONE object — and this corpus makes the positive half unfakeable.** The
/// canonical root name is `notes`; the vault name is `ZT wiki`.
///
///
///
///
///
///
///
#[test]
fn criterion_4_the_stored_form_carries_the_vault_name_and_round_trips() {
    let c = corpus();
    std::fs::write(c.ws.join("page.md"), PAGE).expect("seed");

    let agent = format!("[[{VAULT_ROOT}:notes.md#Design]]");
    let out = c.put(&["put", "page.md"], &put_body(&format!("see {agent}\n")));
    assert_eq!(code(&out), 0, "put refused: {}", said(&out));

    let disk = std::fs::read_to_string(c.ws.join("page.md")).expect("read back");

    // ── (a) THE POSITIVE HALF — a translation actually happened ──────────────
    assert!(
        disk.contains("obsidian://"),
        "the stored form IS an `obsidian://` URI: {disk}",
    );
    assert!(
        disk.contains("vault=ZT%20wiki") || disk.contains("vault=ZT wiki"),
        "carrying the VAULT NAME — and on this corpus the vault name is NOT the \
         canonical root name, so no identity function produces this line: {disk}",
    );
    assert!(
        !disk.contains(&format!("vault={VAULT_ROOT}")),
        "never the agent-plane canonical ROOT name: {disk}",
    );
    assert!(
        !disk.contains(&format!("[[{VAULT_ROOT}:notes.md#Design]]")),
        "and no agent-plane wikilink survives on disk: {disk}",
    );

    // ── (b) ROUND-TRIP IDENTITY, through the production render verb ──────────
    let read = c.run(&["read", "page.md", "--section", "Page/Body"]);
    assert_eq!(code(&read), 0, "read refused: {}", said(&read));
    assert!(
        text(&read).contains(&agent),
        "the rendered face reads back the agent-plane spelling byte-identically: {}",
        text(&read),
    );

    // ── (c) NO FINGERPRINT IN STORED BYTES OR A DISPLAY FIELD ───────────────
    for token in ["fp1.", "fingerprint", "node-rev"] {
        assert!(
            !disk.contains(token),
            "no write introduces `{token}` into stored bytes: {disk}",
        );
    }
    assert!(
        disk.starts_with("---\nroot: SESSION.md\ntitle: the def\n---\n"),
        "and `root:` in frontmatter survives byte-identically — it is a live YAML key \
         in the shipped def grammar, not an address position: {disk}",
    );
}
