//! Atomic disposable-cache publication. The drawer lock coordinates saves,
//! cleanup and GC; no registry or workspace-observation lock is involved.
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write<T>(
    dir: &Path,
    name: &str,
    encode: impl FnOnce(&mut BufWriter<File>) -> io::Result<T>,
) -> io::Result<T> {
    fs::create_dir_all(dir)?;
    let Some(_drawer) = cache::DrawerLock::try_acquire(dir)? else {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "cache drawer busy",
        ));
    };
    let prefix = format!(".{name}.tmp.");
    // A process can die before cleanup. No active writer of this payload
    // holds a staging file while we own the drawer lock.
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) && entry.file_type()?.is_file()
        {
            fs::remove_file(entry.path())?;
        }
    }
    let tmp = dir.join(format!(
        "{prefix}{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        let mut writer = BufWriter::new(file);
        let value = encode(&mut writer)?;
        writer.flush()?;
        ::fs::honest_sync(writer.get_ref())?;
        fs::rename(&tmp, dir.join(name))?;
        ::fs::honest_sync_path(dir)?;
        Ok(value)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_encoding_preserves_the_old_snapshot_and_removes_partial_bytes() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("drawer");
        write(&dir, "snapshot", |out| out.write_all(b"old")).unwrap();
        let result: io::Result<()> = write(&dir, "snapshot", |out| {
            out.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(dir.join("snapshot")).unwrap(), b"old");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn abandoned_staging_is_cleaned_but_other_payloads_are_preserved() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path();
        fs::write(dir.join(".snapshot.tmp.dead"), b"abandoned").unwrap();
        fs::write(dir.join(".another.tmp.live"), b"other payload").unwrap();
        write(dir, "snapshot", |out| out.write_all(b"complete")).unwrap();
        assert!(!dir.join(".snapshot.tmp.dead").exists());
        assert!(dir.join(".another.tmp.live").exists());
        assert_eq!(fs::read(dir.join("snapshot")).unwrap(), b"complete");
    }
}
