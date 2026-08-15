//! Merkle law 2 — the fixed-256 radix child map (`docs/node-rev-merkle-spec.md`
//! §4.2; ruled by `decisions/2026-08-15-width-sharding-now.md`: fixed-256 from
//! the first scoped-token version, deferral rejected).
//!
//! One instance holds ONE directory's child set `C` — entries of
//! `(name bytes, kind, 32-byte hash)` — as the canonical radix trie of §4.2.1,
//! every vertex carrying its current §4.2.2 hash. The trie shape is a pure
//! function of the entry set: insertion and deletion history never affect
//! [`RadixChildMap::dir_value`], and a delete re-canonicalizes as if the entry
//! had never existed — no tombstone slots, no retained split points.
//!
//! The cost law lives in the data structure (§4.2.4): changing one entry
//! re-hashes the vertices on that entry's key path — bounded by the name's
//! byte length and the fixed 256 fanout, never by directory width. The
//! [`RadixChildMap::vertex_hashes`] instrument makes that assertable.
//!
//! **Vertices are hash-law internals (§4.3).** A caller premise names PATH
//! nodes only; nothing below a path node holds a scope. This module enforces
//! the posture structurally: the vertex type, its hash, and the trie shape
//! are private. The public surface accepts child entries and answers the
//! directory VALUE — no vertex or bucket handle exists to mint, compare, or
//! put on a wire.
//!
//! Carried over from law 1 unchanged (§4.2): blake3-256 (§1); exact on-disk
//! name bytes with zero normalization (§9 — a name is opaque bytes here, and
//! `0x2f` is a byte like any other); byte-order comparisons on unsigned byte
//! values (the trie induces the sort); leaf hashes stay the plain untagged
//! blake3 of file bytes (§3). Domain tags are §4.3's: `mrk2.vtx` under every
//! vertex, `mrk2.dir` around the directory value.

use std::mem;

/// The 8-byte ASCII domain tag opening every vertex pre-image (§4.3): no
/// terminator, no length prefix.
const TAG_VERTEX: &[u8; 8] = b"mrk2.vtx";
/// The 8-byte ASCII domain tag opening a directory-value pre-image (§4.3).
/// Differs from [`TAG_VERTEX`] at byte 5, so the two are prefix-free.
const TAG_DIR: &[u8; 8] = b"mrk2.dir";

/// A child entry's kind — the `kind_byte` of §4.2.2's one-terminal frame,
/// and the structural guard that keeps cross-kind hashes unconfusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildKind {
    /// A file child; its hash is the file's §3 leaf.
    File,
    /// A directory child; its hash is that directory's §4.2.3 value.
    Dir,
}

impl ChildKind {
    fn byte(self) -> u8 {
        match self {
            ChildKind::File => 0x00,
            ChildKind::Dir => 0x01,
        }
    }
}

/// Hash-work counters — instruments, not caches (the §4.2.4 cost law made
/// assertable). Monotonic; readers diff before/after.
#[derive(Default, Clone, Copy)]
struct Counters {
    vertex_hashes: u64,
    hashed_bytes: u64,
}

/// One directory's child map under merkle law 2: the canonical fixed-256
/// radix trie over the child names (§4.2.1), maintained incrementally with
/// every vertex hash current (§4.2.2).
///
/// Keys are exact on-disk name bytes. One name may hold BOTH a file and a
/// directory value — the §4.4 collision case, kept representable so no bytes
/// sit outside the integrity surface (the collision terminal is §4.2.2's
/// `0x02` frame). Linting the collision loudly and refusing to ADDRESS the
/// colliding path are the tree layer's laws, not this structure's.
///
/// [`Self::dir_value`] answers `blake3("mrk2.dir" ‖ vhash(root))` — for the
/// workspace root that value IS the workspace fingerprint (§4.2.3). The empty
/// map answers `blake3("mrk2.dir")`, legal only for the workspace root:
/// non-root empty directories are pruned bottom-up by the caller and never
/// reach this rule.
#[derive(Default)]
pub struct RadixChildMap {
    root: Option<Box<Vertex>>,
    ops: Counters,
}

impl RadixChildMap {
    /// The empty child map (`C = ∅`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True while `C = ∅`. The tree layer prunes an emptied non-root
    /// directory bottom-up (§4.2 carry-over list); this is its signal.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Insert or update the `(name, kind)` entry's 32-byte hash.
    ///
    /// A name already holding the OTHER kind becomes the §4.4 collision key:
    /// one terminal carrying both values. Re-hashes exactly the vertices on
    /// the name's key path (§4.2.4).
    pub fn set(&mut self, name: &[u8], kind: ChildKind, hash: [u8; 32]) {
        match &mut self.root {
            Some(root) => set_rec(root, name, 0, kind, hash, &mut self.ops),
            None => {
                self.root = Some(new_leaf(name.to_vec(), kind, hash, &mut self.ops));
            }
        }
    }

    /// The hash `kind` currently holds at `name`, `None` when no such entry
    /// exists. A read-only key-path walk — no vertex, bucket, or slot handle
    /// escapes (§4.3). The tree layer consults it for idempotent updates and
    /// for §4.4 collision detection (does the OTHER kind already hold this
    /// name?).
    #[must_use]
    pub fn get(&self, name: &[u8], kind: ChildKind) -> Option<[u8; 32]> {
        let mut v = self.root.as_deref()?;
        let mut pos = 0;
        loop {
            let rem = &name[pos..];
            let k = lcp(rem, &v.ext);
            if k < v.ext.len() {
                return None;
            }
            if rem.len() == v.ext.len() {
                return match (v.terminal, kind) {
                    (Terminal::One(k0, hash), _) if k0 == kind => Some(hash),
                    (Terminal::Both { file, .. }, ChildKind::File) => Some(file),
                    (Terminal::Both { dir, .. }, ChildKind::Dir) => Some(dir),
                    _ => None,
                };
            }
            let branch_pos = pos + v.ext.len();
            let slot = name[branch_pos];
            match v.children.binary_search_by_key(&slot, |(b, _)| *b) {
                Ok(i) => {
                    v = &v.children[i].1;
                    pos = branch_pos + 1;
                }
                Err(_) => return None,
            }
        }
    }

    /// Visit every `(name, kind, hash)` entry, names strictly ascending in
    /// byte order (the trie induces the sort, §4.2.1); a §4.4 collision key
    /// visits its File arm before its Dir arm. Read-only — no vertex, bucket,
    /// or slot handle escapes (§4.3): the caller sees child ENTRIES, which are
    /// the directory listing, never trie structure. O(width) over the map —
    /// the §4.3.1 forest expansion's listing read.
    pub(crate) fn for_each_entry(&self, f: &mut dyn FnMut(&[u8], ChildKind, &[u8; 32])) {
        fn walk(v: &Vertex, acc: &mut Vec<u8>, f: &mut dyn FnMut(&[u8], ChildKind, &[u8; 32])) {
            let base = acc.len();
            acc.extend_from_slice(&v.ext);
            match &v.terminal {
                Terminal::None => {}
                Terminal::One(kind, hash) => f(acc, *kind, hash),
                Terminal::Both { file, dir } => {
                    f(acc, ChildKind::File, file);
                    f(acc, ChildKind::Dir, dir);
                }
            }
            for (slot, child) in &v.children {
                acc.push(*slot);
                walk(child, acc, f);
                acc.pop();
            }
            acc.truncate(base);
        }
        if let Some(root) = &self.root {
            walk(root, &mut Vec::new(), f);
        }
    }

    /// Remove the `(name, kind)` entry; `false` when no such entry exists.
    ///
    /// The map re-canonicalizes as if the entry had never existed (§4.2.2's
    /// empty-bucket rules): a vertex left with one child and no terminal
    /// merges into that child, and an emptied slot vanishes from its parent's
    /// frame. Removing one kind of a §4.4 collision key leaves the other.
    pub fn remove(&mut self, name: &[u8], kind: ChildKind) -> bool {
        let Some(root) = &mut self.root else {
            return false;
        };
        match remove_rec(root, name, 0, kind, &mut self.ops) {
            Outcome::NotFound => false,
            Outcome::Removed => true,
            Outcome::VertexDead => {
                self.root = None;
                true
            }
        }
    }

    /// The directory's 32-byte value (§4.2.3): `blake3("mrk2.dir" ‖
    /// vhash(child map))`, or `blake3("mrk2.dir")` while `C = ∅` (the empty
    /// workspace root). For the workspace root this is the fingerprint.
    #[must_use]
    pub fn dir_value(&self) -> [u8; 32] {
        let mut pre = [0u8; 40];
        pre[..8].copy_from_slice(TAG_DIR);
        match &self.root {
            Some(root) => {
                pre[8..].copy_from_slice(&root.vhash);
                *blake3::hash(&pre).as_bytes()
            }
            None => *blake3::hash(&pre[..8]).as_bytes(),
        }
    }

    /// Vertex hashes computed since construction (monotonic instrument).
    ///
    /// Read before and after one operation and assert the difference: per
    /// §4.2.4 it is bounded by the changed name's byte length plus the
    /// re-canonicalization constant — sibling count appears nowhere.
    #[must_use]
    pub fn vertex_hashes(&self) -> u64 {
        self.ops.vertex_hashes
    }

    /// Pre-image bytes hashed since construction (monotonic instrument).
    /// Each vertex pre-image is bounded by the fixed 256 fanout (§4.2.4:
    /// ≈ 8.5 KiB at full fanout), never by directory width.
    #[must_use]
    pub fn hashed_bytes(&self) -> u64 {
        self.ops.hashed_bytes
    }
}

/// §4.2.2's `terminal_frame` — exactly one of three markers.
#[derive(Clone, Copy)]
enum Terminal {
    /// `0x00` — no terminal at this vertex.
    None,
    /// `0x01 ‖ kind_byte ‖ hash` — one terminal.
    One(ChildKind, [u8; 32]),
    /// `0x02 ‖ file_hash ‖ dir_hash` — the §4.4 collision terminal: both
    /// kinds, fixed order, no kind bytes (the order spells them).
    Both { file: [u8; 32], dir: [u8; 32] },
}

impl Terminal {
    /// Set `kind`'s value, keeping the other kind's if present (the §4.4
    /// collision key is how a name composes to both kinds).
    fn set(&mut self, kind: ChildKind, hash: [u8; 32]) {
        *self = match (*self, kind) {
            (Terminal::None, k) => Terminal::One(k, hash),
            (Terminal::One(ChildKind::File, _), ChildKind::File) => {
                Terminal::One(ChildKind::File, hash)
            }
            (Terminal::One(ChildKind::Dir, _), ChildKind::Dir) => {
                Terminal::One(ChildKind::Dir, hash)
            }
            (
                Terminal::One(ChildKind::File, file) | Terminal::Both { file, .. },
                ChildKind::Dir,
            ) => Terminal::Both { file, dir: hash },
            (Terminal::One(ChildKind::Dir, dir) | Terminal::Both { dir, .. }, ChildKind::File) => {
                Terminal::Both { file: hash, dir }
            }
        };
    }

    /// Clear `kind`'s value; `false` when that kind holds no value here.
    fn clear(&mut self, kind: ChildKind) -> bool {
        match (*self, kind) {
            (Terminal::None, _) => false,
            (Terminal::One(k0, _), k) => {
                if k0 == k {
                    *self = Terminal::None;
                    true
                } else {
                    false
                }
            }
            (Terminal::Both { dir, .. }, ChildKind::File) => {
                *self = Terminal::One(ChildKind::Dir, dir);
                true
            }
            (Terminal::Both { file, .. }, ChildKind::Dir) => {
                *self = Terminal::One(ChildKind::File, file);
                true
            }
        }
    }
}

/// One trie vertex — hash-law internal (§4.3), never public. `children` is
/// the occupied-slots-only frame: `(slot byte, child)` strictly ascending,
/// which is exactly §4.2.2's empty-bucket rule (an unoccupied slot
/// contributes nothing).
struct Vertex {
    ext: Vec<u8>,
    terminal: Terminal,
    children: Vec<(u8, Box<Vertex>)>,
    vhash: [u8; 32],
}

/// A fresh single-entry vertex, hashed.
fn new_leaf(ext: Vec<u8>, kind: ChildKind, hash: [u8; 32], ops: &mut Counters) -> Box<Vertex> {
    let mut v = Box::new(Vertex {
        ext,
        terminal: Terminal::One(kind, hash),
        children: Vec::new(),
        vhash: [0; 32],
    });
    rehash(&mut v, ops);
    v
}

/// §4.2.2's vertex pre-image: `"mrk2.vtx" ‖ varint(len(ext)) ‖ ext ‖
/// terminal_frame ‖ children_frame`. Varints are minimal-form unsigned
/// LEB128 (§4.2 definitions).
fn vertex_preimage(v: &Vertex) -> Vec<u8> {
    let mut pre = Vec::with_capacity(8 + 2 + v.ext.len() + 66 + 2 + v.children.len() * 33);
    pre.extend_from_slice(TAG_VERTEX);
    write_uleb128(&mut pre, v.ext.len());
    pre.extend_from_slice(&v.ext);
    match &v.terminal {
        Terminal::None => pre.push(0x00),
        Terminal::One(kind, hash) => {
            pre.push(0x01);
            pre.push(kind.byte());
            pre.extend_from_slice(hash);
        }
        Terminal::Both { file, dir } => {
            pre.push(0x02);
            pre.extend_from_slice(file);
            pre.extend_from_slice(dir);
        }
    }
    write_uleb128(&mut pre, v.children.len());
    for (slot, child) in &v.children {
        pre.push(*slot);
        pre.extend_from_slice(&child.vhash);
    }
    pre
}

/// Recompute and store `v`'s hash; count the work.
fn rehash(v: &mut Vertex, ops: &mut Counters) {
    let pre = vertex_preimage(v);
    v.vhash = *blake3::hash(&pre).as_bytes();
    ops.vertex_hashes += 1;
    ops.hashed_bytes += u64::try_from(pre.len()).unwrap_or(u64::MAX);
}

/// Longest common prefix length of two byte strings.
fn lcp(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Insert/update `(name, kind) → hash` in the subtree at `v`, which owns
/// `name[pos..]`. Re-hashes on unwind, so exactly the descent path (plus a
/// split's one relocated vertex) is recomputed.
fn set_rec(
    v: &mut Vertex,
    name: &[u8],
    pos: usize,
    kind: ChildKind,
    hash: [u8; 32],
    ops: &mut Counters,
) {
    let rem = &name[pos..];
    let k = lcp(rem, &v.ext);
    if k < v.ext.len() {
        // The new name diverges inside this vertex's `ext`: split at `k`.
        // The vertex's current content moves one level down under its old
        // continuation byte; its `ext` changed, so it re-hashes (its subtree
        // below is untouched and keeps every cached hash).
        let moved_slot = v.ext[k];
        let mut moved = Box::new(Vertex {
            ext: v.ext[k + 1..].to_vec(),
            terminal: mem::replace(&mut v.terminal, Terminal::None),
            children: mem::take(&mut v.children),
            vhash: [0; 32],
        });
        rehash(&mut moved, ops);
        v.ext.truncate(k);
        if rem.len() == k {
            // The name ends exactly at the split: it becomes this vertex's
            // terminal, with the moved vertex as the sole child.
            v.terminal = Terminal::One(kind, hash);
            v.children = vec![(moved_slot, moved)];
        } else {
            // A true branch: two children, slots strictly ascending.
            let new_slot = rem[k];
            let leaf = new_leaf(rem[k + 1..].to_vec(), kind, hash, ops);
            v.children = if new_slot < moved_slot {
                vec![(new_slot, leaf), (moved_slot, moved)]
            } else {
                vec![(moved_slot, moved), (new_slot, leaf)]
            };
        }
    } else if rem.len() == v.ext.len() {
        // The name terminates at this vertex.
        v.terminal.set(kind, hash);
    } else {
        // Descend on the next byte, or open its slot.
        let branch_pos = pos + v.ext.len();
        let slot = name[branch_pos];
        match v.children.binary_search_by_key(&slot, |(b, _)| *b) {
            Ok(i) => set_rec(&mut v.children[i].1, name, branch_pos + 1, kind, hash, ops),
            Err(i) => {
                let leaf = new_leaf(name[branch_pos + 1..].to_vec(), kind, hash, ops);
                v.children.insert(i, (slot, leaf));
            }
        }
    }
    rehash(v, ops);
}

/// Result of a removal inside the subtree.
enum Outcome {
    /// No `(name, kind)` entry — nothing changed, nothing re-hashed.
    NotFound,
    /// Removed; this subtree is re-canonicalized and re-hashed.
    Removed,
    /// Removed, and this vertex's subtree became empty: the parent must drop
    /// the slot (an emptied slot vanishes from its parent's frame, §4.2.2).
    VertexDead,
}

/// Remove `(name, kind)` from the subtree at `v`. On unwind, apply §4.2.2's
/// delete re-canonicalization and re-hash exactly the touched path.
fn remove_rec(
    v: &mut Vertex,
    name: &[u8],
    pos: usize,
    kind: ChildKind,
    ops: &mut Counters,
) -> Outcome {
    let rem = &name[pos..];
    let k = lcp(rem, &v.ext);
    if k < v.ext.len() {
        return Outcome::NotFound;
    }
    if rem.len() == v.ext.len() {
        if !v.terminal.clear(kind) {
            return Outcome::NotFound;
        }
        if matches!(v.terminal, Terminal::None) {
            match v.children.len() {
                0 => return Outcome::VertexDead,
                1 => merge_only_child(v),
                _ => {}
            }
        }
        rehash(v, ops);
        return Outcome::Removed;
    }
    let branch_pos = pos + v.ext.len();
    let slot = name[branch_pos];
    let Ok(i) = v.children.binary_search_by_key(&slot, |(b, _)| *b) else {
        return Outcome::NotFound;
    };
    match remove_rec(&mut v.children[i].1, name, branch_pos + 1, kind, ops) {
        Outcome::NotFound => Outcome::NotFound,
        Outcome::Removed => {
            rehash(v, ops);
            Outcome::Removed
        }
        Outcome::VertexDead => {
            v.children.remove(i);
            if matches!(v.terminal, Terminal::None) && v.children.len() == 1 {
                merge_only_child(v);
            }
            rehash(v, ops);
            Outcome::Removed
        }
    }
}

/// §4.2.2's delete rule: a vertex left with one child and no terminal merges
/// into that child — the prefix re-extends, and no split point is retained.
fn merge_only_child(v: &mut Vertex) {
    let Some((slot, child)) = v.children.pop() else {
        unreachable!("caller guarantees exactly one child");
    };
    let child = *child;
    v.ext.push(slot);
    v.ext.extend_from_slice(&child.ext);
    v.terminal = child.terminal;
    v.children = child.children;
}

/// Minimal-form unsigned LEB128 (§4.2 definitions): low 7 bits per byte,
/// high bit = "more". Emitting by value makes minimality structural — a
/// non-minimal varint is not a legal encoding, and none can be produced.
/// Crate-visible: the §4.3.1 forest fold ([`crate::forest`]) uses the same
/// varint law, so the two encodings cannot drift.
pub(crate) fn write_uleb128(out: &mut Vec<u8>, mut value: usize) {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::*;

    /// Shadow model of the entry set `C`: name → per-kind values. The §4.4
    /// collision key is one name with both arms occupied.
    #[derive(Debug, Default, Clone, Copy)]
    struct Val {
        file: Option<[u8; 32]>,
        dir: Option<[u8; 32]>,
    }

    type Shadow = BTreeMap<Vec<u8>, Val>;

    fn shadow_set(s: &mut Shadow, name: &[u8], kind: ChildKind, hash: [u8; 32]) {
        let v = s.entry(name.to_vec()).or_default();
        match kind {
            ChildKind::File => v.file = Some(hash),
            ChildKind::Dir => v.dir = Some(hash),
        }
    }

    fn shadow_remove(s: &mut Shadow, name: &[u8], kind: ChildKind) -> bool {
        let Some(v) = s.get_mut(name) else {
            return false;
        };
        let hit = match kind {
            ChildKind::File => v.file.take().is_some(),
            ChildKind::Dir => v.dir.take().is_some(),
        };
        if v.file.is_none() && v.dir.is_none() {
            s.remove(name);
        }
        hit
    }

    /// §4.2.1's `build(S, pos)` recursion, transliterated as the test oracle:
    /// a from-scratch pure function of the entry set, sharing no code with
    /// the incremental structure beyond the leaf helpers it checks.
    fn oracle_build(set: &[(&Vec<u8>, &Val)], pos: usize, vertices: &mut usize) -> [u8; 32] {
        assert!(!set.is_empty(), "build(S) demands nonempty S");
        let first = &set[0].0[pos..];
        let mut ext_len = first.len();
        for (name, _) in &set[1..] {
            ext_len = ext_len.min(lcp(first, &name[pos..]));
        }
        let posp = pos + ext_len;
        let terminals: Vec<&Val> = set
            .iter()
            .filter(|(n, _)| n.len() == posp)
            .map(|(_, v)| *v)
            .collect();
        assert!(terminals.len() <= 1, "names are unique within C");
        let mut groups: BTreeMap<u8, Vec<(&Vec<u8>, &Val)>> = BTreeMap::new();
        for &(n, v) in set {
            if n.len() > posp {
                groups.entry(n[posp]).or_default().push((n, v));
            }
        }
        let mut pre = Vec::new();
        pre.extend_from_slice(TAG_VERTEX);
        write_uleb128(&mut pre, ext_len);
        pre.extend_from_slice(&first[..ext_len]);
        match terminals.first() {
            None => pre.push(0x00),
            Some(Val {
                file: Some(f),
                dir: Some(d),
            }) => {
                pre.push(0x02);
                pre.extend_from_slice(f);
                pre.extend_from_slice(d);
            }
            Some(Val {
                file: Some(f),
                dir: None,
            }) => {
                pre.push(0x01);
                pre.push(0x00);
                pre.extend_from_slice(f);
            }
            Some(Val {
                file: None,
                dir: Some(d),
            }) => {
                pre.push(0x01);
                pre.push(0x01);
                pre.extend_from_slice(d);
            }
            Some(Val {
                file: None,
                dir: None,
            }) => unreachable!("shadow drops empty values"),
        }
        write_uleb128(&mut pre, groups.len());
        for (slot, group) in &groups {
            pre.push(*slot);
            pre.extend_from_slice(&oracle_build(group, posp + 1, vertices));
        }
        *vertices += 1;
        *blake3::hash(&pre).as_bytes()
    }

    /// §4.2.3 over the oracle recursion, with the §4.2.1 vertex-count bound
    /// asserted on the way.
    fn oracle_value(s: &Shadow) -> [u8; 32] {
        let mut pre = TAG_DIR.to_vec();
        if !s.is_empty() {
            let refs: Vec<(&Vec<u8>, &Val)> = s.iter().collect();
            let mut vertices = 0usize;
            let vhash = oracle_build(&refs, 0, &mut vertices);
            // < 2·|C| is the same integer fact as ≤ 2·|C| − 1 (§4.2.1).
            assert!(vertices < 2 * s.len(), "vertex count ≤ 2·|C| − 1");
            pre.extend_from_slice(&vhash);
        }
        *blake3::hash(&pre).as_bytes()
    }

    /// Walk the live structure and assert every invariant §4.2.1's recursion
    /// forces, plus cache coherence (every stored vhash equals a fresh
    /// §4.2.2 computation over the current frame).
    fn check_invariants(map: &RadixChildMap) {
        fn walk(v: &Vertex, vertices: &mut usize, names: &mut usize) {
            *vertices += 1;
            if matches!(v.terminal, Terminal::None) {
                assert!(
                    v.children.len() >= 2,
                    "a vertex with no terminal has ≥ 2 children (else ext was not longest)"
                );
            } else {
                *names += 1;
            }
            if v.children.is_empty() {
                assert!(
                    !matches!(v.terminal, Terminal::None),
                    "a vertex with no children has a terminal"
                );
            }
            assert!(v.children.len() <= 256, "fanout ≤ 256");
            assert!(
                v.children.windows(2).all(|w| w[0].0 < w[1].0),
                "slot bytes strictly ascending"
            );
            assert_eq!(
                v.vhash,
                *blake3::hash(&vertex_preimage(v)).as_bytes(),
                "stored vhash must equal a fresh computation"
            );
            for (_, child) in &v.children {
                walk(child, vertices, names);
            }
        }
        if let Some(root) = &map.root {
            let (mut vertices, mut names) = (0usize, 0usize);
            walk(root, &mut vertices, &mut names);
            assert!(names >= 1, "a live trie holds at least one key");
            // < 2·|C| is the same integer fact as ≤ 2·|C| − 1 (§4.2.1).
            assert!(vertices < 2 * names, "vertex count ≤ 2·|C| − 1");
        }
    }

    fn kind(dir: bool) -> ChildKind {
        if dir { ChildKind::Dir } else { ChildKind::File }
    }

    fn h(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    /// Byte-vector names over a tiny alphabet with short lengths, so prefix
    /// splits, merges, collisions, and the empty name all occur constantly.
    /// `0x2f` is included on purpose: a name is opaque bytes here (§9).
    fn name_strat() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(
            prop_oneof![Just(0u8), Just(b'a'), Just(b'b'), Just(b'/'), Just(0xff)],
            0..6,
        )
    }

    #[derive(Debug, Clone)]
    enum Op {
        Set(Vec<u8>, bool, u8),
        Remove(Vec<u8>, bool),
    }

    fn op_strat() -> impl Strategy<Value = Op> {
        prop_oneof![
            (name_strat(), any::<bool>(), any::<u8>()).prop_map(|(n, d, s)| Op::Set(n, d, s)),
            (name_strat(), any::<bool>()).prop_map(|(n, d)| Op::Remove(n, d)),
        ]
    }

    proptest! {
        /// The canonicality gate: after ANY operation history the incremental
        /// map equals the §4.2.1 batch recursion over the surviving entry
        /// set, every forced invariant holds, and removal reports match.
        #[test]
        fn history_never_affects_the_result(ops in prop::collection::vec(op_strat(), 0..40)) {
            let mut map = RadixChildMap::new();
            let mut shadow = Shadow::new();
            for op in &ops {
                match op {
                    Op::Set(n, d, s) => {
                        map.set(n, kind(*d), h(*s));
                        shadow_set(&mut shadow, n, kind(*d), h(*s));
                    }
                    Op::Remove(n, d) => {
                        let got = map.remove(n, kind(*d));
                        let want = shadow_remove(&mut shadow, n, kind(*d));
                        prop_assert_eq!(got, want, "removal report diverged");
                    }
                }
                prop_assert_eq!(map.is_empty(), shadow.is_empty());
            }
            check_invariants(&map);
            prop_assert_eq!(map.dir_value(), oracle_value(&shadow));
            // The read face agrees with the shadow at every surviving key —
            // and answers None at a name the shadow never held.
            for (name, val) in &shadow {
                prop_assert_eq!(map.get(name, ChildKind::File), val.file);
                prop_assert_eq!(map.get(name, ChildKind::Dir), val.dir);
            }
            prop_assert_eq!(map.get(b"never-inserted", ChildKind::File), None);
        }

        /// Insertion order independence, stated directly: one entry set
        /// through three histories (given order, reverse, overwrite-then-fix)
        /// folds to one value.
        #[test]
        fn insertion_order_never_matters(
            entries in prop::collection::btree_map(name_strat(), (any::<bool>(), any::<u8>()), 1..24)
        ) {
            let mut forward = RadixChildMap::new();
            for (n, (d, s)) in &entries {
                forward.set(n, kind(*d), h(*s));
            }
            let mut reverse = RadixChildMap::new();
            for (n, (d, s)) in entries.iter().rev() {
                reverse.set(n, kind(*d), h(*s));
            }
            let mut rewritten = RadixChildMap::new();
            for (n, (d, s)) in &entries {
                rewritten.set(n, kind(*d), h(s.wrapping_add(1)));
            }
            for (n, (d, s)) in entries.iter().rev() {
                rewritten.set(n, kind(*d), h(*s));
            }
            let value = forward.dir_value();
            prop_assert_eq!(reverse.dir_value(), value);
            prop_assert_eq!(rewritten.dir_value(), value);
            check_invariants(&forward);
            check_invariants(&reverse);
            check_invariants(&rewritten);
        }

        /// The empty-bucket delete rule: S ∪ X built then X removed is
        /// byte-identical to S built raw — as if X had never existed.
        #[test]
        fn delete_leaves_no_trace(
            base in prop::collection::btree_map(name_strat(), (any::<bool>(), any::<u8>()), 0..16),
            extras in prop::collection::btree_map(name_strat(), (any::<bool>(), any::<u8>()), 1..16),
        ) {
            let mut plain = RadixChildMap::new();
            for (n, (d, s)) in &base {
                plain.set(n, kind(*d), h(*s));
            }
            let mut churned = RadixChildMap::new();
            for (n, (d, s)) in &base {
                churned.set(n, kind(*d), h(*s));
            }
            for (n, (d, s)) in &extras {
                if !base.contains_key(n) {
                    churned.set(n, kind(*d), h(*s));
                }
            }
            for (n, (d, _)) in extras.iter().rev() {
                if !base.contains_key(n) {
                    prop_assert!(churned.remove(n, kind(*d)));
                }
            }
            prop_assert_eq!(churned.dir_value(), plain.dir_value());
            check_invariants(&churned);
        }
    }

    /// §4.2.3: the empty child map — the fold of the empty workspace root.
    #[test]
    fn empty_dir_value_is_the_bare_tag_hash() {
        assert_eq!(
            RadixChildMap::new().dir_value(),
            *blake3::hash(b"mrk2.dir").as_bytes()
        );
        assert!(RadixChildMap::new().is_empty());
    }

    /// §4.2.2 byte layout, hand-assembled with no helper in common: one
    /// file entry `x.md`.
    #[test]
    fn single_entry_preimage_bytes() {
        let mut map = RadixChildMap::new();
        map.set(b"x.md", ChildKind::File, h(7));

        let mut vtx = Vec::new();
        vtx.extend_from_slice(b"mrk2.vtx");
        vtx.push(4); // varint(len("x.md"))
        vtx.extend_from_slice(b"x.md");
        vtx.push(0x01); // one terminal
        vtx.push(0x00); // kind byte: file
        vtx.extend_from_slice(&h(7));
        vtx.push(0x00); // children_frame: n = 0
        let mut dir = Vec::new();
        dir.extend_from_slice(b"mrk2.dir");
        dir.extend_from_slice(blake3::hash(&vtx).as_bytes());
        assert_eq!(map.dir_value(), *blake3::hash(&dir).as_bytes());
    }

    /// §4.4 collision terminal: `0x02 ‖ file_hash ‖ dir_hash`, fixed order,
    /// no kind bytes — and clearing one kind restores the plain terminal,
    /// byte-identical to a map that never held it.
    #[test]
    fn collision_terminal_bytes_and_recovery() {
        let mut map = RadixChildMap::new();
        map.set(b"x", ChildKind::Dir, h(2));
        map.set(b"x", ChildKind::File, h(1));

        let mut vtx = Vec::new();
        vtx.extend_from_slice(b"mrk2.vtx");
        vtx.push(1);
        vtx.push(b'x');
        vtx.push(0x02); // collision terminal
        vtx.extend_from_slice(&h(1)); // file first
        vtx.extend_from_slice(&h(2)); // dir second
        vtx.push(0x00);
        let mut dir = Vec::new();
        dir.extend_from_slice(b"mrk2.dir");
        dir.extend_from_slice(blake3::hash(&vtx).as_bytes());
        assert_eq!(map.dir_value(), *blake3::hash(&dir).as_bytes());

        assert!(map.remove(b"x", ChildKind::File));
        let mut dir_only = RadixChildMap::new();
        dir_only.set(b"x", ChildKind::Dir, h(2));
        assert_eq!(map.dir_value(), dir_only.dir_value());
        check_invariants(&map);
    }

    /// A name past 127 bytes frames its `ext` length as a two-byte minimal
    /// varint.
    #[test]
    fn long_name_varint_is_minimal() {
        let name = vec![b'a'; 130];
        let mut map = RadixChildMap::new();
        map.set(&name, ChildKind::File, h(9));

        let mut vtx = Vec::new();
        vtx.extend_from_slice(b"mrk2.vtx");
        vtx.extend_from_slice(&[0x82, 0x01]); // uleb128(130), minimal form
        vtx.extend_from_slice(&name);
        vtx.push(0x01);
        vtx.push(0x00);
        vtx.extend_from_slice(&h(9));
        vtx.push(0x00);
        let mut dir = Vec::new();
        dir.extend_from_slice(b"mrk2.dir");
        dir.extend_from_slice(blake3::hash(&vtx).as_bytes());
        assert_eq!(map.dir_value(), *blake3::hash(&dir).as_bytes());
    }

    /// The LEB128 emitter against the standard vectors.
    #[test]
    fn uleb128_minimal_vectors() {
        let cases: [(usize, &[u8]); 6] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (300, &[0xac, 0x02]),
            (16384, &[0x80, 0x80, 0x01]),
        ];
        for (value, want) in cases {
            let mut out = Vec::new();
            write_uleb128(&mut out, value);
            assert_eq!(out.as_slice(), want, "uleb128({value})");
        }
    }

    /// The counters are live instruments: the first insert hashes exactly
    /// one vertex, and a not-found removal hashes none.
    #[test]
    fn counters_count_the_work() {
        let mut map = RadixChildMap::new();
        assert_eq!(map.vertex_hashes(), 0);
        map.set(b"a", ChildKind::File, h(1));
        assert_eq!(map.vertex_hashes(), 1);
        let before = map.vertex_hashes();
        assert!(!map.remove(b"missing", ChildKind::File));
        assert_eq!(map.vertex_hashes(), before, "a miss re-hashes nothing");
        assert!(map.hashed_bytes() > 0);
    }
}
