//! C2 crate-topology gate — view-never-store as a build gate (design §Crate
//! topology). `crates/view` is a **write-only leaf**: it reads the parsed corpus
//! and writes `DuckDB`, and **no correctness crate may depend on it**, so a DB row
//! can never re-enter a fact/model correctness path (`use view::` is impossible
//! from a fact-answering crate). Mirrors `perfsuite/tests/inventory_assert.rs`:
//! a structural unit test parsing each crate's `Cargo.toml`.

use std::path::PathBuf;

/// The correctness crates that MUST NEVER list `view` as a dependency (design
/// §Crate topology: "the forbidden Cargo directions, the enforced line").
const FORBIDDEN_DEPENDERS: &[&str] = &[
    "model",
    "query",
    "fs",
    "syntax",
    "sidecar",
    "wire",
    "wire-serve",
];

/// Dependency tables a forbidden edge could hide in.
const DEP_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

fn crate_manifest(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name)
        .join("Cargo.toml")
}

#[test]
fn view_is_a_write_only_leaf_no_correctness_crate_depends_on_it() {
    for crate_name in FORBIDDEN_DEPENDERS {
        let manifest = crate_manifest(crate_name);
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        let toml: toml::Value =
            toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", manifest.display()));

        for table in DEP_TABLES {
            if let Some(deps) = toml.get(table).and_then(toml::Value::as_table) {
                assert!(
                    !deps.contains_key("view"),
                    "C2 VIOLATION: correctness crate `{crate_name}` lists `view` in [{table}] \
                     — the DB sink cannot leak inward (view-never-store). Offender: {}",
                    manifest.display(),
                );
            }
        }
    }
}
