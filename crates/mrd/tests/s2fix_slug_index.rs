//! R44 P0-2 (R45 shape B) — a convention SLUG cannot carry a forged `@fp` claim
//! into the ARMED INDEX, the enforcement substrate itself.
//!
//! The door: `policy::converge` under `Truth::File` re-renders every armed row
//! through `generate_index`, which interpolates the slug UNESCAPED **twice** (the
//! row label, and the wikilink to its `CHECK.md` — which also nests a wikilink),
//! and `mrd realise --truth file` writes those bytes with a bare `std::fs::write`
//! (`realise_cmd.rs:253`, the only production writer of `conventions/INDEX.md`).
//!
//! R45 ruled the INTAKE shape: the slug takes the one identifier charset at
//! `policy::validate_slug`, so a hostile folder never sweeps, never arms, and
//! never renders — the same owner that closes the journal door, and every
//! renderer anyone adds later.
//!
//! THE ASSERT IS THE ARTIFACT (R26): `syntax::fp_removals` over the INDEX bytes
//! ON DISK, after the SHIPPED BINARY wrote them. R44 scopes this unit to the
//! forgery; the same legal path's missing flock / CAS / journal row is a ruled
//! stage-3 rider and is deliberately NOT asserted here.
//!
//! Redden at `c72d144c` (pre-fix), quoted in the unit card.

use std::path::Path;
use std::process::Command;

use policy::{
    CheckLimits, ConventionFiles, Enforcement, arm, armed_from_index, evidence_rev, generate_index,
    sweep,
};

/// An `@fp` claim token in a claim-link position, spelled as a directory name —
/// admissible to the path-traversal-only `validate_slug` at `c72d144c`.
const TOKEN_SLUG: &str = "[[guide#^goal@green.b3af12cd|G]]";

/// The placeholder the hostile row is rendered through, so the fixture reads the
/// exact row grammar the engine emits rather than a hand-typed imitation.
const TOKEN_PLACEHOLDER: &str = "harness-token-slug";

/// The in-charset convention that must SURVIVE the convergence — the control that
/// keeps "no token" from being "no conventions".
const LIVE_SLUG: &str = "harness-live-law";

fn check_md(marker: &str) -> String {
    format!(
        "---\npaths:\n  - tasks/**\n---\n\n# harness convention {marker}\n\n\
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

/// One armed row at `check`'s live rev.
fn armed_row(slug: &str, check: &str) -> policy::IndexEntry {
    let swept = sweep(&MemConv(check.to_string()), slug, CheckLimits::default()).expect("sweeps");
    let rev = swept.rev().to_string();
    arm(swept, &rev, Enforcement::Block).expect("arms at the live rev")
}

fn pinned_rev(index: &str, slug: &str) -> Option<String> {
    armed_from_index(index)
        .into_iter()
        .find(|r| r.slug == slug)
        .map(|r| r.armed_rev)
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, body).expect("write");
}

/// **THE GATE.** The reviewer's own reproduction: `converge` under `Truth::File`,
/// driven through the shipped binary, over an INDEX arming a convention whose SLUG
/// is a claim token. The INDEX bytes on disk carry no `@fp` claim afterwards.
#[test]
fn a_convention_slug_lands_no_claim_token_in_the_armed_index() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    // The attested INDEX arms two conventions at v1: one in-charset (the control),
    // one whose folder name is a claim token. Rendered by the ENGINE, then re-keyed
    // — so the hostile row is byte-exactly what `generate_index` emits.
    let v1 = check_md("v1");
    let index_v1 = generate_index(&[armed_row(LIVE_SLUG, &v1), armed_row(TOKEN_PLACEHOLDER, &v1)])
        .replace(TOKEN_PLACEHOLDER, TOKEN_SLUG);
    let attested = pinned_rev(&index_v1, LIVE_SLUG).expect("the control row is armed");

    // The live law DRIFTS to v2 — so file-truth has a real re-pin to deploy and the
    // write is a genuine convergence, never a byte-identical no-op.
    let v2 = check_md("v2-edited");
    write(root, ".meridian.toml", "");
    write(root, "conventions/INDEX.md", &index_v1);
    write(root, &format!("conventions/{LIVE_SLUG}/CHECK.md"), &v2);
    write(root, &format!("conventions/{TOKEN_SLUG}/CHECK.md"), &v1);

    let out = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(["realise", "--truth", "file", "--json"])
        .current_dir(root)
        .output()
        .expect("run mrd realise --truth file");
    assert!(out.status.success(), "file-truth exits 0: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(
        v["realise_truth"]["index_written"], true,
        "file-truth deploys (writes) the INDEX — or nothing was measured: {stdout}"
    );

    let after = std::fs::read_to_string(root.join("conventions/INDEX.md")).expect("the INDEX");

    // THE ASSERT — the artifact, first, so a regression quotes the ranges.
    assert!(
        syntax::fp_removals(&after).is_empty(),
        "an `@fp` claim token stands in a claim-link position in the ARMED INDEX — \
         the enforcement substrate itself.\nfp_removals = {:?}\nINDEX:\n{after}",
        syntax::fp_removals(&after)
    );

    // CONTROL — the in-charset convention SURVIVED and was re-pinned at its live
    // (drifted) rev. Without this, an INDEX that lost every row would pass the
    // assertion above while silently disarming the workspace.
    assert_eq!(
        pinned_rev(&after, LIVE_SLUG).as_deref(),
        Some(evidence_rev(&v2).as_str()),
        "the in-charset law stays armed and re-pins at the live rev:\n{after}"
    );
    assert_ne!(
        pinned_rev(&after, LIVE_SLUG).expect("armed"),
        attested,
        "the convergence really moved the pin (a no-op write proves nothing)"
    );

    // And the hostile folder is gone from the substrate entirely — not escaped
    // into some other shape, and not left addressing a convention.
    assert!(
        !after.contains("guide#^goal"),
        "the out-of-charset row is dropped, not re-rendered:\n{after}"
    );
}
