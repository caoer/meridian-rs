//! The workspace's drawer digest memo — the run plane's F8 instrument
//! ([`fs::digestmemo`]), persisted in the per-workspace cache drawer beside
//! `sql.duckdb` as `run-digests.v1`.
//!
//! One memo, every CLI door that folds without parsing: the bash bracket's
//! three observations ([`crate::dispatch_bash`]) and `mrd sql`'s post-result
//! `live` sample (`node-rev-merkle-spec.md` §6.7, "fingerprint-only doors
//! outside the daemon") load it at entry and save it at exit, so one
//! process's fold warms the next's whichever verb ran it. The daemon never
//! touches it: its doors observe through the resident [`fs::DomainCache`].
//!
//! The memo is evidence cache, never authority — every failure here is a
//! COLD memo (absence only costs reads), and a failed save never costs a
//! command that already succeeded.

use fs::digestmemo::DigestMemo;

/// The digest memo's basename inside the per-workspace cache drawer (beside
/// `sql.duckdb`): run's corpus observations amortise there the same way
/// sql's projection does (F8). Version rides the name — an older binary
/// simply reads cold. The `run-` prefix is historical: the memo is the
/// workspace's, and `mrd sql` shares it.
pub const DIGEST_MEMO_FILENAME: &str = "run-digests.v1";

/// Load the digest memo from the workspace cache drawer. Every failure — no
/// cache root, no drawer, no file, alien bytes — is a cold memo: the memo is
/// evidence cache, never authority, and absence only costs reads.
#[must_use]
pub fn load_memo(root: &fs::WorkspaceRoot) -> DigestMemo {
    let Ok(canonical) = root.0.canonicalize() else {
        return DigestMemo::new();
    };
    let drawer = cache::CacheDrawer::open(&canonical);
    let Some(dir) = drawer.dir() else {
        return DigestMemo::new();
    };
    match std::fs::read(dir.join(DIGEST_MEMO_FILENAME)) {
        Ok(bytes) => DigestMemo::from_bytes(&bytes),
        Err(_) => DigestMemo::new(),
    }
}

/// Persist the memo back to the drawer, atomic (temp + rename) and silent:
/// concurrent processes last-writer-win over a cache whose worst staleness
/// is an extra read, and a failed save must never cost a command that
/// already succeeded.
pub fn save_memo(root: &fs::WorkspaceRoot, memo: &DigestMemo) {
    let Ok(canonical) = root.0.canonicalize() else {
        return;
    };
    let drawer = cache::CacheDrawer::open(&canonical);
    let Some(dir) = drawer.dir() else {
        return;
    };
    // The sentinel is gc bookkeeping; its failure must not cost the save.
    let _ = drawer.register();
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!("{DIGEST_MEMO_FILENAME}.tmp.{}", std::process::id()));
    if std::fs::write(&tmp, memo.to_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if std::fs::rename(&tmp, dir.join(DIGEST_MEMO_FILENAME)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}
