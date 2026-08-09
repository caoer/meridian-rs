//! Cross-surface gates for the color planes — one corpus read by both `mrd walk`
//! and `mrd status`, over the real binary (`CARGO_BIN_EXE_mrd`).
//!
//! Every gate is a plane-vs-plane agreement, not a rendering check: finding 6 (a
//! red pin deduped away behind a green one on the same ref), finding 17 (one pin
//! projected as two rows with two verdicts), finding 26 (a malformed `objects:`
//! value dropped before the vibe-debt gauge counted it, reading as a true zero).

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

    /// A workspace declared a root by `mrd init`; the verbs resolve it as the
    /// cwd default, which is enough for every plane under test here.
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

/// A well-formed `fp1.…` token that is NOT the live one — a pin that measures drift. Minted
/// over different bytes, so it is a real token with a real digest, never a syntactic fake the
/// reader could reject as malformed instead.
fn drifted_fingerprint() -> String {
    live_fingerprint("# Target\n\nbytes that are not on disk\n")
}

/// The canonical `meridian-lock` fence for `pins`, written by hand so the gate depends on the
/// CLI's own reader, never on the writer that produced the bytes.
fn lock_block(pins: &[(&str, &str)]) -> String {
    lock_block_with_hashes(
        &pins
            .iter()
            .map(|(r, f)| (*r, "9ae3f1deadbeef", *f))
            .collect::<Vec<_>>(),
    )
}

/// [`lock_block`] with each pin's blob `hash` spelled out — for the gates that measure the
/// retrieval plane (R4 moved it onto the pin row). `version: 2` is the lock-file schema
/// version, not the wire protocol version.
fn lock_block_with_hashes(pins: &[(&str, &str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("```meridian-lock\nversion: 2\npins:\n");
    for (declared_ref, hash, fingerprint) in pins {
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
            "  - object: \"[[{object}]]\"\n    hash: \"{hash}\"\n    path: [{path}]\n    \
             fingerprint: \"{fingerprint}\""
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

/// F6 gate — two pins on one ref, one live and one drifted: `mrd walk` renders the red row
/// beside the green one and exits 1, and `mrd status` agrees on the same corpus.
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

// ── finding 17 — RETIRED WITH ITS SUBJECT ──────────────────────────────────

// ── finding 26 — a malformed `objects:` sha is unknown, never zero ──────────

/// F26 gate — an `objects:` value that is not an object id lands in the vibe-debt gauge's
/// `unknown` slot: git cannot be asked about it, so that entry's debt is unmeasurable, and
/// dropping it would read a corrupt retrieval plane as a true zero.
#[test]
fn a_malformed_objects_sha_is_counted_unknown_not_dropped_to_zero() {
    let sb = sandbox();
    let ws = sb.workspace("badsha");
    std::fs::write(
        ws.join("effect.md"),
        format!(
            "# Effect\n\n{}\n",
            lock_block_with_hashes(&[(
                "payload",
                "not-a-sha",
                &format!("fp1.span2.b3.{}", "0".repeat(64))
            )])
        ),
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

/// The token a hand-authored lock carries for an empty-normalizing span: `blake3` of NO bytes.
/// Not a syntactic fake — a real, well-formed `fp1.…` token with a real digest, which is
/// exactly what made it dangerous.
fn empty_span_fingerprint() -> String {
    format!("fp1.span2.b3.{}", blake3::hash(b"").to_hex())
}

/// R31 gate — a stored empty-span pin can never read green.
///
/// The empty-span class is unreachable through `mrd pin` (every ref form refuses at mint),
/// so it reaches the product exactly one way: a hand- or tool-authored `meridian-lock`
/// block — which is why this gate drives the real binary and hand-writes its lock.
///
/// The assert is a refusal of the green, not a colour preference: neither row may say
/// `green`, on either plane, in either state. Grey would still ship a pin that cannot
/// drift, so the row must be red and the walk must exit 1.
#[test]
fn a_hand_authored_empty_span_pin_never_reads_green() {
    let sb = sandbox();
    let ws = sb.workspace("emptyspan");
    std::fs::create_dir_all(ws.join("sources")).expect("sources dir");

    // Two of the enumerated empty-normalizing forms, at the product surface: an own-line
    // `^anchor` (the Block form) and a whole-page ref over a file that is nothing but an
    // own-line anchor (the Page form).
    let ownline_v1 = "# H\n\n^guideline\n\noriginal body\n";
    let anchors_only = "^a\n";
    std::fs::write(ws.join("sources/ownline.md"), ownline_v1).expect("write ownline");
    std::fs::write(ws.join("sources/anchors.md"), anchors_only).expect("write anchors");

    // A THIRD pin that is honest, minted over real content. It is the non-vacuity control: it
    // proves this corpus can still render green, so the two reds below are the empty spans and not
    // a blanket verdict on the walk.
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

        // Rewrite both targets end to end: the second pass proves the verdict does not depend
        // on the target's bytes at all, because there are no bytes it covers.
        std::fs::write(
            ws.join("sources/ownline.md"),
            "# H\n\n^guideline\n\nTOTALLY DIFFERENT BODY\n",
        )
        .expect("rewrite ownline");
        std::fs::write(ws.join("sources/anchors.md"), "^a\n^b\n").expect("rewrite anchors");
    }
}
