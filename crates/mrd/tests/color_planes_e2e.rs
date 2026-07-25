//! Cross-surface gates for the color planes — ONE corpus read by BOTH `mrd walk`
//! and `mrd status`, over the real binary (`CARGO_BIN_EXE_mrd`).
//!
//! Every gate here is a plane-vs-plane agreement, not a rendering check. Each was
//! a shipped defect where two surfaces answered ONE question differently, and the
//! wrong answer was the reassuring one:
//!
//! - **finding 6** — `walk` deduped its listing by canonical selector, so two
//!   pins on one ref (one live, one drifted) rendered as the live one alone:
//!   `walk` printed green and exited 0 while `status` rolled the same corpus up
//!   `lock red content-drifted [2 pins]`.
//! - **finding 17** — a `meridian-lock` fence trailed by an `^inputs` anchor was
//!   read twice, once as the form-3 pin it is and once as a form-2 chain block,
//!   so ONE pin projected TWO rows carrying two verdicts.
//! - **finding 26** — an `objects:` value that is not an object id was dropped
//!   before the vibe-debt gauge counted it, so a corrupt retrieval plane read as
//!   a true zero.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
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
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A bare workspace with an `mrd init` marker so the verbs resolve it.
    fn workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
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

/// The LIVE fingerprint token of a page's document root — what a correct pin
/// holds, minted through the engine's own parse so the fixture cannot pin a token
/// the reader would not recompute.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

/// A well-formed `fp1.…` token that is NOT the live one — a pin that measures
/// drift. Minted over different bytes, so it is a real token with a real digest,
/// never a syntactic fake the reader could reject as malformed instead.
fn drifted_fingerprint() -> String {
    live_fingerprint("# Target\n\nbytes that are not on disk\n")
}

/// The canonical `meridian-lock` fence for `pins`, written by hand so the gate
/// depends on the CLI's own reader, never on the writer that produced the bytes.
fn lock_block(pins: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("```meridian-lock\nversion: 1\npins:\n");
    for (declared_ref, fingerprint) in pins {
        let _ = writeln!(
            out,
            "  - ref: \"{declared_ref}\"\n    fingerprint: \"{fingerprint}\""
        );
    }
    out.push_str("```");
    out
}

/// The `depth N …` listing lines of a human `walk` render.
fn walk_rows(human: &str) -> Vec<String> {
    human
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("depth "))
        .map(str::to_owned)
        .collect()
}

// ── finding 6 — a red pin must not vanish behind a green one ────────────────

/// F6 GATE — two pins on ONE ref, one live and one drifted: `mrd walk` renders
/// the RED row and exits 1 (its red contract), and `mrd status` agrees on the
/// same corpus.
///
/// The claim is the assert: the red row EXISTS in the listing, the green row is
/// still there beside it, and the exit code is the finding leg. A dedupe that
/// "fixed" this by keeping the red and dropping the green would fail the
/// row-count assert; a dedupe that keeps the green and drops the red — the
/// shipped behavior — fails the red-row assert and the exit assert. Only a
/// listing that carries BOTH verdicts passes.
///
/// Before the fix, verbatim on the deployed binary: `walk` printed
/// `depth 1  green  …` and exited 0 while `status` printed
/// `lock red content-drifted [2 pins]`.
#[test]
fn walk_renders_the_red_pin_that_shares_a_ref_with_a_green_one() {
    let sb = sandbox();
    let ws = sb.workspace("twopins");
    std::fs::create_dir_all(ws.join("sources")).expect("sources dir");

    let target = "# Target\n\nbody v1\n";
    std::fs::write(ws.join("sources/target.md"), target).expect("write target");
    std::fs::write(
        ws.join("claim.md"),
        format!(
            "# Claim\n\ndraws from it twice\n\n{}\n",
            lock_block(&[
                ("sources/target.md", &live_fingerprint(target)),
                ("sources/target.md", &drifted_fingerprint()),
            ])
        ),
    )
    .expect("write claim");

    let walk = sb.run(&ws, &["walk", "claim.md"]);
    let listing = stdout(&walk);
    let rows = walk_rows(&listing);

    assert_eq!(
        rows.len(),
        2,
        "both pins render — one row per pin, neither verdict collapsed: {listing}"
    );
    assert!(
        rows.iter().any(|r| r.contains("red content-drifted")),
        "the drifted pin renders RED in the listing: {listing}"
    );
    assert!(
        rows.iter().any(|r| r.contains("  green  ")),
        "the live pin still renders green beside it: {listing}"
    );
    assert_eq!(
        code(&walk),
        1,
        "a red edge is the finding leg of the exit triad: {}",
        stderr(&walk)
    );
    assert!(
        stderr(&walk).contains("1 red edge(s) in the walk"),
        "walk names the count it found: {}",
        stderr(&walk)
    );

    // The other plane, same corpus, same question: status must not disagree.
    let status = stdout(&sb.run(&ws, &["status"]));
    assert!(
        status.contains("lock red content-drifted [2 pins]"),
        "status rolls the same two pins up red: {status}"
    );
}

// ── finding 17 — one pin, one row, one verdict ──────────────────────────────

/// F17 GATE — a `meridian-lock` fence followed by an `^inputs` block anchor is
/// ONE pin and projects exactly ONE row.
///
/// The trailing anchor is the form-2 chain-block marker, so the form-2 reader
/// used to re-read the engine's own lock block: `status` counted `[2 pins]` for
/// one declared pin and the extra row carried a grey `declared-unpinned` verdict
/// the page never declared — grey is above green in the worst-of roll-up, so one
/// pin's green rendered grey.
///
/// The claim is the assert: the row COUNT is 1 and the verdict is the pin's own.
#[test]
fn a_lock_fence_trailed_by_an_inputs_anchor_is_one_pin_one_row() {
    let sb = sandbox();
    let ws = sb.workspace("form3anchor");
    std::fs::create_dir_all(ws.join("sources")).expect("sources dir");

    let target = "# Target\n\nbody v1\n";
    std::fs::write(ws.join("sources/target.md"), target).expect("write target");
    std::fs::write(
        ws.join("claim.md"),
        format!(
            "# Claim\n\ndraws from it\n\n{}\n\n^inputs\n",
            lock_block(&[("sources/target.md", &live_fingerprint(target))])
        ),
    )
    .expect("write claim");

    let status = stdout(&sb.run(&ws, &["status"]));
    assert!(
        status.contains("lock green [1 pin]"),
        "one declared pin is one row, and it carries the pin's OWN verdict: {status}"
    );

    let walk = sb.run(&ws, &["walk", "claim.md"]);
    let listing = stdout(&walk);
    assert_eq!(
        walk_rows(&listing).len(),
        1,
        "the walk lists the same one pin, not a second row for the same fence: {listing}"
    );
    assert_eq!(code(&walk), 0, "no red edge: {}", stderr(&walk));
}

// ── finding 26 — a malformed `objects:` sha is unknown, never zero ──────────

/// F26 GATE — an `objects:` value that is not an object id lands in the vibe-debt
/// gauge's `unknown` slot.
///
/// git cannot be asked about a value that is not an oid, so that entry's debt is
/// unmeasurable. Dropping it read a corrupt retrieval plane as a true `0 blobs
/// (0 bytes)` — a false clean in the one gauge whose whole purpose is to prevent
/// one. The claim is the assert: `unknown` is SET and names the damaged entry.
#[test]
fn a_malformed_objects_sha_is_counted_unknown_not_dropped_to_zero() {
    let sb = sandbox();
    let ws = sb.workspace("badsha");
    std::fs::write(
        ws.join("effect.md"),
        "# Effect\n\n```meridian-lock\nversion: 1\nobjects:\n  \"payload\": \"not-a-sha\"\n```\n",
    )
    .expect("write effect");

    let out = sb.run(&ws, &["status", "--json"]);
    assert!(
        out.status.success() || code(&out) == 1,
        "status --json ran: {}",
        stderr(&out)
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("status json");
    let debt = &doc["composed"]["vibe_debt"];

    assert!(
        debt["unknown"].is_string(),
        "a value git cannot be asked about reads unknown, never a measured zero: {debt}"
    );
    let detail = debt["unknown"].as_str().expect("unknown detail");
    assert!(
        detail.contains("effect.md") && detail.contains("payload"),
        "the reading names WHERE the retrieval plane is damaged: {detail}"
    );
    assert!(
        debt["label"]
            .as_str()
            .expect("label")
            .starts_with("unknown"),
        "the human gauge word carries the same reading: {debt}"
    );

    // The lock axis is untouched beside it — the two planes stay independent.
    let human = stdout(&sb.run(&ws, &["status"]));
    assert!(
        human.contains("vibe-debt unknown"),
        "the composed line renders the unknown gauge: {human}"
    );
}

// ── R31 — the empty-span pin that could never drift ─────────────────────────

/// The token a hand-authored lock carries for an empty-normalizing span:
/// `blake3` of NO bytes. Not a syntactic fake — a real, well-formed `fp1.…`
/// token with a real digest, which is exactly what made it dangerous.
fn empty_span_fingerprint() -> String {
    format!("fp1.span2.b3.{}", blake3::hash(b"").to_hex())
}

/// **R31 GATE — a stored empty-span pin can never read green.**
///
/// The empty-span class is unreachable through `mrd pin` (every ref form refuses
/// at mint — `s2fix_empty_span_mint.rs` asserts that door by door), so it
/// reaches the product exactly one way: a HAND- or TOOL-AUTHORED `meridian-lock`
/// block. This gate drives that door on the real binary, which is why it lives
/// here and hand-writes its lock rather than minting one.
///
/// Verbatim on the base build, both forms: `depth 1  green  …` and exit 0 —
/// before ANY edit and again after the targets were rewritten end to end. A pin
/// no edit anywhere could ever turn red, in the module whose entire product is
/// that green means green.
///
/// The claim is the assert, and it is a REFUSAL of the green, not a colour
/// preference: neither row may say `green`, on either plane, in either state.
/// A fix that rendered these grey and left the pin in place would still ship a
/// pin that cannot drift — so the row must be RED and the walk must exit 1.
#[test]
fn a_hand_authored_empty_span_pin_never_reads_green() {
    let sb = sandbox();
    let ws = sb.workspace("emptyspan");
    std::fs::create_dir_all(ws.join("sources")).expect("sources dir");

    // Two of the enumerated empty-normalizing forms, at the product surface:
    // an own-line `#^anchor` (the Block form) and a whole-page ref over a file
    // that is nothing but an own-line anchor (the Page form — the one the
    // ruling did not predict).
    let ownline_v1 = "# H\n\n^guideline\n\noriginal body\n";
    let anchors_only = "^a\n";
    std::fs::write(ws.join("sources/ownline.md"), ownline_v1).expect("write ownline");
    std::fs::write(ws.join("sources/anchors.md"), anchors_only).expect("write anchors");

    // A THIRD pin that is honest, minted over real content. It is the
    // non-vacuity control: it proves this corpus can still render green, so the
    // two reds below are the empty spans and not a blanket verdict on the walk.
    let honest = "# Target\n\nreal body\n";
    std::fs::write(ws.join("sources/honest.md"), honest).expect("write honest");

    let forged = empty_span_fingerprint();
    let claim = format!(
        "# Claim\n\ndraws from three places\n\n{}\n",
        lock_block(&[
            ("sources/ownline.md#^guideline", &forged),
            ("sources/anchors.md", &forged),
            ("sources/honest.md", &live_fingerprint(honest)),
        ])
    );
    std::fs::write(ws.join("claim.md"), &claim).expect("write claim");

    for state in ["before any edit", "after the targets change"] {
        let walk = sb.run(&ws, &["walk", "claim.md"]);
        let listing = stdout(&walk);
        let rows = walk_rows(&listing);
        assert_eq!(rows.len(), 3, "{state}: three pins, three rows: {listing}");

        let empty_rows: Vec<&String> = rows
            .iter()
            .filter(|r| r.contains("ownline.md") || r.contains("anchors.md"))
            .collect();
        assert_eq!(empty_rows.len(), 2, "{state}: {listing}");
        for row in &empty_rows {
            assert!(
                !row.contains("green"),
                "{state}: an empty-span pin rendered GREEN — the false green is back: {row}"
            );
            assert!(
                row.contains("red content-drifted"),
                "{state}: an empty-span pin must be RED, not merely non-green: {row}"
            );
        }
        assert!(
            rows.iter()
                .any(|r| r.contains("honest.md") && r.contains("  green  ")),
            "{state}: the honest pin still greens — the corpus is not uniformly red: {listing}"
        );
        assert_eq!(
            code(&walk),
            1,
            "{state}: red edges are the finding leg of the exit triad: {}",
            stderr(&walk)
        );

        // The other plane, same corpus, same question.
        let status = stdout(&sb.run(&ws, &["status"]));
        assert!(
            status.contains("lock red content-drifted [3 pins]"),
            "{state}: status rolls the same corpus up red: {status}"
        );

        // Rewrite both targets end to end. On the base build this changed
        // nothing — the pins stayed green. The second pass proves the verdict
        // does not depend on the target's bytes at all, because there are no
        // bytes it covers.
        std::fs::write(
            ws.join("sources/ownline.md"),
            "# H\n\n^guideline\n\nTOTALLY DIFFERENT BODY\n",
        )
        .expect("rewrite ownline");
        std::fs::write(ws.join("sources/anchors.md"), "^a\n^b\n").expect("rewrite anchors");
    }
}
