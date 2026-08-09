//! Quality-gate tests for the cache drawer, exercised through the public API.
//!
//! The sentinel filename `registered.json` is part of the on-disk contract, so
//! tests that plant raw sentinels hardcode it deliberately.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use cache::{CacheDrawer, Probe, Sentinel, probe, register};

const SENTINEL: &str = "registered.json";

/// The version+salt path segment the crate builds (`layout::version_segment`).
/// The version is DERIVED from the workspace stamp so a release bump cannot red
/// these gates; the salt stays a literal, like the sentinel filename above,
/// because it is on-disk contract rather than a stamp.
const VERSION_SEGMENT: &str = concat!(env!("CARGO_PKG_VERSION"), "-s0");

fn drawer(tmp: &Path) -> std::path::PathBuf {
    tmp.join("hash").join(VERSION_SEGMENT)
}

fn parse_sentinel(dir: &Path) -> Sentinel {
    let bytes = fs::read(dir.join(SENTINEL)).expect("sentinel present");
    serde_json::from_slice(&bytes).expect("sentinel parses")
}

// GATE: a half-created drawer (dir exists, no sentinel) is a MISS. mkdir is not
// the registration signal.
#[test]
fn half_created_drawer_is_a_miss() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = drawer(tmp.path());
    fs::create_dir_all(&dir).unwrap();
    assert_eq!(probe(&dir), Probe::Miss);
}

// GATE: a kill mid-registration leaves an orphan temp (never renamed) — still a
// MISS, and the temp is never adoptable on the next probe.
#[test]
fn orphan_temp_is_a_miss_and_never_adopted() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = drawer(tmp.path());
    fs::create_dir_all(&dir).unwrap();
    // A temp with fully-valid sentinel content, but a temp name — the shape a
    // crash after write-temp, before rename/link, leaves behind.
    let valid = br#"{"schema":1,"workspace":"/ws","created_at":1,"last_use":1}"#;
    fs::write(dir.join(".registered.json.999.123.0.tmp"), valid).unwrap();

    assert_eq!(
        probe(&dir),
        Probe::Miss,
        "orphan temp is never a registration"
    );

    // A real registration still succeeds and is the one that lands.
    let s = register(&dir, Path::new("/ws")).unwrap();
    assert_eq!(s.workspace, "/ws");
    assert_eq!(probe(&dir), Probe::Hit(s));
}

// GATE: corrupt sentinels — truncated json, non-json garbage, and a
// current-schema but wrong-shape record — are all a MISS, never Err/panic.
#[test]
fn corrupt_sentinels_are_a_miss_never_error() {
    let tmp = tempfile::tempdir().unwrap();
    for (i, body) in [
        &br#"{"schema":1,"workspace":"/w""#[..], // truncated json
        &b"not json at all {{{"[..],             // garbage bytes
        &br#"{"schema":1}"#[..],                 // schema ok, missing required fields
        &br#"{"schema":"one","workspace":"/w"}"#[..], // schema wrong type
        &b""[..],                                // empty file
    ]
    .iter()
    .enumerate()
    {
        let dir = tmp.path().join(format!("c{i}")).join(VERSION_SEGMENT);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SENTINEL), body).unwrap();
        assert_eq!(probe(&dir), Probe::Miss, "corrupt body {i} must be a miss");
    }
}

// GATE: a future-schema sentinel (bumped schema int, extra fields) probed by
// current code is a cold start (MISS) — and registration never clobbers it
// (downgrade safety).
#[test]
fn future_schema_is_cold_start_and_never_clobbered() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = drawer(tmp.path());
    fs::create_dir_all(&dir).unwrap();
    let future = br#"{"schema":2,"workspace":"/ws","created_at":9,"last_use":9,"new_field":"x"}"#;
    fs::write(dir.join(SENTINEL), future).unwrap();

    assert_eq!(
        probe(&dir),
        Probe::Miss,
        "future schema is a cold-start miss"
    );

    // An older binary registering must not overwrite the newer format.
    let before = fs::read(dir.join(SENTINEL)).unwrap();
    let s = register(&dir, Path::new("/ws")).unwrap();
    let after = fs::read(dir.join(SENTINEL)).unwrap();
    assert_eq!(
        before, after,
        "downgrade must not clobber a future sentinel"
    );
    assert_eq!(
        s.workspace, "/ws",
        "identity still reported by workspace path"
    );
}

// GATE: concurrent first-registration of one drawer — exactly one sentinel wins,
// and every caller converges on identical identity (amendment C2).
#[test]
fn concurrent_registration_converges_on_one_identity() {
    const N: usize = 8;
    let tmp = tempfile::tempdir().unwrap();
    let dir = Arc::new(drawer(tmp.path()));
    let ws = "/ws/shared";
    let barrier = Arc::new(Barrier::new(N));

    let handles: Vec<_> = (0..N)
        .map(|_| {
            let dir = Arc::clone(&dir);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                register(&dir, Path::new(ws)).unwrap()
            })
        })
        .collect();

    let results: Vec<Sentinel> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Exactly one sentinel on disk; every caller returned it byte-for-byte.
    let on_disk = parse_sentinel(&dir);
    assert_eq!(on_disk.workspace, ws);
    for r in &results {
        assert_eq!(*r, on_disk, "every registrar converges on the winner");
    }
    // A follow-up probe still sees exactly that identity.
    assert_eq!(probe(&dir), Probe::Hit(on_disk));
}

// GATE: the ephemeral degrade satisfies the same API and never fails — caching is
// an optimization, not a failure mode.
#[test]
fn ephemeral_drawer_is_a_cold_noop_that_never_fails() {
    let d = CacheDrawer::ephemeral(Path::new("/ws/bare"));
    assert!(d.is_ephemeral());
    assert!(d.dir().is_none());
    assert_eq!(d.probe(), Probe::Miss, "ephemeral is always cold");
    let s = d.register().expect("ephemeral register never fails");
    assert_eq!(s.workspace, "/ws/bare");
    d.stamp_last_use().expect("ephemeral stamp never fails");
    // Still cold after a register — no persistence.
    assert_eq!(d.probe(), Probe::Miss);
}

// A disk drawer round-trips: register → probe hit → stamp advances last_use.
#[test]
fn disk_drawer_round_trip_and_stamp() {
    let d = CacheDrawer::open(Path::new("/ws/opened"));
    // In a normal test environment HOME is set, so open() is disk-backed.
    assert!(!d.is_ephemeral());
    assert_eq!(d.probe(), Probe::Miss, "cold before registration");
    let first = d.register().unwrap();
    match d.probe() {
        Probe::Hit(s) => assert_eq!(s.workspace, "/ws/opened"),
        Probe::Miss => panic!("registered drawer must be a hit"),
    }
    d.stamp_last_use().unwrap();
    let after = d.probe();
    let Probe::Hit(stamped) = after else {
        panic!("still registered")
    };
    assert!(stamped.last_use >= first.last_use);
    // Clean up the real cache drawer this test created.
    if let Some(dir) = d.dir() {
        cache::remove_drawer(dir).unwrap();
    }
}
