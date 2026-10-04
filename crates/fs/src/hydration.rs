//! Parse an explicitly retained member version, never a current-path fallback.

use model::bytes::ByteSource;
use std::io;

/// Verify the selected version and parse it without copying its shared text.
///
/// # Errors
/// Required-source errors propagate; identity mismatch or non-UTF-8 content
/// is `InvalidData`. Empty valid bytes are a valid empty document.
pub fn document(_source: &dyn ByteSource) -> io::Result<model::Document> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "retained document hydration is not implemented",
    ))
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
        ] {
            let mut source = Source::new(b"original".to_vec());
            source.fault = Some(kind);
            let error = document(&source).unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "required version unavailable");
        }
    }

    #[test]
    fn wrong_digest_or_length_refuses_the_returned_version() {
        let mut source = Source::new(b"original".to_vec());
        source.bytes = SharedBytes::from(b"new text".to_vec());
        assert_eq!(
            document(&source).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        source.bytes = SharedBytes::from(b"original".to_vec());
        source.identity.len += 1;
        assert_eq!(
            document(&source).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn invalid_utf8_is_an_error_but_empty_bytes_are_a_document() {
        let binary = Source::new(vec![0xff, 0, 0xfe]);
        assert_eq!(
            document(&binary).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let empty = document(&Source::new(Vec::new())).unwrap();
        assert!(empty.raw.is_empty());
        assert_eq!(empty.root.span, 0..0);
        assert_eq!(
            empty.root.node_rev,
            model::build(String::new(), Vec::new()).root.node_rev
        );
    }
}
