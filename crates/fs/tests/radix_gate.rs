//! The flat-100k operation-count gate (merkle-spec §4.2.4; merged plan §7(c);
//! codex gate 9's same-vertex-count discipline): one change in a flat
//! directory re-hashes a number of vertices bounded by the changed NAME,
//! never by directory width. The counts below are exact and deterministic —
//! a green run IS the published table.
//!
//! Public-surface only, deliberately: these tests name child entries and
//! directory values, and no vertex, bucket, or slot handle exists to reach
//! anything below a path node (§4.3 — vertices are hash-law internals). This
//! file is the compile-time receipt of that posture.

use fs::radix::{ChildKind, RadixChildMap};

/// The §7(c) slope-gate widths.
const WIDTHS: [usize; 4] = [100, 1_000, 8_000, 100_000];

fn h(seed: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&seed.to_le_bytes());
    out
}

/// The same-vertex-count arm, constructed so the changed entry's key path is
/// IDENTICAL at every width: fillers all share the first byte `f`, the target
/// starts with `t`, so the root fans `{f, t}` and the target's path is the
/// root plus its own vertex — exactly 2 re-hashes, whether 100 or 100,000
/// siblings sit under `f`. Width appears nowhere, in count OR in bytes.
#[test]
fn one_change_rehashes_the_same_vertex_count_at_every_width() {
    let mut table: Vec<(usize, u64, u64)> = Vec::new();
    for width in WIDTHS {
        let mut map = RadixChildMap::new();
        for i in 0..width {
            map.set(
                format!("f{i:07}.md").as_bytes(),
                ChildKind::File,
                h(u64::try_from(i).expect("width fits u64")),
            );
        }
        map.set(b"target.md", ChildKind::File, h(1));
        let (v0, b0) = (map.vertex_hashes(), map.hashed_bytes());
        map.set(b"target.md", ChildKind::File, h(2));
        table.push((width, map.vertex_hashes() - v0, map.hashed_bytes() - b0));
    }
    eprintln!("RADIX-GATE same-vertex-count (width, vertex re-hashes, bytes): {table:?}");
    for (width, ops, bytes) in &table {
        assert_eq!(
            *ops, 2,
            "width {width}: the key path is target vertex + root = 2, got {ops} ({bytes} B)"
        );
    }
    assert!(
        table
            .windows(2)
            .all(|w| (w[0].1, w[0].2) == (w[1].1, w[1].2)),
        "per-change work must not move with width: {table:?}"
    );
}

/// The realistic arm: a flat directory of `2026-08-15-NNNNNN.md` names (the
/// spec's own compression example — the shared prefix collapses into one
/// `ext`, divergence fans below it). Update, insert, and delete each touch a
/// bounded key path at every width, and the delete restores the exact prior
/// value — §4.2.2's re-canonicalization demonstrated at width 100,000.
#[test]
fn flat_directory_update_insert_delete_stay_bounded() {
    let mut rows: Vec<(usize, u64, u64, u64)> = Vec::new();
    for width in WIDTHS {
        let mut map = RadixChildMap::new();
        for i in 0..width {
            map.set(
                format!("2026-08-15-{i:06}.md").as_bytes(),
                ChildKind::File,
                h(u64::try_from(i).expect("width fits u64")),
            );
        }
        let mid = format!("2026-08-15-{:06}.md", width / 2);
        let v0 = map.vertex_hashes();
        map.set(mid.as_bytes(), ChildKind::File, h(0xdead));
        let update_ops = map.vertex_hashes() - v0;

        let value_before = map.dir_value();
        let v0 = map.vertex_hashes();
        map.set(b"2026-08-15-zzz.md", ChildKind::File, h(0xbeef));
        let insert_ops = map.vertex_hashes() - v0;

        let v0 = map.vertex_hashes();
        assert!(map.remove(b"2026-08-15-zzz.md", ChildKind::File));
        let delete_ops = map.vertex_hashes() - v0;
        assert_eq!(
            map.dir_value(),
            value_before,
            "width {width}: a removed entry must leave no trace"
        );
        rows.push((width, update_ops, insert_ops, delete_ops));
    }
    eprintln!("RADIX-GATE flat-dir ops (width, update, insert, delete): {rows:?}");
    for (width, update_ops, insert_ops, delete_ops) in &rows {
        for (label, ops) in [
            ("update", update_ops),
            ("insert", insert_ops),
            ("delete", delete_ops),
        ] {
            assert!(
                *ops <= 12,
                "width {width}: {label} re-hashed {ops} vertices — the key path \
                 is bounded by the name, never by the {width} siblings"
            );
        }
    }
}

/// The live-calibration arm: width 428 — the live-corpus maximum directory
/// width (k3 census, merged plan §4.2) — and width 100,000 under
/// uniform-random 16-hex names (multiplicative-scramble bijection, so names
/// are unique and leading bytes spread across the full hex alphabet).
#[test]
fn uniform_names_at_live_and_flat100k_widths() {
    let name = |i: usize| {
        format!(
            "{:016x}.md",
            (u64::try_from(i).expect("width fits u64")).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        )
    };
    let mut rows: Vec<(usize, u64, u64)> = Vec::new();
    for width in [428, 100_000] {
        let mut map = RadixChildMap::new();
        for i in 0..width {
            map.set(
                name(i).as_bytes(),
                ChildKind::File,
                h(u64::try_from(i).expect("width fits u64")),
            );
        }
        let target = name(width / 3);
        let v0 = map.vertex_hashes();
        let b0 = map.hashed_bytes();
        let t0 = std::time::Instant::now();
        map.set(target.as_bytes(), ChildKind::File, h(0xf00d));
        let dt = t0.elapsed();
        eprintln!(
            "RADIX-GATE uniform width {width}: {} vertex re-hashes, {} B, {dt:?}",
            map.vertex_hashes() - v0,
            map.hashed_bytes() - b0
        );
        rows.push((width, map.vertex_hashes() - v0, map.hashed_bytes() - b0));
    }
    for (width, ops, bytes) in &rows {
        assert!(
            *ops <= 8,
            "width {width}: uniform-name update re-hashed {ops} vertices ({bytes} B)"
        );
        assert!(
            *bytes <= 32 * 1024,
            "width {width}: per-change bytes are fanout-bounded (§4.2.4 ≈ 8.5 KiB \
             per vertex at full fanout), got {bytes}"
        );
    }
}

/// §5/§5.1 worked example under law 2 — byte-identity against the values
/// pinned in `docs/node-rev-merkle-spec.md` §5.1, generated independently by
/// `worked-example-gen.go` (a second implementation in a second language).
#[test]
fn spec_worked_example_law2_byte_identity() {
    let x_v0: &[u8] =
        b"---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body\n";
    let x_v2: &[u8] =
        b"---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body v2\n";
    let notes: &[u8] = b"# Notes\n\nhello\n";

    let hex = |v: [u8; 32]| blake3::Hash::from_bytes(v).to_hex().to_string();

    let mut tasks = RadixChildMap::new();
    tasks.set(b"x.md", ChildKind::File, *blake3::hash(x_v0).as_bytes());
    let mut root = RadixChildMap::new();
    root.set(
        b"notes.md",
        ChildKind::File,
        *blake3::hash(notes).as_bytes(),
    );
    root.set(b"tasks", ChildKind::Dir, tasks.dir_value());

    assert_eq!(hex(tasks.dir_value()), PIN_DIR_TASKS);
    assert_eq!(hex(root.dir_value()), PIN_FINGERPRINT);

    // The §5 incremental splice: Beta's body moves, one path recomputes.
    tasks.set(b"x.md", ChildKind::File, *blake3::hash(x_v2).as_bytes());
    root.set(b"tasks", ChildKind::Dir, tasks.dir_value());
    assert_eq!(hex(tasks.dir_value()), PIN_DIR_TASKS_V2);
    assert_eq!(hex(root.dir_value()), PIN_FINGERPRINT_V2);
}

// §5.1 pinned values (spec `docs/node-rev-merkle-spec.md` §5.1): generated
// by `worked-example-gen.go`'s law-2 arm and re-derived here byte-for-byte —
// two implementations, two languages, one encoding.
const PIN_DIR_TASKS: &str = "ef0e7e2eca3cacfcc3bf8fded1454d65645a5a20359c770d6e2dea009d285bd2";
const PIN_FINGERPRINT: &str = "d53c447167825d40f442c65b10f5ae2c6176a49e1e2d8237902d7eaa3008319e";
const PIN_DIR_TASKS_V2: &str = "e4f51f04970d9feb5c680de5534e1824b27d2660577395e5fadcd9d82fb8a967";
const PIN_FINGERPRINT_V2: &str = "6aab1dd1ef89648508430e0ded866c6ad964b1074fc9b624d025f5c27d10fc58";
