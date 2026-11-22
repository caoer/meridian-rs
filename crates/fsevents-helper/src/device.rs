//! The `/dev/fsevents` clone read buffer, parsed without allocation.
//!
//! One kernel record is `i32 type`, `i32 pid`, then arguments `u16 argtype,
//! u16 len, len bytes` until [`ARG_DONE`], which has no length. A path is
//! [`ARG_STRING`] (NUL-terminated), and the [`ARG_MODE`] after it belongs to
//! that path. A rename, exchange or clone carries a second path group, the
//! destination.

use crate::fse;
use crate::wire::Node;

pub const ARG_STRING: u16 = 0x0002;
pub const ARG_MODE: u16 = 0x000a;
pub const ARG_DONE: u16 = 0xb33f;
/// `type & TYPE_MASK` is the event type; the bits above are flags.
pub const TYPE_MASK: i32 = 0x0fff;
/// The kernel's "this watcher lost events" record.
pub const EVENTS_DROPPED: i32 = 999;
/// `FSE_CONTAINS_DROPPED_EVENTS << FSE_FLAG_SHIFT`: the event's path is not
/// trustworthy (it could not be built, or was truncated). Set only for a
/// watcher that asked for extended info.
pub const CONTAINS_DROPPED: i32 = 0x2000;

/// One event as the subscriber router needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event<'a> {
    /// An [`fse`] type; [`fse::DROPPED`] for the kernel's loss record.
    pub kind: u8,
    /// The process that raised the event, as the kernel recorded it.
    pub pid: i32,
    /// Events were lost here: the subscriber must assume anything changed.
    pub lost: bool,
    /// Source then destination, each with the node its mode names.
    pub paths: [Option<(&'a [u8], Node)>; 2],
}

/// The buffer ends inside a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

/// Iterate the events in one `read` of the clone descriptor. Types the helper
/// does not report and unknown argument types are skipped by length.
#[must_use]
pub fn parse(buf: &[u8]) -> Events<'_> {
    Events { buf, pos: 0 }
}

/// Iterator returned by [`parse`]; yields `Err(Malformed)` once, then ends.
#[derive(Debug)]
pub struct Events<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Events<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        let end = self.pos.checked_add(n).ok_or(Malformed)?;
        let s = self.buf.get(self.pos..end).ok_or(Malformed)?;
        self.pos = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, Malformed> {
        let b = self.take(2)?;
        Ok(u16::from_ne_bytes([b[0], b[1]]))
    }

    fn i32(&mut self) -> Result<i32, Malformed> {
        let b = self.take(4)?;
        Ok(i32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// One whole kernel record; `None` for a record the helper skips.
    fn record(&mut self) -> Result<Option<Event<'a>>, Malformed> {
        let ty = self.i32()?;
        let pid = self.i32()?;
        let code = ty & TYPE_MASK;
        let mut paths: [Option<(&'a [u8], Node)>; 2] = [None, None];
        let mut n = 0;
        loop {
            let at = self.u16()?;
            if at == ARG_DONE {
                break;
            }
            let len = usize::from(self.u16()?);
            let data = self.take(len)?;
            match at {
                ARG_STRING if n < 2 => {
                    let path = data.strip_suffix(&[0]).unwrap_or(data);
                    paths[n] = Some((path, Node::Unknown));
                    n += 1;
                }
                ARG_MODE if n > 0 && len == 4 => {
                    let mode = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
                    if let Some((_, node)) = &mut paths[n - 1] {
                        *node = Node::from_mode(mode);
                    }
                }
                _ => {}
            }
        }
        if code == EVENTS_DROPPED {
            return Ok(Some(Event {
                kind: fse::DROPPED,
                pid,
                lost: true,
                paths: [None, None],
            }));
        }
        match u8::try_from(code) {
            Ok(kind) if fse::reported(kind) => Ok(Some(Event {
                kind,
                pid,
                lost: ty & CONTAINS_DROPPED != 0,
                paths,
            })),
            _ => Ok(None),
        }
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = Result<Event<'a>, Malformed>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.buf.len() {
            match self.record() {
                Ok(Some(ev)) => return Some(Ok(ev)),
                Ok(None) => {}
                Err(e) => {
                    self.pos = self.buf.len();
                    return Some(Err(e));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ARG_DONE, ARG_MODE, ARG_STRING, EVENTS_DROPPED, Event, Malformed, parse};
    use crate::fse;
    use crate::wire::Node;

    /// Builds kernel-format records the way `copy_out_kfse` lays them out.
    #[derive(Default)]
    struct Buf(Vec<u8>);

    impl Buf {
        fn head(&mut self, ty: i32) -> &mut Self {
            self.0.extend_from_slice(&ty.to_ne_bytes());
            self.0.extend_from_slice(&4242_i32.to_ne_bytes());
            self
        }
        fn arg(&mut self, at: u16, data: &[u8]) -> &mut Self {
            self.0.extend_from_slice(&at.to_ne_bytes());
            self.0
                .extend_from_slice(&u16::try_from(data.len()).unwrap().to_ne_bytes());
            self.0.extend_from_slice(data);
            self
        }
        fn path(&mut self, p: &str, mode: Option<u32>) -> &mut Self {
            let mut s = p.as_bytes().to_vec();
            s.push(0);
            self.arg(ARG_STRING, &s);
            // dev, ino, mode, uid, gid; absent for a rename's new destination.
            if let Some(m) = mode {
                self.arg(9, &7_u32.to_ne_bytes())
                    .arg(7, &9_u64.to_ne_bytes())
                    .arg(ARG_MODE, &(m | 1 << 31).to_ne_bytes())
                    .arg(8, &501_u32.to_ne_bytes())
                    .arg(11, &20_u32.to_ne_bytes());
            }
            self
        }
        fn done(&mut self) -> &mut Self {
            self.arg(5, &1_u64.to_ne_bytes());
            self.0.extend_from_slice(&ARG_DONE.to_ne_bytes());
            self
        }
    }

    #[test]
    fn parse_reads_paths_modes_and_loss_and_skips_unknown_types() {
        let mut b = Buf::default();
        b.head(i32::from(fse::CREATE_FILE))
            .path("/r/a.md", Some(0o100_644))
            .done();
        b.head(11).arg(ARG_STRING, b"/r/docid\0").done();
        b.head(i32::from(fse::RENAME))
            .path("/r/a.md", Some(0o100_644))
            .path("/r/b.md", None)
            .done();
        b.head(EVENTS_DROPPED);
        b.0.extend_from_slice(&ARG_DONE.to_ne_bytes());
        b.head(i32::from(fse::CONTENT_MODIFIED) | 0x2000)
            .path("/r", Some(0o040_755))
            .done();
        let evs: Vec<_> = parse(&b.0).collect::<Result<_, _>>().unwrap();
        let p = |s: &'static str, n| Some((s.as_bytes(), n));
        assert_eq!(
            evs,
            [
                Event {
                    kind: fse::CREATE_FILE,
                    pid: 4242,
                    lost: false,
                    paths: [p("/r/a.md", Node::File), None]
                },
                Event {
                    kind: fse::RENAME,
                    pid: 4242,
                    lost: false,
                    paths: [p("/r/a.md", Node::File), p("/r/b.md", Node::Unknown)],
                },
                Event {
                    kind: fse::DROPPED,
                    pid: 4242,
                    lost: true,
                    paths: [None, None]
                },
                Event {
                    kind: fse::CONTENT_MODIFIED,
                    pid: 4242,
                    lost: true,
                    paths: [p("/r", Node::Dir), None]
                },
            ]
        );
        for cut in [3, 9, b.0.len() - 1] {
            assert_eq!(
                parse(&b.0[..cut]).last(),
                Some(Err(Malformed)),
                "cut at {cut}"
            );
        }
    }
}
