//! A commit through the settle race: a reader observes a late rewrite
//! between a write door's observation and its settle step. A splice and a set
//! take `root_before` from the memo the settle step classified, so their
//! frame chains onto the settle frame. `create` starts its frame at the
//! door's own observation; this pins today's degrade for it (two chain
//! breaks, the born file told twice — wire-contract §7.1), so a slice that
//! makes it chain must change this test.
//!
//! The reader keeps observing until it sees each write
//! ([`crate::ring::observe_past`]).

use super::*;
use crate::ring::observe_past;
use serde_json::json;

/// A frame as `(root_before, root_after, sorted paths)`, roots by name.
type Told = (String, String, Vec<String>);

const PLAN: &str = "# Goals\n\nship by August\n";

fn setup() -> (tempfile::TempDir, fs::WorkspaceRoot, Registry) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("plan.md"), PLAN).unwrap();
    std::fs::write(root.join("notes.md"), "# Notes\n\nship by August\n").unwrap();
    std::fs::write(root.join("other.md"), "# Other\n").unwrap();
    std::fs::write(root.join("late.md"), "# Late\n").unwrap();
    let mut config = Config::for_cache_root(dir.path().join("cache"));
    config.socket_path = dir.path().join("daemon.sock");
    config.state_path = dir.path().join("state.json");
    let registry = in_process_registry(&config).unwrap();
    let root = fs::WorkspaceRoot(std::fs::canonicalize(root).unwrap());
    registry.register(&root.0);
    (dir, root, registry)
}

fn edit(path: &str) -> Value {
    json!({"path": path, "edits": [{
        "target": {"hpath": [{"h": if path == "plan.md" { "Goals" } else { "Notes" }}]},
        "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
    }]})
}

/// Run `op` through the door with a reader's observation of a late rewrite
/// parked between the door's observation and its settle step (an edit to
/// `other.md` pending at entry when `pending`), then one detect cycle. The
/// frames, roots named: START the baseline, PRE the door's observation, MID
/// the reader's, FIN the disk at the end.
fn race(op: &Value, pending: bool) -> Vec<Told> {
    let (_dir, root, registry) = setup();
    let ring = registry.ring(&root.0);
    let (start, _) = ring.prime(&root, &registry).unwrap();
    let cache = registry.domain_cache(&root.0);
    let mut pre = start.clone();
    if pending {
        std::fs::write(root.0.join("other.md"), "# Other, edited before the door\n").unwrap();
        pre = observe_past(&registry, &root, &cache, &start);
    }
    let (announced, resume) = ring.pause_next_door_observation();
    let op = wire_serve::decode::decode(op.as_object().unwrap(), Rev::V3).unwrap();
    let mid = thread::scope(|scope| {
        let door =
            scope.spawn(|| dispatch_read(&registry, Some(&root.0), &mut None, Some(1), op, true));
        announced.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(
            root.0.join("late.md"),
            "# Late, rewritten by an outside writer\n",
        )
        .unwrap();
        let mid = observe_past(&registry, &root, &cache, &pre);
        resume.send(()).unwrap();
        let landed = door.join().unwrap();
        assert!(landed.is_ok(), "the door refused: {landed:?}");
        mid
    });
    ring.cycle_now(&root, &registry).unwrap();
    let fin = Root(fs::domain_snapshot(&root).unwrap().1.0);
    assert_ne!(pre, mid, "the reader saw the late rewrite");
    let name = |at: &Root| {
        [
            ("START", &start),
            ("PRE", &pre),
            ("MID", &mid),
            ("FIN", &fin),
        ]
        .into_iter()
        .find(|(_, root)| *root == at)
        .map_or_else(|| format!("{at:?}"), |(name, _)| name.to_owned())
    };
    let frames = ring.frames_after(0);
    assert_eq!(
        frames.last().map(|frame| &frame.delta.root_after),
        Some(&fin),
        "the cursor ends at the disk"
    );
    frames
        .iter()
        .map(|frame| {
            let mut paths: Vec<String> = frame
                .delta
                .files
                .iter()
                .map(|file| file.path.0.clone())
                .collect();
            paths.sort();
            (
                name(&frame.delta.root_before),
                name(&frame.delta.root_after),
                paths,
            )
        })
        .collect()
}

fn told(before: &str, after: &str, paths: &[&str]) -> Told {
    (
        before.to_owned(),
        after.to_owned(),
        paths.iter().map(|path| (*path).to_owned()).collect(),
    )
}

fn splice() -> Value {
    let mut op = edit("plan.md");
    op["op"] = json!("splice");
    op["force"] = json!(true);
    op
}

fn set() -> Value {
    json!({"op": "splice", "force": true, "files": [edit("plan.md"), edit("notes.md")]})
}

fn create() -> Value {
    json!({"op": "create", "path": "born.md", "body": "# Born\n"})
}

#[test]
fn a_splice_chains_onto_the_settle_frame_of_a_moved_memo() {
    assert_eq!(
        race(&splice(), true),
        [
            told("START", "MID", &["late.md", "other.md"]),
            told("MID", "FIN", &["plan.md"]),
        ]
    );
    assert_eq!(
        race(&splice(), false),
        [
            told("START", "MID", &["late.md"]),
            told("MID", "FIN", &["plan.md"]),
        ]
    );
}

#[test]
fn a_set_chains_onto_the_settle_frame_of_a_moved_memo() {
    assert_eq!(
        race(&set(), true),
        [
            told("START", "MID", &["late.md", "other.md"]),
            told("MID", "FIN", &["notes.md", "plan.md"]),
        ]
    );
    assert_eq!(
        race(&set(), false),
        [
            told("START", "MID", &["late.md"]),
            told("MID", "FIN", &["notes.md", "plan.md"]),
        ]
    );
}

/// Today's degrade, pinned: `create` starts its frame at the door's own
/// observation, so it breaks the chain twice and the detector re-names the
/// born file.
#[test]
fn a_create_through_a_moved_memo_takes_the_degrade() {
    assert_eq!(
        race(&create(), true),
        [
            told("START", "MID", &["late.md", "other.md"]),
            told("PRE", "FIN", &["born.md"]),
            told("MID", "FIN", &["born.md"]),
        ]
    );
    assert_eq!(
        race(&create(), false),
        [
            told("START", "MID", &["late.md"]),
            told("START", "FIN", &["born.md"]),
            told("MID", "FIN", &["born.md"]),
        ]
    );
}
