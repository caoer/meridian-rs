//! Parse an explicitly retained member version, never a current-path fallback.

use model::bytes::ByteSource;
use std::{fmt, io};

/// Verify the selected version and parse it without copying its shared text.
///
/// # Errors
/// Source faults, identity mismatch and invalid UTF-8 remain distinct;
/// none is absence. Empty valid bytes are a valid empty document.
pub fn document(source: &dyn ByteSource) -> Result<model::Document, HydrationError> {
    let identity = source.identity();
    let bytes = source.read().map_err(HydrationError::Source)?;
    if bytes.len() as u64 != identity.len || model::leaf_digest(bytes.as_bytes()) != identity.digest
    {
        return Err(HydrationError::IdentityMismatch);
    }
    let text = bytes.into_text().map_err(|_| HydrationError::InvalidUtf8)?;
    let nodes = syntax::parse(&text);
    Ok(model::build_shared(text, nodes))
}

/// A required version failed to become a parsed document. In particular, a
/// source's `InvalidData` fault is not the member-level UTF-8 condition.
#[derive(Debug)]
pub enum HydrationError {
    Source(io::Error),
    IdentityMismatch,
    InvalidUtf8,
}

impl fmt::Display for HydrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(f),
            Self::IdentityMismatch => {
                f.write_str("retained bytes do not match the selected identity")
            }
            Self::InvalidUtf8 => f.write_str("retained document bytes are not UTF-8"),
        }
    }
}

impl std::error::Error for HydrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::IdentityMismatch | Self::InvalidUtf8 => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::bytes::{ByteIdentity, SharedBytes};

    #[derive(Debug)]
    struct Source {
        identity: ByteIdentity,
        bytes: SharedBytes,
        fault: Option<io::ErrorKind>,
    }

    impl Source {
        fn new(bytes: Vec<u8>) -> Self {
            let bytes = SharedBytes::from(bytes);
            Self {
                identity: bytes.identity(),
                bytes,
                fault: None,
            }
        }
    }

    impl ByteSource for Source {
        fn identity(&self) -> ByteIdentity {
            self.identity
        }
        fn read(&self) -> io::Result<SharedBytes> {
            match self.fault {
                Some(kind) => Err(io::Error::new(kind, "required version unavailable")),
                None => Ok(self.bytes.clone()),
            }
        }
    }

    #[test]
    fn retained_hydration_shares_bytes_and_matches_eager_spans_and_revs() {
        let raw = "---\naliases: [old]\n---\n# Version A\n\n[[other#Heading]]\n";
        let source = Source::new(raw.as_bytes().to_vec());
        let hydrated = document(&source).expect("hydrate exact retained bytes");
        let eager = model::build(raw.to_owned(), syntax::parse(raw));
        assert_eq!(hydrated.root, eager.root);
        assert_eq!(hydrated.raw.as_str(), raw);
        assert_eq!(hydrated.raw.as_ptr(), source.bytes.as_bytes().as_ptr());
        drop(source);
        assert_eq!(hydrated.raw.as_str(), raw);
    }

    #[test]
    fn source_errors_are_not_absence_or_empty_documents() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::InvalidData,
        ] {
            let mut source = Source::new(b"original".to_vec());
            source.fault = Some(kind);
            let HydrationError::Source(error) = document(&source).unwrap_err() else {
                panic!("required-source error was reclassified");
            };
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "required version unavailable");
        }
    }

    #[test]
    fn wrong_digest_or_length_refuses_the_returned_version() {
        let mut source = Source::new(b"original".to_vec());
        source.bytes = SharedBytes::from(b"new text".to_vec());
        assert!(matches!(
            document(&source),
            Err(HydrationError::IdentityMismatch)
        ));
        source.bytes = SharedBytes::from(b"original".to_vec());
        source.identity.len += 1;
        assert!(matches!(
            document(&source),
            Err(HydrationError::IdentityMismatch)
        ));
    }

    #[test]
    fn invalid_utf8_is_an_error_but_empty_bytes_are_a_document() {
        let binary = Source::new(vec![0xff, 0, 0xfe]);
        assert!(matches!(
            document(&binary),
            Err(HydrationError::InvalidUtf8)
        ));
        let empty = document(&Source::new(Vec::new())).unwrap();
        assert!(empty.raw.is_empty());
        assert_eq!(empty.root.span, 0..0);
        assert_eq!(
            empty.root.node_rev,
            model::build(String::new(), Vec::new()).root.node_rev
        );
    }
}
