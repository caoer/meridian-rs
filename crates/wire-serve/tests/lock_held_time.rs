//! Guarded-write lock-held time harness (card write-path-overlay, gate 3).
//!
//! `#[ignore]`d: a measurement instrument, not a CI gate. Run it on the CI
//! host against BOTH trees — the pre-change baseline and the resident-tree
//! write path — and record the printed numbers:
//!
//! ```text
//! cargo test -p wire-serve --test lock_held_time --release -- --ignored --nocapture
//! ```
//!
//! It times the whole `splice()` door call. Pre-flock work is path
//! validation only (sub-microsecond) and the flock is uncontended here, so
//! the door-call time IS the lock-held time to measurement precision.
//! Deliberately restricted to the public door surface so the same file
//! compiles unmodified on the pre-change tree.

use std::time::Instant;

use std::collections::BTreeMap;
use wire::{Edit, EditShape, HpathSeg, Path, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const CORPUS_FILES: usize = 2_000;
const BODY_PAD: usize = 1_024;
const TIMED_WRITES: usize = 20;

fn splice_args(path: &str, old: &str, new: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: Path(path.into()),
        actor: Some("alice".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![
                    HpathSeg {
                        h: "Alpha".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "Beta".into(),
                        n: None,
                    },
                ],
            },
            edit: EditShape::Match {
                old: old.into(),
                new: new.into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
        fields: BTreeMap::default(),
    }
}

#[test]
#[ignore = "measurement harness — run explicitly with --ignored --nocapture"]
fn measure_guarded_write_lock_held_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let pad = "lorem ipsum dolor sit amet ".repeat(BODY_PAD / 27);
    for i in 0..CORPUS_FILES {
        let sub = dir.path().join(format!("notes{:02}", i % 32));
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(
            sub.join(format!("member{i}.md")),
            format!("# Alpha\n\n## Beta\n\n{pad}\n"),
        )
        .expect("member");
    }
    std::fs::write(
        dir.path().join("target.md"),
        "# Alpha\n\n## Beta\n\nship by w0\n",
    )
    .expect("target");

    // Cold: the first guarded write in this process.
    let t = Instant::now();
    splice(
        &root,
        None,
        &splice_args("target.md", "w0", "w1"),
        &[],
        None,
    )
    .expect("cold splice");
    let cold = t.elapsed();

    let mut warm_ms: Vec<f64> = Vec::with_capacity(TIMED_WRITES);
    for step in 1..=TIMED_WRITES {
        let args = splice_args("target.md", &format!("w{step}"), &format!("w{}", step + 1));
        let t = Instant::now();
        splice(&root, None, &args, &[], None)
            .unwrap_or_else(|e| panic!("timed splice {step}: {e:?}"));
        warm_ms.push(t.elapsed().as_secs_f64() * 1e3);
    }
    warm_ms.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let median = warm_ms[warm_ms.len() / 2];

    // Scale bar: one full-corpus law-1 disk fold on the same tree.
    let t = Instant::now();
    let _ = wire_serve::ambient_root(&root).expect("oracle fold");
    let full_fold = t.elapsed();

    println!(
        "guarded-write lock-held time over {CORPUS_FILES} files \
         (~{BODY_PAD} B each), {TIMED_WRITES} timed writes:"
    );
    println!("  cold first write : {:8.2} ms", cold.as_secs_f64() * 1e3);
    println!(
        "  warm min/med/max : {:8.2} / {:8.2} / {:8.2} ms",
        warm_ms.first().expect("timed"),
        median,
        warm_ms.last().expect("timed"),
    );
    println!(
        "  one full corpus fold (scale bar): {:8.2} ms",
        full_fold.as_secs_f64() * 1e3
    );
}
