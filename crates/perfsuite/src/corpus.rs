//! Corpus cache: `<meridian-cache-root>/corpora/<recipe-id>/`, generated on
//! first use, keyed by recipe hash. Corpora are never committed — the recipe is
//! the artifact.
//!
//! The cache lives under the per-user meridian cache root
//! (`$XDG_CACHE_HOME/meridian`, else `$HOME/.cache/meridian`), NEVER inside the
//! repo: the repo is a registerable workspace, and an in-tree corpus enters its
//! §12 hash domain — a 180k-file perf vault under `target/corpora/` made the
//! resident daemon warm multi-GB engines and re-read the whole corpus every
//! pre-warm tick (2026-07-31). The deny ceiling already refuses the cache root
//! as a workspace, so corpora there are structurally un-warmable.
//! Regeneration is byte-identical on a given platform; across platforms libm
//! (`ln`/`cos`) may differ in the last ulp, shifting sampled sizes by bytes,
//! so cross-platform bit-identity is deliberately not claimed. Claim runs
//! always generate-or-reuse locally, so measurements never depend on it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::generator::generate_file;
use crate::inventory::Inventory;
use crate::profile::{GENERATOR_VERSION, Recipe, fnv1a64};

/// The corpus receipt: what was generated, from what, hashing to what.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusManifest {
    pub schema_version: u32,
    pub generator_version: u32,
    pub profile_name: String,
    pub recipe_hash: String,
    pub seed: u64,
    pub files: u32,
    pub total_bytes: u64,
    /// FNV-1a fold over `(rel_path, content)` pairs in generation order.
    pub corpus_hash: String,
    pub created_unix_secs: u64,
}

/// Workspace `target/` dir (honors `CARGO_TARGET_DIR`).
#[must_use]
pub fn workspace_target() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("target")
        },
        PathBuf::from,
    )
}

/// The corpora cache root: `$XDG_CACHE_HOME/meridian/corpora`, else
/// `$HOME/.cache/meridian/corpora` (the same two-rung resolution as
/// `cache::cache_root` — hand-rolled here so the harness stays seam-free).
///
/// Falls back to `target/corpora/` only when NEITHER env var is set: such an
/// environment cannot run the resident daemon either (its `Config::resolve`
/// needs the same cache root), so the in-tree fallback can never be warmed.
#[must_use]
pub fn corpora_root() -> PathBuf {
    let non_empty = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty());
    if let Some(xdg) = non_empty("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("meridian").join("corpora");
    }
    if let Some(home) = non_empty("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("meridian")
            .join("corpora");
    }
    workspace_target().join("corpora")
}

#[must_use]
pub fn cache_dir(recipe: &Recipe) -> PathBuf {
    corpora_root().join(recipe.id())
}

/// Generate-or-reuse: returns the corpus directory, valid manifest guaranteed.
pub fn ensure(recipe: &Recipe) -> io::Result<PathBuf> {
    let dir = cache_dir(recipe);
    let manifest_path = dir.join("manifest.json");
    if let Ok(text) = fs::read_to_string(&manifest_path)
        && let Ok(manifest) = serde_json::from_str::<CorpusManifest>(&text)
        && manifest.recipe_hash == format!("{:016x}", recipe.hash())
        && manifest.generator_version == GENERATOR_VERSION
    {
        return Ok(dir);
    }
    generate_into(&dir, recipe)?;
    Ok(dir)
}

/// Unconditional generation into `dir` (wiped first). Writes each file, its
/// `.inventory.json` sidecar, the corpus-level `inventory.json`, and
/// `manifest.json` last — a manifest present means the corpus is complete.
pub fn generate_into(dir: &Path, recipe: &Recipe) -> io::Result<CorpusManifest> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    let mut total_bytes = 0u64;
    let mut corpus_hash = 0xcbf2_9ce4_8422_2325u64;
    let mut corpus_inv = Inventory::new();
    for index in 0..recipe.files {
        let gf = generate_file(recipe, index);
        let path = dir.join(&gf.rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &gf.text)?;
        fs::write(
            path.with_extension("md.inventory.json"),
            serde_json::to_string_pretty(&gf.inventory)?,
        )?;
        total_bytes += gf.text.len() as u64;
        corpus_hash ^= fnv1a64(gf.rel_path.as_bytes()) ^ fnv1a64(gf.text.as_bytes());
        corpus_hash = corpus_hash.wrapping_mul(0x0000_0100_0000_01b3);
        corpus_inv.merge(&gf.inventory);
    }
    fs::write(
        dir.join("inventory.json"),
        serde_json::to_string_pretty(&corpus_inv)?,
    )?;
    let manifest = CorpusManifest {
        schema_version: 1,
        generator_version: GENERATOR_VERSION,
        profile_name: recipe.profile.name.clone(),
        recipe_hash: format!("{:016x}", recipe.hash()),
        seed: recipe.seed,
        files: recipe.files,
        total_bytes,
        corpus_hash: format!("{corpus_hash:016x}"),
        created_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

pub fn read_manifest(dir: &Path) -> io::Result<CorpusManifest> {
    let text = fs::read_to_string(dir.join("manifest.json"))?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

/// All markdown files (rel path, content), sorted by path — the bench sweep's
/// working set.
pub fn load_files(dir: &Path) -> io::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The corpus-level planted inventory.
pub fn load_inventory(dir: &Path) -> io::Result<Inventory> {
    let text = fs::read_to_string(dir.join("inventory.json"))?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            let rel = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_string_lossy()
                .into_owned();
            out.push((rel, fs::read_to_string(&path)?));
        }
    }
    Ok(())
}
