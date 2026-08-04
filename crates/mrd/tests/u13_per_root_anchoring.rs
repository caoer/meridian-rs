//! **U13 — per-root anchoring, driven through the REAL `mrd status`.**
//!
//! Ratified `2026-07-24-cross-root-addressing.md` §4, verbatim: *"The
//! blob-anchoring check ... runs against **that root's** git repo — six roots,
//! six object stores, one law."*
//!
//! The fixture is built so that **no single-store implementation can produce its
//! numbers.** Two object ids each appear under TWO roots, and each is
//! `pending-anchor` in one of them and absent from the other:
//!
//! | pin `object` | `hash` | store asked | state | debt |
//! |---|---|---|---|---|
//! | `alpha:anchored` | A | alpha | anchored | — |
//! | `alpha:pending` | B | alpha | pending-anchor | **+B** |
//! | `alpha:not-here` | C | alpha | never-anchored | — |
//! | `beta:pending` | C | beta | pending-anchor | **+C** |
//! | `beta:not-here` | B | beta | never-anchored | — |
//! | `local` | D | ambient workspace | pending-anchor | **+D** |
//!
//! R4 retired the top-level `objects:` table; the blob hash rides the pin row it
//! was minted for, so the six entries above are six PINS carrying six hashes.
//!
//! So the gauge reads **3 blobs**, and its byte sum is exactly `B + C + D`.
//! Asking one store for all six entries cannot land there whatever it does with
//! the root prefix: the ambient repository alone reads 1 blob (`D`), alpha alone
//! reads `B`, beta alone reads `C`. **The oid is not the variable — the store
//! is.**
//!
//! Both arms of S3-R8(c) ride in that one reading: anchoring **succeeds** in the
//! correct store (`alpha:anchored` is found anchored, and `alpha:pending`
//! is found owed) **and degrades honestly** when the blob is absent from the
//! store asked (`alpha:not-here` / `beta:not-here` classify
//! `never-anchored` rather than borrowing the other root's answer).
//!
//! The remaining gates ride their own fixtures: a mounted root that is **not a
//! git repository** (honest degradation, never a fabricated sha), a root the
//! mount table does not bind at all, and — on every one of them — **`mrd
//! status`'s exit triad is UNCHANGED** (R12): the gauge is a meter, never a gate.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// Bytes committed in `alpha` — a ref reaches this blob there.
const ANCHORED_IN_ALPHA: &str = "alpha: committed, so a ref reaches it\n";
/// Bytes eagerly written in `alpha` only — owed THERE, absent from `beta`.
const PENDING_IN_ALPHA: &str = "alpha: eager, reachable from nothing at all\n";
/// Bytes eagerly written in `beta` only — owed THERE, absent from `alpha`.
const PENDING_IN_BETA: &str = "beta: eager\n";
/// Bytes eagerly written in the ambient workspace — the pre-U13 arm, unchanged.
const PENDING_IN_PROJECT: &str = "ambient: eager, the arm that always worked\n";

/// Run a fixture-setup git command, isolated from the operator's global and
/// system config so the fixture is the same repository on every machine.
fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("LC_ALL", "C")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "fixture `git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_owned()
}

/// An initialized repository with a commit identity, no hooks, no signing.
fn git_init(dir: &Path) {
    git_in(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
    git_in(dir, &["config", "user.name", "u13 fixture"]);
    git_in(dir, &["config", "user.email", "u13@fixture.invalid"]);
    git_in(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_all(dir: &Path, message: &str) {
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

/// Write `name` and hand back the object id git WOULD store — the eager vibe
/// write (`hash-object -w`), so the blob is present and reachable from nothing.
fn eager_blob(dir: &Path, name: &str, body: &str) -> String {
    std::fs::write(dir.join(name), body).expect("write fixture file");
    git_in(dir, &["hash-object", "-w", "--", name])
}

/// A root's own self-declaration (INV-5) — without it the mount binds grey
/// `undeclared` and every acceptance below would be vacuous.
fn declare_root(dir: &Path, name: &str) {
    std::fs::write(
        dir.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name} root\n"),
    )
    .expect("root declaration");
}

/// One `meridian-mount` block. `kind` is deliberately a parameter: the anchoring
/// law is ONE law, so a `git-folder` root and a `vault` root are asked the same
/// question of their own object stores.
fn mount_block(name: &str, path: &Path, kind: &str) -> String {
    let vault = if kind == "vault" {
        format!("vault: {name}\n")
    } else {
        String::new()
    };
    format!(
        "```meridian-mount\nname: {name}\npath: {}\nkind: {kind}\n{vault}```\n",
        path.display()
    )
}

/// A hand-written R4 (`version: 2`) `meridian-lock` retrieval plane — the gate
/// depends on the CLI's own reader, never on the writer that produced the bytes.
///
/// R4 retired the shared top-level `objects:` table and moved the git blob hash
/// onto the PIN ROW that needs it, so there is no successor table: each former
/// `objects:` entry is now a whole-page pin (`path: []`) whose `hash` is that
/// blob. `object` is the wiki-link inner text and is carried VERBATIM by the
/// parser, so it is spelled here exactly as the key it replaces named the target
/// — that spelling is what says WHICH root's object store is being asked, and it
/// is this file's whole subject.
///
/// The `fingerprint` is a well-formed token that matches nothing: every gate here
/// reads `composed.vibe_debt`, and a pin's CLAIM colour is a different plane that
/// neither the gauge nor the exit triad reads (R12).
///
/// NOTE FOR REVIEWERS: `version: 1` became `version: 2`. That is the LOCK FILE
/// schema version, not the wire protocol version.
fn lock_page(objects: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let token = format!("fp1.span2.b3.{}", "0".repeat(64));
    let mut out = String::from("# Effect\n\n```meridian-lock\nversion: 2\npins:\n");
    for (object, sha) in objects {
        let _ = writeln!(
            out,
            "  - object: \"[[{object}]]\"\n    hash: \"{sha}\"\n    path: []\n    \
             fingerprint: \"{token}\""
        );
    }
    out.push_str("```\n");
    out
}

struct Sandbox {
    /// Load-bearing: the sandbox tree is deleted when this drops, so it is held
    /// for its Drop and read by nothing.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    config: PathBuf,
    /// The ambient workspace.
    ws: PathBuf,
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

    /// The vibe-debt gauge of one `mrd status --json` run, with the exit code
    /// beside it — the exit triad is asserted on every reading, never assumed.
    fn gauge(&self) -> (Value, i32) {
        let out = self.run(&["status", "--json"]);
        let code = out.status.code().unwrap_or(-1);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let doc: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
            panic!(
                "status --json is not json ({e}): {text}\nstderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (doc["composed"]["vibe_debt"].clone(), code)
    }

    fn human(&self) -> String {
        String::from_utf8_lossy(&self.run(&["status"]).stdout).into_owned()
    }
}

/// The sandbox tree: an ambient workspace, plus the roots named in `mounts`
/// bound through a sandboxed `MERIDIAN.md`. `HOME` is sandboxed too, so the
/// mount table read here is this fixture's and never the operator's.
fn sandbox(mounts: &[(&str, &Path, &str)]) -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    for d in [&home, &ws] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    let mut raw = "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n".to_string();
    for (name, path, kind) in mounts {
        raw.push_str(&mount_block(name, path, kind));
        raw.push('\n');
    }
    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, raw).expect("config");

    let sb = Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        home,
        config,
        ws,
        tmp,
    };
    git_init(&sb.ws);
    let init = sb.run(&["init"]);
    assert!(
        init.status.success(),
        "mrd init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    sb
}

/// **GATE 1 + 2 — six roots, six object stores, one law, proven across THREE
/// distinct stores in ONE reading.**
///
/// Per-root anchoring is a per-CRITERION claim, so one root proving its own
/// surface would not be a proof of it. Two object ids each appear under two
/// roots here and answer differently in each, which is a statement about the
/// STORE and nothing else — an implementation asking one repository for every
/// entry cannot reproduce the count or the byte sum.
#[test]
fn the_anchoring_check_runs_against_that_roots_own_object_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let alpha = tmp.path().join("alpha");
    let beta = tmp.path().join("beta");
    for d in [&alpha, &beta] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    // `alpha` — a VAULT root. Its declaration and the anchored page are
    // committed FIRST, so the eager blob below is never swept into a commit.
    git_init(&alpha);
    declare_root(&alpha, "alpha");
    std::fs::write(alpha.join("anchored.md"), ANCHORED_IN_ALPHA).expect("write");
    commit_all(&alpha, "alpha: anchor one blob");
    let oid_a = git_in(&alpha, &["rev-parse", "HEAD:anchored.md"]);
    let oid_b = eager_blob(&alpha, "pending.md", PENDING_IN_ALPHA);

    // `beta` — a GIT-FOLDER root. Same law, different kind.
    git_init(&beta);
    declare_root(&beta, "beta");
    commit_all(&beta, "beta: declare");
    let oid_c = eager_blob(&beta, "pending.md", PENDING_IN_BETA);

    let sb = sandbox(&[
        ("alpha", alpha.as_path(), "vault"),
        ("beta", beta.as_path(), "git-folder"),
    ]);
    let oid_d = eager_blob(&sb.ws, "local.md", PENDING_IN_PROJECT);

    // Every oid is distinct, so a mis-attributed blob cannot hide behind a
    // collision.
    let mut distinct = vec![&oid_a, &oid_b, &oid_c, &oid_d];
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "four distinct blobs: {distinct:?}");

    // BASELINE — the ambient arm ALONE, so the instrument is shown reading a
    // measured value before the rooted keys are added. Without this, the reading
    // below could not be told from a gauge that only ever reports one number.
    std::fs::write(sb.ws.join("effect.md"), lock_page(&[("local", &oid_d)])).expect("write");
    let (ambient, code) = sb.gauge();
    assert_eq!(code, 0, "the exit triad is unchanged by any gauge reading");
    assert_eq!(ambient["unknown"], Value::Null, "measured: {ambient}");
    assert_eq!(ambient["blobs"], 1, "the ambient store owes one: {ambient}");
    assert_eq!(
        ambient["bytes"],
        PENDING_IN_PROJECT.len(),
        "and its bytes are the ambient blob's: {ambient}"
    );

    // THE READING — three stores, six entries, two oids asked of two roots each.
    std::fs::write(
        sb.ws.join("effect.md"),
        lock_page(&[
            ("alpha:anchored", &oid_a),
            ("alpha:pending", &oid_b),
            ("alpha:not-here", &oid_c),
            ("beta:pending", &oid_c),
            ("beta:not-here", &oid_b),
            ("local", &oid_d),
        ]),
    )
    .expect("write");

    let (debt, code) = sb.gauge();
    assert_eq!(
        code, 0,
        "R12: the exit triad is UNCHANGED — the gauge is a meter, never a gate"
    );
    assert_eq!(
        debt["unknown"],
        Value::Null,
        "every store was asked and answered: {debt}"
    );
    let expected_bytes = PENDING_IN_ALPHA.len() + PENDING_IN_BETA.len() + PENDING_IN_PROJECT.len();
    assert_eq!(
        (debt["blobs"].as_u64(), debt["bytes"].as_u64()),
        (Some(3), Some(expected_bytes as u64)),
        "each store answered for ITS OWN objects — one store for all six entries \
         reads (1, {}) ambient / (1, {}) alpha / (1, {}) beta, never (3, {expected_bytes}): {debt}",
        PENDING_IN_PROJECT.len(),
        PENDING_IN_ALPHA.len(),
        PENDING_IN_BETA.len(),
    );

    // The same reading on the surface a human sees.
    let human = sb.human();
    assert!(
        human.contains(&format!("vibe-debt 3 blobs ({expected_bytes} bytes)")),
        "the composed line carries the per-root reading: {human}"
    );
}

/// **GATE 3 — a mounted root that is NOT a git repository degrades honestly.**
///
/// The existing `Repo` convention: `GitFail::NotARepo`, surfaced as the gauge's
/// `unknown` with the ROOT NAMED — never a fabricated sha, and never a silent
/// fall back to the ambient store, which would answer a different repository's
/// question in this root's name. The degradation is ASSERTED, not merely the
/// absence of a crash.
#[test]
fn a_mounted_root_that_is_not_a_git_repo_degrades_honestly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).expect("mkdir");
    declare_root(&plain, "plain");
    std::fs::write(plain.join("payload.md"), "not under git at all\n").expect("write");

    let sb = sandbox(&[("plain", plain.as_path(), "git-folder")]);
    // A real, well-formed oid — the fixture must fail on the STORE, not on a
    // malformed value, which is a different reading with a different cause.
    std::fs::write(sb.ws.join("local-only.md"), PENDING_IN_PROJECT).expect("write");
    let oid = git_in(&sb.ws, &["hash-object", "--", "local-only.md"]);
    std::fs::write(
        sb.ws.join("effect.md"),
        lock_page(&[("plain:payload", &oid)]),
    )
    .expect("write");

    let (debt, code) = sb.gauge();
    assert_eq!(
        code, 0,
        "R12: the exit triad is UNCHANGED by an unmeasurable store"
    );
    let detail = debt["unknown"].as_str().unwrap_or_else(|| {
        panic!("an unaskable store reads unknown, never a measured zero: {debt}")
    });
    assert!(
        detail.contains("plain"),
        "the refusal names WHICH root could not be asked: {detail}"
    );
    assert!(
        detail.contains("not a git repository"),
        "and carries git's own reason verbatim: {detail}"
    );
    assert_eq!(debt["blobs"], 0, "unknown is not a count: {debt}");
    assert!(
        sb.human().contains("vibe-debt unknown"),
        "the composed line carries the same reading: {}",
        sb.human()
    );
}

/// **A root the mount table does not bind is unmeasurable, never ambient.**
///
/// The false negative this closes: silently resolving `ghost:payload.md` in the
/// ambient store would ask the WRONG object database and answer confidently —
/// `never-anchored` for a blob that is anchored where it actually lives. The
/// gauge says it cannot be asked, and teaches the declaration.
#[test]
fn an_unbound_root_is_unmeasurable_never_silently_ambient() {
    let sb = sandbox(&[]);
    // The blob IS present in the ambient store, so a silent fall back would
    // produce a confident reading rather than a visible refusal — the fixture
    // is built to make the wrong answer LOOK right.
    let oid = eager_blob(&sb.ws, "payload.md", PENDING_IN_PROJECT);
    std::fs::write(
        sb.ws.join("effect.md"),
        lock_page(&[("ghost:payload", &oid)]),
    )
    .expect("write");

    let (debt, code) = sb.gauge();
    assert_eq!(code, 0, "R12: the exit triad is UNCHANGED");
    let detail = debt["unknown"].as_str().unwrap_or_else(|| {
        panic!("an unbound root reads unknown, never the ambient answer: {debt}")
    });
    assert!(
        detail.contains("ghost") && detail.contains("not mounted"),
        "the refusal names the root and what is missing: {detail}"
    );
    assert_eq!(
        debt["blobs"], 0,
        "the ambient store's answer is NOT borrowed for another root: {debt}"
    );
}

/// **A key that is not an address names no store — so git cannot be asked.**
///
/// `addr::Addr::parse` refuses a malformed root rather than reading it as a
/// literal path, precisely so a typo cannot degrade into a confident lookup
/// somewhere else. That refusal is carried into the gauge as the SAME reading a
/// malformed sha produces, because it is the same reading: the question cannot
/// be put to git.
#[test]
fn a_key_that_names_no_store_is_unknown_not_ambient() {
    let sb = sandbox(&[]);
    let oid = eager_blob(&sb.ws, "payload.md", PENDING_IN_PROJECT);
    // Two colons in the head — refused by the address grammar, never
    // reinterpreted. The `.md` is kept ON PURPOSE where every other object in
    // this file drops it: the subject here is a spelling that is NOT an address,
    // so there is no vault path to canonicalize, and the parser carries the
    // object's inner text verbatim — the reading must name it exactly as written.
    std::fs::write(
        sb.ws.join("effect.md"),
        lock_page(&[("a:b:payload.md", &oid)]),
    )
    .expect("write");

    let (debt, code) = sb.gauge();
    assert_eq!(code, 0, "R12: the exit triad is UNCHANGED");
    let detail = debt["unknown"]
        .as_str()
        .unwrap_or_else(|| panic!("a key naming no store reads unknown: {debt}"));
    assert!(
        detail.contains("effect.md") && detail.contains("a:b:payload.md"),
        "the reading names WHERE the retrieval plane is unreadable: {detail}"
    );
    assert_eq!(debt["blobs"], 0, "and counts nothing: {debt}");
}
