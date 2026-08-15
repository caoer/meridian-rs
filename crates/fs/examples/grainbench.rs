//! grainbench — measures what the corpus root actually costs today, and what
//! an incrementally maintained tree would cost instead.
//!
//! Usage: cargo run --release --example grainbench -- <workspace-root> [iters]
//!
//! Lanes:
//!   A. `fs::domain_snapshot`      — the write path's `ambient_root` (full
//!      byte read + digest + fold; runs TWICE per write, inside the flock).
//!   B. `DomainCache::root` warm   — the currency pass (stat sweep + full fold).
//!   C. `merkle_root_of_leaves`    — the pure re-fold over already-memoized
//!      leaves (every currency pass pays this even when nothing changed).
//!   D. incremental prototype      — resident tree with per-directory cached
//!      folds: one leaf update + ancestor-chain refold. Root equality against
//!      lane C's is asserted every iteration.

use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("usage: grainbench <workspace-root> [iters]");
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let ws = fs::WorkspaceRoot(std::path::PathBuf::from(&root));

    // Lane A: the write path's ambient_root (domain_snapshot), full cost.
    let mut a_ms: Vec<f64> = Vec::new();
    let mut member_count = 0usize;
    let mut byte_count = 0usize;
    for _ in 0..iters.min(3) {
        let t = Instant::now();
        let (files, _root) = fs::domain_snapshot(&ws).expect("domain_snapshot");
        a_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        member_count = files.len();
        byte_count = files.iter().map(|(_, b)| b.len()).sum();
    }

    // Lane B: warm currency pass through the resident memo.
    let mut cache = fs::DomainCache::new();
    let first = Instant::now();
    let root_b = cache.root(&ws).expect("cache root cold");
    let cold_ms = first.elapsed().as_secs_f64() * 1000.0;
    let mut b_ms: Vec<f64> = Vec::new();
    for _ in 0..iters {
        let t = Instant::now();
        let r = cache.root(&ws).expect("cache root warm");
        b_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(r.0, root_b.0, "warm root drifted");
    }

    // Leaves for lanes C/D — one snapshot, reused.
    let (_files, leaves, folded) =
        fs::domain_snapshot_with_leaves(&ws).expect("snapshot with leaves");
    let named: Vec<(Vec<u8>, [u8; 32])> = leaves
        .iter()
        .map(|(p, d)| (path_bytes(p), *d))
        .collect();

    // Lane C: the pure fold over memoized leaves (what every currency pass
    // pays on top of the stat sweep, even with zero changes).
    let mut c_ms: Vec<f64> = Vec::new();
    for _ in 0..iters {
        let refs: Vec<(&[u8], [u8; 32])> =
            named.iter().map(|(n, d)| (n.as_slice(), *d)).collect();
        let t = Instant::now();
        let r = model::merkle_root_of_leaves(&refs, 0);
        c_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(strip_prefix(&r.0), strip_prefix(&folded.0), "pure fold drifted");
    }

    // Lane D: incremental — resident tree, cached folds, one-leaf update.
    let mut tree = IncTree::default();
    let build_t = Instant::now();
    for (name, digest) in &named {
        tree.insert(name, *digest);
    }
    let inc_root = tree.root();
    let build_ms = build_t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(hex(&inc_root), strip_prefix(&folded.0), "incremental tree root != model root");

    // Deepest member exercises the longest ancestor chain.
    let deepest = named
        .iter()
        .max_by_key(|(n, _)| n.iter().filter(|b| **b == b'/').count())
        .expect("nonempty corpus")
        .0
        .clone();
    let depth = deepest.iter().filter(|b| **b == b'/').count() + 1;
    let mut d_us: Vec<f64> = Vec::new();
    let mut flip = [0u8; 32];
    for i in 0..iters.max(100) {
        flip[0] = flip[0].wrapping_add(1);
        let t = Instant::now();
        tree.insert(&deepest, flip);
        let _r = tree.root();
        d_us.push(t.elapsed().as_secs_f64() * 1_000_000.0);
        let _ = i;
    }
    // Restore the true leaf and re-verify equality against the model fold.
    let true_leaf = named.iter().find(|(n, _)| *n == deepest).unwrap().1;
    tree.insert(&deepest, true_leaf);
    assert_eq!(hex(&tree.root()), strip_prefix(&folded.0), "restore drifted");

    println!("workspace: {root}");
    println!("members: {member_count}  bytes: {byte_count}  deepest-depth: {depth}");
    println!("A ambient_root (domain_snapshot, full read+digest+fold): {}", stats_ms(&a_ms));
    println!("   -> per guarded write TODAY: 2x A inside the write flock");
    println!("B currency pass warm (DomainCache::root: stat sweep + full fold): {}", stats_ms(&b_ms));
    println!("   (cold first build: {cold_ms:.1} ms)");
    println!("C pure fold over memoized leaves (merkle_root_of_leaves): {}", stats_ms(&c_ms));
    println!("D incremental: one leaf update + ancestor refold: {}", stats_us(&d_us));
    println!("   (resident tree first build from leaves: {build_ms:.1} ms)");
}

fn path_bytes(p: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    p.as_os_str().as_bytes().to_vec()
}

fn strip_prefix(token: &str) -> String {
    token.split(':').nth(1).unwrap_or(token).to_string()
}

fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn stats_ms(v: &[f64]) -> String {
    let (min, med, max) = three(v);
    format!("min {min:.1} ms / med {med:.1} ms / max {max:.1} ms (n={})", v.len())
}

fn stats_us(v: &[f64]) -> String {
    let (min, med, max) = three(v);
    format!("min {min:.1} us / med {med:.1} us / max {max:.1} us (n={})", v.len())
}

fn three(v: &[f64]) -> (f64, f64, f64) {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    (s[0], s[s.len() / 2], s[s.len() - 1])
}

/// Minimal resident merkle tree with per-directory cached folds — the
/// prototype of the fix. Same §12.2 encoding as `model::MerkleDir`, plus a
/// cached fold per directory invalidated along the ancestor chain on insert.
#[derive(Default)]
struct IncTree {
    root: IncDir,
}

#[derive(Default)]
struct IncDir {
    entries: BTreeMap<Vec<u8>, IncEntry>,
    cached: Option<[u8; 32]>,
}

enum IncEntry {
    File([u8; 32]),
    Dir(IncDir),
}

impl IncTree {
    fn insert(&mut self, path: &[u8], digest: [u8; 32]) {
        let segs: Vec<&[u8]> = path.split(|b| *b == b'/').filter(|s| !s.is_empty()).collect();
        let Some((file, dirs)) = segs.split_last() else { return };
        let mut dir = &mut self.root;
        dir.cached = None;
        for seg in dirs {
            let entry = dir
                .entries
                .entry(seg.to_vec())
                .or_insert_with(|| IncEntry::Dir(IncDir::default()));
            match entry {
                IncEntry::Dir(sub) => {
                    sub.cached = None;
                    dir = sub;
                }
                IncEntry::File(_) => return,
            }
        }
        dir.entries.insert(file.to_vec(), IncEntry::File(digest));
    }

    fn root(&mut self) -> [u8; 32] {
        Self::fold(&mut self.root)
    }

    fn fold(dir: &mut IncDir) -> [u8; 32] {
        if let Some(h) = dir.cached {
            return h;
        }
        let mut enc: Vec<u8> = Vec::new();
        for (name, entry) in &mut dir.entries {
            let (is_dir, hash) = match entry {
                IncEntry::File(d) => (false, *d),
                IncEntry::Dir(sub) if !sub.entries.is_empty() => (true, Self::fold(sub)),
                IncEntry::Dir(_) => continue,
            };
            uleb(&mut enc, name.len());
            enc.extend_from_slice(name);
            enc.push(u8::from(is_dir));
            enc.extend_from_slice(&hash);
        }
        let h = *blake3::hash(&enc).as_bytes();
        dir.cached = Some(h);
        h
    }
}

fn uleb(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let byte = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}
