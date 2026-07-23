//! `compose_rev` + `rev_class` — the SECOND of the two merkles (design 2 §2.3).
//!
//! # The two merkles, never conflated
//! design 2 §2.3 rules two distinct hash laws with distinct jobs, and this crate
//! carries BOTH without ever collapsing one into the other:
//!
//! - **The workspace tree merkle** ([`crate::merkle_root`]) — guard/freshness.
//!   Doc-grain leaves (`blake3(raw file bytes)`), tree-composed to the 32-byte
//!   root that mints `root_before`/`root_after` and anchors CAS `if_root`. It
//!   **never expands embeds**: `![[x]]` is just bytes in the embedding file, so
//!   editing the embedded doc moves ONLY the embedded doc's leaf, never the
//!   embedding doc's.
//! - **`compose_rev`** (this module) — check-time chain composition. The leaf is
//!   `blake3(node_rev)`; an `![[embed]]` **expands** (content genuinely includes
//!   content, so the embedded rev folds in), a `[[wikilink]]` **never** does (a
//!   reference is not an inclusion); a compose cycle terminates with a
//!   **sentinel**; a dangling/ambiguous embed is an **error, never a partial
//!   compose**. Composed per run, **never persisted** — the workspace root lives
//!   in a lock, a `compose_rev` is recomputed every check.
//!
//! The divergence is the whole point: for a doc with an embed, its workspace
//! leaf ≠ its `compose_rev`, and editing the embedded section reddens the
//! embedding pinner (`compose_rev` moves) while the embedding doc's own
//! workspace leaf stays put.
//!
//! # `rev_class` — one hasher, two classes
//! A pinned rev carries a class (design 2 §2.3, A2 — C5 without a second
//! hasher):
//! - [`RevClass::Content`] — a `compose_rev`, the engine's blake3 composition
//!   (the default). The ONE hasher.
//! - [`RevClass::Object`] — the git object id at `commit:location`, a **source-2
//!   fact** carried verbatim and verified by **equality**. The engine never
//!   hashes it (git owns that content-addressing), so no second hash family
//!   enters the markdown law.

use std::collections::{BTreeMap, BTreeSet};

use crate::walk::walk;
use crate::{ByteSpan, CorpusIndex, Document, Node, NodeKind};

/// The check-time chain-composition rev (design 2 §2.3): `blake3` over a
/// target's own `node_rev` folded with the `compose_rev` of every `![[embed]]`
/// it expands. A DISTINCT type from [`crate::MerkleRoot`] (the workspace tree
/// merkle) and [`crate::NodeRev`] (one span) on purpose — the two merkles are
/// never conflated. 16 lowercase hex, the same rev grain a lock pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeRev(pub String);

/// Why a `compose_rev` refused — never a partial compose (design 2 §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// An `![[embed]]` resolved to no file/section — the compose is incomplete,
    /// so it refuses rather than hash a hole.
    Dangling {
        /// The unresolved embed linktext (`path#sub` / `#sub` / `path#^id`).
        linktext: String,
    },
}

/// The cycle sentinel: the fixed bytes folded in place of a recursion that would
/// re-enter a `(path, span)` already on the compose stack. A compose cycle
/// (`a` embeds `b` embeds `a`, or a self-embed) terminates HERE with a
/// well-defined value instead of hanging or overflowing the stack (design 2
/// §2.3 "cycle → sentinel").
const CYCLE_SENTINEL: &[u8] = b"\x00mrd-compose-cycle\x00";

/// A visited-node key on the compose stack — `(path, span.start, span.end)`.
/// A `ByteSpan` (`Range`) is not `Ord`, so the span is flattened to its two
/// endpoints for the [`BTreeSet`].
type VisitKey = (String, usize, usize);

/// Compose the check-time chain rev of `target` (a node in the `from` document):
/// `blake3(node_rev)` folded with the `compose_rev` of every `![[embed]]`
/// reachable under `target`, expanding embeds (content includes content) but
/// never wikilinks. A compose cycle terminates with [`CYCLE_SENTINEL`]; a
/// dangling embed refuses.
///
/// `index` + `docs` are the corpus the embeds resolve against (the walk plane);
/// `from` is `target`'s own vault path, so a same-file embed (`![[#Section]]`)
/// resolves relative to it.
///
/// # Errors
/// [`ComposeError::Dangling`] when any expanded `![[embed]]` resolves to no
/// file/section — a compose is all-or-nothing, never partial.
pub fn compose_rev(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from: &str,
    target: &Node,
) -> Result<ComposeRev, ComposeError> {
    let mut visiting: BTreeSet<VisitKey> = BTreeSet::new();
    visiting.insert(visit_key(from, &target.span));
    compose_node(index, docs, from, target, &mut visiting)
}

/// Compose one node: fold its own `node_rev` leaf with each expanded embed.
fn compose_node(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from: &str,
    node: &Node,
    visiting: &mut BTreeSet<VisitKey>,
) -> Result<ComposeRev, ComposeError> {
    let mut hasher = blake3::Hasher::new();
    // Leaf = blake3 node_rev: the node's own bytes enter through its rev.
    hasher.update(b"L");
    hasher.update(node.node_rev.0.as_bytes());

    let mut embeds: Vec<&Node> = Vec::new();
    collect_embeds(node, &mut embeds);
    embeds.sort_by_key(|e| e.span.start);

    for embed in embeds {
        let linktext = embed_linktext(embed);
        hasher.update(b"E");
        // Wikilinks are NOT here — only `Embed` nodes reach this loop, so a
        // `[[wikilink]]` never expands (its bytes already rode the leaf rev).
        let Ok(loc) = walk(index, docs, from, &linktext) else {
            return Err(ComposeError::Dangling { linktext });
        };
        let child_key = visit_key(&loc.dest, &loc.span);
        if visiting.contains(&child_key) {
            // Cycle: re-entering a node already on the compose stack. Fold the
            // sentinel and stop descending — terminates, never hangs.
            hasher.update(CYCLE_SENTINEL);
            continue;
        }
        let Some(child_doc) = docs.get(&loc.dest) else {
            // walk resolved a dest not in `docs` — treat as dangling (defensive;
            // walk only returns a dest it found in `docs`).
            return Err(ComposeError::Dangling { linktext });
        };
        let child_node = node_at_span(&child_doc.root, &loc.span).unwrap_or(&child_doc.root);
        visiting.insert(child_key.clone());
        let child = compose_node(index, docs, &loc.dest, child_node, visiting)?;
        visiting.remove(&child_key);
        hasher.update(child.0.as_bytes());
    }

    Ok(ComposeRev(
        hasher.finalize().to_hex().as_str()[..16].to_string(),
    ))
}

/// Every `Embed` node under `node` (its whole subtree), unordered — the caller
/// sorts by span start for a deterministic fold.
fn collect_embeds<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    if matches!(node.kind, NodeKind::Embed { .. }) {
        out.push(node);
    }
    for child in &node.children {
        collect_embeds(child, out);
    }
}

/// The Obsidian linktext an `Embed` node points at: `target`, plus `#^block` or
/// `#heading` when present (block wins, mirroring the walk grammar). An empty
/// `target` is a same-file embed (`![[#Section]]`), which [`walk`] resolves to
/// `from` itself.
fn embed_linktext(embed: &Node) -> String {
    let NodeKind::Embed {
        target,
        heading,
        block,
        ..
    } = &embed.kind
    else {
        return String::new();
    };
    let mut s = target.clone();
    if let Some(block) = block {
        s.push('#');
        s.push('^');
        s.push_str(block);
    } else if let Some(heading) = heading {
        s.push('#');
        s.push_str(heading);
    }
    s
}

/// The node in `doc` addressing exactly `span` — the deepest node whose span
/// equals `span`, else the tightest node that contains it. A resolved embed
/// span (whole file, a section, or a block-leaf) maps to a real node; the
/// container fallback keeps the compose total for any in-bounds span.
fn node_at_span<'a>(node: &'a Node, span: &ByteSpan) -> Option<&'a Node> {
    if node.span == *span {
        return Some(node);
    }
    if node.span.start <= span.start && span.end <= node.span.end {
        for child in &node.children {
            if let Some(found) = node_at_span(child, span) {
                return Some(found);
            }
        }
        // No child contains `span` more tightly — this node is the container.
        return Some(node);
    }
    None
}

/// Flatten a `(path, span)` into the `Ord` visit key.
fn visit_key(path: &str, span: &ByteSpan) -> VisitKey {
    (path.to_string(), span.start, span.end)
}

/// The class of a pinned rev (design 2 §2.3, A2). The class DECISION is the
/// caller's (which refs are pointed effects is domain meaning); computing and
/// verifying BOTH classes is core, under ONE engine hasher (blake3, for the
/// content class only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevClass {
    /// A `compose_rev` — the engine's blake3 chain composition. The default.
    Content,
    /// The git object id at `commit:location` — a source-2 fact carried
    /// verbatim, verified by equality; the engine never computes it.
    Object,
}

impl RevClass {
    /// The wire/lock spelling (`edge.rev_class`): `content` | `object`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RevClass::Content => "content",
            RevClass::Object => "object",
        }
    }
}

/// Verify a **content-class** pin: recompute the `compose_rev` of `target` and
/// compare it to `pinned` (design 2 §2.3). Green iff they match. A dangling
/// embed makes the target uncomposable, so it can never be green — `false`
/// (fail-closed, never a partial-compose green).
#[must_use]
pub fn verify_content(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from: &str,
    target: &Node,
    pinned: &str,
) -> bool {
    compose_rev(index, docs, from, target).is_ok_and(|rev| rev.0 == pinned)
}

/// Verify an **object-class** pin: pure equality of the pinned git object id
/// against the one git observed now (design 2 §2.3). The engine performs NO
/// hashing here — git owns the content-addressing (the one-hasher law: blake3
/// is the engine's only hash family, used for the content class alone). Green
/// iff the ids are byte-equal.
#[must_use]
pub fn verify_object(pinned: &str, observed_git_oid: &str) -> bool {
    pinned == observed_git_oid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;

    fn corpus(files: &[(&str, &str)]) -> (CorpusIndex, BTreeMap<String, Document>) {
        let mut index = CorpusIndex::new();
        let mut docs = BTreeMap::new();
        for (path, raw) in files {
            let doc = build((*raw).to_string(), syntax::parse(raw));
            index.insert(path, &doc);
            docs.insert((*path).to_string(), doc);
        }
        (index, docs)
    }

    /// The workspace tree merkle's doc-grain leaf — `blake3(raw bytes)`, full 32
    /// hex — exactly what [`crate::merkle_root`] hashes per file. Computed here
    /// (not through `merkle_root`, which folds a whole corpus) so the test can
    /// name ONE file's leaf and watch it move (or not).
    fn workspace_leaf(raw: &str) -> String {
        blake3::hash(raw.as_bytes()).to_hex().to_string()
    }

    /// A TEST-ONLY flat composer that hashes only a node's own `node_rev` and
    /// **never expands embeds** — the "collapse the two computations to one"
    /// falsification made durable. The divergence-under-edit assertions below
    /// hold for the real [`compose_rev`] and FAIL for this flat one; that
    /// contrast proves embed expansion is load-bearing, not incidental.
    fn compose_flat(node: &Node) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"L");
        hasher.update(node.node_rev.0.as_bytes());
        hasher.finalize().to_hex().as_str()[..16].to_string()
    }

    /// LOAD-BEARING — the embed-divergence fixture (task U2.8 gate).
    ///
    /// A doc with `![[embedded]]` yields workspace-leaf ≠ `compose_rev`; editing
    /// the embedded section reddens the embedding pinner (`compose_rev` moves)
    /// while the workspace root moves only on the embedded doc (the embedding
    /// doc's own leaf stays put).
    #[test]
    fn embed_divergence_fixture() {
        let embedding = "# Doc\n\nintro\n\n![[embedded]]\n";
        let embedded_v1 = "# Embedded\n\nsection body v1\n";
        let embedded_v2 = "# Embedded\n\nsection body v2 — edited\n";

        let (index, docs) = corpus(&[("embedding.md", embedding), ("embedded.md", embedded_v1)]);

        // 1. Divergence at rest: the embedding doc's workspace leaf (raw bytes,
        //    embed NOT expanded) is not its compose_rev (embed expanded).
        let leaf_v1 = workspace_leaf(embedding);
        let compose_v1 = compose_rev(&index, &docs, "embedding.md", &docs["embedding.md"].root)
            .expect("embedding composes (embed resolves)");
        assert_ne!(
            leaf_v1, compose_v1.0,
            "workspace leaf and compose_rev must diverge for an embedding doc"
        );

        // 2. Edit the EMBEDDED section; rebuild only that doc.
        let (index2, docs2) = corpus(&[("embedding.md", embedding), ("embedded.md", embedded_v2)]);

        let compose_v2 = compose_rev(&index2, &docs2, "embedding.md", &docs2["embedding.md"].root)
            .expect("embedding still composes after the embedded edit");

        // 2a. The embedding pinner reddens: compose_rev MOVED, because the embed
        //     expanded the edited content.
        assert_ne!(
            compose_v1.0, compose_v2.0,
            "editing the embedded section must move the embedding doc's compose_rev"
        );

        // 2b. The workspace root moves ONLY on the embedded doc: the embedding
        //     doc's own leaf is byte-identical (its raw bytes never changed),
        //     while the embedded doc's leaf moved.
        assert_eq!(
            workspace_leaf(embedding),
            leaf_v1,
            "the embedding doc's workspace leaf must NOT move on an embedded edit"
        );
        assert_ne!(
            workspace_leaf(embedded_v1),
            workspace_leaf(embedded_v2),
            "the embedded doc's workspace leaf must move on its own edit"
        );

        // 2c. The corpus root moves, and the delta is attributable to the
        //     embedded leaf alone (the embedding leaf is stable across).
        let root_v1 = crate::merkle_root(
            &[
                ("embedding.md", embedding.as_bytes()),
                ("embedded.md", embedded_v1.as_bytes()),
            ],
            0,
        );
        let root_v2 = crate::merkle_root(
            &[
                ("embedding.md", embedding.as_bytes()),
                ("embedded.md", embedded_v2.as_bytes()),
            ],
            0,
        );
        assert_ne!(
            root_v1.0, root_v2.0,
            "the workspace root moves on any leaf change"
        );

        // FALSIFICATION (durable): collapse compose_rev to the doc-grain workspace
        // computation (no embed expansion) and the load-bearing assertion (2a)
        // inverts — the flat compose does NOT move on an embedded edit, because
        // the embedding doc's own bytes/revs are unchanged. Expansion is the
        // only thing that creates the divergence.
        let flat_v1 = compose_flat(&docs["embedding.md"].root);
        let flat_v2 = compose_flat(&docs2["embedding.md"].root);
        assert_eq!(
            flat_v1, flat_v2,
            "collapsed (no-expand) compose must NOT move on an embedded edit — proving \
             the real compose_rev's movement is due to embed expansion"
        );
    }

    /// A content-class pin verifies green while the composed content is current
    /// and reddens once the embedded section drifts.
    #[test]
    fn content_class_verifies_and_reddens() {
        let embedding = "# Doc\n\n![[embedded]]\n";
        let (index, docs) = corpus(&[
            ("embedding.md", embedding),
            ("embedded.md", "# E\n\nbody v1\n"),
        ]);
        let pin = compose_rev(&index, &docs, "embedding.md", &docs["embedding.md"].root)
            .expect("composes")
            .0;
        assert!(
            verify_content(
                &index,
                &docs,
                "embedding.md",
                &docs["embedding.md"].root,
                &pin
            ),
            "a fresh content pin is green"
        );

        let (index2, docs2) = corpus(&[
            ("embedding.md", embedding),
            ("embedded.md", "# E\n\nbody v2\n"),
        ]);
        assert!(
            !verify_content(
                &index2,
                &docs2,
                "embedding.md",
                &docs2["embedding.md"].root,
                &pin
            ),
            "the content pin reddens once the embedded section drifts"
        );
    }

    /// A dangling embed refuses — never a partial compose (design 2 §2.3).
    #[test]
    fn dangling_embed_refuses() {
        let (index, docs) = corpus(&[("embedding.md", "# Doc\n\n![[nosuchfile]]\n")]);
        let got = compose_rev(&index, &docs, "embedding.md", &docs["embedding.md"].root);
        assert_eq!(
            got,
            Err(ComposeError::Dangling {
                linktext: "nosuchfile".to_string()
            }),
        );
    }

    /// CYCLE SENTINEL — a compose cycle terminates with the sentinel, never hangs
    /// or overflows the stack (task U2.8 gate).
    #[test]
    fn cycle_sentinel_terminates() {
        // a ⇄ b mutual embed, plus c self-embed.
        let (index, docs) = corpus(&[
            ("a.md", "# A\n\n![[b]]\n"),
            ("b.md", "# B\n\n![[a]]\n"),
            ("c.md", "# C\n\n![[c]]\n"),
        ]);

        // Reaching an Ok verdict at all is the proof of termination — an
        // unbroken cycle would recurse forever (hang) or overflow the stack.
        let a = compose_rev(&index, &docs, "a.md", &docs["a.md"].root).expect("a terminates");
        let b = compose_rev(&index, &docs, "b.md", &docs["b.md"].root).expect("b terminates");
        let c = compose_rev(&index, &docs, "c.md", &docs["c.md"].root).expect("c self terminates");

        // Deterministic: a second run yields the same rev (the sentinel is fixed).
        let a2 = compose_rev(&index, &docs, "a.md", &docs["a.md"].root).expect("a again");
        assert_eq!(a.0, a2.0, "compose over a cycle is deterministic");

        // The three cycles are distinct compositions (different leaves before the
        // sentinel), so the sentinel does not flatten them into one value.
        assert_ne!(a.0, b.0);
        assert_ne!(a.0, c.0);
    }

    /// OBJECT-CLASS REV — an object-class pin verifies against THE git object id
    /// (task U2.8 gate). The ground-truth id is produced by `git hash-object`
    /// itself; the engine only compares equality (no second hasher).
    #[test]
    fn object_class_verifies_against_git_object_id() {
        let content = b"section body v1\n";
        let oid = git_hash_object(content);
        // Sanity: a real git blob oid is 40 lowercase hex (sha1) — not a blake3
        // rev (which is 16 or 64 hex). The class is genuinely git's, not ours.
        assert_eq!(oid.len(), 40, "git blob oid is 40 hex");
        assert!(oid.chars().all(|c| c.is_ascii_hexdigit()));

        // Pin the object-class rev = the git object id; verify against git's id.
        assert!(
            verify_object(&oid, &oid),
            "an object-class pin is green against the same git object id"
        );

        // Tamper the content → git mints a different oid → the pin reddens.
        let oid_other = git_hash_object(b"section body v2 edited\n");
        assert_ne!(oid, oid_other, "git ids differ for different content");
        assert!(
            !verify_object(&oid, &oid_other),
            "the object-class pin reddens against a different git object id"
        );

        // The class spelling is stable for the lock/edge surface.
        assert_eq!(RevClass::Object.as_str(), "object");
        assert_eq!(RevClass::Content.as_str(), "content");
    }

    /// The canonical git object id of a blob: `git hash-object --stdin` over
    /// `content`. Test-only ground truth — the engine never computes this.
    fn git_hash_object(content: &[u8]) -> String {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn `git hash-object`");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(content)
            .expect("write content");
        let out = child.wait_with_output().expect("git hash-object output");
        assert!(out.status.success(), "git hash-object failed");
        String::from_utf8(out.stdout)
            .expect("utf8 oid")
            .trim()
            .to_string()
    }
}
