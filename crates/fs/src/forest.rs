//! The §4.3.1 forest fold (`docs/node-rev-merkle-spec.md`) — set premises
//! over DERIVED match sets ("all files matching `a/*.md`"; "the rows this
//! query actually scanned"), the merged plan §4.5's first law made mechanical.
//!
//! A set premise guards a match set, not a directory. Its fold carries its
//! own domain tag (`mrk2.fst`, distinct from `mrk2.dir` at byte 5) over
//! EXACTLY the matching members — sorted `(path bytes, §3 leaf hash)` pairs —
//! computed from the resident listings ([`ResidentTree::files_under`]) at
//! O(subtree width), zero byte I/O. A new MATCHING sibling joins the
//! re-expansion and moves the fold — the membership hole stays closed. A
//! non-matching sibling never moves it — the false conflict dies.
//!
//! **Two-point set comparison** (merged plan §4.5, card set-premises step 2):
//! the premise check compares the ENTRY expansion's fold against the LIVE
//! expansion's fold ([`check`]) — deletes and renames move the live
//! expansion's member paths, so the fold moves; no journal is consulted, no
//! vacuous windows exist, and the mechanism is epoch-free. The subtree fold
//! (`mrk2.dir` via [`ResidentTree::fold_at`]) remains the conservative
//! fallback and the explicit directory-premise form.
//!
//! **Grammar-agnostic on purpose.** The ONE glob grammar in the system is
//! `policy::glob_match` (§ A.7 — no second grammar), and `policy` sits above
//! this crate; expansion therefore takes the match predicate as a closure
//! over full member path bytes, plus the literal directory prefix that
//! bounds the walk. The consumer (the script door's touch-set recording, the
//! sql provenance plane) binds the grammar; this module owns only the fold
//! law. A non-UTF-8 member name simply never matches a UTF-8 pattern — it
//! stays addressable through the literal `scope_bytes` premise arm instead
//! (wire-contract §5.4).
//!
//! Consistency law (merged plan §4.5, stated here because this module is the
//! instrument): every set premise — pattern root, selector root, sql
//! provenance — validates against the TREE, the same instrument as every
//! other guard. No premise anywhere consults the journal: nothing in this
//! module's signatures can even name one.

use std::path::Path;

use crate::resident::{ResidentTree, ScopeRefusal};

/// The 8-byte ASCII domain tag opening a forest-fold pre-image (§4.3):
/// prefix-free against `mrk2.vtx`/`mrk2.dir` (differs at byte 5), so a
/// forest fold can never collide with a directory value.
const TAG_FOREST: &[u8; 8] = b"mrk2.fst";

/// The §4.3.1 fold over a derived match set `M`:
/// `blake3("mrk2.fst" ‖ varint(n) ‖ n × (varint(len(path)) ‖ path ‖ leaf))`.
///
/// `members` must be strictly ascending by path bytes — [`expand`] returns
/// exactly that, and the entry-side restriction of pinned leaves preserves
/// it. Varints are the same minimal-form unsigned LEB128 the vertex
/// pre-images use, shared with [`crate::radix`] so the encodings cannot
/// drift. `n = 0` is legal: the fold of "nothing matches" — a mintable
/// premise that guards a match set's CONTINUED emptiness.
#[must_use]
pub fn fold(members: &[(Vec<u8>, [u8; 32])]) -> [u8; 32] {
    debug_assert!(
        members.windows(2).all(|w| w[0].0 < w[1].0),
        "forest members must be strictly ascending by path bytes (§4.3.1)"
    );
    let mut hasher = blake3::Hasher::new();
    hasher.update(TAG_FOREST);
    let mut varint: Vec<u8> = Vec::with_capacity(10);
    crate::radix::write_uleb128(&mut varint, members.len());
    hasher.update(&varint);
    for (path, leaf) in members {
        varint.clear();
        crate::radix::write_uleb128(&mut varint, path.len());
        hasher.update(&varint);
        hasher.update(path);
        hasher.update(leaf);
    }
    *hasher.finalize().as_bytes()
}

/// Expand a pattern against the resident listings: every file leaf under
/// `prefix` (the pattern's literal directory prefix, bounding the walk) whose
/// FULL workspace-relative path bytes satisfy `matches`. Strictly ascending
/// by path bytes; zero byte I/O; no refold, no `&mut`.
///
/// # Errors
/// [`ScopeRefusal`] when the walk meets a §4.4 collision key at, through, or
/// under `prefix` — §4.3.1's stated precondition ([`ResidentTree::files_under`]).
pub fn expand(
    tree: &ResidentTree,
    prefix: &Path,
    mut matches: impl FnMut(&[u8]) -> bool,
) -> Result<Vec<(Vec<u8>, [u8; 32])>, ScopeRefusal> {
    let mut members = tree.files_under(prefix)?;
    members.retain(|(path, _)| matches(path));
    Ok(members)
}

/// The live forest digest: [`fold`] over [`expand`] — the two-point
/// comparison's LIVE point. Mint it at plan time as the premise token; mint
/// it again at check time and compare ([`check`]).
///
/// # Errors
/// [`ScopeRefusal`] as [`expand`].
pub fn digest(
    tree: &ResidentTree,
    prefix: &Path,
    matches: impl FnMut(&[u8]) -> bool,
) -> Result<[u8; 32], ScopeRefusal> {
    Ok(fold(&expand(tree, prefix, matches)?))
}

/// A forest premise check's answer (merkle-spec §7, forest-fold row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForestVerdict {
    /// The set the caller derived is still exactly this — membership AND
    /// member content.
    Pass,
    /// The premise MOVED — `fingerprint_mismatch` naming the set premise is
    /// the wire spelling (§7); expected/actual ride the refusal.
    Mismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
}

/// The two-point set comparison, as one call: re-expand the pattern against
/// the LIVE listings, fold, and compare against the ENTRY-side token. A
/// matching creation, deletion, either half of a rename, or a member content
/// change moves the live fold and answers [`ForestVerdict::Mismatch`]; churn
/// outside the match set never does.
///
/// # Errors
/// [`ScopeRefusal`] as [`expand`] — `scope_unresolved` (fix class) on the
/// wire, distinct from a mismatch (resync class) by the §5.7 error split.
pub fn check(
    tree: &ResidentTree,
    prefix: &Path,
    matches: impl FnMut(&[u8]) -> bool,
    expected: &[u8; 32],
) -> Result<ForestVerdict, ScopeRefusal> {
    let actual = digest(tree, prefix, matches)?;
    Ok(if actual == *expected {
        ForestVerdict::Pass
    } else {
        ForestVerdict::Mismatch {
            expected: *expected,
            actual,
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::resident::{RefusalReason, ScopeFold};

    use super::*;

    fn h(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// The one glob grammar (`policy::glob_match`), bridged to path bytes the
    /// way a consumer binds it: a non-UTF-8 name never matches.
    fn glob(pattern: &'static str) -> impl FnMut(&[u8]) -> bool {
        move |path| std::str::from_utf8(path).is_ok_and(|s| policy::glob_match(pattern, s))
    }

    /// A small workspace: two matching drafts, a non-matching sibling file,
    /// a non-matching binary, and an unrelated subtree.
    fn tree() -> ResidentTree {
        let mut tree = ResidentTree::new();
        tree.set_leaf(&p("a/draft-1.md"), h(1));
        tree.set_leaf(&p("a/draft-3.md"), h(3));
        tree.set_leaf(&p("a/x.bin"), h(9));
        tree.set_leaf(&p("b/other.md"), h(7));
        tree
    }

    /// Worked §4.3.1 values, exact pre-images: the empty fold is
    /// `blake3("mrk2.fst" ‖ 0x00)`, and a one-member fold is the tag, the
    /// varint pair, the path bytes, and the leaf — nothing else.
    #[test]
    fn fold_preimage_is_the_spec_shape() {
        assert_eq!(fold(&[]), *blake3::hash(b"mrk2.fst\x00").as_bytes());
        let mut pre: Vec<u8> = Vec::new();
        pre.extend_from_slice(b"mrk2.fst");
        pre.push(0x01); // varint(n = 1)
        pre.push(0x04); // varint(len("a.md"))
        pre.extend_from_slice(b"a.md");
        pre.extend_from_slice(&h(5));
        assert_eq!(
            fold(&[(b"a.md".to_vec(), h(5))]),
            *blake3::hash(&pre).as_bytes()
        );
    }

    /// Expansion reads the listings in path-byte order, bounded by the
    /// prefix, and filters by the FULL path — and it costs zero interior
    /// hash work (no refold: the instrument counters stand still).
    #[test]
    fn expand_is_sorted_bounded_and_refold_free() {
        let mut tree = tree();
        tree.fingerprint(); // settle folds so the counter baseline is clean
        let before = tree.stats().vertex_hashes;
        let members = expand(&tree, &p("a"), glob("a/draft-*.md")).expect("a resolves");
        assert_eq!(
            members,
            vec![
                (b"a/draft-1.md".to_vec(), h(1)),
                (b"a/draft-3.md".to_vec(), h(3)),
            ]
        );
        assert_eq!(
            tree.stats().vertex_hashes,
            before,
            "expansion is a listing read — no vertex is ever re-hashed"
        );
        // Deep members under the prefix join too, in byte order.
        let mut deep = tree;
        deep.set_leaf(&p("a/sub/draft-9.md"), h(4));
        let members = expand(&deep, &p("a"), glob("a/**")).expect("a resolves");
        let paths: Vec<&[u8]> = members.iter().map(|(p, _)| p.as_slice()).collect();
        assert_eq!(
            paths,
            vec![
                b"a/draft-1.md".as_slice(),
                b"a/draft-3.md",
                b"a/sub/draft-9.md",
                b"a/x.bin"
            ]
        );
    }

    /// The §7 forest-membership row, both directions (codex gate 4's shape):
    /// glob `a/draft-*.md` — a new MATCHING sibling refuses; a new
    /// NON-matching sibling passes; modification of a non-matching member
    /// passes. The digest moves with its match set and with nothing else.
    #[test]
    fn membership_both_directions() {
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), glob("a/draft-*.md")).expect("mint");

        // A new matching sibling joins the re-expansion → refuse.
        tree.set_leaf(&p("a/draft-2.md"), h(2));
        assert!(matches!(
            check(&tree, &p("a"), glob("a/draft-*.md"), &entry).expect("check"),
            ForestVerdict::Mismatch { expected, .. } if expected == entry
        ));
        tree.remove_leaf(&p("a/draft-2.md"));

        // A new NON-matching sibling never moves the digest → pass.
        tree.set_leaf(&p("a/final.md"), h(8));
        assert_eq!(
            check(&tree, &p("a"), glob("a/draft-*.md"), &entry).expect("check"),
            ForestVerdict::Pass
        );

        // Modification of a non-matching member → pass.
        tree.set_leaf(&p("a/x.bin"), h(10));
        assert_eq!(
            check(&tree, &p("a"), glob("a/draft-*.md"), &entry).expect("check"),
            ForestVerdict::Pass
        );

        // A matching member's CONTENT moving refuses (the content half).
        tree.set_leaf(&p("a/draft-1.md"), h(11));
        assert!(matches!(
            check(&tree, &p("a"), glob("a/draft-*.md"), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));
    }

    /// The §7 selector row: matching creation, deletion, and BOTH halves of
    /// a rename each trip the premise — deletes and renames are the
    /// two-point comparison's own catch (entry expansion vs live expansion).
    #[test]
    fn creation_deletion_and_both_rename_halves_trip() {
        let matcher = || glob("a/draft-*.md");
        // Creation.
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), matcher()).expect("mint");
        tree.set_leaf(&p("a/draft-0.md"), h(12));
        assert!(matches!(
            check(&tree, &p("a"), matcher(), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));

        // Deletion.
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), matcher()).expect("mint");
        tree.remove_leaf(&p("a/draft-3.md"));
        assert!(matches!(
            check(&tree, &p("a"), matcher(), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));

        // Rename half OUT (a/draft-3.md → a/final-3.md): leaves the set.
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), matcher()).expect("mint");
        tree.remove_leaf(&p("a/draft-3.md"));
        tree.set_leaf(&p("a/final-3.md"), h(3));
        assert!(matches!(
            check(&tree, &p("a"), matcher(), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));

        // Rename half IN (a/x.bin's twin arriving as a/draft-9.md): joins.
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), matcher()).expect("mint");
        tree.remove_leaf(&p("a/x.bin"));
        tree.set_leaf(&p("a/draft-9.md"), h(9));
        assert!(matches!(
            check(&tree, &p("a"), matcher(), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));

        // A WITHIN-set rename (same content, new matching name) also trips:
        // the fold covers member PATHS, not just leaves.
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), matcher()).expect("mint");
        tree.remove_leaf(&p("a/draft-3.md"));
        tree.set_leaf(&p("a/draft-4.md"), h(3));
        assert!(matches!(
            check(&tree, &p("a"), matcher(), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));
    }

    /// `n = 0` guards continued emptiness: minted over "nothing matches",
    /// the premise passes while nothing matches and trips on a matching
    /// birth. An absent prefix expands empty the same way (set semantics,
    /// never a refusal).
    #[test]
    fn empty_match_set_is_a_mintable_premise() {
        let mut tree = tree();
        let entry = digest(&tree, &p("a"), glob("a/todo-*.md")).expect("mint");
        assert_eq!(entry, fold(&[]));
        assert_eq!(
            check(&tree, &p("a"), glob("a/todo-*.md"), &entry).expect("check"),
            ForestVerdict::Pass
        );
        tree.set_leaf(&p("a/todo-1.md"), h(13));
        assert!(matches!(
            check(&tree, &p("a"), glob("a/todo-*.md"), &entry).expect("check"),
            ForestVerdict::Mismatch { .. }
        ));
        // Absent prefix: empty expansion, n = 0 fold — a creation-guard form.
        let tree = ResidentTree::new();
        assert_eq!(
            digest(&tree, &p("never"), glob("never/*")).expect("mint"),
            fold(&[])
        );
    }

    /// The false-conflict dies, and the fallback stays conservative: the same
    /// non-matching sibling that the forest premise ignores DOES move the
    /// directory's subtree fold — the explicit directory-premise form keeps
    /// its meaning (merged plan §4.5).
    #[test]
    fn forest_narrows_where_the_subtree_fold_stays_conservative() {
        let mut tree = tree();
        let forest_entry = digest(&tree, &p("a"), glob("a/draft-*.md")).expect("mint");
        let dir_entry = tree.fold_at(&p("a")).expect("a resolves");
        tree.set_leaf(&p("a/final.md"), h(8));
        assert_eq!(
            check(&tree, &p("a"), glob("a/draft-*.md"), &forest_entry).expect("check"),
            ForestVerdict::Pass,
            "the forest premise ignores the non-matching sibling"
        );
        assert_ne!(
            tree.fold_at(&p("a")).expect("a resolves"),
            dir_entry,
            "the directory premise (conservative fallback) still refuses"
        );
        assert!(matches!(dir_entry, ScopeFold::Value(_)));
    }

    /// §4.3.1's stated precondition: a §4.4 collision key at, through, or
    /// under the prefix refuses expansion — an ambiguous member set is no
    /// premise. Sibling subtrees keep serving.
    #[test]
    fn collision_under_prefix_refuses() {
        let mut tree = tree();
        tree.set_leaf(&p("a/name"), h(1));
        tree.set_leaf(&p("a/name/inner.md"), h(2));
        let err = expand(&tree, &p("a"), glob("a/*")).expect_err("collision refuses");
        assert_eq!(err.reason, RefusalReason::Collision);
        assert_eq!(err.path, "a/name");
        // Through the collision refuses too; a sibling prefix still serves.
        let err = expand(&tree, &p("a/name"), glob("a/name/*")).expect_err("at the key");
        assert_eq!(err.reason, RefusalReason::Collision);
        assert!(expand(&tree, &p("b"), glob("b/*")).is_ok());
    }

    /// Root-scope expansion covers the whole corpus once (no double-counted
    /// root listing) and stays strictly ascending across directories.
    #[test]
    fn root_scope_expands_every_member_once() {
        let mut tree = tree();
        tree.set_leaf(&p("top.md"), h(14));
        let members = expand(&tree, Path::new(""), glob("**")).expect("root resolves");
        let paths: Vec<&[u8]> = members.iter().map(|(p, _)| p.as_slice()).collect();
        assert_eq!(
            paths,
            vec![
                b"a/draft-1.md".as_slice(),
                b"a/draft-3.md",
                b"a/x.bin",
                b"b/other.md",
                b"top.md"
            ]
        );
    }
}
