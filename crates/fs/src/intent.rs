//! Checksummed `O_EXCL` transaction intents — recovery truth, not semantic
//! truth (authority contract §5; `docs/laws.md` Amendment — the one state
//! owner).
//!
//! Lives under `.meridian/intents/` (dot-prefix, outside the hash domain).
//! Markdown remains sole semantic truth; an intent is bounded recovery
//! material so a crash after the durable decision can finish or restore
//! the COMPLETE declared set.
//!
//! `O_EXCL` (`create_new`) prevents duplicate txn ids. It is not the
//! publisher fence — the authority lease plus reservations are.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{WorkspaceRoot, honest_sync, honest_sync_dir};

/// Workspace-relative directory holding one file per transaction.
pub const INTENTS_DIR: &str = ".meridian/intents";

const FORMAT: &str = "MRD-INTENT/1";
const GROUP_FORMAT: &str = "MRD-GROUP/1";

/// Publication phase recorded in the intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Durable decision: no remaining refusal rung; dests may now change.
    Decided,
    /// All dests durable, root/frame assigned, recovery material reclaimable.
    Applied,
}

/// What recovery does with a decided-but-incomplete intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Put every declared dest back to its old identity.
    Restore,
    /// Push every declared dest forward to its new identity.
    Forward,
}

/// One destination the intent covers, recovery images included.
///
/// The images are the member's own bytes, not paths to side files. The
/// contract's word is that a transaction durably creates ONE checksummed
/// `O_EXCL` record; a record plus four side files it references is further
/// from that than a self-contained record is, and it carries a failure mode
/// this shape does not have — a crash cannot leave a partial image set
/// behind a record that names it, because the record is atomically complete
/// or absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentMember {
    /// Workspace-relative destination.
    pub rel: PathBuf,
    /// Recovery image of the pre-image bytes; empty means the destination
    /// was absent (the first-append / create case).
    pub old: Vec<u8>,
    /// Recovery image of the bytes this transaction publishes.
    pub new: Vec<u8>,
    /// Workspace-relative (or absolute) staging temp holding the new bytes
    /// at decision time — may be gone after a successful rename.
    pub tmp: PathBuf,
}

/// The checksummed per-transaction record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub epoch_hex: String,
    pub txn_hex: String,
    pub phase: Phase,
    pub policy: Policy,
    pub members: Vec<IntentMember>,
}

/// How a live destination compares to the recorded identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestClass {
    Old,
    New,
    Neither,
    /// Dest is missing and the old identity is the empty digest — the
    /// first-append / create case, treated as Old.
    AbsentOld,
}

/// A group-commit membership record (§3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupManifest {
    pub gid_hex: String,
    pub members: Vec<String>,
}

/// BLAKE3 of `bytes` — the same family as the merkle law.
#[must_use]
pub fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// Write `bytes` to `path` via tmp + honest-sync + rename + parent sync. The
/// parent sync is grouped ([`honest_sync_dir`]) — it is the half concurrent
/// writers into the same directory can share.
///
/// # Errors
/// I/O at any write, sync, or rename step.
pub fn write_honest(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp-intent");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        honest_sync(&f)?;
    }
    fs::rename(&tmp, path)?;
    match path.parent() {
        Some(parent) => honest_sync_dir(parent),
        None => Ok(()),
    }
}

/// Persist the `Decided` intent — recovery images inside it — `O_EXCL` on
/// the intent file. After this returns, no error on this transaction is a
/// refusal.
///
/// Two honest syncs, the record's own and the directory's, both before the
/// caller's first visible rename: the record has to be durable before any
/// destination changes, and that ordering is the contract.
///
/// # Errors
/// `AlreadyExists` when the txn id is reused; any I/O/sync failure.
pub fn create_decided(root: &WorkspaceRoot, intent: &Intent) -> io::Result<()> {
    fs::create_dir_all(root.0.join(INTENTS_DIR))?;
    let path = intent_path(root, &intent.txn_hex);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let body = encode(intent);
    file.write_all(body.as_bytes())?;
    // The one honest sync that must sit inside the reservation: the decision
    // follows the reverify. Uncapped for exactly that reason — a slot wait
    // taken under a reservation is a deadlock against a writer holding a
    // slot and waiting for that reservation.
    honest_sync(&file)?;
    honest_sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(())
}

/// Rewrite the intent as `Applied` (tmp+honest-sync+rename) so a later
/// recovery reclaims rather than re-applies.
///
/// # Errors
/// I/O/sync failure.
pub fn finalize(root: &WorkspaceRoot, intent: &Intent) -> io::Result<()> {
    let mut done = intent.clone();
    done.phase = Phase::Applied;
    write_honest(
        &intent_path(root, &intent.txn_hex),
        encode(&done).as_bytes(),
    )
}

/// Remove the intent file. The record carries its own recovery images, so
/// there is nothing else of the transaction's to remove.
///
/// # Errors
/// I/O failure other than a missing path.
pub fn reclaim(root: &WorkspaceRoot, txn_hex: &str) -> io::Result<()> {
    match fs::remove_file(intent_path(root, txn_hex)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Every parseable intent under the workspace, plus unparseable files as
/// [`Found::Indeterminate`].
///
/// # Errors
/// Directory I/O failure (a missing intents dir is empty, not an error).
pub fn scan(root: &WorkspaceRoot) -> io::Result<Vec<Found>> {
    let dir = root.0.join(INTENTS_DIR);
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for ent in rd {
        let ent = ent?;
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("intent") {
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                out.push(Found::Indeterminate {
                    path,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        match decode(&bytes) {
            Some(intent) => out.push(Found::Intent(intent)),
            None => out.push(Found::Indeterminate {
                path,
                reason: "truncated or checksum-failed intent".into(),
            }),
        }
    }
    out.sort_by_key(Found::txn_key);
    Ok(out)
}

/// A scan row.
#[derive(Debug, Clone)]
pub enum Found {
    Intent(Intent),
    Indeterminate { path: PathBuf, reason: String },
}

impl Found {
    fn txn_key(&self) -> String {
        match self {
            Self::Intent(i) => i.txn_hex.clone(),
            Self::Indeterminate { path, .. } => path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

/// Classify one live destination against the recorded identities.
///
/// # Errors
/// I/O other than `NotFound`.
pub fn classify(root: &WorkspaceRoot, member: &IntentMember) -> io::Result<DestClass> {
    let dest = root.0.join(&member.rel);
    match fs::read(&dest) {
        Ok(bytes) => {
            let d = digest(&bytes);
            if d == digest(&member.old) {
                Ok(DestClass::Old)
            } else if d == digest(&member.new) {
                Ok(DestClass::New)
            } else {
                Ok(DestClass::Neither)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if member.old.is_empty() {
                Ok(DestClass::AbsentOld)
            } else {
                Ok(DestClass::Neither)
            }
        }
        Err(e) => Err(e),
    }
}

/// Write the recorded old image onto the destination (tmp+honest-sync+rename),
/// or unlink when the old identity is empty.
///
/// # Errors
/// I/O at write/unlink.
pub fn restore_member(root: &WorkspaceRoot, member: &IntentMember) -> io::Result<()> {
    let dest = root.0.join(&member.rel);
    if member.old.is_empty() {
        match fs::remove_file(&dest) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        if let Some(parent) = dest.parent() {
            honest_sync_dir(parent)?;
        }
        return Ok(());
    }
    write_honest(&dest, &member.old)
}

/// Write the recorded new image onto the destination.
///
/// # Errors
/// I/O at write.
pub fn forward_member(root: &WorkspaceRoot, member: &IntentMember) -> io::Result<()> {
    write_honest(&root.0.join(&member.rel), &member.new)
}

/// Write a group membership manifest, `O_EXCL`.
///
/// # Errors
/// `AlreadyExists` on a reused group id; any I/O/sync failure.
pub fn create_group(root: &WorkspaceRoot, group: &GroupManifest) -> io::Result<()> {
    let dir = root.0.join(INTENTS_DIR).join("groups");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.manifest", group.gid_hex));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(encode_group(group).as_bytes())?;
    honest_sync(&file)?;
    honest_sync_dir(&dir)?;
    Ok(())
}

/// Scan group manifests. Unparseable files are skipped (membership is
/// recovered from the intents themselves).
///
/// # Errors
/// Directory I/O failure other than `NotFound`.
pub fn scan_groups(root: &WorkspaceRoot) -> io::Result<Vec<GroupManifest>> {
    let dir = root.0.join(INTENTS_DIR).join("groups");
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for ent in rd {
        let ent = ent?;
        let Ok(bytes) = fs::read(ent.path()) else {
            continue;
        };
        if let Some(g) = decode_group(&bytes) {
            out.push(g);
        }
    }
    Ok(out)
}

/// Quarantine marker for a destination that matched neither identity.
///
/// # Errors
/// I/O at write.
pub fn quarantine(root: &WorkspaceRoot, txn_hex: &str, paths: &[PathBuf]) -> io::Result<()> {
    let rel = PathBuf::from(format!("{INTENTS_DIR}/{txn_hex}.ambiguous"));
    let mut body = String::from("recovery_ambiguous\n");
    for p in paths {
        body.push_str(&p.display().to_string());
        body.push('\n');
    }
    write_honest(&root.0.join(rel), body.as_bytes())
}

fn intent_path(root: &WorkspaceRoot, txn_hex: &str) -> PathBuf {
    root.0.join(INTENTS_DIR).join(format!("{txn_hex}.intent"))
}

fn encode(intent: &Intent) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "{FORMAT}");
    let _ = writeln!(body, "epoch={}", intent.epoch_hex);
    let _ = writeln!(body, "txn={}", intent.txn_hex);
    let _ = writeln!(
        body,
        "phase={}",
        match intent.phase {
            Phase::Decided => "decided",
            Phase::Applied => "applied",
        }
    );
    let _ = writeln!(
        body,
        "policy={}",
        match intent.policy {
            Policy::Restore => "restore",
            Policy::Forward => "forward",
        }
    );
    let _ = writeln!(body, "n={}", intent.members.len());
    for (i, m) in intent.members.iter().enumerate() {
        let _ = writeln!(body, "m{i}.path={}", m.rel.display());
        let _ = writeln!(body, "m{i}.tmp={}", m.tmp.display());
        // The recovery images ride the record. Hex keeps the whole record one
        // checksummed line-oriented text, so a truncated write stays
        // indeterminate instead of decoding into half a member.
        let _ = writeln!(body, "m{i}.old={}", hex(&m.old));
        let _ = writeln!(body, "m{i}.new={}", hex(&m.new));
    }
    let sum = hex(&digest(body.as_bytes()));
    let _ = writeln!(body, "checksum={sum}");
    body
}

fn decode(bytes: &[u8]) -> Option<Intent> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (body, checksum_line) = text.rsplit_once("checksum=")?;
    let want = checksum_line.trim();
    if hex(&digest(body.as_bytes())) != want {
        return None;
    }
    let mut lines = body.lines();
    if lines.next()? != FORMAT {
        return None;
    }
    let mut epoch_hex = String::new();
    let mut txn_hex = String::new();
    let mut phase = Phase::Decided;
    let mut policy = Policy::Restore;
    let mut n = 0usize;
    let mut fields: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for line in lines {
        let (k, v) = line.split_once('=')?;
        match k {
            "epoch" => v.clone_into(&mut epoch_hex),
            "txn" => v.clone_into(&mut txn_hex),
            "phase" => {
                phase = match v {
                    "decided" => Phase::Decided,
                    "applied" => Phase::Applied,
                    _ => return None,
                };
            }
            "policy" => {
                policy = match v {
                    "restore" => Policy::Restore,
                    "forward" => Policy::Forward,
                    _ => return None,
                };
            }
            "n" => n = v.parse().ok()?,
            other => {
                fields.insert(other.to_owned(), v.to_owned());
            }
        }
    }
    if epoch_hex.is_empty() || txn_hex.is_empty() {
        return None;
    }
    let mut members = Vec::with_capacity(n);
    for i in 0..n {
        members.push(IntentMember {
            rel: PathBuf::from(fields.get(&format!("m{i}.path"))?),
            tmp: PathBuf::from(fields.get(&format!("m{i}.tmp"))?),
            old: unhex(fields.get(&format!("m{i}.old"))?)?,
            new: unhex(fields.get(&format!("m{i}.new"))?)?,
        });
    }
    Some(Intent {
        epoch_hex,
        txn_hex,
        phase,
        policy,
        members,
    })
}

fn encode_group(g: &GroupManifest) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "{GROUP_FORMAT}");
    let _ = writeln!(body, "gid={}", g.gid_hex);
    let _ = writeln!(body, "members={}", g.members.join(","));
    let sum = hex(&digest(body.as_bytes()));
    let _ = writeln!(body, "checksum={sum}");
    body
}

fn decode_group(bytes: &[u8]) -> Option<GroupManifest> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (body, checksum_line) = text.rsplit_once("checksum=")?;
    if hex(&digest(body.as_bytes())) != checksum_line.trim() {
        return None;
    }
    let mut lines = body.lines();
    if lines.next()? != GROUP_FORMAT {
        return None;
    }
    let mut gid_hex = String::new();
    let mut members = Vec::new();
    for line in lines {
        let (k, v) = line.split_once('=')?;
        match k {
            "gid" => v.clone_into(&mut gid_hex),
            "members" => {
                members = if v.is_empty() {
                    Vec::new()
                } else {
                    v.split(',').map(str::to_owned).collect()
                };
            }
            _ => {}
        }
    }
    if gid_hex.is_empty() {
        return None;
    }
    Some(GroupManifest { gid_hex, members })
}

fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(H[usize::from(b >> 4)] as char);
        out.push(H[usize::from(b & 0x0f)] as char);
    }
    out
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in 0..s.len() / 2 {
        out.push(u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    #[test]
    fn decided_round_trips_and_o_excl_refuses_a_duplicate() {
        let (_d, root) = ws();
        let intent = Intent {
            epoch_hex: "ee".into(),
            txn_hex: "aa".into(),
            phase: Phase::Decided,
            policy: Policy::Restore,
            members: vec![IntentMember {
                rel: PathBuf::from("notes/a.md"),
                tmp: PathBuf::from("notes/.a.md.tmp"),
                old: b"old".to_vec(),
                new: b"new".to_vec(),
            }],
        };
        create_decided(&root, &intent).expect("first");
        let err = create_decided(&root, &intent).expect_err("O_EXCL");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        let found = scan(&root).expect("scan");
        match &found[..] {
            [Found::Intent(got)] => assert_eq!(got, &intent),
            other => panic!("expected one intent, got {other:?}"),
        }
    }

    #[test]
    fn a_truncated_intent_is_indeterminate_not_a_guess() {
        let (_d, root) = ws();
        let dir = root.0.join(INTENTS_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("dead.intent"), b"MRD-INTENT/1\nepoch=x\n").unwrap();
        let found = scan(&root).expect("scan");
        assert!(
            matches!(&found[..], [Found::Indeterminate { .. }]),
            "truncated bytes never decode: {found:?}"
        );
    }
}
