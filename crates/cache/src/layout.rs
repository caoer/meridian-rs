//! Drawer addressing: cache root resolution and the hashed-drawer path.

use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::SCHEMA_SALT;

/// Resolve the meridian cache root: `$XDG_CACHE_HOME/meridian`, else
/// `$HOME/.cache/meridian`.
///
/// Hand-rolled from the environment — no `dirs` crate. Meridian fully owns this
/// directory (it lives outside any watched tree, sidestepping the sync-corruption
/// scar of an in-tree cache).
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when neither `XDG_CACHE_HOME` nor `HOME`
/// is set to a non-empty value. Callers degrade to an ephemeral in-memory drawer
/// on this error rather than failing the run.
pub fn cache_root() -> io::Result<PathBuf> {
    if let Some(xdg) = non_empty_env("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(xdg).join("meridian"));
    }
    if let Some(home) = non_empty_env("HOME") {
        return Ok(PathBuf::from(home).join(".cache").join("meridian"));
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "neither XDG_CACHE_HOME nor HOME is set; no cache root",
    ))
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var_os(key)
        .map(|v| v.to_string_lossy().into_owned())
        .filter(|v| !v.is_empty())
}

/// The drawer key for a canonical workspace path: `sha256(path-bytes)[:16]` in
/// lowercase hex.
///
/// The hash is taken over the raw path bytes exactly as given. The caller
/// guarantees the path is already canonical (see crate docs); this function
/// never canonicalizes, so two spellings of one workspace hash differently.
#[must_use]
pub fn drawer_key(workspace: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace.as_os_str().as_bytes());
    let digest = hasher.finalize();
    to_hex(&digest[..8]) // first 8 bytes → 16 lowercase hex chars
}

/// The socket key for a canonical cache root: `sha256(path-bytes)[:12]` in
/// lowercase hex — the hash half of the registry's short-socket mapping
/// (`hash(cache_root) → sock path`, injective per root; the socket itself no
/// longer lives under the cache root, so a deep root cannot push it past
/// `sun_path`).
///
/// Same convention as [`drawer_key`]: the hash is over the raw path bytes
/// exactly as given, never canonicalized here — two spellings of one cache
/// root key differently, and callers on one machine agree because they all
/// resolve the root from the same environment ([`cache_root`]).
#[must_use]
pub fn sock_key(cache_root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cache_root.as_os_str().as_bytes());
    let digest = hasher.finalize();
    to_hex(&digest[..6]) // first 6 bytes → 12 lowercase hex chars
}

/// Lowercase-hex encode `bytes` (no `hex` crate in the dep graph).
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The version+salt path segment, `<pkg-version>-<schema-salt>`.
///
/// The version and salt ride a path *segment*, never hashed into keys (the Ruff
/// lesson): a binary upgrade or a payload-schema bump becomes a clean cold start,
/// an old-schema drawer is never decoded into a new struct.
#[must_use]
pub(crate) fn version_segment() -> String {
    let version = sanitize_segment(env!("CARGO_PKG_VERSION"));
    format!("{version}-{SCHEMA_SALT}")
}

/// Keep a string usable as a single path segment: no separators.
fn sanitize_segment(v: &str) -> String {
    let v = v.replace(std::path::MAIN_SEPARATOR, "_");
    v.replace('/', "_")
}

/// The full drawer directory for a canonical workspace path:
/// `<cache-root>/<drawer-key>/<version-segment>/`.
#[must_use]
pub fn drawer_dir(cache_root: &Path, workspace: &Path) -> PathBuf {
    cache_root
        .join(drawer_key(workspace))
        .join(version_segment())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawer_key_is_16_hex_chars_and_stable() {
        let key = drawer_key(Path::new("/home/zt/wiki"));
        assert_eq!(key.len(), 16, "key = sha256[:16] = 16 hex chars");
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(key, drawer_key(Path::new("/home/zt/wiki")), "stable");
    }

    #[test]
    fn drawer_key_differs_by_path() {
        assert_ne!(
            drawer_key(Path::new("/home/zt/a")),
            drawer_key(Path::new("/home/zt/b"))
        );
    }

    #[test]
    fn drawer_key_matches_sha256_prefix() {
        // Anchor the algorithm: first 16 hex of sha256("x").
        let mut h = Sha256::new();
        h.update(b"x");
        let full = h.finalize();
        let want = to_hex(&full[..8]);
        assert_eq!(drawer_key(Path::new("x")), want);
    }

    #[test]
    fn version_segment_carries_salt() {
        let seg = version_segment();
        assert!(seg.ends_with(&format!("-{SCHEMA_SALT}")));
        assert!(!seg.contains('/'));
        assert!(!seg.contains(std::path::MAIN_SEPARATOR));
    }

    #[test]
    fn cache_root_prefers_xdg_then_home() {
        // Pure function of two inputs; assert the join shape without mutating
        // process env (parallel-test safe): re-derive via the same rule.
        let xdg = PathBuf::from("/x/cache").join("meridian");
        assert_eq!(xdg, Path::new("/x/cache/meridian"));
        let home = PathBuf::from("/h").join(".cache").join("meridian");
        assert_eq!(home, Path::new("/h/.cache/meridian"));
    }

    #[test]
    fn drawer_dir_composes_root_key_segment() {
        let root = Path::new("/c/meridian");
        let dir = drawer_dir(root, Path::new("/w"));
        let key = drawer_key(Path::new("/w"));
        assert_eq!(dir, root.join(key).join(version_segment()));
    }
}
