//! Read-is-the-mint: the in-memory read-receipt ledger (stage-2 S6, D6/D9/D16).
//!
//! One actor read one selector at one rev. That fact — minted at the
//! composed-read seam, held in daemon-session memory — is what a later pin
//! gate (S7) consults: you cannot attest content that was never in your
//! context.
//!
//! Not hung off the warm engine (H1): a pin writes, and the registry rebuilds
//! a workspace's warm engine on content-hash change, so a receipt living in
//! `WorkspaceEngine` would be evaporated by the very write it authorized.
//! `registry::Registry::read_mints` is the production holder; the survival
//! gate is `crates/registry/tests/read_mint.rs`.
//!
//! Selector-grained, never doc-level (D6): the key is (actor, path, selector),
//! so reading section A cannot gate a pin into unseen section B. Content is a
//! minimal mechanical fact (D9): actor, selector, rev — never a verdict
//! envelope, never content bytes.
//!
//! Memory only, per daemon-session (D6): no I/O, no path to disk. The
//! persisted `^receipt` projection is a different thing
//! ([`render_line`](crate::render_line)).
//!
//! D12: the key carries the `path` spelling verbatim and never parses it, so
//! a later `root:` prefix rides through unchanged.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// The per-actor receipt cap: the oldest receipt is evicted past this many
/// distinct (path, selector) pairs for one actor. Re-reading replaces in
/// place, so ordinary traffic never approaches the cap — eviction is the
/// memory backstop, not the lifecycle.
const MAX_RECEIPTS_PER_ACTOR: usize = 1024;

/// One minted read fact: this actor read this selector of this path, and the
/// bytes it was served carried this rev — the three D9 facts and nothing
/// else.
///
/// `selector` is the read face's own tagged selector ([`wire::ReadSel`],
/// U14), so the mint site and the pin gate cannot key on two spellings of one
/// address. `sec_rev` is the node CAS token over exactly the bytes served —
/// what a pin gate re-checks against disk inside its flock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadReceipt {
    /// The daemon-derived actor (D13) the read was stamped with.
    pub actor: String,
    /// The workspace-relative document path the read served.
    pub path: String,
    /// The canonical selector the read served, in the tagged read grammar.
    pub selector: wire::ReadSel,
    /// The node CAS token (`sec_rev`) of the bytes served.
    pub sec_rev: String,
}

/// The daemon-session read-receipt ledger: a mutex-guarded map keyed by the
/// daemon-derived actor, each actor holding its receipts in mint order.
///
/// Interior mutability is deliberate — the composed-read arm mints through a
/// shared `&` borrow, so a read path never needs a `&mut` it cannot have.
#[derive(Debug, Default)]
pub struct ReadMintStore {
    actors: Mutex<HashMap<String, Vec<ReadReceipt>>>,
}

impl ReadMintStore {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint (or refresh) the receipt for `actor` reading `selector` of `path`
    /// at `sec_rev`, returning the minted fact.
    ///
    /// Re-reading the same (path, selector) replaces the prior receipt — a
    /// stale rev never lingers beside a fresh one for the same address.
    ///
    /// `actor` must be a real identity: the `actor == None` no-mint door (D16)
    /// is the caller's branch (`wire_serve::read`), so this method never opens
    /// a shared empty-string bucket.
    pub fn mint(
        &self,
        actor: &str,
        path: &str,
        selector: &wire::ReadSel,
        sec_rev: &str,
    ) -> ReadReceipt {
        let receipt = ReadReceipt {
            actor: actor.to_owned(),
            path: path.to_owned(),
            selector: selector.clone(),
            sec_rev: sec_rev.to_owned(),
        };
        let mut actors = self.actors.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = actors.entry(actor.to_owned()).or_default();
        if let Some(at) = rows
            .iter()
            .position(|r| r.path == path && &r.selector == selector)
        {
            rows[at] = receipt.clone();
        } else {
            rows.push(receipt.clone());
            if rows.len() > MAX_RECEIPTS_PER_ACTOR {
                rows.remove(0);
            }
        }
        receipt
    }

    /// The receipt for `actor` reading `selector` of `path`, or `None` when
    /// this actor did not read exactly that selector in this session.
    ///
    /// Matching is exact on all three key parts — a nested selector is a miss
    /// (reading `Notes/Plan` does not answer `Notes/Plan/Q3`): the gate fails
    /// closed, and the remedy is to read the exact selector to be pinned.
    ///
    /// Answers "was it read", never "is it current": a caller gating a write
    /// must re-check `sec_rev` against disk inside its own flock — a receipt
    /// is not a lease.
    #[must_use]
    pub fn lookup(&self, actor: &str, path: &str, selector: &wire::ReadSel) -> Option<ReadReceipt> {
        let actors = self.actors.lock().unwrap_or_else(PoisonError::into_inner);
        actors
            .get(actor)
            .and_then(|rows| {
                rows.iter()
                    .rev()
                    .find(|r| r.path == path && &r.selector == selector)
            })
            .cloned()
    }

    /// Whether any actor in this session holds a receipt for (path, selector).
    ///
    /// The D7 refusal's rotation half only: it lets a gate that missed on the
    /// caller's identity say "your identity rotated" instead of "you never
    /// read this". A boolean on purpose — naming the other identity would
    /// publish one actor's read history to another. Never an authorization
    /// input: no caller may pass a gate on this.
    #[must_use]
    pub fn any_actor_read(&self, path: &str, selector: &wire::ReadSel) -> bool {
        self.actors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .any(|rows| {
                rows.iter()
                    .any(|r| r.path == path && &r.selector == selector)
            })
    }

    /// How many receipts the ledger holds across every actor. Introspection for
    /// gates ("this read minted nothing"), never a correctness input.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(Vec::len)
            .sum()
    }

    /// Whether the ledger holds no receipt at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    //! The ledger's own semantics: the three key parts each isolate, a re-read
    //! refreshes in place, and the cap bounds one actor's memory.

    use super::*;

    /// A selector from its human spelling, through the one ingress door.
    fn sel(s: &str) -> wire::ReadSel {
        wire::ReadSel::parse(s)
    }

    #[test]
    fn a_minted_receipt_carries_the_three_d9_facts_and_is_found_by_its_key() {
        let store = ReadMintStore::new();
        let minted = store.mint("agent-7", "notes/plan.md", &sel("Notes/Q3"), "b3:aa11");
        assert_eq!(
            minted,
            ReadReceipt {
                actor: "agent-7".into(),
                path: "notes/plan.md".into(),
                selector: sel("Notes/Q3"),
                sec_rev: "b3:aa11".into(),
            }
        );
        assert_eq!(
            store.lookup("agent-7", "notes/plan.md", &sel("Notes/Q3")),
            Some(minted),
            "the lookup key is (actor, path, selector)"
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn each_key_part_isolates_actor_path_and_selector() {
        let store = ReadMintStore::new();
        store.mint("mine", "a.md", &sel("A"), "rev-1");

        assert!(
            store.lookup("foreign", "a.md", &sel("A")).is_none(),
            "a foreign actor's lookup never sees my receipt"
        );
        assert!(
            store.lookup("mine", "b.md", &sel("A")).is_none(),
            "the same selector in another file is a different fact"
        );
        assert!(
            store.lookup("mine", "a.md", &sel("B")).is_none(),
            "a sibling selector is a different fact (D6 grain)"
        );
    }

    /// U14: the key is tagged, so the three read grammars isolate — a receipt
    /// for the heading `1.2` answers neither the dewey row `1.2` nor the
    /// block `^1.2`.
    #[test]
    fn the_grammars_isolate_a_heading_is_not_its_dewey_twin_nor_a_block_of_that_name() {
        let store = ReadMintStore::new();
        store.mint("mine", "a.md", &sel("1.2"), "rev-1");
        assert!(
            store
                .lookup(
                    "mine",
                    "a.md",
                    &wire::ReadSel::Hpath {
                        hpath: vec![wire::HpathSeg {
                            h: "1.2".to_owned(),
                            n: None
                        }]
                    }
                )
                .is_none(),
            "the dewey receipt does not cover the heading literally named `1.2`"
        );
        assert!(
            store.lookup("mine", "a.md", &sel("^1.2")).is_none(),
            "nor does it cover a block of that id"
        );
        assert_eq!(
            store.lookup("mine", "a.md", &sel("1.2")).map(|r| r.sec_rev),
            Some("rev-1".to_owned()),
            "and the plane it WAS minted in still answers"
        );
    }

    #[test]
    fn a_nested_selector_is_a_miss_the_gate_fails_closed() {
        let store = ReadMintStore::new();
        store.mint("mine", "a.md", &sel("Notes/Plan"), "rev-1");
        assert!(
            store
                .lookup("mine", "a.md", &sel("Notes/Plan/Q3"))
                .is_none(),
            "a child selector is NOT covered by its parent's receipt"
        );
        assert!(
            store.lookup("mine", "a.md", &sel("Notes")).is_none(),
            "nor is the parent covered by a child's receipt"
        );
        assert!(
            store.lookup("mine", "a.md", &sel("^some-block")).is_none(),
            "nor is a block anchor inside the read subtree"
        );
    }

    #[test]
    fn re_reading_a_selector_replaces_its_receipt_rather_than_adding_one() {
        let store = ReadMintStore::new();
        store.mint("mine", "a.md", &sel("A"), "rev-1");
        store.mint("mine", "a.md", &sel("A"), "rev-2");
        assert_eq!(store.len(), 1, "a re-read refreshes in place");
        assert_eq!(
            store.lookup("mine", "a.md", &sel("A")).map(|r| r.sec_rev),
            Some("rev-2".to_owned()),
            "the fresh rev wins — a stale rev never lingers for the same address"
        );
    }

    #[test]
    fn the_per_actor_cap_bounds_memory_by_evicting_the_oldest() {
        let store = ReadMintStore::new();
        for i in 0..=MAX_RECEIPTS_PER_ACTOR {
            store.mint("mine", "a.md", &sel(&format!("S{i}")), "rev-1");
        }
        assert_eq!(
            store.len(),
            MAX_RECEIPTS_PER_ACTOR,
            "one actor's ledger is capped"
        );
        assert!(
            store.lookup("mine", "a.md", &sel("S0")).is_none(),
            "the oldest receipt was evicted"
        );
        assert!(
            store
                .lookup("mine", "a.md", &sel(&format!("S{MAX_RECEIPTS_PER_ACTOR}")))
                .is_some(),
            "the newest receipt is held"
        );
    }

    #[test]
    fn any_actor_read_separates_a_rotated_identity_from_a_selector_never_read() {
        let store = ReadMintStore::new();
        store.mint("before-rotation", "a.md", &sel("A"), "rev-1");

        assert!(
            store.lookup("after-rotation", "a.md", &sel("A")).is_none(),
            "the gate still misses — a receipt is keyed to the identity that read"
        );
        assert!(
            store.any_actor_read("a.md", &sel("A")),
            "but the selector WAS read in this session: that is the rotation cause"
        );
        assert!(
            !store.any_actor_read("a.md", &sel("B")),
            "a selector nobody read is the never-read cause"
        );
        assert!(
            !store.any_actor_read("b.md", &sel("A")),
            "and the path is part of the question, exactly as in lookup"
        );
    }

    #[test]
    fn a_fresh_ledger_is_empty() {
        let store = ReadMintStore::new();
        assert!(store.is_empty());
        assert!(store.lookup("anyone", "a.md", &sel("A")).is_none());
    }
}
