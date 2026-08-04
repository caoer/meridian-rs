//! `rulepack-api@2` — the change struct + door fact surface (U1.1).
//!
//! # What this is
//! The `@1` surface ([`crate::pack`]) injects a single document's world-model
//! facts into a `def check(doc)` predicate. The `@2` surface injects a *change*
//! — the before/after states of a write plus its declared evidence — into a
//! `def check_change(change)` predicate. One shape, two customers: the `mrd test`
//! harness (U1.2) and the door's `gate()` (U4.2) read the SAME [`Change`], so a
//! scenario proves exactly what the door enforces (rulings § sequencing gift).
//!
//! # The law it obeys (ZT rulings, "door fact surface — yes. no git")
//! A door-site CHECK reads the change (before/after docs, edits, properties,
//! actor, force) plus ONE hop of declared pinned evidence at pinned revs,
//! **fail-closed on drift**. There are NO git facts, NO clock, NO search, and NO
//! randomness at the door — git and freshness questions belong to `status` and
//! the sweep, never here. Every fact is a pure function of the two markdown
//! STATES (before, after) and the pinned evidence bytes — never of the request's
//! phrasing (cross-client robustness) and never of the world outside the vault.
//!
//! The [purity guard](../../tests/purity.rs) is the load-bearing enforcement of
//! that law: it enumerates the [closed 14-key vocabulary](CHANGE_FACT_VOCAB) and
//! FAILS the moment a git/clock/random/io-sourced key enters it.

use std::collections::{BTreeMap, BTreeSet};

use model::{Document, Edit, EditKind, NodeKind, PutAt, Ref, YamlMap, resolve};

use crate::BudgetClass;

/// The rule-language / injected-fact-API pin this surface implements. Kept as a
/// stable external wire-contract identifier (plan decision #18): the `@1`→`@2`
/// bump is the sanctioned load-gate door — a change to the injected fact surface
/// is a pin bump, gated at load, never a silent widening.
pub const RULEPACK_API_V2: &str = "rulepack-api@2";

/// The write operation a change describes (wire §4). Pure classification — the
/// op *name*, never its bytes, offsets, or identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOp {
    /// A batch `splice` into an existing document.
    Splice,
    /// A guarded `create` — a document born at this write.
    Create,
    /// A guarded `remove` — a document deleted at this write.
    Remove,
}

impl ChangeOp {
    /// The fact string a predicate reads (`change.op`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeOp::Splice => "splice",
            ChangeOp::Create => "create",
            ChangeOp::Remove => "remove",
        }
    }
}

/// One world-model node exposed to a `check_change` predicate — the `@2` twin of
/// the `@1` [`crate::pack`] node surface, carried by value so the two pinned
/// surfaces version independently. Every field is a pure parse fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFact {
    /// `"heading"` / `"paragraph"` / … — the world-model node kind.
    pub kind: &'static str,
    /// Heading level (1..=6), or `0` for non-headings.
    pub level: u32,
    /// The node's text fact (heading text, link target, best-effort span text).
    pub text: String,
    /// Byte span `(start, end)` into the document the node was parsed from.
    pub span: (usize, usize),
    /// The node's content-addressed rev (`blake3(span bytes)[:16]`).
    pub node_rev: String,
    /// Heading path root → node (world-model fact).
    pub hpath: Vec<String>,
}

/// One document's facts: path, nodes (document order), frontmatter properties,
/// and declared pinned-evidence edges. This is the shape behind `change.doc`,
/// `change.before`, and an edge's `target_facts` — one plane, three positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocFacts {
    /// The document path.
    pub path: String,
    /// World-model nodes in document order.
    pub nodes: Vec<NodeFact>,
    /// Frontmatter properties `(key, value)` in document order — the "properties"
    /// the ruling names, a pure parse fact of the state.
    pub frontmatter: Vec<(String, String)>,
    /// Declared pinned evidence — populated only on `change.doc` (the after
    /// state's `inputs:` lock); empty on `change.before` and on an edge's own
    /// `target_facts` (one hop only, enforced structurally).
    pub edges: Vec<Edge>,
}

/// One declared edge as data BEFORE resolution: the ref, the rev it is pinned at,
/// and the (engine-ignored) claim. The caller declares these from the after
/// document's `meridian-lock` block; [`derive_change`] resolves each to an
/// [`Edge`]. (R1.3 corrected the source named here — the legacy `^inputs` lock
/// was this slot's origin when the type was written; the type is unchanged.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeDecl {
    /// The declared ref (`selector`), exactly as the lock spells it.
    pub reference: String,
    /// The rev the lock pins the target at (`rev`).
    pub pinned_rev: String,
    /// The human `claim:` — carried as inert data, never interpreted (a passenger).
    pub claim: Option<String>,
}

/// One hop of declared pinned evidence, resolved. `target_facts` is present ONLY
/// when the target resolved AND its current rev equals the pinned rev; any drift
/// (or an unresolvable ref) yields `drifted = true` and `target_facts = None` —
/// **fail-closed on drift**, the rulings' one-hop evidence law. A predicate that
/// reads a drifted edge's facts reads `None`, so it cannot pass on stale evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The declared ref.
    pub reference: String,
    /// The rev the lock pins the target at.
    pub pinned_rev: String,
    /// The rev the target resolves to NOW; `None` when the ref does not resolve.
    pub current_rev: Option<String>,
    /// The engine-ignored `claim:` passenger, carried as data.
    pub claim: Option<String>,
    /// `true` when the target is unresolved or has drifted off its pinned rev.
    pub drifted: bool,
    /// The target's facts at the pinned rev — `Some` only on a fresh (undrifted)
    /// edge; its own `edges` are always empty (one hop, no recursion).
    pub target_facts: Option<DocFacts>,
}

/// A resolved write target's identity: the ref plus the node rev it addressed in
/// the BEFORE state (`None` when the target was absent — a create).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFact {
    /// The rendered target ref.
    pub reference: String,
    /// The pre-batch node rev the ref resolved to, or `None` if absent.
    pub node_rev: Option<String>,
}

/// One edit's op-shape fact: which target, which edit kind, and (for a put) the
/// position. The `edits`/`targets` facts describe the operation; the *change*
/// facts ([`Change::fields_changed`], [`Change::sections_changed`]) are derived
/// from the STATES, never from these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditFact {
    /// The rendered target ref this edit addresses.
    pub target: String,
    /// `"match"` (edit-exact) or `"put"` (whole-slot write).
    pub kind: &'static str,
    /// The put position (`"all"` / `"content"` / `"end"` / `"upsert"`); `None`
    /// for a match edit.
    pub at: Option<&'static str>,
}

/// The `rulepack-api@2` change surface — everything a `check_change(change)`
/// predicate may read, and NOTHING else. Injected identically into the harness
/// and the door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The write operation kind (`change.op`).
    pub op: ChangeOp,
    /// The write's target identities, resolved against the before state.
    pub targets: Vec<TargetFact>,
    /// The write's edits, as op-shape facts.
    pub edits: Vec<EditFact>,
    /// The opaque actor string, or `None` for an external (out-of-engine) edit —
    /// the engine never invents identity it was not given.
    pub actor: Option<String>,
    /// Whether the write was forced past the checks (`--force`).
    pub force: bool,
    /// The document's facts BEFORE the write.
    pub before: DocFacts,
    /// The document's facts AFTER the write, carrying the declared evidence edges.
    pub doc: DocFacts,
    /// Frontmatter keys whose value was added, removed, or changed — DERIVED by
    /// diffing the before/after states, sorted, deduplicated.
    pub fields_changed: Vec<String>,
    /// Section hpaths whose node rev differs between the before/after states
    /// (added, removed, or edited) — DERIVED from the states, sorted.
    pub sections_changed: Vec<Vec<String>>,
}

/// The runtime config of the write invocation — what the recorded operation
/// cannot carry (rulings: "frontmatter = runtime config of the invocation").
/// `now` is deliberately absent: the door reads no clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invocation<'a> {
    /// The write operation kind.
    pub op: ChangeOp,
    /// The opaque actor string, or `None` for an external (out-of-engine) edit.
    pub actor: Option<&'a str>,
    /// Whether the write was forced past the checks (`--force`).
    pub force: bool,
}

/// Build a [`Change`] from the two markdown STATES, the edits, the invocation
/// config, and the declared evidence — the ONE derivation the harness and the
/// door share.
///
/// `fields_changed` and `sections_changed` are computed from `before`/`after`
/// ALONE (the edits are not consulted): two different requests that reach the
/// same after state produce identical change facts (cross-client robustness).
/// `resolve_edge` is the caller-injected one-hop resolver (policy stays I/O-free,
/// as `model` is) — given a ref it returns `(current_rev, target document)` or
/// `None`; a drifted or unresolved edge fails closed.
#[must_use]
pub fn derive_change(
    before: &Document,
    after: &Document,
    edits: &[Edit],
    invocation: Invocation<'_>,
    edge_decls: &[EdgeDecl],
    resolve_edge: &dyn Fn(&str) -> Option<(String, Document)>,
) -> Change {
    let targets = edits
        .iter()
        .map(|e| TargetFact {
            reference: render_ref(&e.target),
            node_rev: resolve(before, &e.target).ok().map(|t| t.node_rev.0),
        })
        .collect();

    let edit_facts = edits
        .iter()
        .map(|e| EditFact {
            target: render_ref(&e.target),
            kind: edit_kind_str(&e.edit),
            at: put_at_str(&e.edit),
        })
        .collect();

    let edges = edge_decls
        .iter()
        .map(|decl| resolve_one_hop(decl, resolve_edge))
        .collect();

    Change {
        op: invocation.op,
        targets,
        edits: edit_facts,
        actor: invocation.actor.map(str::to_owned),
        force: invocation.force,
        before: doc_facts(before, Vec::new()),
        doc: doc_facts(after, edges),
        fields_changed: diff_fields(before, after),
        sections_changed: diff_sections(before, after),
    }
}

/// Resolve one declared edge, failing closed on drift (rulings one-hop law).
fn resolve_one_hop(
    decl: &EdgeDecl,
    resolve_edge: &dyn Fn(&str) -> Option<(String, Document)>,
) -> Edge {
    match resolve_edge(&decl.reference) {
        // Resolved AND on its pinned rev — fresh evidence; attach one hop of facts
        // (whose own edges are empty: one hop, never a chain).
        Some((current_rev, target)) if current_rev == decl.pinned_rev => Edge {
            reference: decl.reference.clone(),
            pinned_rev: decl.pinned_rev.clone(),
            current_rev: Some(current_rev),
            claim: decl.claim.clone(),
            drifted: false,
            target_facts: Some(doc_facts(&target, Vec::new())),
        },
        // Resolved but DRIFTED off the pinned rev — fail closed: no facts.
        Some((current_rev, _)) => Edge {
            reference: decl.reference.clone(),
            pinned_rev: decl.pinned_rev.clone(),
            current_rev: Some(current_rev),
            claim: decl.claim.clone(),
            drifted: true,
            target_facts: None,
        },
        // Unresolvable ref — fail closed: no facts.
        None => Edge {
            reference: decl.reference.clone(),
            pinned_rev: decl.pinned_rev.clone(),
            current_rev: None,
            claim: decl.claim.clone(),
            drifted: true,
            target_facts: None,
        },
    }
}

/// Build a document's facts, reusing the `@1` real-AST node walk so the two
/// pinned surfaces agree on what a node fact is.
fn doc_facts(doc: &Document, edges: Vec<Edge>) -> DocFacts {
    let fd = crate::pack::facts_from_document(doc);
    let nodes = fd
        .nodes
        .iter()
        .map(|n| NodeFact {
            kind: n.kind,
            level: n.level,
            text: n.text.clone(),
            span: n.span,
            node_rev: n.node_rev.clone(),
            hpath: n.hpath.clone(),
        })
        .collect();
    DocFacts {
        path: fd.path,
        nodes,
        frontmatter: frontmatter_pairs(doc),
        edges,
    }
}

/// The document's frontmatter `(key, value)` pairs in document order, or empty
/// when the document has no frontmatter block.
fn frontmatter_pairs(doc: &Document) -> Vec<(String, String)> {
    frontmatter_map(doc)
        .map(|m| m.0.clone())
        .unwrap_or_default()
}

/// The frontmatter map of a document, if it carries a frontmatter node.
fn frontmatter_map(doc: &Document) -> Option<&YamlMap> {
    doc.root.children.iter().find_map(|c| match &c.kind {
        NodeKind::Frontmatter { map } => Some(map),
        _ => None,
    })
}

/// Frontmatter keys added, removed, or changed between the two states. Facts from
/// STATES: a pure diff of the parsed maps, sorted and deduplicated.
fn diff_fields(before: &Document, after: &Document) -> Vec<String> {
    let b: BTreeMap<&str, &str> = frontmatter_map(before)
        .map(|m| m.0.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect())
        .unwrap_or_default();
    let a: BTreeMap<&str, &str> = frontmatter_map(after)
        .map(|m| m.0.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect())
        .unwrap_or_default();

    let mut changed = BTreeSet::new();
    for (k, av) in &a {
        if b.get(k) != Some(av) {
            changed.insert((*k).to_owned());
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            changed.insert((*k).to_owned());
        }
    }
    changed.into_iter().collect()
}

/// Section hpaths whose node rev differs between the two states (added, removed,
/// or edited). Facts from STATES: a pure diff of the section trees keyed by
/// hpath. Ancestors of an edited section change implicitly (rev = span hash) and
/// so appear here too — an honest state fact, not the wire delta's deepest-only
/// compression.
fn diff_sections(before: &Document, after: &Document) -> Vec<Vec<String>> {
    let b = section_revs(before);
    let a = section_revs(after);

    let mut changed: BTreeSet<Vec<String>> = BTreeSet::new();
    for (hpath, rev) in &a {
        if b.get(hpath) != Some(rev) {
            changed.insert(hpath.clone());
        }
    }
    for hpath in b.keys() {
        if !a.contains_key(hpath) {
            changed.insert(hpath.clone());
        }
    }
    changed.into_iter().collect()
}

/// Map every section's hpath to its node rev (last occurrence wins on duplicate
/// hpaths — a rev change on any occurrence still surfaces the hpath as changed).
fn section_revs(doc: &Document) -> BTreeMap<Vec<String>, String> {
    let mut out = BTreeMap::new();
    collect_sections(&doc.root, &mut out);
    out
}

/// Preorder walk collecting `Section` nodes' `(hpath, node_rev)`.
fn collect_sections(node: &model::Node, out: &mut BTreeMap<Vec<String>, String>) {
    if let NodeKind::Section { .. } = &node.kind
        && let Some(hpath) = &node.hpath
    {
        out.insert(hpath.clone(), node.node_rev.0.clone());
    }
    for child in &node.children {
        collect_sections(child, out);
    }
}

/// Render a mint-plane ref to its fact string: `A/B` (hpath), `^id` (anchor),
/// `fm:key` (frontmatter key).
fn render_ref(r: &Ref) -> String {
    match r {
        Ref::Hpath(segs) => segs
            .iter()
            .map(|s| s.h.clone())
            .collect::<Vec<_>>()
            .join("/"),
        Ref::Anchor(id) => format!("^{id}"),
        Ref::FmKey(k) => format!("fm:{k}"),
    }
}

/// `"match"` or `"put"` for an edit kind.
fn edit_kind_str(e: &EditKind) -> &'static str {
    match e {
        EditKind::Match { .. } => "match",
        EditKind::Put { .. } => "put",
    }
}

/// The put position string, or `None` for a match edit.
fn put_at_str(e: &EditKind) -> Option<&'static str> {
    match e {
        EditKind::Put { at, .. } => Some(match at {
            PutAt::All => "all",
            PutAt::Content => "content",
            PutAt::End => "end",
            PutAt::Upsert => "upsert",
        }),
        EditKind::Match { .. } => None,
    }
}

// ── The closed 14-key vocabulary + the purity guard ──────────────────────────

/// The closed vocabulary of the `@2` change surface: the 14 fact keys a
/// `check_change` predicate may read, each tagged with the cost class of the
/// plane it reads (change-local, whole-document, or one-hop cross-document).
///
/// This IS the "14-assertion vocabulary" (plan U1.1): the enumerated allowed
/// fact keys the [purity guard](../../tests/purity.rs) enforces. Every key is a
/// pure function of the before/after STATES and the pinned evidence — there is,
/// by construction, no git/clock/random/io key here, and the guard fails the
/// instant one is added.
pub const CHANGE_FACT_VOCAB: [(&str, BudgetClass); 14] = [
    // change-local facts (cheapest — scalars and small lists over the request +
    // the derived state diff)
    ("op", BudgetClass::Node),
    ("targets", BudgetClass::Node),
    ("edits", BudgetClass::Node),
    ("actor", BudgetClass::Node),
    ("force", BudgetClass::Node),
    ("fields_changed", BudgetClass::Node),
    ("sections_changed", BudgetClass::Node),
    // whole-document facts (a full parse of one state)
    ("before", BudgetClass::File),
    ("doc", BudgetClass::File),
    ("path", BudgetClass::File),
    ("nodes", BudgetClass::File),
    ("frontmatter", BudgetClass::File),
    // one-hop cross-document evidence (resolve + parse a second document)
    ("edges", BudgetClass::Corpus),
    ("target_facts", BudgetClass::Corpus),
];

/// A class-tier default per-eval p99 (µs) for [`crate::vocab`]. These are
/// monotonic class DEFAULTS (a one-hop fact costs more than a change-local one),
/// NOT per-key measured SLAs — per-key bench calibration against the frozen
/// corpus is U1.5's (`test --corpus` fuel budgets). Kept small and declared so
/// the surface has a cost shape without inventing a measurement.
#[must_use]
pub(crate) fn class_default_p99(class: BudgetClass) -> u32 {
    match class {
        BudgetClass::Node => 100,
        BudgetClass::File => 1_000,
        BudgetClass::Corpus => 10_000,
    }
}

/// Classify a fact key as git/clock/random/io-sourced, returning the impurity
/// class name when it is one, or `None` when the key names a pure state/evidence
/// fact. Substring match against the impure roots, case-insensitive — a
/// `git_commit_sha`, a `mtime`, a `random_seed`, a `fetched_at` are all caught.
///
/// The roots are chosen NOT to collide with any of the 14 allowed keys, so the
/// guard is exact: it passes the whole vocabulary and rejects every impure key.
#[must_use]
pub fn impure_source(key: &str) -> Option<&'static str> {
    /// `(root, class)` — a key containing `root` is sourced from `class`.
    const IMPURE_ROOTS: &[(&str, &str)] = &[
        // git / VCS state — the door reads markdown, never the repository
        ("git", "git"),
        ("commit", "git"),
        ("sha", "git"),
        ("oid", "git"),
        ("head", "git"),
        ("branch", "git"),
        ("blob", "git"),
        // wall clock / freshness — belongs to status and the sweep, not the door
        ("now", "clock"),
        ("time", "clock"),
        ("clock", "clock"),
        ("date", "clock"),
        ("stamp", "clock"),
        ("mtime", "clock"),
        ("ctime", "clock"),
        ("epoch", "clock"),
        // non-determinism — a hermetic evaluator admits none
        ("random", "random"),
        ("rand", "random"),
        ("uuid", "random"),
        ("nonce", "random"),
        ("seed", "random"),
        ("entropy", "random"),
        // ambient I/O — the door reads the change, never reaches outward
        ("fetch", "io"),
        ("search", "io"),
        ("http", "io"),
        ("url", "io"),
        ("socket", "io"),
        ("exec", "io"),
        ("getenv", "io"),
        ("readfile", "io"),
        ("open", "io"),
        ("disk", "io"),
    ];
    let k = key.to_ascii_lowercase();
    IMPURE_ROOTS
        .iter()
        .find(|(root, _)| k.contains(root))
        .map(|(_, class)| *class)
}

/// The named purity assertion: every key in `keys` names a pure state/evidence
/// fact. FAILS loud (with the offending key + its impurity class) the instant a
/// git/clock/random/io-sourced key appears — the load-bearing negative.
///
/// # Errors
/// A message naming the first impure key and its class.
pub fn assert_vocab_pure(keys: &[&str]) -> Result<(), String> {
    for key in keys {
        if let Some(class) = impure_source(key) {
            return Err(format!(
                "impure fact key `{key}` ({class}-sourced): the rulepack-api@2 door \
                 surface admits only facts derived from the before/after states and \
                 pinned evidence — no git, clock, randomness, or I/O"
            ));
        }
    }
    Ok(())
}

/// The vocabulary's keys, in declaration order — the allowed fact keys the guard
/// enumerates.
#[must_use]
pub fn vocab_keys() -> Vec<&'static str> {
    CHANGE_FACT_VOCAB.iter().map(|(k, _)| *k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{HpathSeg, PutAt};

    /// Build a real `model::Document` from markdown, stamping its path (what the
    /// disk edge does; `model::build` is I/O-free and leaves the path empty).
    fn doc_of(path: &str, md: &str) -> Document {
        let nodes = syntax::parse(md);
        let mut doc = model::build(md.to_string(), nodes);
        if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    fn hpath(seg: &str) -> Ref {
        Ref::Hpath(vec![HpathSeg {
            h: seg.to_string(),
            n: None,
        }])
    }

    fn no_edges(_: &str) -> Option<(String, Document)> {
        None
    }

    /// `derive_change` with the op fixed to `Splice` — the derivation tests vary
    /// states/edits/evidence, never the op (its pass-through is covered by
    /// `op_fact_strings`).
    fn derive(
        before: &Document,
        after: &Document,
        edits: &[Edit],
        actor: Option<&str>,
        force: bool,
        edge_decls: &[EdgeDecl],
        resolve_edge: &dyn Fn(&str) -> Option<(String, Document)>,
    ) -> Change {
        derive_change(
            before,
            after,
            edits,
            Invocation {
                op: ChangeOp::Splice,
                actor,
                force,
            },
            edge_decls,
            resolve_edge,
        )
    }

    // ── struct-shape ─────────────────────────────────────────────────────────

    /// The change surface exposes every declared key, and the vocabulary matches
    /// the struct's actual fields (nothing enumerated that the struct lacks).
    #[test]
    fn change_surface_exposes_the_declared_keys() {
        let before = doc_of("notes/plan.md", "---\ntitle: a\n---\n# A\n\nbody\n");
        let after = doc_of("notes/plan.md", "---\ntitle: b\n---\n# A\n\nbody\n");
        let edits = [Edit {
            target: Ref::FmKey("title".into()),
            edit: EditKind::Put {
                at: PutAt::Upsert,
                text: "b".into(),
            },
            if_node_rev: None,
        }];
        let change = derive(
            &before,
            &after,
            &edits,
            Some("agent:x"),
            false,
            &[],
            &no_edges,
        );

        // Touch every top-level key — the struct-shape contract.
        assert_eq!(change.op, ChangeOp::Splice);
        assert_eq!(change.targets.len(), 1);
        assert_eq!(change.edits.len(), 1);
        assert_eq!(change.actor.as_deref(), Some("agent:x"));
        assert!(!change.force);
        assert_eq!(change.before.path, "notes/plan.md");
        assert_eq!(change.doc.path, "notes/plan.md");
        assert!(change.doc.edges.is_empty());
        assert_eq!(change.fields_changed, vec!["title".to_string()]);
        assert!(change.sections_changed.is_empty(), "no section rev moved");

        // The vocabulary is the closed 14 keys.
        assert_eq!(vocab_keys().len(), 14);
    }

    /// The op fact strings are stable.
    #[test]
    fn op_fact_strings() {
        assert_eq!(ChangeOp::Splice.as_str(), "splice");
        assert_eq!(ChangeOp::Create.as_str(), "create");
        assert_eq!(ChangeOp::Remove.as_str(), "remove");
    }

    /// A put edit carries its position; a match edit carries none.
    #[test]
    fn edit_facts_carry_kind_and_position() {
        let before = doc_of("f.md", "# A\n\nx\n");
        let after = doc_of("f.md", "# A\n\ny\n");
        let edits = [
            Edit {
                target: hpath("A"),
                edit: EditKind::Match {
                    old: "x".into(),
                    new: "y".into(),
                },
                if_node_rev: None,
            },
            Edit {
                target: hpath("A"),
                edit: EditKind::Put {
                    at: PutAt::End,
                    text: "z".into(),
                },
                if_node_rev: None,
            },
        ];
        let change = derive(&before, &after, &edits, None, false, &[], &no_edges);
        assert_eq!(change.edits[0].kind, "match");
        assert_eq!(change.edits[0].at, None);
        assert_eq!(change.edits[1].kind, "put");
        assert_eq!(change.edits[1].at, Some("end"));
        // An external edit (no actor) records absence, never an invented identity.
        assert_eq!(change.actor, None);
    }

    // ── fact-derivation: facts from STATES, never request phrasing ────────────

    /// `fields_changed` diffs the parsed frontmatter STATES: a changed value, an
    /// added key, and a removed key all surface; an untouched key does not.
    #[test]
    fn fields_changed_is_the_state_diff() {
        let before = doc_of(
            "f.md",
            "---\nkeep: 1\nchange: old\ndrop: x\n---\n# A\n\nb\n",
        );
        let after = doc_of("f.md", "---\nkeep: 1\nchange: new\nadd: y\n---\n# A\n\nb\n");
        let change = derive(&before, &after, &[], None, false, &[], &no_edges);
        assert_eq!(
            change.fields_changed,
            vec!["add".to_string(), "change".to_string(), "drop".to_string()],
            "added + changed + removed keys, sorted; `keep` excluded"
        );
    }

    /// `sections_changed` reports the hpath of a section whose rev moved between
    /// states, and leaves an untouched sibling out.
    #[test]
    fn sections_changed_is_the_state_diff() {
        let before = doc_of("f.md", "# A\n\nalpha\n\n# B\n\nbeta\n");
        let after = doc_of("f.md", "# A\n\nalpha\n\n# B\n\nBETA edited\n");
        let change = derive(&before, &after, &[], None, false, &[], &no_edges);
        assert_eq!(
            change.sections_changed,
            vec![vec!["B".to_string()]],
            "only B's rev moved; A is byte-identical"
        );
    }

    /// The change facts are a function of the STATES, not the request phrasing:
    /// two different edit requests that reach the SAME after state produce
    /// identical `fields_changed` / `sections_changed` (cross-client robustness).
    #[test]
    fn change_facts_ignore_request_phrasing() {
        let before = doc_of("f.md", "# A\n\nold\n");
        let after = doc_of("f.md", "# A\n\nnew\n");

        let via_match = [Edit {
            target: hpath("A"),
            edit: EditKind::Match {
                old: "old".into(),
                new: "new".into(),
            },
            if_node_rev: None,
        }];
        let via_put = [Edit {
            target: hpath("A"),
            edit: EditKind::Put {
                at: PutAt::Content,
                text: "new\n".into(),
            },
            if_node_rev: None,
        }];

        let a = derive(&before, &after, &via_match, None, false, &[], &no_edges);
        let b = derive(&before, &after, &via_put, None, false, &[], &no_edges);
        assert_eq!(a.fields_changed, b.fields_changed);
        assert_eq!(a.sections_changed, b.sections_changed);
        assert_eq!(
            a.sections_changed,
            vec![vec!["A".to_string()]],
            "A's body moved under both phrasings"
        );
    }

    // ── edges: one hop, pinned revs, fail-closed on drift ─────────────────────

    /// A fresh edge (current rev == pinned rev) attaches one hop of target facts;
    /// its own edges are empty (no recursion).
    #[test]
    fn edge_fresh_attaches_one_hop_of_facts() {
        let after = doc_of("f.md", "# A\n\nb\n");
        let target = doc_of("dep.md", "# Dep\n\nevidence\n");
        let target_rev = crate::pack::facts_from_document(&target).nodes[0]
            .node_rev
            .clone();
        let decl = EdgeDecl {
            reference: "dep.md#Dep".into(),
            pinned_rev: target_rev.clone(),
            claim: Some("evidence for A".into()),
        };
        let resolve = |r: &str| {
            if r == "dep.md#Dep" {
                Some((target_rev.clone(), target.clone()))
            } else {
                None
            }
        };
        let change = derive(
            &after,
            &after,
            &[],
            None,
            false,
            std::slice::from_ref(&decl),
            &resolve,
        );
        let edge = &change.doc.edges[0];
        assert!(!edge.drifted);
        assert_eq!(edge.current_rev.as_deref(), Some(target_rev.as_str()));
        assert_eq!(edge.claim.as_deref(), Some("evidence for A"));
        let facts = edge
            .target_facts
            .as_ref()
            .expect("fresh edge carries facts");
        assert_eq!(facts.path, "dep.md");
        assert!(
            facts.edges.is_empty(),
            "one hop only — the target's own edges are never followed"
        );
    }

    /// A drifted edge (current rev != pinned rev) fails closed: no target facts.
    #[test]
    fn edge_drift_fails_closed() {
        let after = doc_of("f.md", "# A\n\nb\n");
        let target = doc_of("dep.md", "# Dep\n\nevidence\n");
        let decl = EdgeDecl {
            reference: "dep.md#Dep".into(),
            pinned_rev: "stale00000000000".into(),
            claim: None,
        };
        let resolve = |_: &str| Some(("fresh11111111111".to_string(), target.clone()));
        let change = derive(
            &after,
            &after,
            &[],
            None,
            false,
            std::slice::from_ref(&decl),
            &resolve,
        );
        let edge = &change.doc.edges[0];
        assert!(edge.drifted, "current rev != pinned rev");
        assert_eq!(edge.current_rev.as_deref(), Some("fresh11111111111"));
        assert!(
            edge.target_facts.is_none(),
            "fail-closed: a drifted edge exposes no facts"
        );
    }

    /// An unresolvable edge fails closed: no rev, no facts, drifted.
    #[test]
    fn edge_unresolved_fails_closed() {
        let after = doc_of("f.md", "# A\n\nb\n");
        let decl = EdgeDecl {
            reference: "gone.md#Nope".into(),
            pinned_rev: "whatever00000000".into(),
            claim: None,
        };
        let change = derive(
            &after,
            &after,
            &[],
            None,
            false,
            std::slice::from_ref(&decl),
            &no_edges,
        );
        let edge = &change.doc.edges[0];
        assert!(edge.drifted);
        assert_eq!(edge.current_rev, None);
        assert!(edge.target_facts.is_none());
    }

    // ── purity classifier (the guard's engine) ───────────────────────────────

    /// Every allowed key classifies pure; representative impure keys classify to
    /// their source.
    #[test]
    fn purity_classifier_is_exact() {
        for (key, _) in CHANGE_FACT_VOCAB {
            assert_eq!(impure_source(key), None, "`{key}` is a pure state fact");
        }
        assert_eq!(impure_source("git_commit_sha"), Some("git"));
        assert_eq!(impure_source("committed_at"), Some("git"));
        assert_eq!(impure_source("now"), Some("clock"));
        assert_eq!(impure_source("wall_clock"), Some("clock"));
        assert_eq!(impure_source("mtime"), Some("clock"));
        assert_eq!(impure_source("random_seed"), Some("random"));
        assert_eq!(impure_source("fetched_at"), Some("io"));
        assert_eq!(impure_source("head_ref"), Some("git"));
    }

    /// The named assertion passes on the real vocabulary and FAILS the instant a
    /// git/clock/random/io key is introduced.
    #[test]
    fn assert_vocab_pure_is_load_bearing() {
        assert!(assert_vocab_pure(&vocab_keys()).is_ok());

        let mut poisoned = vocab_keys();
        poisoned.push("git_head");
        let err = assert_vocab_pure(&poisoned).unwrap_err();
        assert!(err.contains("git_head"), "names the offending key: {err}");
        assert!(err.contains("git-sourced"), "names the class: {err}");
    }
}
