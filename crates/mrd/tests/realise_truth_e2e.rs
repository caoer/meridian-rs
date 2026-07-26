//! U3.5b `--truth` plumbing gate — `mrd realise --truth index|file` drives
//! `policy::converge` (@d107128) over a real file↔index convention divergence, in
//! BOTH directions, through the shipped binary. The load-bearing symmetry: one
//! divergence, two directions, two observably different converged INDEX pins
//! (file-truth re-pins + writes the INDEX at the live rev; index-truth keeps the
//! attested rev and reports the file-side restore).

use std::path::Path;
use std::process::Command;

use policy::{
    CheckLimits, ConventionFiles, Enforcement, arm, armed_from_index, generate_index, sweep,
};

/// The convention slug the fixture arms.
const SLUG: &str = "reviewer-not-owner";

/// A loadable `CHECK.md` body, varied by `marker` so two versions hash differently.
fn check_md(marker: &str) -> String {
    format!(
        "---\npaths:\n  - tasks/**\n---\n\n# {SLUG} {marker}\n\n\
         ```starlark\ndef check_change(change):\n    pass\n```\n"
    )
}

/// A one-file convention accessor (`CHECK.md` → body) for `policy::sweep`.
struct MemConv(String);
impl ConventionFiles for MemConv {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        if rel == "CHECK.md" {
            Ok(self.0.clone())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, rel))
        }
    }
    fn exists(&self, rel: &str) -> bool {
        rel == "CHECK.md"
    }
}

/// The attested INDEX arming `SLUG` at Block, pinned to `check`'s live rev.
fn armed_index(check: &str) -> String {
    let swept = sweep(&MemConv(check.to_string()), SLUG, CheckLimits::default()).expect("sweeps");
    let rev = swept.rev().to_string();
    let armed = arm(swept, &rev, Enforcement::Block).expect("arms at live rev");
    generate_index(&[armed])
}

/// The pinned rev the INDEX carries for `SLUG`.
fn pinned_rev(index: &str) -> String {
    armed_from_index(index)
        .into_iter()
        .find(|r| r.slug == SLUG)
        .expect("slug armed")
        .armed_rev
}

fn mrd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn truth_file_and_index_resolve_the_same_divergence_differently() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Arm at the ORIGINAL law; the INDEX pins v1's rev.
    let original = check_md("v1");
    let index_v1 = armed_index(&original);
    let attested = pinned_rev(&index_v1);

    // The live CHECK.md DRIFTS to v2 (an out-of-band law edit). The INDEX still
    // pins v1 — a file↔index divergence.
    let drifted = check_md("v2-edited");
    write(root, ".meridian.toml", "");
    write(root, "conventions/INDEX.md", &index_v1);
    write(root, "conventions/reviewer-not-owner/CHECK.md", &drifted);

    // ── file-truth: deploy the edited law — re-pin + WRITE the INDEX at v2's rev.
    let out = mrd()
        .args(["realise", "--truth", "file", "--json"])
        .current_dir(root)
        .output()
        .expect("run mrd realise --truth file");
    assert!(out.status.success(), "file-truth exits 0: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(v["realise_truth"]["truth"], "file");
    assert_eq!(
        v["realise_truth"]["index_written"], true,
        "file-truth deploys (writes) the INDEX: {stdout}"
    );
    let after_file = std::fs::read_to_string(root.join("conventions/INDEX.md")).unwrap();
    let file_rev = pinned_rev(&after_file);
    assert_ne!(
        file_rev, attested,
        "file-truth re-pins at the LIVE (drifted) rev"
    );

    // ── restore the INDEX to v1, then index-truth: keep the attested rev, report
    //    the file-side restore, write NOTHING.
    write(root, "conventions/INDEX.md", &index_v1);
    let out = mrd()
        .args(["realise", "--truth", "index", "--json"])
        .current_dir(root)
        .output()
        .expect("run mrd realise --truth index");
    assert!(out.status.success(), "index-truth exits 0: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(v["realise_truth"]["truth"], "index");
    assert_eq!(
        v["realise_truth"]["index_written"], false,
        "index-truth keeps the attested INDEX — writes nothing: {stdout}"
    );
    let actions = v["realise_truth"]["file_actions"]
        .as_array()
        .expect("actions");
    assert_eq!(actions.len(), 1, "one restore declared: {stdout}");
    assert_eq!(actions[0]["action"], "restore_file");
    assert_eq!(actions[0]["slug"], SLUG);
    assert_eq!(actions[0]["to_rev"], attested);
    // index-truth left the INDEX byte-identical (still pinning v1).
    let after_index = std::fs::read_to_string(root.join("conventions/INDEX.md")).unwrap();
    assert_eq!(
        pinned_rev(&after_index),
        attested,
        "index-truth kept the attested rev"
    );

    // The load-bearing symmetry: the two directions resolve the SAME divergence to
    // two observably different INDEX pins.
    assert_ne!(
        file_rev,
        pinned_rev(&after_index),
        "file-truth and index-truth diverge on the same divergence"
    );
}

/// U31: **the armed policy INDEX no longer lands through a bare
/// `std::fs::write`.** `mrd realise --truth file` deploys
/// `conventions/INDEX.md` — the file `mrd check` reads its rules from — and did
/// so with no candidate and not even this engine's atomic write discipline. It
/// now rides `fs::replace_file`, which DEMANDS a `model::CandidateDocument`.
///
/// # The state change asserted, not the exit code (R40)
/// An exit code reports that the command ran. Three disk facts report what it
/// DID, and each of them separates `fs::replace_file` from `std::fs::write`:
///
/// 1. **The inode changes.** `replace_file` stages a temp beside the
///    destination and renames it over — a new inode. `std::fs::write` truncates
///    and rewrites the SAME inode. This assertion reddens against any pre-U31
///    binary, which is what makes it evidence.
/// 2. **The landed rev is the deployed law's rev**, checked against an
///    independently computed INDEX (this test's own `armed_index`, not the
///    engine's return value).
/// 3. **No staged temp survives** — the rename committed, nothing leaked into
///    the policy folder.
#[test]
fn truth_file_lands_the_index_through_the_atomic_candidate_write() {
    use std::os::unix::fs::MetadataExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let original = check_md("v1");
    let drifted = check_md("v2-edited");
    write(root, ".meridian.toml", "");
    write(root, "conventions/INDEX.md", &armed_index(&original));
    write(root, "conventions/reviewer-not-owner/CHECK.md", &drifted);

    let index_path = root.join("conventions/INDEX.md");
    let before_bytes = std::fs::read_to_string(&index_path).unwrap();
    let before_ino = std::fs::metadata(&index_path).unwrap().ino();
    let file_rev = |raw: &str| {
        model::build(raw.to_owned(), syntax::parse(raw))
            .root
            .node_rev
            .0
    };
    let before_rev = file_rev(&before_bytes);

    let out = mrd()
        .args(["realise", "--truth", "file"])
        .current_dir(root)
        .output()
        .expect("run mrd realise --truth file");
    assert!(out.status.success(), "the deploy runs: {out:?}");

    let after_bytes = std::fs::read_to_string(&index_path).unwrap();
    let after_ino = std::fs::metadata(&index_path).unwrap().ino();

    // (1) THE ATOMIC-RENAME FACT: a bare `std::fs::write` rewrites the same
    // inode in place. A tmp+fsync+rename installs a new one.
    assert_ne!(
        before_ino, after_ino,
        "the INDEX was replaced by an atomic rename, not written in place \
         (inode {before_ino} → {after_ino})"
    );

    // (2) THE REV MOVED, to the rev of the law this deploy was supposed to pin —
    // computed here, independently of anything the command printed.
    let after_rev = file_rev(&after_bytes);
    assert_ne!(before_rev, after_rev, "the INDEX's file rev moved");
    assert_eq!(
        after_rev,
        file_rev(&armed_index(&drifted)),
        "the landed INDEX is the edited law re-pinned at its live rev"
    );

    // (3) The rename committed: no staged temp leaked into the policy folder.
    let leftovers: Vec<String> = std::fs::read_dir(root.join("conventions"))
        .unwrap()
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            name.contains(".tmp").then_some(name)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "no staged temp survives a committed deploy: {leftovers:?}"
    );
}
