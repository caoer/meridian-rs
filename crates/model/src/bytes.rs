//! Immutable member bytes and the retained-version access contract (merkle §6.8).
//! The owner supplies I/O; model supplies no path lookup, cache or registry.

use std::io;
use std::sync::Arc;

/// The identity selected by a serving descriptor or watcher baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteIdentity {
    pub digest: [u8; 32],
    pub len: u64,
}

/// Access to one retained version, independent of the current workspace file.
/// Implementations keep the identity fixed for the source's entire lifetime.
/// Reading can fail; it cannot substitute another version or report absence.
pub trait ByteSource: std::fmt::Debug + Send + Sync {
    fn identity(&self) -> ByteIdentity;

    /// Read the selected version. The consumer verifies identity before use.
    ///
    /// # Errors
    /// Required backing is unavailable, unreadable or corrupt.
    fn read(&self) -> io::Result<SharedBytes>;
}

/// One immutable payload, shared by UTF-8 documents and byte-level consumers.
/// Both variants retain the original owned buffer, including its capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedBytes(Content);

// Canonical: valid UTF-8 is always Text, including the empty payload.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    Text(Arc<String>),
    Binary(Arc<Vec<u8>>),
}

impl SharedBytes {
    /// Identity of this shared-owner allocation, valid only while an owner
    /// retains it. Opaque: never dereference or persist this value.
    #[must_use]
    pub fn allocation_id(&self) -> usize {
        match &self.0 {
            Content::Text(text) => Arc::as_ptr(text) as usize,
            Content::Binary(bytes) => Arc::as_ptr(bytes) as usize,
        }
    }

    /// Requested heap bytes of the shared allocation and its payload capacity.
    /// Count equal live `allocation_id`s once, not once per clone.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        match &self.0 {
            Content::Text(text) => {
                crate::allocation::arc_bytes::<String>().saturating_add(text.capacity())
            }
            Content::Binary(bytes) => {
                crate::allocation::arc_bytes::<Vec<u8>>().saturating_add(bytes.capacity())
            }
        }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            Content::Text(text) => text.as_bytes(),
            Content::Binary(bytes) => bytes.as_slice(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// Take the shared UTF-8 view without copying the payload.
    ///
    /// # Errors
    /// Returns the unchanged shared bytes when they are not UTF-8.
    pub fn into_text(self) -> Result<Arc<String>, Self> {
        match self.0 {
            Content::Text(text) => Ok(text),
            Content::Binary(_) => Err(self),
        }
    }
}

impl From<Vec<u8>> for SharedBytes {
    fn from(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(text) => Self::from(text),
            Err(error) => Self(Content::Binary(Arc::new(error.into_bytes()))),
        }
    }
}

impl From<String> for SharedBytes {
    fn from(text: String) -> Self {
        Self::from(Arc::new(text))
    }
}

impl From<Arc<String>> for SharedBytes {
    fn from(text: Arc<String>) -> Self {
        Self(Content::Text(text))
    }
}

/// Eager in-memory sources use the same fallible contract as retained backing.
impl ByteSource for SharedBytes {
    fn identity(&self) -> ByteIdentity {
        ByteIdentity {
            digest: crate::leaf_digest(self.as_bytes()),
            len: self.len() as u64,
        }
    }

    fn read(&self) -> io::Result<Self> {
        Ok(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_clones_share_the_immutable_raw_allocation() {
        let raw = "# Version A\n\nretained bytes\n".to_owned();
        let doc = crate::build(raw.clone(), syntax::parse(&raw));
        let entry = doc.clone();
        assert_eq!(
            doc.raw.as_ptr(),
            entry.raw.as_ptr(),
            "document clone copied the raw body"
        );
        drop(doc);
        assert_eq!(entry.raw.as_str(), raw);
    }

    #[test]
    fn owned_bytes_and_all_views_share_one_payload_allocation() {
        let mut bytes = Vec::with_capacity(8192);
        bytes.extend_from_slice(b"# Shared\n");
        let pointer = bytes.as_ptr();
        let body = SharedBytes::from(bytes);
        let watcher = body.read().unwrap();
        let text = body.into_text().unwrap();
        assert_eq!(pointer, watcher.as_bytes().as_ptr());
        assert_eq!(pointer, text.as_ptr());
        assert_eq!(text.capacity(), 8192);
        drop(text);
        assert_eq!(watcher.as_bytes(), b"# Shared\n");
    }

    #[test]
    fn binary_and_empty_versions_keep_their_exact_identity() {
        for bytes in [vec![0xff, 0, 0xfe], Vec::new()] {
            let pointer = bytes.as_ptr();
            let expected = bytes.clone();
            let body = SharedBytes::from(bytes);
            assert_eq!(body.as_bytes().as_ptr(), pointer);
            assert_eq!(body.identity().digest, crate::leaf_digest(&expected));
            assert_eq!(body.identity().len, expected.len() as u64);
            assert_eq!(body.read().unwrap(), body);
            assert_eq!(body.clone().into_text().is_ok(), expected.is_empty());
            assert_eq!(body.as_bytes(), expected);
        }
    }
}
