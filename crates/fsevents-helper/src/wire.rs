//! Helper-to-engine records: `kind: u8`, `node: u8`, `len: u16` little-endian,
//! then `len` bytes of path relative to the subscribed root (no leading slash;
//! empty for the root itself and for [`crate::fse::DROPPED`]).

/// What the event's subject is, from the kernel's mode argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    Unknown,
    File,
    Dir,
    Other,
}

impl Node {
    /// Classify a `st_mode`; a zero mode (no mode argument) is `Unknown`.
    #[must_use]
    pub fn from_mode(mode: u32) -> Node {
        match mode & 0o170_000 {
            0 => Node::Unknown,
            0o100_000 => Node::File,
            0o040_000 => Node::Dir,
            _ => Node::Other,
        }
    }

    fn byte(self) -> u8 {
        match self {
            Node::Unknown => 0,
            Node::File => 1,
            Node::Dir => 2,
            Node::Other => 3,
        }
    }

    fn from_byte(b: u8) -> Node {
        match b {
            1 => Node::File,
            2 => Node::Dir,
            3 => Node::Other,
            _ => Node::Unknown,
        }
    }
}

/// One decoded record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: u8,
    pub node: Node,
    pub path: Vec<u8>,
}

/// Bytes before the path.
pub const HEADER: usize = 4;

/// Append one record to `out`. Returns `false` and appends nothing when the
/// path is longer than `u16::MAX` bytes; the caller treats that as loss.
pub fn encode(kind: u8, node: Node, path: &[u8], out: &mut Vec<u8>) -> bool {
    let Ok(len) = u16::try_from(path.len()) else {
        return false;
    };
    out.push(kind);
    out.push(node.byte());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(path);
    true
}

/// The length in bytes of the record starting at `buf[0]`, if its header is
/// complete.
#[must_use]
pub fn record_len(buf: &[u8]) -> Option<usize> {
    let h = buf.get(..HEADER)?;
    Some(HEADER + usize::from(u16::from_le_bytes([h[2], h[3]])))
}

/// Reassembles records from a byte stream fed in arbitrary pieces.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    pos: usize,
}

impl Decoder {
    /// Feed bytes read from the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        } else if self.pos > self.buf.len() / 2 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete record; `None` means more bytes are needed.
    pub fn next_record(&mut self) -> Option<Record> {
        let rest = &self.buf[self.pos..];
        let len = record_len(rest)?;
        let rec = rest.get(..len)?;
        let out = Record {
            kind: rec[0],
            node: Node::from_byte(rec[1]),
            path: rec[HEADER..].to_vec(),
        };
        self.pos += len;
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoder, Node, Record, encode};
    use crate::fse;

    #[test]
    fn records_survive_a_stream_fed_one_byte_at_a_time() {
        let sent = [
            Record {
                kind: fse::CREATE_FILE,
                node: Node::File,
                path: b"a/b.md".to_vec(),
            },
            Record {
                kind: fse::DROPPED,
                node: Node::Unknown,
                path: Vec::new(),
            },
            Record {
                kind: fse::CREATE_DIR,
                node: Node::Dir,
                path: vec![b'x'; 300],
            },
        ];
        let mut bytes = Vec::new();
        for r in &sent {
            assert!(encode(r.kind, r.node, &r.path, &mut bytes));
        }
        assert!(!encode(
            fse::DELETE,
            Node::File,
            &vec![b'y'; 70_000],
            &mut bytes
        ));
        let mut d = Decoder::default();
        let mut got = Vec::new();
        for b in &bytes {
            d.push(std::slice::from_ref(b));
            got.extend(d.next_record());
        }
        assert_eq!(got, sent);
        assert_eq!(d.next_record(), None);
    }
}
