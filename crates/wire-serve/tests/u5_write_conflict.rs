//! U5+U6 (D8/D9): concurrent writers outside replay (A-C2).
//!
//! Pins: no torn file; busy → typed `workspace_busy`/retry; under flock
//! `present == ok` exactly (no lost update among cooperating writers);
//! refused token absent. Out-of-band `write_conflict`: `crates/fs` gates.

use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};

use std::collections::BTreeMap;
use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, PutAt, Recovery, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const PAGE: &str = "---\ntitle: Race\n---\n# Log\n\nseed line\n";

/// Guardless racer: unique `put at:end` token (engine conflict only, not CAS).
fn racer_args(i: usize) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
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
        fields: BTreeMap::default(),
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
                (i, splice(&root, None, &args, &[], None).map(|_| ()))
            })
        })
        .collect();

    let mut ok = BTreeSet::new();
    let mut busy = 0usize;
    for handle in handles {
        // Panic-free: join fails the suite.
        let (i, outcome) = handle.join().expect("no racer may panic");
        match outcome {
            Ok(()) => {
                ok.insert(i);
            }
            Err(e) => {
                // Cooperating refusal is only typed `workspace_busy` (U6).
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
    // Race outcome (visible under --nocapture).
    eprintln!("u5/u6 race: ok={} workspace_busy={busy}", ok.len());

    // No tear: base intact; each extra line is one complete token.
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
    // `present == ok` exactly (no lost update; refused writes land nothing).
    assert_eq!(
        present, ok,
        "under the flock, landed tokens equal ok racers exactly (no lost update)"
    );

    // No surviving staged temps.
    let litter: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(litter.is_empty(), "no staged temp survives: {litter:?}");
}
