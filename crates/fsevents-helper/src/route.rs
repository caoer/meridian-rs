//! Which wire records one subscriber gets for one device event.

use crate::device::Event;
use crate::fse;
use crate::wire::{self, Node};

/// What [`records_for`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// At least one record was appended.
    Sent,
    /// The event is outside the subscription.
    Nothing,
    /// The subscription's root was deleted or renamed away: its prefix no
    /// longer names the tree, so the subscriber is dropped.
    RootGone,
}

/// `path` relative to `prefix`: `Some(b"")` for the root itself, `Some(rest)`
/// when `path` is under it by whole segments, else `None`.
#[must_use]
pub fn relative<'a>(prefix: &[u8], path: &'a [u8]) -> Option<&'a [u8]> {
    let rest = path.strip_prefix(prefix)?;
    if rest.is_empty() || prefix.ends_with(b"/") {
        return Some(rest);
    }
    rest.strip_prefix(b"/")
}

/// True when `path_with_trailing_slash` is under a directory the kernel
/// withholds from a watcher that is not an Apple system service.
#[must_use]
pub fn ignored(path_with_trailing_slash: &[u8]) -> bool {
    [
        &b"/.Spotlight-V100/"[..],
        b"/.MobileBackups/",
        b"/Backups.backupdb/",
    ]
    .iter()
    .any(|needle| {
        path_with_trailing_slash
            .windows(needle.len())
            .any(|w| w == *needle)
    })
}

/// Append to `out` the records the subscriber of `prefix` gets for `ev`.
pub fn records_for(prefix: &[u8], ev: &Event<'_>, out: &mut Vec<u8>) -> Outcome {
    if ev.lost {
        wire::encode(fse::DROPPED, Node::Unknown, b"", out);
        return Outcome::Sent;
    }
    let [first, second] = ev.paths;
    let gone =
        |p: &[u8]| p == prefix || prefix.strip_prefix(p).is_some_and(|r| r.starts_with(b"/"));
    match (ev.kind, first) {
        (fse::DELETE, Some((p, _))) if p == prefix => return Outcome::RootGone,
        (fse::RENAME, Some((p, _))) if gone(p) => return Outcome::RootGone,
        _ => {}
    }
    let subjects = match ev.kind {
        fse::RENAME | fse::EXCHANGE => [first, second],
        fse::CLONE => [second, None],
        _ => [first, None],
    };
    let mut outcome = Outcome::Nothing;
    for (path, node) in subjects.into_iter().flatten() {
        if let Some(rel) = relative(prefix, path) {
            if !wire::encode(ev.kind, node, rel, out) {
                wire::encode(fse::DROPPED, Node::Unknown, b"", out);
            }
            outcome = Outcome::Sent;
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::{Outcome, records_for, relative};
    use crate::device::Event;
    use crate::fse;
    use crate::wire::{Decoder, Node, Record};

    fn route(prefix: &str, kind: u8, a: &str, b: Option<&str>) -> (Outcome, Vec<(u8, String)>) {
        let ev = Event {
            kind,
            pid: 1,
            lost: false,
            paths: [
                Some((a.as_bytes(), Node::File)),
                b.map(|b| (b.as_bytes(), Node::File)),
            ],
        };
        let mut out = Vec::new();
        let outcome = records_for(prefix.as_bytes(), &ev, &mut out);
        let mut d = Decoder::default();
        d.push(&out);
        let recs = std::iter::from_fn(|| d.next_record())
            .map(|Record { kind, path, .. }| (kind, String::from_utf8(path).unwrap()))
            .collect();
        (outcome, recs)
    }

    #[test]
    fn routing_matches_by_segment_and_drops_a_vanished_root() {
        assert_eq!(relative(b"/a/b", b"/a/bc"), None);
        assert_eq!(relative(b"/a/b", b"/a/b"), Some(&b""[..]));
        assert_eq!(relative(b"/", b"/x/y"), Some(&b"x/y"[..]));
        let (o, r) = route("/a/b", fse::CREATE_FILE, "/a/bc/x", None);
        assert_eq!((o, r), (Outcome::Nothing, vec![]));
        let (o, r) = route("/a/b", fse::RENAME, "/a/b/x.md", Some("/a/elsewhere/x.md"));
        assert_eq!((o, r), (Outcome::Sent, vec![(fse::RENAME, "x.md".into())]));
        let (o, r) = route("/a/b", fse::RENAME, "/tmp/x", Some("/a/b/d/x"));
        assert_eq!((o, r), (Outcome::Sent, vec![(fse::RENAME, "d/x".into())]));
        let (o, r) = route("/a/b", fse::CLONE, "/a/b/src", Some("/a/b/dst"));
        assert_eq!((o, r), (Outcome::Sent, vec![(fse::CLONE, "dst".into())]));
        assert_eq!(
            route("/a/b", fse::DELETE, "/a/b", None).0,
            Outcome::RootGone
        );
        assert_eq!(route("/a/b", fse::DELETE, "/a/b/x", None).0, Outcome::Sent);
        assert_eq!(
            route("/a/b", fse::RENAME, "/a", Some("/z")).0,
            Outcome::RootGone
        );
        assert_eq!(
            route("/a/b", fse::RENAME, "/a/bc", Some("/z")).0,
            Outcome::Nothing
        );
    }
}
