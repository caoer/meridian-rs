//! Read-is-the-mint: the in-memory read-receipt ledger (stage-2 S6, D6/D9/D16).
//!
//! # What this is
//! One actor read one selector at one rev. That fact — minted at the composed-read seam,
//! held in daemon-session memory — is what a later pin gate (S7) consults to enforce the
//! property **you cannot attest content that was never in your context**.
//!
//! # Why it is NOT hung off the warm engine (H1, the reason this exists)
//! The registry daemon rebuilds a workspace's warm engine whenever the corpus content
//! hash changes (`registry::Registry::warm_or_build`). A pin WRITES — so a receipt living
//! inside `WorkspaceEngine` would be evaporated by the very write the receipt authorized.
//! This ledger is therefore a SEPARATE daemon-session layer, held beside the engines and
//! untouched by any rebuild. `registry::Registry::read_mints` is its production holder;
//! the gate proving survival is `crates/registry/tests/read_mint.rs`.
//!
//! # Grain (D6) and content (D9)
//! **Selector-grained, never doc-level:** the key is (actor, path, selector), so reading
//! section A cannot gate a pin into unseen section B. **A minimal mechanical fact:**
//! actor, target selector, rev — never a `predicate_type` verdict envelope (that
//! unification is stage-3), never any content bytes.
//!
//! # No persistence (D6)
//! Memory only, per daemon-session, dropped when the daemon exits or the workspace is
//! idle-reaped. This ledger performs NO I/O and holds no path to disk; the persisted
//! `^receipt` projection is a different thing (this crate's
//! [`render_line`](crate::render_line)) and stays stage-3's to unify.
//!
//! # D12 (cross-root seam)
//! The key carries a `path` spelling verbatim and never parses it, so a later `root:`
//! prefix rides through unchanged. Mount identity is the HOLDER's key (one store per
//! canonical workspace in the registry), never a field here — nothing in this module
//! knows that there is exactly one root.
//!
//!

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// The per-actor receipt cap. A resident daemon must not grow without bound, so
/// the oldest receipt is evicted past this many DISTINCT (path, selector) pairs
/// for one actor. Re-reading a selector replaces its receipt in place rather
/// than adding one, so ordinary agent traffic never approaches the cap;
/// eviction is the memory backstop, not the lifecycle (TTL is deliberately
/// unimplemented — ratified as implementation space, and Core ships none).
const MAX_RECEIPTS_PER_ACTOR: usize = 1024;

/// One minted read fact: this actor read this selector of this path, and the
/// bytes it was served carried this rev.
///
/// The three D9 facts and nothing else. `path` is the workspace-relative wire
/// spelling verbatim; `selector` is the read face's own TAGGED selector
/// ([`wire::ReadSel`], U14) — the ledger re-derives no address and, since the
/// key is structured, the mint site and the pin gate cannot key on two
/// spellings of one address. `sec_rev` is the node CAS token over exactly the
/// bytes the read served, which is what a pin gate re-checks against disk
/// inside its flock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadReceipt {
    /// The DAEMON-derived actor (D13) the read was stamped with.
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

    /// Mint (or refresh) the receipt for `actor` reading `selector` of `path` at
    /// `sec_rev`, returning the minted fact.
    ///
    /// Re-reading the same (path, selector) REPLACES the prior receipt — the
    /// rev moves with the bytes, so a stale rev never lingers beside a fresh
    /// one for the same address.
    ///
    /// `actor` must be a real identity: the `actor == None` no-mint door (D16)
    /// is the CALLER's explicit branch (`wire_serve::read`), so this method is
    /// never reached with an absent actor and can never open a shared
    /// empty-string bucket.
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

    /// The receipt for `actor` reading `selector` of `path`, or `None` when this
    /// actor did not read exactly that selector in this session.
    ///
    /// # Matching is EXACT on all three key parts
    /// A different actor, a different path, or a different selector is a MISS —
    /// including a NESTED selector. Reading `Notes/Plan` does not answer a
    /// lookup for `Notes/Plan/Q3` even though the sections face served the
    /// subtree's bytes: the gate fails CLOSED, and the caller's remedy is to
    /// read the exact selector it intends to pin. (Widening this to
    /// span-containment is a deliberate non-goal of Core — a permissive authz
    /// answer needs its own ratified decision, and no ratified decision asks
    /// for one.)
    ///
    /// # This answers "was it read", never "is it current"
    /// The returned receipt carries the `sec_rev` the bytes were served at. A
    /// caller gating a WRITE must re-check that rev against disk inside its own
    /// flock — a receipt is not a lease.
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

    /// A selector from its human spelling, through the ONE ingress door.
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

    /// U14: the key is TAGGED, so the three read grammars isolate from each
    /// other too. A receipt for the heading named `1.2` does not answer for the
    /// dewey row `1.2`, and neither answers for the block `^1.2` — before the
    /// union these were one string and one key, so reading any of them opened
    /// the pin gate for all three.
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
    fn a_fresh_ledger_is_empty() {
        let store = ReadMintStore::new();
        assert!(store.is_empty());
        assert!(store.lookup("anyone", "a.md", &sel("A")).is_none());
    }
}
