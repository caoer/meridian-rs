//! Requested Rust heap layout for immutable corpus bodies (merkle §6.4).
//! Allocator bookkeeping and native memory are outside this census.

use crate::Document;

/// Heap owned by a document, with raw bytes separate for shared-owner dedup.
/// Excludes the inline root `Document`, which belongs to its enclosing owner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocumentBytes {
    pub raw: usize,
    pub tree: usize,
}

/// Count the capacities and shared allocation that this document owns.
#[must_use]
pub fn document(_document: &Document) -> DocumentBytes {
    DocumentBytes::default()
}

/// Size requested for a sized `Arc` allocation, including its two counters.
#[must_use]
pub const fn arc_bytes<T>() -> usize {
    // Red-first scaffold; the calibrated implementation follows.
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spare_tree_capacity_is_part_of_the_charge() {
        let mut doc = crate::build("# A\n".to_owned(), syntax::parse("# A\n"));
        let before = document(&doc);
        let capacity = doc.root.children.capacity();
        doc.root.children.reserve_exact(31);
        let added = (doc.root.children.capacity() - capacity) * std::mem::size_of::<crate::Node>();
        assert!(added > 0);
        assert_eq!(document(&doc).tree - before.tree, added);
    }

    #[test]
    fn a_document_and_its_byte_view_name_one_raw_allocation() {
        let mut raw = String::with_capacity(8192);
        raw.push_str("# Shared\n");
        let doc = crate::build(raw, syntax::parse("# Shared\n"));
        let bytes = crate::bytes::SharedBytes::from(std::sync::Arc::clone(&doc.raw));
        assert_eq!(document(&doc).raw, bytes.heap_bytes());
        assert!(document(&doc).raw >= 8192 + std::mem::size_of::<String>());
        assert_eq!(bytes.allocation_id(), bytes.clone().allocation_id());
    }
}
