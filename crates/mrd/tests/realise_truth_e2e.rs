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
