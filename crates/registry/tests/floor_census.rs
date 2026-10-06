//! Calibrates each floor census against real deallocation (merkle-spec §6.4):
//! an owner is built, censused, then dropped, and the bytes its drop frees
//! are the measurement. A census must not be below it (the floor never
//! under-charges) and stays within `SLACK` times it (the `BTreeMap` node
//! bound is the loose term). One test, so no other test thread allocates
//! while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
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

fn member(n: usize) -> String {
    format!(
        "---\naliases: [alias-{n}, other {n}]\n---\n# Member {n}\n\nLinks [[member-{}]] and \
         [[member-{}#Part]].\n\n## Part\n\ntext ^b{n}\n",
        (n + 1) % 400,
        (n + 7) % 400
    )
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
            actor: (seq % 2 == 0).then(|| "agent-0001".to_owned()),
            now: (seq % 2 == 0).then(|| "2026-10-06T00:00:00Z".to_owned()),
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
                                    h: format!("Part {n}"),
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
        rescope: None,
        overflow: None,
    }
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

    // The name index.
    let mut index = model::CorpusIndex::new();
    for (path, doc) in &docs {
        index.insert(path, doc);
    }
    let census = index.heap_bytes();
    check("corpus-index", census, freed(index));

    // A workspace memo after a full observation.
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

    // A ring holding frames with files, nodes and effects.
    let mut ring = wire_serve::ring::RootRing::new();
    for seq in 1..=64 {
        ring.advance(frame(seq, 1 + (seq as usize % 9)));
    }
    let census = ring.heap_bytes();
    check("root-ring", census, freed(ring));

    // A primed eager watch state (in-memory baseline bodies).
    let (_, leaves, folded) = fs::domain_snapshot_with_leaves(&ws).unwrap();
    let leaves: std::collections::BTreeMap<String, [u8; 32]> = leaves
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

    // A resident generation's metadata: its map and index first, with the
    // slots kept alive by a second map, then the slots themselves.
    let backing = tempfile::tempdir().unwrap();
    let controller = registry::residency::controller::Controller::new(0);
    let store = registry::raw_store::RawStore::new(backing.path()).unwrap();
    let catalog =
        registry::residency::catalog::Catalog::capture(&docs, &controller, &store, 1 << 20)
            .unwrap();
    let slots = catalog.slots().clone();
    let mut seen = registry::residency::census::Seen::default();
    let _ = slots.heap_bytes(&mut seen);
    let census = catalog.heap_bytes(&mut seen);
    drop(seen);
    check("generation-map-and-index", census, freed(catalog));

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
        "generation-slots",
        census,
        measured.saturating_sub(released),
    );
}
