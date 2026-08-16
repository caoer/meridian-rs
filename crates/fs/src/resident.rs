//! The resident merkle tree (`docs/node-rev-merkle-spec.md` §6.1) — the
//! interior folds [`model`]'s law-1 recursion computes and throws away, kept
//! addressable and updatable, radix-256 from birth (§4.2; ruled by
//! `decisions/2026-08-15-width-sharding-now.md`).
//!
//! One instance holds ONE workspace's directory nodes, keyed by
//! workspace-relative path bytes. Per node (§6.1): the §4.2 child map (a
//! [`RadixChildMap`], every vertex hash current), the cached 32-byte §4.2.3
//! directory value, the dirty flag, and the `last_seq` stamp (§6.3 —
//! advanced by [`ResidentTree::stamp_chain`] from [`crate::DomainCache`]'s
//! guarded write path, the same path that maintains the digests; the one
//! state owner of `docs/laws.md` binds who may advance it).
//!
//! **This structure does no I/O and serves no wire.** `fs::DomainCache` owns
//! the one instance and feeds it observed leaves; between construction step 3
//! and the cutover every SERVED token stays an old-law value (merged plan §6
//! step 3), so nothing here escapes to a client — the law-2 values exist to
//! be measured, tested, and built upon by the sibling cards.
//!
//! Cost law carried from the child map (§4.2.4): a leaf update re-hashes the
//! key path inside its directory's map plus one map update per filesystem
//! ancestor — bounded by name bytes and depth, never by directory width
//! (lane D class). Foreign changes mark the touched chain dirty; folds
//! recompute lazily on demand (§6.1), so maintenance cost follows change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::radix::{ChildKind, RadixChildMap};

/// One resident directory node. The dirty flag lives in
/// [`ResidentTree::dirty`] as set membership — one owner for the fact, and a
/// refold's "which descendants moved" question becomes a prefix range scan
/// instead of a full-tree sweep.
struct Node {
    /// The §4.2 child map. File entries are EAGER (written at
    /// [`ResidentTree::set_leaf`] time); directory entries are LAZY — a child
    /// directory's value lands at refold, which is what the dirty set tracks.
    map: RadixChildMap,
    /// Cached §4.2.3 directory value, current while the node is not dirty.
    fold: [u8; 32],
    /// §6.3 stamp: highest journal seq beneath this node, advanced by
    /// [`ResidentTree::stamp_chain`] (max-only, so cross-epoch leftovers can
    /// only add conservatism) and read through [`ResidentTree::stamp_at`].
    last_seq: u64,
}

impl Node {
    fn new() -> Node {
        let map = RadixChildMap::new();
        Node {
            fold: map.dir_value(),
            map,
            last_seq: 0,
        }
    }
}

/// One listed member (merkle-spec §4.3.1): the workspace-relative path BYTES
/// ([`crate::hash_name`] spelling) and the member's §3 leaf hash — the pair
/// the forest fold consumes and [`ResidentTree::files_under`] lists.
pub type MemberLeaf = (Vec<u8>, [u8; 32]);

/// A resolved scope fold (merkle-spec §7 scope rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFold {
    /// The 32-byte value at the scope: a directory's §4.2.3 value (the
    /// workspace fingerprint at the root scope) or a file's §3 leaf.
    Value([u8; 32]),
    /// A lawful path with no node — never created, emptied, pruned. A value,
    /// not an error (§7): absence of the whole prefix is still `absent`.
    Absent,
}

/// Why a scope cannot be evaluated (§4.4 / §7): the wire card spells every
/// arm `scope_unresolved` (fix class); the split names WHICH fact refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRefusal {
    /// Display spelling ([`crate::display_name`]) of the refusing path — the
    /// collision key, or the file a deeper premise tried to traverse.
    pub path: String,
    /// Which fact refused.
    pub reason: RefusalReason,
}

/// The two `scope_unresolved` facts the tree itself can answer (path-escape
/// is the address layer's, upstream of any tree question).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The scope names, or passes through, a §4.4 file/dir collision key: no
    /// premise can say WHICH kind, and an ambiguous premise is no premise.
    Collision,
    /// A non-final prefix segment exists as a FILE: the path conflicts in
    /// kind with an existing entry and cannot be traversed.
    KindConflict,
}

/// Instrument totals over every node's child map ([`RadixChildMap`]'s own
/// monotonic counters, summed) plus structure counts — the probe's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidentStats {
    /// Directory nodes resident (the workspace root included).
    pub dir_nodes: usize,
    /// Live §4.4 collision keys.
    pub collisions: usize,
    /// Vertex hashes computed since construction, all maps.
    pub vertex_hashes: u64,
    /// Pre-image bytes hashed since construction, all maps.
    pub hashed_bytes: u64,
}

/// The resident tree: every directory's child map with its cached fold,
/// dirty-set invalidation, §4.4 collision ledger, and scope resolution.
///
/// Keys are workspace-relative path BYTES ([`crate::hash_name`] spelling):
/// `/` separates segments, every other byte is a name byte, zero
/// normalization (§9). The workspace root is the empty key and always has a
/// node; the empty tree's fingerprint is `blake3("mrk2.dir")` (§4.2.3).
pub struct ResidentTree {
    /// Every resident directory node by path bytes.
    nodes: BTreeMap<Vec<u8>, Node>,
    /// Directory paths whose cached fold may be stale. Invariant: a dirty
    /// path's parent is dirty — marks walk to the root, refolds clear
    /// bottom-up — so a scoped refold is one contiguous range scan.
    dirty: BTreeSet<Vec<u8>>,
    /// §4.4 collision keys: paths holding BOTH a file and a directory.
    collisions: BTreeSet<Vec<u8>>,
}

impl Default for ResidentTree {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ResidentTree {
    /// Summary form: node internals are hash-law internals (§4.3) and never
    /// print; the stats line is the whole public truth.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentTree")
            .field("dir_nodes", &self.nodes.len())
            .field("dirty", &self.dirty.len())
            .field("collisions", &self.collisions.len())
            .finish()
    }
}

impl ResidentTree {
    /// The empty workspace tree: a root node with `C = ∅`.
    #[must_use]
    pub fn new() -> ResidentTree {
        let mut nodes = BTreeMap::new();
        nodes.insert(Vec::new(), Node::new());
        ResidentTree {
            nodes,
            dirty: BTreeSet::new(),
            collisions: BTreeSet::new(),
        }
    }

    /// Insert or update one file leaf; `false` when the tree already holds
    /// exactly this `(path, digest)` — an idempotent re-apply re-hashes
    /// nothing and marks nothing dirty.
    ///
    /// Ancestor directories compose as needed. A path reaching a name the
    /// OTHER kind already holds becomes a §4.4 collision key: both kinds
    /// enter the fold, the composition lints loudly (once per key), and
    /// [`Self::fold_at`] refuses the key and everything through it.
    pub fn set_leaf(&mut self, rel: &Path, digest: [u8; 32]) -> bool {
        let path = crate::hash_name(rel);
        let segs: Vec<&[u8]> = split_segments(path);
        let Some((name, dirs)) = segs.split_last() else {
            return false;
        };
        // Compose the directory chain, detecting collisions as they form.
        let mut dir_key: Vec<u8> = Vec::new();
        for seg in dirs {
            let parent_key = dir_key.clone();
            push_segment(&mut dir_key, seg);
            if !self.nodes.contains_key(&dir_key) {
                let collides = self
                    .node(&parent_key)
                    .map
                    .get(seg, ChildKind::File)
                    .is_some();
                if collides {
                    self.lint_collision(&dir_key);
                }
                self.nodes.insert(dir_key.clone(), Node::new());
            }
        }
        if self.node(&dir_key).map.get(name, ChildKind::File) == Some(digest) {
            return false;
        }
        let mut file_key = dir_key.clone();
        push_segment(&mut file_key, name);
        if self.nodes.contains_key(&file_key) {
            // A directory already lives at this name (its Dir entry may
            // still be pending refold — nodes, not the map, hold that fact).
            self.lint_collision(&file_key);
        }
        self.node_mut(&dir_key)
            .map
            .set(name, ChildKind::File, digest);
        self.mark_dirty(dir_key);
        true
    }

    /// Remove one file leaf; `false` when no such leaf exists. Emptied
    /// non-root directories prune bottom-up (§4.2 carry-over list) — the
    /// child map re-canonicalizes as if the entry had never existed, and an
    /// emptied path mints `absent` thereafter (§8).
    pub fn remove_leaf(&mut self, rel: &Path) -> bool {
        let path = crate::hash_name(rel);
        let segs: Vec<&[u8]> = split_segments(path);
        let Some((name, dirs)) = segs.split_last() else {
            return false;
        };
        let mut dir_key: Vec<u8> = Vec::new();
        for seg in dirs {
            push_segment(&mut dir_key, seg);
            if !self.nodes.contains_key(&dir_key) {
                return false;
            }
        }
        if !self.node_mut(&dir_key).map.remove(name, ChildKind::File) {
            return false;
        }
        let mut file_key = dir_key.clone();
        push_segment(&mut file_key, name);
        // The file arm of a collision key is gone; the key stops colliding.
        self.collisions.remove(&file_key);
        self.mark_dirty(dir_key.clone());
        // Prune emptied directories bottom-up; the root always survives.
        // Emptiness consults the child map AND the live child NODES: a
        // directory entry publishes into its parent's map only at refold, so
        // a freshly composed subtree can be invisible to the map while its
        // nodes are live — pruning on the map alone would orphan it (the
        // proptest-found `a/ab` + `a/a/a` + remove(`a/ab`) sequence).
        let mut key = dir_key;
        while !key.is_empty()
            && self.nodes.get(&key).is_some_and(|node| node.map.is_empty())
            && !self.has_child_nodes(&key)
        {
            self.nodes.remove(&key);
            self.dirty.remove(&key);
            // The dir arm of a collision key is gone with the node.
            self.collisions.remove(&key);
            let (parent, name) = split_parent(&key);
            let parent_key = parent.to_vec();
            // `false` when the Dir entry never landed (created and emptied
            // between refolds) — nothing to un-publish then. Never a
            // published empty value: pruning REMOVES the entry (§4.2.3 —
            // only the workspace root may be empty).
            self.node_mut(&parent_key).map.remove(name, ChildKind::Dir);
            self.mark_dirty(parent_key.clone());
            key = parent_key;
        }
        true
    }

    /// The file leaf currently held at `rel`, `None` when absent. Eager by
    /// construction — file entries land at [`Self::set_leaf`] time.
    #[must_use]
    pub fn file_leaf(&self, rel: &Path) -> Option<[u8; 32]> {
        let path = crate::hash_name(rel);
        let segs: Vec<&[u8]> = split_segments(path);
        let (name, dirs) = segs.split_last()?;
        let mut dir_key: Vec<u8> = Vec::new();
        for seg in dirs {
            push_segment(&mut dir_key, seg);
        }
        self.nodes.get(&dir_key)?.map.get(name, ChildKind::File)
    }

    /// The workspace fingerprint under law 2 (§4.2.3): refold what is dirty,
    /// answer the root's directory value. NOT servable before the cutover —
    /// the interim served token stays a law-1 value (merged plan §6 step 3).
    pub fn fingerprint(&mut self) -> [u8; 32] {
        self.refold(&[]);
        self.node(&[]).fold
    }

    /// Resolve one scope (merkle-spec §7 scope rows): the workspace root, a
    /// folder, a file leaf, or `absent` — refolding lazily under the scope.
    ///
    /// Refusals are the tree's two `scope_unresolved` facts: the scope names
    /// or passes through a §4.4 collision key, or a prefix conflicts in kind
    /// with an existing FILE. Path-escape refusal is the address layer's,
    /// upstream of this call.
    ///
    /// # Errors
    /// [`ScopeRefusal`] naming the refusing path and which fact refused.
    pub fn fold_at(&mut self, scope: &Path) -> Result<ScopeFold, ScopeRefusal> {
        let path = crate::hash_name(scope);
        let segs: Vec<&[u8]> = split_segments(path);
        let Some((last, interior)) = segs.split_last() else {
            return Ok(ScopeFold::Value(self.fingerprint()));
        };
        let mut key: Vec<u8> = Vec::new();
        for seg in interior {
            let parent_key = key.clone();
            push_segment(&mut key, seg);
            self.refuse_collision(&key)?;
            if self.nodes.contains_key(&key) {
                continue;
            }
            if self
                .node(&parent_key)
                .map
                .get(seg, ChildKind::File)
                .is_some()
            {
                return Err(ScopeRefusal {
                    path: crate::display_name(&key),
                    reason: RefusalReason::KindConflict,
                });
            }
            // The whole remaining prefix is absent — still `absent` (§7's
            // chain law: creation-guard plans stand on exactly this).
            return Ok(ScopeFold::Absent);
        }
        let parent_key = key.clone();
        push_segment(&mut key, last);
        self.refuse_collision(&key)?;
        if self.nodes.contains_key(&key) {
            self.refold(&key);
            return Ok(ScopeFold::Value(self.node(&key).fold));
        }
        match self.node(&parent_key).map.get(last, ChildKind::File) {
            Some(leaf) => Ok(ScopeFold::Value(leaf)),
            None => Ok(ScopeFold::Absent),
        }
    }

    /// Every file leaf at-or-under a directory `scope` (root = empty path):
    /// `(workspace-relative path bytes, §3 leaf hash)` pairs, strictly
    /// ascending by path bytes — the resident listings a §4.3.1 forest
    /// expansion reads. Zero byte I/O and zero refold work: file entries are
    /// eager by construction, and directories are enumerated from the node
    /// table, never from lazily published Dir entries — so this read needs no
    /// `&mut` and disturbs no dirty state.
    ///
    /// A scope that names a FILE or an absent path answers the EMPTY listing —
    /// set semantics (§4.3.1): nothing can match there now, and `n = 0` is a
    /// legal fold guarding continued emptiness. Contrast [`Self::fold_at`],
    /// the path-premise form, which refuses a kind conflict.
    ///
    /// # Errors
    /// [`ScopeRefusal`] when `scope` names, passes through, or CONTAINS a §4.4
    /// collision key — §4.3.1's stated precondition: a collision key cannot
    /// say WHICH kind is a member, and an ambiguous member set is no premise.
    pub fn files_under(&self, scope: &Path) -> Result<Vec<MemberLeaf>, ScopeRefusal> {
        let path = crate::hash_name(scope);
        let mut key: Vec<u8> = Vec::new();
        for seg in split_segments(path) {
            push_segment(&mut key, seg);
            self.refuse_collision(&key)?;
        }
        // A collision strictly below the scope is inside every candidate
        // member set — refuse it too (the containment half of the §4.3.1
        // precondition; the chain above covered at-and-through).
        let mut lo = key.clone();
        if !lo.is_empty() {
            lo.push(b'/');
        }
        if let Some(ck) = self
            .collisions
            .range(lo.clone()..)
            .take_while(|k| k.starts_with(&lo))
            .next()
        {
            return Err(ScopeRefusal {
                path: crate::display_name(ck),
                reason: RefusalReason::Collision,
            });
        }
        let mut out: Vec<MemberLeaf> = Vec::new();
        let mut collect = |dir_key: &[u8], node: &Node| {
            node.map.for_each_entry(&mut |name, kind, hash| {
                if kind == ChildKind::File {
                    let mut member = dir_key.to_vec();
                    push_segment(&mut member, name);
                    out.push((member, *hash));
                }
            });
        };
        match self.nodes.get(&key) {
            Some(node) => collect(&key, node),
            // A file or an absent path holds no directory node — the empty
            // member set, never a refusal (set semantics, doc law above).
            None => return Ok(out),
        }
        for (dir_key, node) in self
            .nodes
            .range(lo.clone()..)
            .take_while(|(k, _)| k.starts_with(&lo))
        {
            // At the root scope `lo` is empty and the range re-visits the
            // root node collected above — skip the one repeat.
            if *dir_key == key {
                continue;
            }
            collect(dir_key, node);
        }
        // Nodes interleave with their subdirectories' files in walk order
        // (`a/z.md` walks before `a/c/x.md`); the fold law wants path-byte
        // order (§4.3.1), so sort once here.
        out.sort_unstable();
        Ok(out)
    }

    /// Advance the §6.3 stamp on every ancestor directory node of leaf
    /// `rel`, the workspace root included: `last_seq = max(last_seq, seq)`.
    /// Ancestors pruned out from under a removal are skipped — the surviving
    /// chain still records that something died beneath it — and nothing is
    /// ever composed just to carry a stamp.
    pub fn stamp_chain(&mut self, rel: &Path, seq: u64) {
        let path = crate::hash_name(rel);
        let segs: Vec<&[u8]> = split_segments(path);
        let Some((_, dirs)) = segs.split_last() else {
            return;
        };
        let mut key: Vec<u8> = Vec::new();
        let mut chain: Vec<Vec<u8>> = vec![key.clone()];
        for seg in dirs {
            push_segment(&mut key, seg);
            chain.push(key.clone());
        }
        for key in chain {
            if let Some(node) = self.nodes.get_mut(&key) {
                node.last_seq = node.last_seq.max(seq);
            }
        }
    }

    /// The §6.3 stamp answering for `scope`, or `None` when stamps cannot
    /// answer there. A directory node answers its own `last_seq`; a live
    /// file leaf answers its parent directory's (every change to the file
    /// stamps that chain, so sibling churn only ever adds conservatism); a
    /// deleted, renamed-away, or never-created path has no current node to
    /// carry a stamp and never answers — stamps never answer for the dead.
    #[must_use]
    pub fn stamp_at(&self, scope: &Path) -> Option<u64> {
        let key = crate::hash_name(scope);
        let key: Vec<u8> = split_segments(key).join(&b'/');
        if let Some(node) = self.nodes.get(&key) {
            return Some(node.last_seq);
        }
        let (parent, name) = split_parent(&key);
        let parent = self.nodes.get(parent)?;
        parent
            .map
            .get(name, ChildKind::File)
            .map(|_| parent.last_seq)
    }

    /// Live §4.4 collision keys, display-spelled — the lint's queryable
    /// face (the loud half prints once per key at composition time).
    #[must_use]
    pub fn collision_paths(&self) -> Vec<String> {
        self.collisions
            .iter()
            .map(|key| crate::display_name(key))
            .collect()
    }

    /// Instrument totals (probe surface): structure counts plus every child
    /// map's monotonic hash-work counters, summed.
    #[must_use]
    pub fn stats(&self) -> ResidentStats {
        let (mut vertex_hashes, mut hashed_bytes) = (0u64, 0u64);
        for node in self.nodes.values() {
            vertex_hashes += node.map.vertex_hashes();
            hashed_bytes += node.map.hashed_bytes();
        }
        ResidentStats {
            dir_nodes: self.nodes.len(),
            collisions: self.collisions.len(),
            vertex_hashes,
            hashed_bytes,
        }
    }

    /// The node at `key`. Private on purpose: the lookup panics only on a
    /// violated internal invariant (every caller walks existing chains), and
    /// keeping the panic out of public bodies keeps the public surface
    /// panic-free by construction.
    fn node(&self, key: &[u8]) -> &Node {
        self.nodes.get(key).expect("caller walks existing chains")
    }

    /// [`Self::node`], mutable.
    fn node_mut(&mut self, key: &[u8]) -> &mut Node {
        self.nodes
            .get_mut(key)
            .expect("caller walks existing chains")
    }

    /// Whether any node lives strictly below `key` — the subtree-liveness
    /// half of the prune test (the child map alone cannot answer it: dir
    /// entries publish lazily at refold).
    fn has_child_nodes(&self, key: &[u8]) -> bool {
        if key.is_empty() {
            return self.nodes.len() > 1;
        }
        let mut lo = key.to_vec();
        lo.push(b'/');
        self.nodes
            .range(lo.clone()..)
            .next()
            .is_some_and(|(k, _)| k.starts_with(&lo))
    }

    /// Refuse a scope that names or passes through a §4.4 collision key:
    /// either way no premise can say WHICH kind, and an ambiguous premise is
    /// no premise.
    fn refuse_collision(&self, key: &[u8]) -> Result<(), ScopeRefusal> {
        if self.collisions.contains(key) {
            return Err(ScopeRefusal {
                path: crate::display_name(key),
                reason: RefusalReason::Collision,
            });
        }
        Ok(())
    }

    /// Mark `key`'s fold stale, and its whole ancestor chain — stopping at
    /// the first already-dirty ancestor, whose own chain is dirty by the
    /// invariant.
    fn mark_dirty(&mut self, key: Vec<u8>) {
        let mut key = key;
        loop {
            if !self.dirty.insert(key.clone()) || key.is_empty() {
                return;
            }
            key = split_parent(&key).0.to_vec();
        }
    }

    /// Recompute every stale fold under `scope` (inclusive), children before
    /// parents, publishing each changed directory value into its parent's
    /// child map. The scope's own new value publishes upward too, but the
    /// ancestors OUTSIDE the scope stay dirty — a later, wider refold picks
    /// them up.
    fn refold(&mut self, scope: &[u8]) {
        let batch: Vec<Vec<u8>> = if scope.is_empty() {
            self.dirty.iter().cloned().collect()
        } else {
            let mut keys: Vec<Vec<u8>> = Vec::new();
            if self.dirty.contains(scope) {
                keys.push(scope.to_vec());
            }
            let mut lo = scope.to_vec();
            lo.push(b'/');
            keys.extend(
                self.dirty
                    .range(lo.clone()..)
                    .take_while(|key| key.starts_with(&lo))
                    .cloned(),
            );
            keys
        };
        // Byte order puts a parent (proper prefix) before its children;
        // reverse iteration folds children first.
        for key in batch.iter().rev() {
            let node = self.nodes.get_mut(key).expect("a dirty key has a node");
            let value = node.map.dir_value();
            let changed = value != node.fold;
            node.fold = value;
            self.dirty.remove(key);
            if changed && !key.is_empty() {
                let split = split_parent(key);
                let parent = self
                    .nodes
                    .get_mut(split.0)
                    .expect("a live node's parent exists");
                parent.map.set(split.1, ChildKind::Dir, value);
            }
        }
    }

    /// Record a §4.4 collision key. LOUD once per key at composition time
    /// (spec: a named diagnostic, never silence — and serving continues);
    /// the ledger stays queryable via [`Self::collision_paths`].
    fn lint_collision(&mut self, key: &[u8]) {
        if self.collisions.insert(key.to_vec()) {
            eprintln!(
                "merkle: file/dir name collision at '{}' — both kinds stay in the fold; \
                 the path and every path through it refuse scope_unresolved at mint and \
                 at guard (merkle-spec §4.4)",
                crate::display_name(key)
            );
        }
    }
}

/// Path bytes → segments, exactly [`model`]'s law-1 split: on `0x2f`, empty
/// segments dropped (UTF-8 continuation bytes are ≥ `0x80`, so no multi-byte
/// sequence hides a `/`). One segmentation for both laws — the member sets
/// can never drift.
fn split_segments(path: &[u8]) -> Vec<&[u8]> {
    path.split(|b| *b == b'/')
        .filter(|s| !s.is_empty())
        .collect()
}

/// Append one segment to a path key (`/`-joined byte spelling).
fn push_segment(key: &mut Vec<u8>, seg: &[u8]) {
    if !key.is_empty() {
        key.push(b'/');
    }
    key.extend_from_slice(seg);
}

/// A non-root key's `(parent key, own name)` halves.
fn split_parent(key: &[u8]) -> (&[u8], &[u8]) {
    match key.iter().rposition(|b| *b == b'/') {
        Some(i) => (&key[..i], &key[i + 1..]),
        None => (&[], key),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proptest::prelude::*;

    use super::*;

    fn h(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// Short path strategy over a tiny grammar so shared prefixes, deep
    /// chains, kind swaps, and prunes occur constantly.
    fn path_strat() -> impl Strategy<Value = PathBuf> {
        prop::collection::vec(
            prop_oneof![Just("a"), Just("b"), Just("ab"), Just("c.md")],
            1..4,
        )
        .prop_map(|segs| PathBuf::from(segs.join("/")))
    }

    #[derive(Debug, Clone)]
    enum Op {
        Set(PathBuf, u8),
        Remove(PathBuf),
    }

    fn op_strat() -> impl Strategy<Value = Op> {
        prop_oneof![
            (path_strat(), any::<u8>()).prop_map(|(p, s)| Op::Set(p, s)),
            path_strat().prop_map(Op::Remove),
        ]
    }

    proptest! {
        /// Tree-level canonicality: after ANY operation history, the
        /// incremental tree's fingerprint equals a fresh tree built from the
        /// surviving leaf set — insertion, deletion, and refold history
        /// never affect the result. Interleaved scoped refolds must not
        /// disturb the outcome either.
        #[test]
        fn history_never_affects_the_fingerprint(
            ops in prop::collection::vec(op_strat(), 0..32),
            probe in path_strat(),
        ) {
            let mut tree = ResidentTree::new();
            let mut shadow: BTreeMap<PathBuf, [u8; 32]> = BTreeMap::new();
            for (i, op) in ops.iter().enumerate() {
                match op {
                    Op::Set(path, seed) => {
                        // Mirror the tree's collision-composition semantics
                        // in the shadow: a shadow leaf set may hold both
                        // `a` and `a/b` — the fresh build composes the same
                        // §4.4 collision, so the comparison stays honest.
                        tree.set_leaf(path, h(*seed));
                        shadow.insert(path.clone(), h(*seed));
                    }
                    Op::Remove(path) => {
                        let got = tree.remove_leaf(path);
                        let want = shadow.remove(path).is_some();
                        prop_assert_eq!(got, want, "removal report diverged");
                    }
                }
                if i % 3 == 0 {
                    // Interleaved scoped refold — must be invisible to the
                    // final fingerprint.
                    let _ = tree.fold_at(&probe);
                }
            }
            let mut fresh = ResidentTree::new();
            for (path, digest) in &shadow {
                fresh.set_leaf(path, *digest);
            }
            prop_assert_eq!(tree.fingerprint(), fresh.fingerprint());
        }
    }

    /// The empty tree's fingerprint is the §4.2.3 empty workspace root.
    #[test]
    fn empty_tree_is_the_bare_dir_tag() {
        assert_eq!(
            ResidentTree::new().fingerprint(),
            *blake3::hash(b"mrk2.dir").as_bytes()
        );
    }

    /// Idempotent re-apply: same `(path, digest)` twice — the second is a
    /// no-op that hashes nothing and dirties nothing.
    #[test]
    fn set_leaf_is_idempotent() {
        let mut tree = ResidentTree::new();
        assert!(tree.set_leaf(&p("a/b.md"), h(1)));
        tree.fingerprint();
        let before = tree.stats().vertex_hashes;
        assert!(!tree.set_leaf(&p("a/b.md"), h(1)));
        assert_eq!(tree.stats().vertex_hashes, before, "a no-op hashes nothing");
        assert!(tree.dirty.is_empty(), "a no-op dirties nothing");
    }

    /// The proptest-found orphaning sequence, pinned deterministically: a
    /// scoped refold publishes `a`'s value, a second leaf composes the
    /// UNPUBLISHED child `a/a`, and removing `a`'s last file must NOT prune
    /// `a` — its map looks empty but the child node is live. The fingerprint
    /// then equals the fresh build's, and the surviving subtree resolves.
    #[test]
    fn prune_spares_a_dir_with_unpublished_children() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a/ab"), h(1));
        let _ = tree.fold_at(&p("a"));
        tree.set_leaf(&p("a/a/a"), h(2));
        assert!(tree.remove_leaf(&p("a/ab")));
        assert_eq!(tree.fold_at(&p("a/a/a")), Ok(ScopeFold::Value(h(2))));
        let mut fresh = ResidentTree::new();
        fresh.set_leaf(&p("a/a/a"), h(2));
        assert_eq!(tree.fingerprint(), fresh.fingerprint());
        // And the cascade still prunes to empty once the subtree truly dies.
        assert!(tree.remove_leaf(&p("a/a/a")));
        assert_eq!(tree.fingerprint(), ResidentTree::new().fingerprint());
        assert_eq!(tree.fold_at(&p("a")), Ok(ScopeFold::Absent));
    }

    /// Emptied directories prune bottom-up and the emptied path answers
    /// `Absent`; the tree returns to the empty-root value.
    #[test]
    fn remove_prunes_and_mints_absent() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a/b/c.md"), h(1));
        let empty = ResidentTree::new().fingerprint();
        assert!(tree.remove_leaf(&p("a/b/c.md")));
        assert_eq!(tree.fingerprint(), empty);
        assert_eq!(tree.fold_at(&p("a/b")), Ok(ScopeFold::Absent));
        assert_eq!(tree.fold_at(&p("a")), Ok(ScopeFold::Absent));
        assert!(!tree.remove_leaf(&p("a/b/c.md")), "already gone");
    }

    /// Scoped refold refreshes the named subtree only; the root fold stays
    /// correct afterwards (ancestors outside the scope stay dirty until a
    /// wider refold).
    #[test]
    fn scoped_refold_then_root() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a/x.md"), h(1));
        tree.set_leaf(&p("b/y.md"), h(2));
        let root_before = tree.fingerprint();
        tree.set_leaf(&p("a/x.md"), h(3));
        let scoped = tree.fold_at(&p("a")).expect("a resolves");
        let mut fresh = ResidentTree::new();
        fresh.set_leaf(&p("a/x.md"), h(3));
        fresh.set_leaf(&p("b/y.md"), h(2));
        assert_eq!(scoped, fresh.fold_at(&p("a")).expect("a resolves"));
        let root_after = tree.fingerprint();
        assert_ne!(root_before, root_after);
        assert_eq!(root_after, fresh.fingerprint());
    }

    /// The §4.4 posture end to end: composition lints the key, both kinds
    /// stay in the fold (the fresh build with the same set matches), the key
    /// and every path through it refuse, siblings and the root still serve,
    /// and removing one kind clears the collision.
    #[test]
    fn collision_lints_refuses_and_clears() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a.md"), h(1));
        tree.set_leaf(&p("other.md"), h(9));
        // Compose a directory through the file name — the overlay shape.
        tree.set_leaf(&p("a.md/inner.md"), h(2));
        assert_eq!(tree.collision_paths(), vec!["a.md".to_owned()]);

        let refusal = tree.fold_at(&p("a.md")).expect_err("collision refuses");
        assert_eq!(refusal.reason, RefusalReason::Collision);
        assert_eq!(refusal.path, "a.md");
        let through = tree
            .fold_at(&p("a.md/inner.md"))
            .expect_err("paths through the collision refuse");
        assert_eq!(through.reason, RefusalReason::Collision);
        assert_eq!(through.path, "a.md");

        // Serving continues: the root folds, both kinds inside (§4.4 — no
        // bytes sit outside the integrity surface).
        let mut fresh = ResidentTree::new();
        fresh.set_leaf(&p("a.md"), h(1));
        fresh.set_leaf(&p("other.md"), h(9));
        fresh.set_leaf(&p("a.md/inner.md"), h(2));
        assert_eq!(tree.fingerprint(), fresh.fingerprint());
        assert!(matches!(
            tree.fold_at(&p("other.md")),
            Ok(ScopeFold::Value(_))
        ));

        // Removing the FILE arm clears the key; the directory arm serves.
        assert!(tree.remove_leaf(&p("a.md")));
        assert!(tree.collision_paths().is_empty());
        assert!(matches!(tree.fold_at(&p("a.md")), Ok(ScopeFold::Value(_))));

        // And the other direction: prune the directory arm out from under a
        // re-composed collision — the file arm serves again.
        tree.set_leaf(&p("a.md"), h(1));
        assert_eq!(tree.collision_paths(), vec!["a.md".to_owned()]);
        assert!(tree.remove_leaf(&p("a.md/inner.md")));
        assert!(tree.collision_paths().is_empty());
        assert_eq!(tree.fold_at(&p("a.md")), Ok(ScopeFold::Value(h(1))));
    }

    /// §7 scope rows: file leaves answer their §3 leaf; a prefix through a
    /// file refuses kind-conflict; a never-created chain is `Absent` however
    /// deep (the chain law).
    #[test]
    fn scope_rows_file_conflict_absent() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("notes.md"), h(4));
        assert_eq!(tree.fold_at(&p("notes.md")), Ok(ScopeFold::Value(h(4))));
        let conflict = tree
            .fold_at(&p("notes.md/deeper/x.md"))
            .expect_err("a file prefix cannot be traversed");
        assert_eq!(conflict.reason, RefusalReason::KindConflict);
        assert_eq!(conflict.path, "notes.md");
        assert_eq!(
            tree.fold_at(&p("never/created/chain")),
            Ok(ScopeFold::Absent)
        );
        assert_eq!(tree.file_leaf(&p("notes.md")), Some(h(4)));
        assert_eq!(tree.file_leaf(&p("gone.md")), None);
    }

    /// §6.3 stamps: a chain stamp advances every ancestor directory node
    /// (root included), max-only, untouched by refolds; `stamp_at` answers a
    /// directory's own stamp and a live file's parent stamp, and never
    /// answers for a missing path.
    #[test]
    fn stamp_chain_advances_ancestors_max_only() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a/b/x.md"), h(1));
        tree.set_leaf(&p("c/y.md"), h(2));
        tree.stamp_chain(&p("a/b/x.md"), 41);
        assert_eq!(tree.stamp_at(&p("a/b")), Some(41));
        assert_eq!(tree.stamp_at(&p("a")), Some(41));
        assert_eq!(tree.stamp_at(Path::new("")), Some(41));
        assert_eq!(tree.stamp_at(&p("c")), Some(0), "sibling chain untouched");
        // A file leaf answers its parent directory's stamp.
        assert_eq!(tree.stamp_at(&p("a/b/x.md")), Some(41));
        // Max-only: a lower stamp never regresses the chain.
        tree.stamp_chain(&p("a/b/x.md"), 7);
        assert_eq!(tree.stamp_at(&p("a/b")), Some(41));
        tree.fingerprint();
        assert_eq!(tree.stamp_at(&p("a/b")), Some(41), "refolds never touch stamps");
        // Never answers for the dead: no node, no leaf — no stamp.
        assert_eq!(tree.stamp_at(&p("missing")), None);
        assert_eq!(tree.stamp_at(&p("a/b/gone.md")), None);
    }

    /// A removal's stamp lands on the surviving chain: pruned directories
    /// are skipped (never re-composed), the root still records the death.
    #[test]
    fn stamp_chain_skips_pruned_directories() {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a/b/x.md"), h(1));
        tree.set_leaf(&p("keep.md"), h(2));
        assert!(tree.remove_leaf(&p("a/b/x.md")));
        tree.stamp_chain(&p("a/b/x.md"), 9);
        assert_eq!(tree.stamp_at(&p("a/b")), None, "pruned — no node revives");
        assert_eq!(tree.stamp_at(&p("a")), None, "pruned too");
        assert_eq!(tree.stamp_at(Path::new("")), Some(9), "the root records it");
    }
}
