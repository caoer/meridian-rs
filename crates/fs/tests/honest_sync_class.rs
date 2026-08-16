//! The ruled durability class is plain fsync, not F_FULLFSYNC.
//!
//! macOS syscall counts (fsync vs fcntl) are a darwin probe, not this
//! CI test. This file only checks that the helper succeeds on a file
//! and a directory.

use std::fs::File;
use std::io::Write;

#[test]
fn honest_sync_file_and_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x");
    let mut f = File::create(&path).unwrap();
    f.write_all(b"ok").unwrap();
    fs::honest_sync(&f).unwrap();
    fs::honest_sync_path(&path).unwrap();
    fs::honest_sync_path(dir.path()).unwrap();
}
