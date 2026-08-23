//! The fixture rule, enforced (card `registry-tests-drain-residue`).
//!
//! Two halves, both of them the class-2 flake (pipelines 1098/1101 —
//! `registry: background drawer rebuild failed for /tmp/.tmp…/ws (No such
//! file or directory)`):
//!
//! 1. A fixture that starts a `registry::RunningServer` must raise
//!    `Config::drain_cold_builds`. The production default is 2 s
//!    (`registry::DEFAULT_DRAIN_COLD_BUILDS`) — a bound on the CLIENT's
//!    flock wait, not a budget for a loaded CI box, where a cold build parks
//!    far longer and shutdown gives up on it.
//! 2. A fixture STRUCT holding both the server and its `TempDir` must declare
//!    the **server first**. Struct fields drop in declaration order; locals
//!    drop in reverse. A `_tmp` declared first deletes the workspace under a
//!    live builder, and the drain in `RunningServer::stop` never runs at all.
//!
//! **Why source text and not a type.** Both halves sit one level below the
//! compiler's reach: an unset `pub` field with a legal default, and a field
//! ORDER. Nothing in the type system objects to either, and a fixture that
//! gets it wrong is green until the box is loaded — the shape that costs a
//! night. So this test reads the two integration-test trees as text.
//!
//! **Aperture** — what it does NOT see:
//! - Only `crates/registry/tests/` and `crates/mrd/tests/`. The in-crate
//!   `#[cfg(test)]` fixtures in `crates/registry/src/server.rs` are outside it
//!   (they carry the budget today, through their own `test_config`), as are
//!   the in-process registry fixtures that start no server at all
//!   (`event_feed.rs`, `sub_detect_quiet.rs`, `write_detector_window.rs`):
//!   having no wire path, they kick no background builder.
//! - Half 1 is FILE grain, which is where a fixture's config helper lives. A
//!   file with two helpers, only one of them raising the budget, passes.
//! - Half 2 reads only a struct whose header opens with a trailing `{` on its
//!   own line; a struct with an explicit `impl Drop` is exempt, since it
//!   orders its own teardown.

use std::fs;
use std::path::{Path, PathBuf};

/// The needle for half 1: a file containing it starts a daemon.
const START: &str = "RunningServer::start(";

/// The knob half 1 demands beside `START`.
const BUDGET: &str = "drain_cold_builds";

/// This file names both needles as data, and would flag itself.
const SELF_FILE: &str = "fixture_drain_budget.rs";

/// The fixture budget every offender is told to set.
const REMEDY: &str = "Duration::from_secs(30)";

/// The integration-test trees this guard reads (§ Aperture).
fn test_trees() -> Vec<PathBuf> {
    let registry = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let crates = registry.parent().expect("crates/ is the manifest's parent");
    vec![registry.join("tests"), crates.join("mrd").join("tests")]
}

/// Every `.rs` under the test trees, as (workspace-relative path, source).
fn sources() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    for tree in test_trees() {
        collect_rs(&tree, &mut paths);
    }
    assert!(
        paths.len() > 20,
        "the guard found only {} sources — the test trees moved and it now \
         guards nothing: {:?}",
        paths.len(),
        test_trees()
    );
    paths.sort();
    paths
        .iter()
        .filter(|p| p.file_name().is_none_or(|n| n != SELF_FILE))
        .map(|p| {
            let text =
                fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            (rel(p), text)
        })
        .collect()
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("the test tree {} must exist: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable dir entry").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `…/crates/mrd/tests/x.rs` → `crates/mrd/tests/x.rs`, so a failure names a
/// path the reader can open.
fn rel(path: &Path) -> String {
    let full = path.display().to_string();
    match full.find("/crates/") {
        Some(i) => full[i + 1..].to_owned(),
        None => full,
    }
}

/// Drop a line comment, so a needle inside prose never counts as code.
fn code(line: &str) -> &str {
    match line.split_once("//") {
        Some((before, _)) => before,
        None => line,
    }
}

/// **Half 1.** A fixture file that starts a daemon raises the drain budget.
#[test]
fn every_fixture_that_starts_a_daemon_raises_the_drain_budget() {
    let offenders: Vec<String> = sources()
        .into_iter()
        .filter(|(_, text)| text.lines().any(|l| code(l).contains(START)) && !text.contains(BUDGET))
        .map(|(path, _)| path)
        .collect();

    assert!(
        offenders.is_empty(),
        "these fixtures start a RunningServer on the PRODUCTION drain default \
         (2 s — a client's flock budget, never a fixture's): {offenders:#?}\n\
         Set `config.{BUDGET} = {REMEDY};` on the config each one starts from. \
         Rule: crates/registry/tests/common/mod.rs § Fixture rule."
    );
}

/// **Half 2.** A fixture struct holding a server and a `TempDir` declares the
/// server first, or orders its own teardown with an explicit `impl Drop`.
#[test]
fn a_fixture_struct_declares_its_server_before_its_tempdir() {
    let mut offenders = Vec::new();
    for (path, text) in sources() {
        for decl in structs(&text) {
            let (Some(server), Some(tmp)) =
                (decl.field_with("RunningServer"), decl.field_with("TempDir"))
            else {
                continue;
            };
            if server > tmp && !text.contains(&format!("impl Drop for {}", decl.name)) {
                offenders.push(format!("{path}:{} struct {}", decl.line, decl.name));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these fixture structs drop their TempDir BEFORE their server, so the \
         workspace vanishes under a live drawer rebuild and the drain never \
         runs: {offenders:#?}\n\
         Move the server field ABOVE the TempDir field (struct fields drop in \
         declaration order, locals in reverse), or write an explicit \
         `impl Drop`. Rule: crates/registry/tests/common/mod.rs § Fixture rule."
    );
}

/// One `struct Name { … }` declaration, as read from source text.
struct StructDecl {
    name: String,
    /// 1-based line of the header, for the failure message.
    line: usize,
    /// Field lines in declaration order — which IS drop order.
    fields: Vec<String>,
}

impl StructDecl {
    /// Position of the first field whose type mentions `needle`.
    fn field_with(&self, needle: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.contains(needle))
    }
}

/// Every brace-struct in `text` whose header ends its line with `{`.
fn structs(text: &str) -> Vec<StructDecl> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(name) = struct_header(code(lines[i])) else {
            i += 1;
            continue;
        };
        let header = i;
        let mut fields = Vec::new();
        i += 1;
        while i < lines.len() {
            let body = code(lines[i]);
            if body.trim_start().starts_with('}') {
                break;
            }
            let field = body.trim();
            if !field.is_empty() && !field.starts_with('#') {
                fields.push(field.to_owned());
            }
            i += 1;
        }
        out.push(StructDecl {
            name,
            line: header + 1,
            fields,
        });
        i += 1;
    }
    out
}

/// `pub(crate) struct Sandbox {` → `Sandbox`. A tuple or unit struct (no
/// trailing `{`) and a header split across lines both answer `None`.
fn struct_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.ends_with('{') {
        return None;
    }
    let after = trimmed
        .strip_prefix("struct ")
        .or_else(|| trimmed.strip_prefix("pub struct "))
        .or_else(|| trimmed.strip_prefix("pub(crate) struct "))?;
    let name: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}
