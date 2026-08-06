//! Def resolution across the layer ladder — Go `internal/defs/cascade.go`:
//! session → … → builtin, NEAREST-WINS-PER-KEY merging (props per-key, section
//! rules per-name, template whole-block). No `extends:` chain — per-key
//! resolution over an ordered dir list cannot form a diamond.

use std::path::{Path, PathBuf};

use super::load::{Def, DefError, parse_def};

/// The Resolve outcome, splitting Go's `ErrNoDef` from a malformed def:
/// a kind nobody defined is not a record contract (the write proceeds); a def
/// that EXISTS but fails to load refuses-unless-forced (fail closed).
#[derive(Debug)]
pub enum ResolveOutcome {
    Resolved(Def),
    /// No def file for the kind anywhere in the ladder.
    NoDef,
    /// A def exists but is malformed — the Go error string, verbatim.
    Malformed(DefError),
}

/// Resolve the def for `kind` (and optional `preset`) across `layers`
/// (ordered defs directories, NEAREST first). Within one layer the
/// preset-qualified `<kind>-<preset>.md` is nearer than `<kind>.md`.
/// `build_doc` supplies the markdown parse (the `syntax` edge is the caller's).
pub fn resolve(
    kind: &str,
    preset: &str,
    layers: &[PathBuf],
    build_doc: &dyn Fn(&str) -> model::Document,
) -> ResolveOutcome {
    let mut merged: Option<Def> = None;
    for dir in layers {
        let mut names = vec![format!("{}.md", kind)];
        if !preset.is_empty() {
            names.insert(0, format!("{kind}-{preset}.md"));
        }
        for name in names {
            let path = dir.join(&name);
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let doc = build_doc(&raw);
            let def = match parse_def(&doc, &path.display().to_string()) {
                Ok(d) => d,
                Err(e) => return ResolveOutcome::Malformed(e), // fail closed
            };
            if def.kind != kind {
                return ResolveOutcome::Malformed(DefError(format!(
                    "def {}: defines {}, resolved for kind {}",
                    path.display(),
                    super::go_fmt::go_quote(&def.kind),
                    super::go_fmt::go_quote(kind)
                )));
            }
            merged = Some(merge_def(merged.take(), def));
        }
    }
    match merged {
        Some(d) => ResolveOutcome::Resolved(d),
        None => ResolveOutcome::NoDef,
    }
}

/// Fold a FARTHER def under an already-merged NEARER one.
fn merge_def(near: Option<Def>, far: Def) -> Def {
    let Some(mut near) = near else {
        return far;
    };
    for (key, spec) in far.props {
        near.props.entry(key).or_insert(spec);
    }
    for (name, rule) in far.sections {
        if near.section(&name).is_none() {
            near.sections.push((name, rule));
        }
    }
    if near.template.is_empty() {
        near.template = far.template;
    }
    if near.version == 0 {
        near.version = far.version;
    }
    near.sources.extend(far.sources);
    near
}

/// The default layer ladder for a record path: every `defs/` directory from
/// the record's own directory upward (nearest first), then `$UCC_HOME/defs`.
/// Same walk the Go daemon runs, to the filesystem root.
pub fn discover_layers(record_path: &Path) -> Vec<PathBuf> {
    let mut layers = Vec::new();
    let mut dir = record_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    // Go uses filepath.Abs — LEXICAL absolutization (cwd-join), no symlink
    // resolution; canonicalize would diverge on symlinked trees.
    if dir.as_os_str().is_empty() {
        dir = PathBuf::from(".");
    }
    if !dir.is_absolute()
        && let Ok(cwd) = std::env::current_dir()
    {
        dir = cwd.join(dir);
    }
    loop {
        let cand = dir.join("defs");
        if cand.is_dir() {
            layers.push(cand);
        }
        let Some(parent) = dir.parent() else {
            break;
        };
        if parent == dir {
            break;
        }
        dir = parent.to_path_buf();
    }
    if let Ok(home) = std::env::var("UCC_HOME")
        && !home.is_empty()
    {
        let cand = Path::new(&home).join("defs");
        if cand.is_dir() {
            layers.push(cand);
        }
    }
    layers
}
