use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn an_unbound_record_is_rejected_even_under_a_current_generation_header() {
    let (mut bytes, fresh) = encoded();
    let wanted = model::leaf_digest(b"# Unique A\n");
    let mut at = b"mrd-parsed-v1\n".len() + 64 + 32 + 8 + "b3c:test".len() + 32;
    while bytes[at] != 255 {
        let len = usize::try_from(u64::from_le_bytes(
            bytes[at + 33..at + 41].try_into().unwrap(),
        ))
        .unwrap();
        if bytes[at + 1..at + 33] == wanted {
            // The earlier unbound checksum cannot establish which parser
            // produced this representation, even when its raw digest matches.
            let mut old = blake3::Hasher::new();
            old.update(&bytes[at..at + 33]);
            old.update(&bytes[at + 73..at + 73 + len]);
            bytes[at + 41..at + 73].copy_from_slice(old.finalize().as_bytes());
            break;
        }
        at += 73 + len;
    }
    let restored =
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &fresh).unwrap();
    assert!(!restored.complete);
    assert!(!restored.docs.contains_key("a.md"));
    assert!(restored.docs.contains_key("b.md"));
}

#[test]
fn a_maximum_unwanted_record_is_streamed_in_small_reads() {
    use std::io::{self, Read};
    struct SmallReads<R>(R);
    impl<R: Read> Read for SmallReads<R> {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            assert!(
                into.len() <= 16 * 1024,
                "an irrelevant record requested a payload-sized buffer"
            );
            self.0.read(into)
        }
    }
    let (mut bytes, _) = encoded();
    let length_at = b"mrd-parsed-v1\n".len() + 64 + 32 + 8 + "b3c:test".len() + 32 + 1 + 32;
    bytes[length_at..length_at + 8]
        .copy_from_slice(&(parse_cache::MAX_ENTRY_BYTES as u64).to_le_bytes());
    let payload_at = length_at + 8 + 32;
    let input =
        (&bytes[..payload_at]).chain(io::repeat(0).take(parse_cache::MAX_ENTRY_BYTES as u64));
    let restored =
        parse_cache::snapshot::read(SmallReads(input), Path::new("/workspace"), &BTreeMap::new())
            .unwrap();
    assert!(restored.docs.is_empty());
    assert!(!restored.complete);
}

fn fixture() -> (
    model::Docs,
    BTreeMap<String, String>,
    BTreeMap<PathBuf, [u8; 32]>,
) {
    let mut docs = model::Docs::new();
    let mut leaves = BTreeMap::new();
    for (path, raw) in [("a.md", "# Unique A\n"), ("b.md", "# Unique B\n")] {
        docs.insert(
            path.to_owned(),
            Arc::new(model::build(raw.to_owned(), syntax::parse(raw))),
        );
        leaves.insert(PathBuf::from(path), model::leaf_digest(raw.as_bytes()));
    }
    leaves.insert(PathBuf::from("bad.md"), model::leaf_digest(&[0xff]));
    (
        docs,
        BTreeMap::from([(
            "bad.md".to_owned(),
            "is not UTF-8 (invalid byte)".to_owned(),
        )]),
        leaves,
    )
}

fn encoded() -> (Vec<u8>, BTreeMap<PathBuf, [u8; 32]>) {
    let (docs, unserved, leaves) = fixture();
    let mut bytes = Vec::new();
    assert_eq!(
        parse_cache::snapshot::write(
            &mut bytes,
            Path::new("/workspace"),
            "b3c:test",
            &docs,
            &unserved,
            &leaves
        )
        .unwrap(),
        2
    );
    (bytes, leaves)
}

#[test]
fn restored_objects_follow_verified_current_paths_and_deduplicate() {
    let (bytes, mut fresh) = encoded();
    let a = fresh.remove(Path::new("a.md")).unwrap();
    fresh.insert(PathBuf::from("renamed.md"), a);
    fresh.insert(PathBuf::from("duplicate.md"), a);
    fresh.remove(Path::new("b.md"));
    let restored =
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &fresh).unwrap();
    assert!(restored.complete);
    assert_eq!(restored.docs.len(), 2);
    assert!(Arc::ptr_eq(
        &restored.docs["renamed.md"],
        &restored.docs["duplicate.md"]
    ));
    assert_eq!(restored.unserved.len(), 1);
    assert_eq!(restored.leaves, fresh);
}

#[test]
fn changed_content_never_reuses_an_older_object_or_invalid_utf8_condition() {
    let (bytes, mut fresh) = encoded();
    fresh.insert(PathBuf::from("a.md"), model::leaf_digest(b"# moved\n"));
    fresh.insert(PathBuf::from("bad.md"), model::leaf_digest(b"valid now"));
    let restored =
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &fresh).unwrap();
    assert!(restored.complete);
    assert_eq!(restored.docs.len(), 1);
    assert!(restored.docs.contains_key("b.md"));
    assert!(restored.unserved.is_empty());
}

#[test]
fn a_bad_record_preserves_other_verified_members() {
    let (mut bytes, fresh) = encoded();
    let raw = b"# Unique A\n";
    let at = bytes.windows(raw.len()).position(|w| w == raw).unwrap();
    bytes[at] ^= 1;
    let restored =
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &fresh).unwrap();
    assert!(!restored.complete);
    assert!(!restored.docs.contains_key("a.md"));
    assert!(restored.docs.contains_key("b.md"));
    assert!(restored.unserved.contains_key("bad.md"));
}

#[test]
fn truncated_footer_keeps_verified_records_but_never_claims_a_complete_save() {
    let (bytes, fresh) = encoded();
    let restored =
        parse_cache::snapshot::read(&bytes[..bytes.len() - 8], Path::new("/workspace"), &fresh)
            .unwrap();
    assert!(!restored.complete);
    assert_eq!(restored.docs.len(), 2);
}

#[test]
fn incompatible_generation_and_workspace_are_whole_cache_misses() {
    let (mut bytes, fresh) = encoded();
    assert!(parse_cache::snapshot::read(bytes.as_slice(), Path::new("/another"), &fresh).is_err());
    bytes[b"mrd-parsed-v1\n".len()] ^= 1;
    assert!(
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &fresh).is_err()
    );
}

#[test]
fn all_truncations_and_length_corruption_are_bounded_and_safe() {
    let (bytes, fresh) = encoded();
    for end in 0..bytes.len() {
        let restored = parse_cache::snapshot::read(&bytes[..end], Path::new("/workspace"), &fresh);
        if let Ok(restored) = restored {
            assert!(!restored.complete);
        }
    }
    let mut bad = bytes;
    // First frame's length: header magic + generation + root binding +
    // fingerprint length/value + header checksum, then tag + content digest.
    let at = b"mrd-parsed-v1\n".len() + 64 + 32 + 8 + "b3c:test".len() + 32 + 1 + 32;
    bad[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let restored =
        parse_cache::snapshot::read(bad.as_slice(), Path::new("/workspace"), &fresh).unwrap();
    assert!(!restored.complete);
    assert!(restored.docs.is_empty());
}

/// A rewrite over the snapshot it replaces copies the document record its
/// leaf set still names, asks for the one member no record covers, drops the
/// member that left, and restores completely (merkle-spec §6.9, saving under
/// a host).
#[test]
fn a_rewrite_carries_the_wanted_document_records_and_encodes_only_the_rest() {
    let (prior, mut leaves) = encoded();
    let (_, unserved, _) = fixture();
    leaves.remove(Path::new("b.md"));
    leaves.insert(PathBuf::from("new.md"), model::leaf_digest(b"# New\n"));

    let mut asked = Vec::new();
    let mut bytes = Vec::new();
    let written = parse_cache::snapshot::rewrite(
        &mut bytes,
        Path::new("/workspace"),
        "b3c:next",
        &unserved,
        &leaves,
        Some(prior.as_slice()),
        |path, digest| {
            asked.push((path.to_owned(), *digest));
            parse_cache::encode(&model::build(
                "# New\n".to_owned(),
                syntax::parse("# New\n"),
            ))
        },
    )
    .unwrap();
    assert_eq!(
        asked,
        [("new.md".to_owned(), model::leaf_digest(b"# New\n"))],
        "a carried record's member is never asked for"
    );
    assert_eq!(
        written,
        parse_cache::snapshot::Rewritten {
            carried: 1,
            encoded: 1
        }
    );

    let restored =
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &leaves).unwrap();
    assert!(restored.complete);
    assert_eq!(restored.fingerprint, "b3c:next");
    assert_eq!(restored.docs["a.md"].raw.as_str(), "# Unique A\n");
    assert_eq!(restored.docs["new.md"].raw.as_str(), "# New\n");
    assert_eq!(restored.docs.len(), 2);
    assert_eq!(restored.unserved.len(), 1);
    assert_eq!(restored.leaves, leaves);

    // The record of the member that left is not in the new snapshot.
    let mut with_b = leaves.clone();
    with_b.insert(PathBuf::from("b.md"), model::leaf_digest(b"# Unique B\n"));
    let restored =
        parse_cache::snapshot::read(bytes.as_slice(), Path::new("/workspace"), &with_b).unwrap();
    assert!(!restored.docs.contains_key("b.md"));
}
