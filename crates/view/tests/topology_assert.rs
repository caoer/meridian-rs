//! C2 topology: view is a write-only leaf — no correctness crate may depend on
//! it (view-never-store).

use std::path::PathBuf;

/// Correctness crates that must never list `view` as a dependency.
const FORBIDDEN_DEPENDERS: &[&str] = &[
    "model",
    "query",
    "fs",
    "syntax",
    "sidecar",
    "wire",
    "wire-serve",
];

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
