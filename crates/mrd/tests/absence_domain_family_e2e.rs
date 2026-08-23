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
//!
//! # The family has TWO axes, and this file drives both
//!
//! **CELL axis** — the table above, six worlds through `check --json`.
//! **FACE axis** — [`face_check_json_names_the_measured_absence`],
//! [`face_walk_names_the_measured_absence`],
//! [`face_status_rolls_the_absence_up_under_its_own_word`] and
//! [`face_commit_gate_refuses_over_the_measured_absence`], sharing the
//! [`absent_cell`] fixture.
//!
//! ⛔ **ONE TEST PER FACE, and that is a correction rather than a style.** The
//! first cut asserted all four faces inside ONE test, and the r15 alarm
//! demonstration exposed it: under a mutated engine the test refused at its
//! FIRST assertion — `check --json` — so `walk`, `status` and the commit gate
//! NEVER SPOKE. **The three assertions that make it a face axis were
//! undemonstrated, and a regression in `status` alone would have been masked by
//! `check` failing first.** That is the same aggregate-masking-a-member defect
//! (charter 02 r9) the one-pin workspace exists to avoid, committed one level up
//! in the fixture that invokes it. Split, each face refuses on its own line and
//! names itself.
//!
//! ⭐ **Why the face axis rides ONE cell rather than all six**, stated because
//! the alternative is a 24-assertion grid that gates nothing extra: the faces do
//! not each carry a verdict of their own. `edge_color` is the single site in the
//! tree that asks the existence question, and `check`, `walk`, `status`, `sql`
//! and `engine` all reach it through `mrd`'s `Mounts::rooted` — so **the faces
//! vary in RENDERING, never in the verdict, and one cell exercises the whole
//! sharing claim.** Six cells × four faces would re-measure one shared computer
//! twenty-four times.
//!
//! ⛔ **That argument is stated here as a PREMISE, and the face tests are what
//! make it FAIL LOUDLY when it stops being true.** A premise no assertion checks
//! is a line that prints.
//!
//! # The alarm demonstration lives in a file, and it expires when this does
//!
//! Charter 02 r15 binds a detector to be SEEN REFUSING before it is believed,
//! and broadcast 0057 sharpens it: **a demonstration is a PROPERTY, not an
//! event, and a detector edited after its demonstration is undemonstrated
//! again.** So the demonstration for these four faces is a re-runnable script,
//! not a transcript that died with its session:
//!
//! `<session>/results/p8-absence-95e58644/r15-face-axis-alarm.sh`
//!
//! It restores the pre-0054 arm order in `edge_color`, asserts its own mutation
//! target is present exactly once first, requires ALL FOUR faces to be named
//! FAILED independently, then restores byte-identically and requires green.
//! ⛔ **RE-RUN IT AFTER ANY EDIT TO `edge_color` OR TO THESE TESTS.**

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
mod common;

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

/// The fixture the FACE axis shares: ONE pin, ONE absent target, green-path
/// control run before the deletion.
///
/// **The FACE axis of the same family** — the test above drives six CELLS
/// through ONE face, and a family has two axes.
///
/// ⛔ **Why this is an assertion and not a paragraph.** The reason one face
/// *could* speak for the others is a source fact: `edge_color` is the single
/// site in the tree that asks the existence question, and `check`, `walk`,
/// `status`, `sql` and `engine` all reach it through `mrd`'s `Mounts::rooted`.
/// **That fact is true today and nothing executes it.** A later change routing
/// one face around `edge_color` would leave the argument still readable and no
/// longer true, and the gate would not notice.
///
/// ⭐ **And this family in particular has already done exactly that**: pass 7
/// measured FOUR FACES DISAGREEING ON ONE PIN AT ONE ENGINE — `repair` and
/// `walk --down` green while `walk up` and `check` reddened. The shared-call-site
/// argument is a claim about mechanism, and a mechanism claim is the thing that
/// failed here before. **A family whose historical failure mode is inter-face
/// disagreement cannot be gated on the assurance that the faces agree.**
///
/// ⛔ **A SEPARATE MINIMAL WORKSPACE — ONE pin, ONE absent target, no other
/// rows — and the reason is a fixture law, not tidiness.** Run on the six-cell
/// corpus above, `status`'s WORST-OF rollup (R26) has FIVE reds to choose from,
/// so an assertion about the word it prints would depend on which member won the
/// roll-up rather than on the verdict under test. **That is a family fixture's
/// aggregate MASKING A MEMBER (charter 02 r9)** — the aggregate would still be
/// red, and this fix could regress with the suite green. One pin makes every
/// face deterministic and leaves `status` exactly one verdict it can name.
///
/// The GREEN-PATH CONTROL below runs BEFORE the deletion, on the faces
/// themselves: `check` exits 0 and `walk` says nothing about absence while the
/// target is PRESENT. **A face that only ever prints one answer proves nothing
/// by printing it** (charter 02 r15, applied to the faces rather than the cells).
#[must_use]
fn absent_cell() -> (Sandbox, PathBuf) {
    let sb = sandbox();
    let ws = sb.tmp.path().join("one-absent-cell");
    std::fs::create_dir_all(&ws).expect("mkdir");
    git(&ws, &["init", "-q"]);
    git(
        &ws,
        &["config", "user.email", "absence-e2e@example.invalid"],
    );
    git(&ws, &["config", "user.name", "absence-e2e"]);
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", said(&init));

    write(&ws, "target.md", TARGET);
    write(&ws, "src.md", "# S\n\nthis page draws from its target.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "corpus"]);
    let pin = sb.run(&ws, &["pin", "src.md", "target.md#T/Body"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "pin"]);

    // GREEN-PATH CONTROL, BEFORE the deletion — every face is shown able to say
    // something OTHER than the verdict under test. A face that only ever prints
    // one answer proves nothing by printing it (charter 02 r15, aimed at the
    // faces rather than the cells).
    let clean_check = sb.run(&ws, &["check"]);
    assert_eq!(
        clean_check.status.code(),
        Some(0),
        "the governed corpus is accepted before the deletion: {}",
        said(&clean_check)
    );
    let clean_walk = sb.run(&ws, &["walk", "src.md"]);
    assert!(
        !said(&clean_walk).contains("file-not-found"),
        "walk says nothing about absence while the target is present: {}",
        said(&clean_walk)
    );

    std::fs::remove_file(ws.join("target.md")).expect("delete target");
    assert!(!ws.join("target.md").exists(), "target.md is off the disk");
    (sb, ws)
}

/// FACE 1 — `check --json`, the structured door.
#[test]
fn face_check_json_names_the_measured_absence() {
    let (sb, ws) = absent_cell();
    let out = sb.run(&ws, &["check", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .unwrap_or_else(|e| panic!("check --json parses: {e}\n{}", said(&out)));
    assert_eq!(
        pin_labels(&json).get("src.md").map(String::as_str),
        Some("red file-not-found (this workspace holds no 'target.md')"),
        "check --json: {}",
        said(&out)
    );
    assert_eq!(out.status.code(), Some(1), "check: {}", said(&out));
}

/// FACE 2 — `walk`, the per-pin human verb.
#[test]
fn face_walk_names_the_measured_absence() {
    let (sb, ws) = absent_cell();
    let out = sb.run(&ws, &["walk", "src.md"]);
    assert!(
        said(&out).contains("red file-not-found"),
        "WALK must name the same absence check does — the two per-pin faces \
         share one colour computer and this is the assertion that says so: {}",
        said(&out)
    );
    assert_eq!(out.status.code(), Some(1), "walk reddens: {}", said(&out));
}

/// FACE 3 — `status`, the WORST-OF rollup (R26 — no per-pin grain).
#[test]
fn face_status_rolls_the_absence_up_under_its_own_word() {
    let (sb, ws) = absent_cell();
    let out = sb.run(&ws, &["status"]);
    assert_eq!(out.status.code(), Some(0), "status: {}", said(&out));
    let lock_axis = String::from_utf8_lossy(&out.stdout)
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
        "status rolls the absence up as a red: {lock_axis:?} — {}",
        said(&out)
    );
    assert!(
        lock_axis.contains("file-not-found"),
        "and it names THIS reason, not a resolution red: {lock_axis:?} — {}",
        said(&out)
    );
}

/// FACE 4 — the commit gate, the only face whose verdict has CONSEQUENCES.
#[test]
fn face_commit_gate_refuses_over_the_measured_absence() {
    let (sb, ws) = absent_cell();
    let out = sb.run(&ws, &["check", "--commit-gate"]);
    assert_ne!(
        out.status.code(),
        Some(0),
        "the commit gate refuses over a pinned target that is GONE — the face \
         that stops a caller rather than informing one: {}",
        said(&out)
    );
    assert!(
        said(&out).contains("file-not-found"),
        "and it refuses with the measured absence, not a resolution red: {}",
        said(&out)
    );
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
