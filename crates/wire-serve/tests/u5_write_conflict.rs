//! U5 (M1 D8): DIRECT concurrent-writer coverage for the splice TOCTOU-gap
//! fix, OUTSIDE the replay harness (A-C2: a single-threaded replay corpus can
//! never contain the interleaves this unit exists to close — the gate would
//! pass precisely because it never exercises the change).
//!
//! What the fix guarantees under concurrency, and what this test pins:
//! - the committed file is NEVER torn: every landed state is a validated
//!   pre-image plus whole span edits (each token intact on its own line);
//! - a splice that loses the race refuses with the TYPED `write_conflict`
//!   frame (refresh) — never a panic, never a stale blind splice;
//! - a refused splice landed NOTHING (its token is absent).
//!
//! Pre-U6 residual (STATED, security-lens): the verify→rename window means a
//! racer that passed its verify can still be overwritten by a same-instant
//! rename — a LOST update (both return ok), never a torn file. So this test
//! asserts `present ⊆ ok`, not equality; U6's flock serializes cooperating
//! writers and upgrades the guarantee (its two-process test owns that half).

use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};

use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, PutAt, Recovery, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const PAGE: &str = "---\ntitle: Race\n---\n# Log\n\nseed line\n";

/// One racer's request: `put at:end` of a unique token under `# Log` —
/// guardless (no `if_root`, no `if_node_rev`), so every refusal that surfaces
/// is the engine's own conflict detection, not a caller CAS.
fn racer_args(i: usize) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: WPath("log.md".into()),
        actor: Some(format!("racer-{i}")),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![HpathSeg {
                    h: "Log".into(),
                    n: None,
                }],
            },
            edit: EditShape::Put {
                at: PutAt::End,
                text: format!("token-{i}\n"),
            },
            if_node_rev: None,
        }],
    }
}

#[test]
fn concurrent_splices_refuse_typed_and_never_tear() {
    const RACERS: usize = 8;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.md"), PAGE).expect("fixture");

    let barrier = Arc::new(Barrier::new(RACERS));
    let handles: Vec<_> = (0..RACERS)
        .map(|i| {
            let barrier = Arc::clone(&barrier);
            let root = fs::WorkspaceRoot(dir.path().to_path_buf());
            std::thread::spawn(move || {
                let args = racer_args(i);
                barrier.wait();
                (i, splice(&root, 0, &args, &[]).map(|_| ()))
            })
        })
        .collect();

    let mut ok = BTreeSet::new();
    let mut conflicts = 0usize;
    for handle in handles {
        // A panicking racer fails the join — the panic-free law, checked free.
        let (i, outcome) = handle.join().expect("no racer may panic");
        match outcome {
            Ok(()) => {
                ok.insert(i);
            }
            Err(e) => {
                // EVERY concurrency refusal is the one typed conflict frame.
                assert_eq!(
                    e.code,
                    ErrorCode::WriteConflict,
                    "racer {i}: concurrency refusals are write_conflict only, got {:?} ({:?})",
                    e.code,
                    e.message
                );
                assert_eq!(e.recovery, Recovery::Refresh, "write_conflict → refresh");
                conflicts += 1;
            }
        }
    }
    assert_eq!(ok.len() + conflicts, RACERS, "every racer resolved");
    assert!(!ok.is_empty(), "at least one racer lands (someone wins)");
    // Observability (run with --nocapture): how the race resolved this run.
    eprintln!("u5 race: ok={} write_conflict={conflicts}", ok.len());

    // The landed state is NEVER torn: the base page survives intact and every
    // extra line is one COMPLETE token (a stale blind splice would interleave
    // spans mid-line; pre-fix this is exactly the silent corruption mode).
    let after = std::fs::read_to_string(dir.path().join("log.md")).expect("read back");
    assert!(
        after.starts_with(PAGE.trim_end_matches('\n')) || after.starts_with(PAGE),
        "the base page bytes survive at the head:\n{after}"
    );
    let mut present = BTreeSet::new();
    for line in after.lines().skip_while(|l| *l != "seed line").skip(1) {
        if line.is_empty() {
            continue;
        }
        let token: usize = line
            .strip_prefix("token-")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("torn/foreign line in committed file: {line:?}"));
        assert!(
            present.insert(token),
            "token-{token} landed twice — a double splice"
        );
    }
    // No refused racer's token may be present (a refusal lands NOTHING).
    // Lost updates (ok minus present) are the STATED pre-U6 residual.
    assert!(
        present.is_subset(&ok),
        "present tokens {present:?} must all come from ok racers {ok:?}"
    );

    // Refused commits clean their staged temps — no litter beside the page.
    let litter: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(litter.is_empty(), "no staged temp survives: {litter:?}");
}
