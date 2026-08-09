//! End-to-end gate for §12.1's absence law AS A FAMILY (session decisions 0049
//! and 0054), driving the real binary over its process boundary.
//!
//! **The family, not the motivating member.** The defect that raised 0054 was
//! measured on ONE cell — an in-domain pinned target, deleted, reading
//! `selector-unresolved`. A fixture asserting that cell alone would pass on a
//! fix that reddened everything, and would be silent about the pairing that
//! carries the whole finding. So this file drives BOTH PLANES × BOTH EXCLUSION
//! CLASSES on ONE workspace in ONE run:
//!
//! | cell | placement | domain | disk | verdict |
//! |---|---|---|---|---|
//! | `src-in` | plain path | IN | deleted | `red file-not-found` |
//! | `src-vin` | `vendor-in/` , no rule | IN | deleted | `red file-not-found` |
//! | `src-vout` | `vendor-out/`, custom rule | OUT | deleted | `red file-not-found` |
//! | `src-dot` | `.hidden/`, dot-segment floor | OUT | deleted | `red file-not-found` |
//! | `src-present` | `vendor-out/`, custom rule | OUT | PRESENT | `grey outside-hash-domain` |
//! | `src-moved` | plain path | IN | PRESENT | `red selector-unresolved` |
//!
//! **`src-vin` and `src-vout` are the control pair and they must stay paired.**
//! Same directory shape, same file name, same bytes, same deletion, same run —
//! the ONLY variable is whether `meridian/domain.md` carries an ignore rule that
//! matches. Before this fix the VERDICT WORD FLIPPED across that pair, which is
//! the whole finding; the law says one physical fact gets one word.
//!
//! **`src-present` and `src-moved` are the no-fire controls**, one per direction
//! a wrong fix could fail in: reddening every target missing from the corpus map
//! breaks `src-present` (0049's surviving grey), and retiring
//! `selector-unresolved` rather than scoping it breaks `src-moved` (the word is
//! TRUE there — the page resolved and the selector failed).
//!
//! The two exclusion classes are one value (`contains == false`) by the time the
//! colour plane sees them, so they are separable only through the REAL
//! `fs::Domain` over a REAL disk — which is why this gate is end-to-end and not
//! a unit test beside `view::walk::edge_color`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The binary every drive here goes through. `MRD_BIN` names another artifact —
/// the fixv convention (`crates/mrd/tests/s2fix_cross_surface.rs`), reused here
/// so the SAME asserts can run against a pre-change build.
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

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn write(ws: &Path, rel: &str, body: &str) {
    let path = ws.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir -p");
    std::fs::write(path, body).expect("write fixture");
}

/// A target page with one pinnable section.
const TARGET: &str = "# T\n\n## Body\n\nthe pinned body\n";

/// Every pin row `check --json` reported, keyed by the page that declares it,
/// with the ONE rendered colour label the single pin computer gave it. Red,
/// grey and `unattested` are merged deliberately: which BUCKET a row lands in is
/// downstream of its colour, and a fixture that read only one bucket would call
/// a row's disappearance a pass.
fn pin_labels(json: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    let pins = &json["pins"];
    let mut out = std::collections::BTreeMap::new();
    for bucket in ["red", "grey", "unattested"] {
        for row in pins[bucket].as_array().unwrap_or(&Vec::new()) {
            let src = row["src_path"].as_str().expect("src_path").to_string();
            let label = row["color"].as_str().expect("color").to_string();
            assert!(
                out.insert(src.clone(), label).is_none(),
                "one row per declaring page in this fixture, so a duplicate means \
                 the fixture stopped isolating its cells: {src}"
            );
        }
    }
    out
}

/// The reason WORD alone — `view::walk::color_label` renders
/// `<tone> <reason> (<detail>)` and the detail is asserted separately, so a
/// wording change to the sentence can never quietly relax a word assertion.
fn word_of(label: &str) -> &str {
    label.split_once(" (").map_or(label, |(word, _)| word)
}

/// §12.1 + decisions 0049/0054: an absent page is `file-not-found` WHEREVER it
/// is absent, and every verdict over a page that is PRESENT is unchanged.
#[test]
fn an_absent_target_reds_file_not_found_on_both_planes_and_both_exclusion_classes() {
    let sb = sandbox();
    let ws = sb.tmp.path().join("absence-family");
    std::fs::create_dir_all(&ws).expect("mkdir");
    git(&ws, &["init", "-q"]);
    git(
        &ws,
        &["config", "user.email", "absence-e2e@example.invalid"],
    );
    git(&ws, &["config", "user.name", "absence-e2e"]);
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", said(&init));

    // The custom ignore rule — the SECOND exclusion class. The dot-segment floor
    // is the first and needs no declaration; that asymmetry is why both are here.
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"vendor-out/**\"\n---\n\nVendored copies do not \
         move this workspace's fingerprint.\n",
    );

    // Six targets. `vendor-in/` and `vendor-out/` differ ONLY by the rule above.
    write(&ws, "in-gone.md", TARGET);
    write(&ws, "vendor-in/target.md", TARGET);
    write(&ws, "vendor-out/target.md", TARGET);
    write(&ws, ".hidden/target.md", TARGET);
    write(&ws, "vendor-out/present.md", TARGET);
    write(&ws, "moved.md", TARGET);

    // Six declaring pages, one pin each, so every cell is its own row.
    let cells = [
        ("src-in.md", "in-gone.md"),
        ("src-vin.md", "vendor-in/target.md"),
        ("src-vout.md", "vendor-out/target.md"),
        ("src-dot.md", ".hidden/target.md"),
        ("src-present.md", "vendor-out/present.md"),
        ("src-moved.md", "moved.md"),
    ];
    for (src, _) in cells {
        write(&ws, src, "# S\n\nthis page draws from its target.\n");
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "corpus"]);

    for (src, target) in cells {
        let reference = format!("{target}#T/Body");
        let pin = sb.run(&ws, &["pin", src, &reference]);
        assert_eq!(
            pin.status.code(),
            Some(0),
            "§12.1: the hash domain gates HASHING, not addressing, so `pin` serves \
             an out-of-domain path by name. {src} → {reference}: {}",
            said(&pin)
        );
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "pins"]);

    // THE GREEN-PATH CONTROL, before any deletion: every verdict below is caused
    // by what this block changes and not by the corpus (S3-R8(c)).
    let clean = sb.run(&ws, &["check", "--json"]);
    let clean_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&clean.stdout))
            .expect("check --json parses on the clean corpus");
    let before = pin_labels(&clean_json);
    for (src, _) in cells {
        assert!(
            before
                .get(src)
                .is_none_or(|l| l.starts_with("grey outside-hash-domain")),
            "before the deletions the only non-green rows are the domain's own \
             exclusions — {src} reads {:?}",
            before.get(src)
        );
    }

    // The four deletions, and the state change is ASSERTED, never inferred from
    // a command's exit status (R40).
    for gone in [
        "in-gone.md",
        "vendor-in/target.md",
        "vendor-out/target.md",
        ".hidden/target.md",
    ] {
        std::fs::remove_file(ws.join(gone)).expect("delete target");
        assert!(!ws.join(gone).exists(), "{gone} is off the disk");
    }
    // And the two survivors are STILL THERE — the control's precondition, stated
    // rather than assumed.
    assert!(ws.join("vendor-out/present.md").exists());
    assert!(ws.join("moved.md").exists());
    // `moved.md` keeps its page and loses its heading: the one world where
    // `selector-unresolved` tells the truth.
    write(&ws, "moved.md", "# T\n\n## Renamed\n\nthe pinned body\n");

    let out = sb.run(&ws, &["check", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or_else(|e| panic!("check --json parses: {e}\n{}", said(&out)));
    let labels = pin_labels(&json);

    // ── The absent family: ONE word, both planes, both exclusion classes. ──
    for (src, target, world) in [
        ("src-in.md", "in-gone.md", "in domain, plain path"),
        (
            "src-vin.md",
            "vendor-in/target.md",
            "in domain, vendored placement, NO rule matches it",
        ),
        (
            "src-vout.md",
            "vendor-out/target.md",
            "out of domain by CUSTOM RULE",
        ),
        (
            "src-dot.md",
            ".hidden/target.md",
            "out of domain by the DOT-SEGMENT FLOOR",
        ),
    ] {
        let label = labels
            .get(src)
            .unwrap_or_else(|| panic!("{src} declares a pin and owes a row: {}", said(&out)));
        assert_eq!(
            word_of(label),
            "red file-not-found",
            "{target} is GONE and the caller is owed the absence, {world}. A \
             resolution red would assert a resolution that did not occur, and a \
             grey would say a file that is nowhere is merely unhashed. Full \
             report: {}",
            said(&out)
        );
        // The detail is the sentence the caller acts on, and it must NAME the
        // path: `file-not-found` with no path is the same collapse one word
        // later.
        assert_eq!(
            label,
            &format!("red file-not-found (this workspace holds no '{target}')"),
            "the absence names the path it measured: {}",
            said(&out)
        );
    }

    // ⭐ THE PAIR THAT CARRIES THE FINDING. Same shape, same bytes, same
    // deletion, same run — only the ignore rule differs.
    assert_eq!(
        word_of(&labels["src-vin.md"]),
        word_of(&labels["src-vout.md"]),
        "the ONLY variable across this pair is whether meridian/domain.md \
         excludes the path, and the verdict word MUST NOT flip on it: existence \
         is a fact about the disk and the disk does not know the domain"
    );

    // ── The present family: every verdict UNCHANGED. ──
    assert_eq!(
        labels.get("src-present.md").map(String::as_str),
        Some("grey outside-hash-domain ('vendor-out/present.md' is not in the hash domain)"),
        "0049's surviving state: out of domain and PRESENT is still grey, and a \
         fix that reddens everything absent from the corpus map dies here. \
         Report: {}",
        said(&out)
    );
    let moved = labels
        .get("src-moved.md")
        .unwrap_or_else(|| panic!("src-moved.md owes a row: {}", said(&out)));
    assert_eq!(
        word_of(moved),
        "red selector-unresolved",
        "and the word is SCOPED, not retired: over a page that EXISTS with its \
         heading moved, `selector-unresolved` is exactly true. Report: {}",
        said(&out)
    );

    // The exit code is the caller-visible half: reds are findings.
    assert_eq!(
        out.status.code(),
        Some(1),
        "a finding rides exit 1 — the triad stays CLOSED (S3-R6): {}",
        said(&out)
    );
}
