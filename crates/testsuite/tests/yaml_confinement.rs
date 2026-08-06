//! The `serde_yaml` confinement instrument.
//!
//! `crates/policy/Cargo.toml` and `crates/config/Cargo.toml` both state one topology
//! claim: `serde_yaml` is taken as a direct production dependency by exactly two crates.
//!
//! The taker set is derived from `cargo metadata --no-deps`, never hand-maintained.
//! `--no-deps` reports each workspace member's own manifest edges, so the
//! direct/transitive distinction is structural: `wire-serve` reaches `serde_yaml`
//! through `policy` and is absent from this data. `kind` carries the same split for
//! dev and build edges: `null` is a production edge, `"dev"` and `"build"` are not.

use serde_json::Value;
use std::collections::BTreeSet;

/// The permitted takers. `policy` is the compile decision; `config` is a
/// stated deviation — user frontmatter is arbitrary YAML and schema §4
/// requires the parser's own message, so a hand-rolled scanner cannot serve
/// it.
const PERMITTED: [&str; 2] = ["config", "policy"];

/// Workspace metadata, read live. Fails loud if cargo cannot be run: an
/// instrument that skips itself when its input is missing is silent in the
/// same way a green tree is.
fn workspace_metadata() -> Value {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let out = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .unwrap_or_else(|e| panic!("run `cargo metadata`: {e}"));
    assert!(
        out.status.success(),
        "`cargo metadata` failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("parse `cargo metadata` output")
}

fn packages(meta: &Value) -> &Vec<Value> {
    meta["packages"].as_array().expect("packages array")
}

/// `null` kind is the production edge; cargo spells the others `"dev"` and
/// `"build"`.
fn edge_kind(dep: &Value) -> &str {
    dep["kind"].as_str().unwrap_or("normal")
}

/// Every workspace member declaring `dep` in `[dependencies]` — the direct
/// production takers, and nothing else.
fn direct_production_takers(meta: &Value, dep: &str) -> BTreeSet<String> {
    let mut takers = BTreeSet::new();
    for pkg in packages(meta) {
        let name = pkg["name"].as_str().expect("package name");
        for edge in pkg["dependencies"].as_array().expect("dependencies array") {
            if edge["name"].as_str() == Some(dep) && edge_kind(edge) == "normal" {
                takers.insert(name.to_string());
            }
        }
    }
    takers
}

/// The kinds under which `pkg` declares `dep`, in manifest order. Used to prove
/// a control edge is still the shape the control assumes.
fn declared_kinds(meta: &Value, pkg: &str, dep: &str) -> Vec<String> {
    packages(meta)
        .iter()
        .filter(|p| p["name"].as_str() == Some(pkg))
        .flat_map(|p| p["dependencies"].as_array().expect("dependencies array"))
        .filter(|e| e["name"].as_str() == Some(dep))
        .map(|e| edge_kind(e).to_string())
        .collect()
}

/// The assertion: exactly `{config, policy}` take `serde_yaml` as a direct
/// production dependency. Equality, not containment — a third crate taking it
/// reddens and is named, and a permitted crate dropping it reddens too.
#[test]
fn serde_yaml_direct_production_takers_are_exactly_the_permitted_crates() {
    let meta = workspace_metadata();
    let takers = direct_production_takers(&meta, "serde_yaml");
    let permitted: BTreeSet<String> = PERMITTED.iter().map(ToString::to_string).collect();

    let unpermitted: Vec<&String> = takers.difference(&permitted).collect();
    assert!(
        unpermitted.is_empty(),
        "serde_yaml confinement broken: {unpermitted:?} take `serde_yaml` as a direct production \
         dependency, but the workspace's YAML decision permits only {PERMITTED:?}. Either the new \
         crate reuses `policy`/`config`, or the claim in crates/policy/Cargo.toml and \
         crates/config/Cargo.toml is amended together with this test."
    );
    let missing: Vec<&String> = permitted.difference(&takers).collect();
    assert!(
        missing.is_empty(),
        "the confinement names {missing:?} as a permitted YAML taker, but it no longer takes \
         `serde_yaml` — strike it from the claim rather than leaving a permission nobody uses"
    );
}

/// A crate reaching `serde_yaml` through another crate is not a taker:
/// `wire-serve` depends on `policy` and so appears under `serde_yaml` in
/// `cargo tree -i`. The control is kept live: the transitive path is asserted
/// to still exist.
#[test]
fn transitive_dependents_are_not_reported_as_takers() {
    let meta = workspace_metadata();
    let takers = direct_production_takers(&meta, "serde_yaml");

    // `contains`, not equality: a crate may declare the same dependency twice
    // (`mrd` takes `policy` under both `[dependencies]` and
    // `[dev-dependencies]`); what the control needs is that a production edge
    // is among them.
    for (crate_name, through) in [("wire-serve", "policy"), ("mrd", "policy")] {
        assert!(
            declared_kinds(&meta, crate_name, through)
                .iter()
                .any(|k| k == "normal"),
            "control edge `{crate_name} -> {through}` must still be a production edge, or the \
             transitive case below proves nothing"
        );
        assert!(
            declared_kinds(&meta, crate_name, "serde_yaml").is_empty(),
            "fixture drift: `{crate_name}` now declares serde_yaml itself, so it is no longer a \
             purely transitive case"
        );
        assert!(
            !takers.contains(crate_name),
            "`{crate_name}` reaches serde_yaml only through `{through}` and must NOT be reported \
             as a direct taker — a false positive here is what gets this instrument deleted"
        );
    }
}

/// `[dev-dependencies]` must not count, or the claim breaks the first time
/// someone writes a YAML fixture test. Controlled on a real dev-only edge in
/// this workspace (`config -> tempfile`), asserted to still BE dev-only, so the
/// control cannot go vacuous without saying so.
#[test]
fn dev_dependencies_do_not_count_as_takers() {
    let meta = workspace_metadata();

    assert_eq!(
        declared_kinds(&meta, "config", "tempfile"),
        vec!["dev"],
        "control edge `config -> tempfile` must still be dev-only, or this test proves nothing \
         about the kind filter"
    );
    assert!(
        !direct_production_takers(&meta, "tempfile").contains("config"),
        "the kind filter admitted a [dev-dependencies] edge"
    );
}
