//! Heading-path values for one Starlark evaluation, each heading segment
//! allocated once (merkle-spec §6.4, "The policy plane keeps it as well").
//!
//! The facts hold every node's chain as the tree's shared `Arc<str>`
//! segments. Copying each segment into the eval heap per node would stage
//! rows times chain length; this memo allocates one Starlark string per
//! distinct segment and hands the same value to every list that holds it.
//! A Starlark string is immutable and has no identity a predicate can read,
//! so the sharing is unobservable. Each node still gets its own list (a list
//! is mutable), whose size is bounded by the heading depth.

use std::collections::HashMap;
use std::sync::Arc;

use starlark::values::{Heap, Value};

/// The segment strings already allocated on one heap, keyed by the
/// address of the shared text. The facts outlive the evaluation's
/// allocation pass, so an address names one text for the memo's life.
pub(crate) struct SegValues<'v> {
    heap: Heap<'v>,
    seen: HashMap<usize, Value<'v>>,
}

impl<'v> SegValues<'v> {
    pub(crate) fn new(heap: Heap<'v>) -> Self {
        Self {
            heap,
            seen: HashMap::new(),
        }
    }

    /// The Starlark string for one shared segment.
    pub(crate) fn segment(&mut self, seg: &Arc<str>) -> Value<'v> {
        let heap = self.heap;
        *self
            .seen
            .entry(Arc::as_ptr(seg).cast::<u8>().addr())
            .or_insert_with(|| heap.alloc(&**seg))
    }

    /// A fresh list of the chain's segment strings (`[]` for no chain).
    pub(crate) fn list(&mut self, chain: Option<&[Arc<str>]>) -> Value<'v> {
        let segs: Vec<Value<'v>> = chain
            .unwrap_or_default()
            .iter()
            .map(|seg| self.segment(seg))
            .collect();
        self.heap.alloc(segs)
    }
}
