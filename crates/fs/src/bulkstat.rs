//! One directory's regular files with their [`StatKey`]s from one kernel
//! conversation: macOS `getattrlistbulk(2)`.
//!
//! The currency pass needs every member's identity. One `lstat` per member
//! resolves the member's whole path each time and the calls contend in the
//! kernel, so eight threads buy less than three times one. The bulk call
//! returns a directory's entries with their attributes together, at about a
//! quarter of the per-member cost.
//!
//! The keys are the ones `lstat` reports, field for field: the memo compares
//! a key taken here against one a read took with `fstat`, so any difference
//! would re-read the member on every pass. Only regular files are answered;
//! every other entry, and any entry the kernel did not fully describe, is
//! left out and the caller stats it by path.

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::unix::io::AsRawFd as _;
use std::path::Path;

use crate::StatKey;

/// The reply buffer, in `u32` words so every record is 4-aligned as the
/// kernel packs it.
const BUF_WORDS: usize = 32 * 1024;

/// `VREG` of `<sys/vnode.h>`: the object type of a regular file.
const VREG: u32 = 1;

/// The attributes asked of every entry, in the order the kernel packs them.
const COMMON: libc::attrgroup_t = libc::ATTR_CMN_RETURNED_ATTRS
    | libc::ATTR_CMN_NAME
    | libc::ATTR_CMN_DEVID
    | libc::ATTR_CMN_OBJTYPE
    | libc::ATTR_CMN_MODTIME
    | libc::ATTR_CMN_CHGTIME
    | libc::ATTR_CMN_FILEID;

/// Every regular file directly inside `abs_dir`, by raw name bytes, with the
/// [`StatKey`] `lstat` would report for it.
///
/// # Errors
/// The directory cannot be opened or listed, or the kernel's reply does not
/// parse. The caller falls back to one `lstat` per member.
pub(crate) fn dir_file_keys(abs_dir: &Path) -> io::Result<HashMap<Vec<u8>, StatKey>> {
    let dir = File::open(abs_dir)?;
    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: COMMON,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };
    let mut buf = vec![0u32; BUF_WORDS];
    let mut keys = HashMap::new();
    loop {
        // SAFETY: `request` and `buf` outlive the call, and the size passed is
        // the buffer's own length in bytes.
        let count = unsafe {
            libc::getattrlistbulk(
                dir.as_raw_fd(),
                std::ptr::from_mut(&mut request).cast::<c_void>(),
                buf.as_mut_ptr().cast::<c_void>(),
                BUF_WORDS * 4,
                0,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            return Ok(keys);
        }
        // SAFETY: the same allocation, read as bytes; `u32` has no padding.
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), BUF_WORDS * 4) };
        let mut at = 0usize;
        for _ in 0..count {
            let len = Record(bytes).u32_at(at).ok_or_else(malformed)? as usize;
            let record = bytes
                .get(at..at.saturating_add(len))
                .filter(|_| len >= 4)
                .ok_or_else(malformed)?;
            if let Some((name, key)) = file_key(record) {
                keys.insert(name.to_vec(), key);
            }
            at += len;
        }
    }
}

fn malformed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "getattrlistbulk returned a record that does not parse",
    )
}

/// One packed record, read by offset.
struct Record<'a>(&'a [u8]);

impl Record<'_> {
    fn u32_at(&self, at: usize) -> Option<u32> {
        let raw = self.0.get(at..at.checked_add(4)?)?;
        Some(u32::from_ne_bytes(raw.try_into().ok()?))
    }

    fn i32_at(&self, at: usize) -> Option<i32> {
        let raw = self.0.get(at..at.checked_add(4)?)?;
        Some(i32::from_ne_bytes(raw.try_into().ok()?))
    }

    fn i64_at(&self, at: usize) -> Option<i64> {
        let raw = self.0.get(at..at.checked_add(8)?)?;
        Some(i64::from_ne_bytes(raw.try_into().ok()?))
    }

    fn u64_at(&self, at: usize) -> Option<u64> {
        let raw = self.0.get(at..at.checked_add(8)?)?;
        Some(u64::from_ne_bytes(raw.try_into().ok()?))
    }
}

/// The name and key of one record when it describes a regular file and
/// carries every attribute asked; `None` otherwise.
///
/// Layout (`getattrlist(2)`): the record length, the returned-attribute set,
/// then each returned attribute in bitmap order, common group first, every
/// field 4-aligned. A name is an offset-and-length pair pointing at
/// NUL-terminated bytes later in the record, the offset counted from the pair
/// itself.
fn file_key(record: &[u8]) -> Option<(&[u8], StatKey)> {
    let rec = Record(record);
    let returned_common = rec.u32_at(4)?;
    let returned_file = rec.u32_at(16)?;
    // The kernel does not always echo the returned-attributes bit itself.
    let asked = COMMON & !libc::ATTR_CMN_RETURNED_ATTRS;
    if returned_common & asked != asked || returned_file & libc::ATTR_FILE_DATALENGTH == 0 {
        return None;
    }
    // length (4) + attribute_set_t (20)
    let name_ref = 24;
    let name_start = name_ref + usize::try_from(rec.i32_at(name_ref)?).ok()?;
    let name_len = rec.u32_at(name_ref + 4)? as usize;
    // name attrreference_t (8) → dev_t (4) → fsobj_type_t (4) → mtime (16)
    // → ctime (16) → file id (8) → data length (8)
    // `st_dev` is a signed 32-bit `dev_t`; `MetadataExt::dev` widens it to
    // `u64` with its sign, and so does this.
    let dev = u64::from_ne_bytes(i64::from(rec.i32_at(32)?).to_ne_bytes());
    if rec.u32_at(36)? != VREG {
        return None;
    }
    let mtime = (rec.i64_at(40)?, rec.i64_at(48)?);
    let ctime = (rec.i64_at(56)?, rec.i64_at(64)?);
    let ino = rec.u64_at(72)?;
    let size = u64::try_from(rec.i64_at(80)?).ok()?;
    let name = record.get(name_start..name_start.checked_add(name_len)?)?;
    let name = name.strip_suffix(&[0]).unwrap_or(name);
    Some((name, StatKey::from_raw_parts(dev, ino, size, mtime, ctime)))
}

#[cfg(test)]
mod tests {
    use super::dir_file_keys;
    use crate::StatKey;
    use std::fs;

    /// The bulk key of every regular file equals the key `lstat` reports, and
    /// nothing but regular files is answered.
    #[test]
    fn bulk_keys_equal_lstat_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for i in 0..300 {
            fs::write(dir.join(format!("f{i:03}.md")), vec![b'x'; i * 7]).unwrap();
        }
        fs::write(dir.join("名前 with space.md"), "# t\n").unwrap();
        fs::write(dir.join("empty.md"), "").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        std::os::unix::fs::symlink(dir.join("f000.md"), dir.join("link.md")).unwrap();

        let keys = dir_file_keys(dir).unwrap();
        assert_eq!(keys.len(), 302, "regular files only");
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let bulk = keys.get(name.as_encoded_bytes()).copied();
            if entry.file_type().unwrap().is_file() {
                assert_eq!(
                    bulk,
                    StatKey::of_path(&entry.path()).unwrap(),
                    "{name:?}: the bulk key is the lstat key"
                );
            } else {
                assert_eq!(bulk, None, "{name:?} is not a regular file");
            }
        }
    }

    /// More entries than one reply buffer holds are all answered.
    #[test]
    fn a_directory_larger_than_one_reply_is_answered_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let long = "n".repeat(200);
        for i in 0..2000 {
            fs::write(dir.join(format!("{long}-{i:04}.md")), "x").unwrap();
        }
        let keys = dir_file_keys(dir).unwrap();
        assert_eq!(keys.len(), 2000);
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(dir_file_keys(&tmp.path().join("absent")).is_err());
    }
}
