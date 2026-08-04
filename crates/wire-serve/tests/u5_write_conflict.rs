//! U5+U6 (M1 D8/D9): DIRECT concurrent-writer coverage for the write path,
//! OUTSIDE the replay harness (A-C2: a single-threaded replay corpus can
//! never contain the interleaves these units exist to close — the gate would
//! pass precisely because it never exercises the change).
//!
//! What the combined fix guarantees under concurrency, and what this test pins:
//! - the committed file is NEVER torn: every landed state is a validated
//!   pre-image plus whole span edits (each token intact on its own line);
//! - a racer that finds the write flock held refuses the TYPED
//!   `workspace_busy` frame (retry) — never a wait, never a panic;
//! - under the flock cooperating racers cannot reach the D8 conflict at all
//!   (read#2→verify→rename is serialized), so NO lost update remains:
//!   `present == ok` EXACTLY. (`write_conflict` still guards out-of-band
//!   writers — pinned deterministically by the fs-seam gates in
//!   `crates/fs`, which drive drift between stage and rename by hand.)
//! - a refused splice landed NOTHING (its token is absent).

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
        origin: wire_serve::guard::Origin::InProcess,
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
        plan_edits: Vec::new(),
        pin: None,
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
                (i, splice(&root, 0, &args, &[], None).map(|_| ()))
            })
        })
        .collect();

    let mut ok = BTreeSet::new();
    let mut busy = 0usize;
    for handle in handles {
        // A panicking racer fails the join — the panic-free law, checked free.
        let (i, outcome) = handle.join().expect("no racer may panic");
        match outcome {
            Ok(()) => {
                ok.insert(i);
            }
            Err(e) => {
                // EVERY concurrency refusal is the one typed busy frame (U6:
                // the flock serializes cooperating writers; the D8 conflict is
                // unreachable for them, and nothing else may leak through).
                assert_eq!(
                    e.code,
                    ErrorCode::WorkspaceBusy,
                    "racer {i}: cooperating-racer refusals are workspace_busy only, got {:?} ({:?})",
                    e.code,
                    e.message
                );
                assert_eq!(e.recovery, Recovery::Retry, "workspace_busy → retry");
                busy += 1;
            }
        }
    }
    assert_eq!(ok.len() + busy, RACERS, "every racer resolved");
    assert!(!ok.is_empty(), "at least one racer lands (someone wins)");
    // Observability (run with --nocapture): how the race resolved this run.
    eprintln!("u5/u6 race: ok={} workspace_busy={busy}", ok.len());

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
    // U6 upgrade: EXACT equality — every ok racer's token landed (the flock
    // leaves NO lost-update window for cooperating writers) and no refused
    // racer's token is present (a refusal lands NOTHING).
    assert_eq!(
        present, ok,
        "under the flock, landed tokens equal ok racers exactly (no lost update)"
    );

    // Refused commits clean their staged temps — no litter beside the page.
    let litter: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(litter.is_empty(), "no staged temp survives: {litter:?}");
}
