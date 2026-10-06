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

/// Upper bound on the node storage of a `BTreeMap<K, V>` (or, with `V = ()`,
/// a `BTreeSet<K>`) holding `len` entries. std does not expose its node
/// allocations; every node but the root holds at least five entries, so at
/// most `len / 5 + 1` nodes exist, each charged here at the internal-node size
/// (the leaf's entries plus twelve edges). Keys' and values' own heap is the
/// caller's to add.
#[must_use]
pub fn btree_bytes<K, V>(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (len / 5 + 1).saturating_mul(btree_internal_node::<K, V>())
}

/// std's internal node in declaration order: parent pointer, parent index,
/// length, eleven keys, eleven values, twelve edges. Declaration order never
/// packs tighter than the compiler's own field order, so this is not below
/// the real node; an impossible layout charges MAX.
fn btree_internal_node<K, V>() -> usize {
    let fields = [
        Layout::new::<Option<std::ptr::NonNull<u8>>>(),
        Layout::new::<u16>(),
        Layout::new::<u16>(),
    ];
    let arrays = [Layout::array::<K>(11), Layout::array::<V>(11)];
    let edges = Layout::array::<std::ptr::NonNull<u8>>(12);
    let mut layout = Layout::new::<()>();
    for field in fields
        .into_iter()
        .map(Ok)
        .chain(arrays)
        .chain(std::iter::once(edges))
    {
        let Ok(field) = field else {
            return usize::MAX;
        };
        let Ok((next, _)) = layout.extend(field) else {
            return usize::MAX;
        };
        layout = next;
    }
    layout.pad_to_align().size()
}

/// Heap of a `Vec<String>`: its buffer and each string's.
#[must_use]
pub fn strings_bytes(strings: &[String], capacity: usize) -> usize {
    strings.iter().fold(
        capacity.saturating_mul(size_of::<String>()),
        |bytes, text| bytes.saturating_add(text.capacity()),
    )
}

/// [`arc_bytes`] for an unsized `Arc<T>` payload, from the value's own size
/// and alignment.
#[must_use]
pub fn arc_dyn_bytes<T: ?Sized>(value: &T) -> usize {
    Layout::from_size_align(size_of_val(value), align_of_val(value))
        .ok()
        .and_then(|value| Layout::new::<[usize; 2]>().extend(value).ok())
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
    fn an_unsized_arc_is_charged_as_its_sized_payload() {
        #[repr(align(64))]
        #[derive(Debug)]
        struct Aligned(u8);
        let aligned = Aligned(1);
        assert_eq!(aligned.0, 1);
        let value: std::sync::Arc<dyn std::fmt::Debug> = std::sync::Arc::new(aligned);
        assert_eq!(arc_dyn_bytes(&*value), arc_bytes::<Aligned>());
    }

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
        let added = (doc.root.children.capacity() - capacity) * size_of::<Node>();
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
