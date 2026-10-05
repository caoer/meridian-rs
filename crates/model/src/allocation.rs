//! Requested Rust heap layout for immutable corpus bodies (merkle §6.4).
//! Allocator bookkeeping and native memory are outside this census.

use std::alloc::Layout;
use std::mem::size_of;

use crate::{Document, Node, NodeKind};

/// Heap owned by a document, with raw bytes separate for shared-owner dedup.
/// Excludes the inline root `Document`, which belongs to its enclosing owner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocumentBytes {
    pub raw: usize,
    pub tree: usize,
}

/// Count the capacities and shared allocation that this document owns.
#[must_use]
pub fn document(document: &Document) -> DocumentBytes {
    let Document { raw, root } = document;
    DocumentBytes {
        raw: arc_bytes::<String>().saturating_add(raw.capacity()),
        tree: node_heap(root),
    }
}

/// Size requested for a sized `Arc` allocation, including its two counters.
#[must_use]
pub fn arc_bytes<T>() -> usize {
    // Arc's repr(C) allocation: strong/weak counters followed by T, with
    // T's alignment and final padding. An impossible layout charges MAX.
    Layout::new::<[usize; 2]>()
        .extend(Layout::new::<T>())
        .map_or(usize::MAX, |(layout, _)| layout.pad_to_align().size())
}

fn node_heap(node: &Node) -> usize {
    // Destructure every field: adding owned model data must revisit its charge.
    let Node {
        kind,
        span: _,
        node_rev,
        hpath,
        children,
    } = node;
    let mut bytes = node_rev.0.capacity();
    bytes = bytes.saturating_add(children.capacity().saturating_mul(size_of::<Node>()));
    if let Some(path) = hpath {
        bytes = bytes.saturating_add(path.capacity().saturating_mul(size_of::<String>()));
        for segment in path {
            bytes = bytes.saturating_add(segment.capacity());
        }
    }
    bytes = bytes.saturating_add(kind_heap(kind));
    for child in children {
        bytes = bytes.saturating_add(node_heap(child));
    }
    bytes
}

fn kind_heap(kind: &NodeKind) -> usize {
    match kind {
        NodeKind::Document {
            path: text,
            line_count: _,
        }
        | NodeKind::Section {
            heading_text: text,
            level: _,
        }
        | NodeKind::Heading { text, level: _ }
        | NodeKind::CodeBlock {
            lang: text,
            unterminated: _,
        }
        | NodeKind::Link { target: text }
        | NodeKind::Anchor { name: text }
        | NodeKind::Tag { name: text } => text.capacity(),
        NodeKind::Callout { r#type, fold } => r#type.capacity().saturating_add(fold.capacity()),
        NodeKind::Frontmatter { map } => map.0.iter().fold(
            map.0
                .capacity()
                .saturating_mul(size_of::<(String, String)>()),
            |bytes, (key, value)| {
                bytes
                    .saturating_add(key.capacity())
                    .saturating_add(value.capacity())
            },
        ),
        NodeKind::Wikilink {
            target,
            heading,
            block,
            alias,
        }
        | NodeKind::Embed {
            target,
            heading,
            block,
            alias,
        } => [heading, block, alias]
            .into_iter()
            .flatten()
            .fold(target.capacity(), |bytes, text| {
                bytes.saturating_add(text.capacity())
            }),
        NodeKind::Paragraph
        | NodeKind::List
        | NodeKind::ListItem
        | NodeKind::TaskItem {
            checked: _,
            depth: _,
        }
        | NodeKind::Table
        | NodeKind::InlineCode
        | NodeKind::Comment => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_layout_accounts_for_alignment_and_empty_payload_owners() {
        #[repr(align(64))]
        struct Aligned(u8);
        let value = Aligned(1);
        assert_eq!(value.0, 1);
        assert_eq!(arc_bytes::<Aligned>(), 128);
        let a = crate::bytes::SharedBytes::from(String::new());
        let b = crate::bytes::SharedBytes::from(String::new());
        assert_ne!(
            a.allocation_id(),
            b.allocation_id(),
            "empty buffers still have distinct shared owners"
        );
        assert_eq!(a.heap_bytes(), arc_bytes::<String>());
        assert_eq!(b.heap_bytes(), a.heap_bytes());
    }

    #[test]
    fn spare_tree_capacity_is_part_of_the_charge() {
        let mut doc = crate::build("# A\n".to_owned(), syntax::parse("# A\n"));
        let before = document(&doc);
        let capacity = doc.root.children.capacity();
        doc.root.children.reserve_exact(31);
        let added = (doc.root.children.capacity() - capacity) * size_of::<crate::Node>();
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
        assert!(document(&doc).raw >= 8192 + size_of::<String>());
        assert_eq!(bytes.allocation_id(), bytes.clone().allocation_id());
    }
}
