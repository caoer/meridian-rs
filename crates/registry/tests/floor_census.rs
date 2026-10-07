//! Calibrates each floor census against real deallocation (merkle-spec §6.4):
//! an owner is built, censused, then dropped, and the bytes its drop frees
//! are the measurement. A census must not be below it (the floor never
//! under-charges) and stays within `SLACK` times it (the `BTreeMap` node
//! bound is the loose term). One test, so no other test thread allocates
//! while it measures.
//!
//! Each owner is measured twice where it can be: at a few hundred members,
//! where the bound holds against real node fill, and tiny — every map at most
//! eleven entries, so one node whose bound exceeds it by twelve edges (96 B) —
//! with names of a few hundred bytes. A term smaller than even that slack
//! is checked as a difference: two owners alike but for the term, so the
//! node bounds cancel and the census must cover what the term adds.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::SeqCst);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::SeqCst);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            LIVE.fetch_add(size, Ordering::SeqCst);
            LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// How far above the measurement a census may sit.
const SLACK: usize = 3;

/// The bytes dropping `owner` frees.
fn freed<T>(owner: T) -> usize {
    let before = LIVE.load(Ordering::SeqCst);
    drop(owner);
    before.saturating_sub(LIVE.load(Ordering::SeqCst))
}

fn check(owner: &str, census: usize, measured: usize) {
    println!("census owner={owner} census={census} measured={measured}");
    assert!(measured > 0, "{owner}: nothing measured");
    assert!(
        census >= measured,
        "{owner}: census {census} is below the {measured} bytes its drop frees"
    );
    assert!(
        census <= measured.saturating_mul(SLACK),
        "{owner}: census {census} is more than {SLACK}x the {measured} bytes its drop frees"
    );
}

/// `with` and `without` are (census, measured) of two owners alike but for
/// `term`: the census must grow by at least what the owner's drop frees more.
fn check_delta(term: &str, with: (usize, usize), without: (usize, usize)) {
    let census = with.0.checked_sub(without.0).unwrap_or_else(|| {
        panic!(
            "{term}: census {} with the term is below {} without",
            with.0, without.0
        )
    });
    let measured = with.1.saturating_sub(without.1);
    println!("census term={term} census={census} measured={measured}");
    assert!(measured > 0, "{term}: nothing measured");
    assert!(
        census >= measured,
        "{term}: the census grows by {census} where the owner's drop frees {measured} more"
    );
}

fn member(n: usize) -> String {
    format!(
        "---\naliases: [alias-{n}, other {n}]\n---\n# Member {n}\n\nLinks [[member-{}]] and \
         [[member-{}#Part]].\n\n## Part\n\ntext ^b{n}\n",
        (n + 1) % 400,
        (n + 7) % 400
    )
}

/// A file-name stem long enough that a census dropping its bytes falls
/// below a tiny owner's drop.
fn long(stem: &str) -> String {
    format!("{stem}-{}", "n".repeat(200))
}

fn write_tree(root: &Path, files: &[(String, Vec<u8>)]) -> fs::WorkspaceRoot {
    for (path, bytes) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    fs::WorkspaceRoot(std::fs::canonicalize(root).unwrap())
}

fn docs(count: usize) -> model::Docs {
    (0..count)
        .map(|n| {
            let raw = member(n);
            let nodes = syntax::parse(&raw);
            (
                format!("dir-{}/member-{n}.md", n % 13),
                Arc::new(model::build(raw, nodes)),
            )
        })
        .collect()
}

fn frame(seq: u64, files: usize) -> wire::DeltaFrame {
    let rev = |n: usize| Some(wire::NodeRev(format!("{n:016x}")));
    wire::DeltaFrame {
        delta: wire::Delta {
            seq,
            root_before: wire::Root(format!("b3a:{seq:064x}")),
            root_after: wire::Root(format!("b3a:{:064x}", seq + 1)),
            actor: seq.is_multiple_of(2).then(|| "agent-0001".to_owned()),
            now: seq
                .is_multiple_of(2)
                .then(|| "2026-10-06T00:00:00Z".to_owned()),
            files: (0..files)
                .map(|n| wire::DeltaFile {
                    path: wire::Path(format!("dir/member-{n}.md")),
                    change: wire::FileChange::Modified,
                    from_path: None,
                    file_rev_before: rev(n),
                    file_rev_after: rev(n + 1),
                    nodes: vec![
                        wire::DeltaNode {
                            target: wire::SecRef::Hpath {
                                hpath: vec![wire::HpathSeg {
                                    h: format!("Part {n}").into(),
                                    n: None,
                                }],
                            },
                            change: wire::NodeChange::Edited,
                            node_rev_before: rev(n),
                            node_rev_after: rev(n + 2),
                            span_after: Some(wire::Span(0, 10)),
                        },
                        wire::DeltaNode {
                            target: wire::SecRef::Anchor {
                                anchor: format!("b{n}"),
                            },
                            change: wire::NodeChange::Added,
                            node_rev_before: None,
                            node_rev_after: rev(n + 3),
                            span_after: Some(wire::Span(10, 20)),
                        },
                    ],
                })
                .collect(),
        },
        effects: vec![wire::EffectEnvelope {
            intents: vec![wire::Intent {
                rule_id: "rule-a".to_owned(),
                seq: 1,
                action: "notify".to_owned(),
                target: Some("someone".to_owned()),
                severity: None,
                payload: Some("payload text".to_owned()),
                receipt: "receipt".to_owned(),
            }],
            narrowed: Vec::new(),
            findings: vec![wire::EffectFinding::ArmedFault {
                rule_id: Some("rule-b".to_owned()),
                detail: "detail".to_owned(),
            }],
            how: "armed".to_owned(),
        }],
        rescope: seq.is_multiple_of(3).then(|| wire::Rescope {
            cause: wire::Path("meridian/domain.md".to_owned()),
            unattested: 1,
            attested: 2,
        }),
        overflow: None,
    }
}

/// A tiny memo after a full observation, its (census, freed). Its tree is
/// refolded (the emptied dirty set keeps its root node). `unprobeable` puts a
/// regular file where `.meridian/` would go, so the calibration probe fails
/// whoever runs the test; `stamped` binds a clock closure the memo alone
/// owns, as the registry's (which captures its ring) is.
fn tiny_memo(root: &Path, stem: &str, unprobeable: bool, stamped: bool) -> (usize, usize) {
    let files = vec![
        (format!("{stem}a.md"), b"# A\n".to_vec()),
        (format!("d/{stem}b.md"), b"# B\n".to_vec()),
        (format!("d/e/{stem}c.md"), b"# C\n".to_vec()),
    ];
    let ws = write_tree(root, &files);
    if unprobeable {
        std::fs::write(root.join(".meridian"), b"").unwrap();
    }
    let mut memo = fs::DomainCache::new();
    memo.root(&ws).unwrap();
    assert_eq!(
        matches!(
            memo.calibration(),
            Some(fs::stable::Calibration::Unavailable { .. })
        ),
        unprobeable,
        "the fixture's calibration is unavailable exactly when unprobeable"
    );
    let ring = Arc::new(AtomicU64::new(7));
    if stamped {
        let seq = Arc::clone(&ring);
        memo.bind_stamps(
            "instance-0001",
            Arc::new(move || seq.load(Ordering::Acquire)),
        );
    }
    let census = memo.heap_bytes();
    let measured = freed(memo);
    drop(ring);
    (census, measured)
}

/// Tiny memos: one with every term, then each small term as a difference —
/// long entry names, an unavailable calibration's reason, a stamp binding —
/// and a resolved tree collision.
fn tiny_memos(tmp: &Path) {
    let names = long("");
    let (census, measured) = tiny_memo(&tmp.join("memo-full"), &names, true, true);
    check("tiny-domain-memo", census, measured);
    check_delta(
        "memo-entry-names",
        tiny_memo(&tmp.join("memo-long"), &names, true, true),
        tiny_memo(&tmp.join("memo-short"), "", true, true),
    );
    check_delta(
        "memo-calibration-reason",
        tiny_memo(&tmp.join("memo-unprobeable"), "", true, false),
        tiny_memo(&tmp.join("memo-probeable"), "", false, false),
    );
    check_delta(
        "memo-stamp-binding",
        tiny_memo(&tmp.join("memo-stamped"), "", false, true),
        tiny_memo(&tmp.join("memo-unstamped"), "", false, false),
    );

    let digest = [7; 32];
    let mut tree = fs::resident::ResidentTree::new();
    let dir = long("x");
    tree.set_leaf(Path::new(&dir), digest);
    tree.set_leaf(&Path::new(&dir).join("y.md"), digest);
    let _ = tree.fingerprint();
    tree.remove_leaf(Path::new(&dir));
    let _ = tree.fingerprint();
    let census = tree.heap_bytes();
    check("tiny-tree-collision-resolved", census, freed(tree));
}

/// One member over a store record: what its drop frees, the controller's
/// floor and the store's census release — its descriptor and the source
/// handle allocation the descriptor alone owns included.
fn controller_member() {
    let backing = tempfile::tempdir().unwrap();
    let controller = registry::residency::controller::Controller::new(0);
    let store = registry::raw_store::RawStore::new(backing.path()).unwrap();
    let member = controller.retain(Arc::new(store.retain(b"# One member\n").unwrap()));
    let floor = controller.report().floor;
    let store_heap = store.heap_bytes().unwrap();
    let measured = freed(member);
    let released = floor.saturating_sub(controller.report().floor)
        + store_heap.saturating_sub(store.heap_bytes().unwrap());
    check("controller-member", released, measured);
}

/// An eager generation's metadata, its bodies kept alive by a second map: the
/// census counts the generation's map, not its documents.
fn eager_engine(owner: &str, ws: &fs::WorkspaceRoot, unserved_rows: usize) {
    let (snapshot, leaves, folded) = fs::domain_snapshot_with_leaves(ws).unwrap();
    let (index, docs, unserved) = fs::build_corpus(snapshot);
    assert_eq!(unserved.len(), unserved_rows, "{owner}: unserved rows");
    let engine = registry::WorkspaceEngine::eager(Arc::new(index), docs, unserved, folded, leaves);
    let bodies = engine.eager_docs().unwrap().clone();
    let census = engine.heap_bytes(&mut registry::residency::census::Seen::default());
    check(owner, census, freed(engine));
    drop(bodies);
}

/// A primed watch state over retained baselines, its (census, measured):
/// what its drop releases from the controller and the store is their charge,
/// not the watch state's. `rules` gives its scope two custom rules; without,
/// the config file has the same length and no rules.
fn retained_watch(root: &Path, rules: bool) -> (usize, usize) {
    let mut files: Vec<(String, Vec<u8>)> = (0..6)
        .map(|n| {
            (
                format!("{}-{n}.md", long("m")),
                format!("# M{n}\n").into_bytes(),
            )
        })
        .collect();
    let config = format!(
        "---\nignore:\n  - \"{}/**\"\n  - \"!{}/keep.md\"\n---\n# Domain\n",
        long("drafts"),
        long("drafts")
    );
    let config = if rules {
        config
    } else {
        format!("# Domain\n\n{}\n", "x".repeat(config.len() - 12))
    };
    files.push(("meridian/domain.md".to_owned(), config.into_bytes()));
    let ws = write_tree(root, &files);
    let (_, leaves, folded) = fs::domain_snapshot_with_leaves(&ws).unwrap();
    let leaves: BTreeMap<String, [u8; 32]> = leaves
        .into_iter()
        .map(|(path, digest)| (path.to_str().unwrap().to_owned(), digest))
        .collect();
    let observed = wire::Root(folded.0);
    let backing = tempfile::tempdir().unwrap();
    let controller = registry::residency::controller::Controller::new(1 << 32);
    let store = Arc::new(registry::raw_store::RawStore::new(backing.path()).unwrap());
    let retention: Arc<dyn model::bytes::ByteRetention> = Arc::new(
        registry::residency::retention::Retention::new(controller.clone(), Arc::clone(&store)),
    );
    let mut watch = wire_serve::watch::WatchState::with_retention(&ws, Arc::clone(&retention));
    let snapshot =
        wire_serve::watch::PrimeSnapshot::take_with_retention(&ws, Arc::clone(&retention)).unwrap();
    assert!(wire_serve::watch::prime(&ws, &mut watch, snapshot, &leaves, &observed).unwrap());
    drop(leaves);
    let census = watch.heap_bytes();
    let before = controller.report();
    let store_before = store.heap_bytes().unwrap();
    let measured = freed(watch);
    let after = controller.report();
    let released = before.floor.saturating_sub(after.floor)
        + before.raw.saturating_sub(after.raw)
        + before.bodies.saturating_sub(after.bodies)
        + store_before.saturating_sub(store.heap_bytes().unwrap());
    (census, measured.saturating_sub(released))
}

/// A resident generation's metadata: its map and index first, with the slots
/// kept alive by a second map, then the slots themselves. A generation whose
/// slots are all counted already charges exactly its map, keys and index.
fn generation(owner: &str, docs: &model::Docs) {
    let backing = tempfile::tempdir().unwrap();
    let controller = registry::residency::controller::Controller::new(0);
    let store = registry::raw_store::RawStore::new(backing.path()).unwrap();
    let catalog =
        registry::residency::catalog::Catalog::capture(docs, &controller, &store).unwrap();
    let slots = catalog.slots().clone();

    let mut index = model::CorpusIndex::new();
    for (path, doc) in docs {
        index.insert(path, doc);
    }
    let index = Arc::new(index);
    let shared = registry::residency::catalog::Catalog::new(slots.clone(), Arc::clone(&index));
    let map = slots.keys().fold(
        model::allocation::btree_bytes::<String, Arc<registry::residency::catalog::Slot>>(
            slots.len(),
        ),
        |bytes, path| bytes + path.capacity(),
    );
    let mut seen = registry::residency::census::Seen::default();
    let _ = slots.heap_bytes(&mut seen);
    assert_eq!(
        shared.heap_bytes(&mut seen),
        map + model::allocation::arc_bytes::<model::CorpusIndex>() + index.heap_bytes(),
        "{owner}: a generation of counted slots charges its map, keys and index"
    );
    assert_eq!(
        shared.heap_bytes(&mut seen),
        map,
        "{owner}: a counted index is not charged again"
    );
    drop(seen);
    drop(shared);
    drop(index);

    let mut seen = registry::residency::census::Seen::default();
    let _ = slots.heap_bytes(&mut seen);
    let census = catalog.heap_bytes(&mut seen);
    drop(seen);
    check(&format!("{owner}-map-and-index"), census, freed(catalog));

    let mut seen = registry::residency::census::Seen::default();
    let census = slots.heap_bytes(&mut seen);
    drop(seen);
    let descriptors = controller.report().floor;
    let store_heap = store.heap_bytes().unwrap();
    let measured = freed(slots);
    // The members' descriptors are the controller's floor, the records the
    // store's census: what they release is not the slots' metadata.
    let released = descriptors.saturating_sub(controller.report().floor)
        + store_heap.saturating_sub(store.heap_bytes().unwrap());
    check(
        &format!("{owner}-slots"),
        census,
        measured.saturating_sub(released),
    );
}

#[test]
fn every_floor_census_covers_what_its_owner_frees() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("ws");
    let docs = docs(400);
    for (path, doc) in &docs {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, doc.raw.as_str()).unwrap();
    }
    let ws = fs::WorkspaceRoot(std::fs::canonicalize(&root).unwrap());

    // The name index, and one emptied by removal (its maps keep their
    // root nodes).
    let mut index = model::CorpusIndex::new();
    for (path, doc) in &docs {
        index.insert(path, doc);
    }
    let census = index.heap_bytes();
    check("corpus-index", census, freed(index));
    let raw = format!("---\naliases: [{}]\n---\n# Gone\n", long("alias"));
    let gone = model::build(raw.clone(), syntax::parse(&raw));
    let path = format!("{}.md", long("gone"));
    let mut index = model::CorpusIndex::new();
    index.insert(&path, &gone);
    index.remove(&path, &gone);
    let census = index.heap_bytes();
    check("tiny-corpus-index-emptied", census, freed(index));

    // A workspace memo after a full observation, then a tiny one.
    let mut config = registry::Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = tmp.path().join("daemon.sock");
    config.state_path = tmp.path().join("state.json");
    let host_registry = registry::in_process_registry(&config).unwrap();
    let memo = Arc::new(Mutex::new(fs::DomainCache::new()));
    host_registry
        .door_observation(&ws.0, &memo, std::time::Duration::from_secs(5))
        .unwrap();
    let census = memo.lock().unwrap().heap_bytes();
    let memo = Arc::into_inner(memo).unwrap().into_inner().unwrap();
    check("domain-memo", census, freed(memo));
    tiny_memos(tmp.path());

    // A ring holding frames with files, nodes, effects and re-scopes.
    let mut ring = wire_serve::ring::RootRing::new();
    for seq in 1..=64 {
        ring.advance(frame(seq, 1 + usize::try_from(seq % 9).unwrap()));
    }
    let census = ring.heap_bytes();
    check("root-ring", census, freed(ring));

    // A primed eager watch state (in-memory baseline bodies), then a tiny
    // retained one under custom rules.
    let (_, leaves, folded) = fs::domain_snapshot_with_leaves(&ws).unwrap();
    let leaves: BTreeMap<String, [u8; 32]> = leaves
        .into_iter()
        .map(|(path, digest)| (path.to_str().unwrap().to_owned(), digest))
        .collect();
    let observed = wire::Root(folded.0);
    let mut watch = wire_serve::watch::WatchState::new(&ws);
    let snapshot = wire_serve::watch::PrimeSnapshot::take(&ws).unwrap();
    assert!(wire_serve::watch::prime(&ws, &mut watch, snapshot, &leaves, &observed).unwrap());
    drop(leaves);
    let census = watch.heap_bytes();
    check("watch-state", census, freed(watch));
    let (census, measured) = retained_watch(&tmp.path().join("watch-rules"), true);
    check("tiny-retained-watch-state", census, measured);
    check_delta(
        "watch-scope-rules",
        (census, measured),
        retained_watch(&tmp.path().join("watch-no-rules"), false),
    );

    // Eager generations: the large corpus, then a tiny one with an unserved
    // member.
    eager_engine("eager-engine", &ws, 0);
    let mut files: Vec<(String, Vec<u8>)> = (0..4)
        .map(|n| {
            (
                format!("{}/{}-{n}.md", long("d"), long("m")),
                format!("---\naliases: [{}-{n}]\n---\n# M{n}\n", long("al")).into_bytes(),
            )
        })
        .collect();
    files.push((format!("bad/{}.md", long("latin1")), vec![0xff, 0xfe, b'x']));
    let tiny = write_tree(&tmp.path().join("tiny-eager"), &files);
    eager_engine("tiny-eager-engine", &tiny, 1);

    // One controller member, then resident generations: the large corpus,
    // then eleven members.
    controller_member();
    generation("generation", &docs);
    let tiny_docs: model::Docs = docs
        .iter()
        .take(11)
        .map(|(path, doc)| (format!("{path}-{}", long("p")), Arc::clone(doc)))
        .collect();
    generation("tiny-generation", &tiny_docs);
}
