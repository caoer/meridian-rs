//! The feed's ordered source on macOS (`docs/fsevents-helper.md` §4, §5): the
//! link to `mrd-fsevents-helper`, the root process that reads `/dev/fsevents`
//! in kernel order.
//!
//! One thread per feed owns the link. It subscribes, qualifies the tree,
//! switches the feed's source to the helper, and admits the helper's records
//! through the same door the kernel watcher's events take
//! ([`FeedSync::path_event`]). The feed's start waits for the first attempt
//! to settle, so a feed that can be on the helper is on it before its first
//! observation. When the subscription ends, for any reason, the thread
//! switches the source back and parks; a write door that finds the feed on
//! its stream wakes it for another attempt ([`HelperLink::retry`]). No timer
//! runs here.
//!
//! Every change of source is a missed-event collapse, so one floor pass
//! covers the hand-over, and no caller sees an error: a barrier in flight
//! answers `Unproven` on the collapse.

use std::io::Read as _;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fsevents_helper::{fse, wire};
use notify::EventKind;
use notify::event::{CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind, RenameMode};

use super::{FeedSync, RescanCause, hideable_prefix};

/// The directory name the kernel withholds events under, from a watcher that
/// is not an Apple system service, and that can hold members: its two
/// siblings (`.Spotlight-V100`, `.MobileBackups`) are dot-prefixed and so
/// outside the hash domain.
const IGNORED_DIR: &str = "Backups.backupdb";

/// How many entries a directory that arrives whole (a rename, a clone) may
/// hold for the link to search it for [`IGNORED_DIR`] in the reader thread;
/// past it the feed returns to its stream and the next attempt walks the
/// whole root.
const WHOLE_DIR_CAP: usize = 1024;

/// The wait between attempts to reach the helper (§5): a write door that
/// finds the feed on its stream wakes the link once this has passed.
// `Duration::from_mins` not const-stable at MSRV 1.96 (the lib.rs precedent).
#[allow(clippy::duration_suboptimal_units)]
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The bound on writing one barrier request: a helper that has stopped
/// reading costs a barrier this much before the barrier's own timeout takes
/// over.
const ASK_TIMEOUT: Duration = Duration::from_millis(1);

/// One feed's link to the helper.
pub(super) struct HelperLink {
    socket: PathBuf,
    /// The subscription's write end while one is open: barrier requests go
    /// out on it, and [`Self::stop`] shuts the reader's socket down through
    /// it.
    stream: Mutex<Option<UnixStream>>,
    gate: Mutex<Gate>,
    wake: Condvar,
}

/// The link thread's park.
struct Gate {
    /// The feed is gone: the thread exits.
    stop: bool,
    /// A door asked for another attempt.
    retry: bool,
    /// When the thread parked after its last attempt; `None` while one runs.
    parked_at: Option<Instant>,
    retry_after: Duration,
}

impl HelperLink {
    /// Start the link thread for `workspace`'s feed and wait for its first
    /// attempt to settle: the helper is the source, or it is not and the feed
    /// stays on its stream. A thread that cannot be spawned is the second
    /// case.
    pub(super) fn spawn(
        sync: Arc<FeedSync>,
        workspace: PathBuf,
        sentinel_prefix: String,
        socket: PathBuf,
    ) -> Arc<HelperLink> {
        let link = Arc::new(HelperLink {
            socket,
            stream: Mutex::new(None),
            gate: Mutex::new(Gate {
                stop: false,
                retry: false,
                parked_at: None,
                retry_after: RETRY_AFTER,
            }),
            wake: Condvar::new(),
        });
        let theirs = Arc::clone(&link);
        let (settled, first_attempt) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("mrd-fsevents-link".to_owned())
            .spawn(move || theirs.run(&sync, &workspace, &sentinel_prefix, settled));
        // An error is the sender dropped: no thread, or one that has ended.
        let _ = first_attempt.recv();
        link
    }

    /// Ask the helper for a fence that answers barrier `serial` (§3): the
    /// record that arrives once every event raised before now is ahead of it
    /// in this feed's stream. `false` when no subscription is open or the
    /// request could not be written whole.
    pub(super) fn ask(&self, serial: u64) -> bool {
        let stream = self.stream.lock().unwrap_or_else(PoisonError::into_inner);
        stream
            .as_ref()
            .is_some_and(|mut stream| stream.write_all(&serial.to_le_bytes()).is_ok())
    }

    /// A write door found the feed on its stream: wake the link for another
    /// attempt once [`RETRY_AFTER`] has passed since the last one ended.
    pub(super) fn retry(&self) {
        let mut gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        if !gate.retry
            && gate
                .parked_at
                .is_some_and(|at| at.elapsed() >= gate.retry_after)
        {
            gate.retry = true;
            self.wake.notify_one();
        }
    }

    /// The feed is gone: end the thread, whether it is parked or reading.
    pub(super) fn stop(&self) {
        self.gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop = true;
        self.wake.notify_one();
        let stream = self.stream.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(stream) = stream.as_ref() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }

    /// Test seam: the wait between attempts.
    #[cfg(test)]
    pub(super) fn set_retry_after(&self, wait: Duration) {
        self.gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retry_after = wait;
    }

    fn stopped(&self) -> bool {
        self.gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop
    }

    /// The thread: one attempt, then park until a door asks for the next.
    /// `settled` is told once, when the first attempt has joined the helper
    /// or failed to.
    fn run(
        &self,
        sync: &FeedSync,
        workspace: &Path,
        sentinel_prefix: &str,
        settled: std::sync::mpsc::Sender<()>,
    ) {
        let mut settled = Some(settled);
        loop {
            let ended = self.subscription(sync, workspace, sentinel_prefix, &mut settled);
            drop(settled.take());
            *self.stream.lock().unwrap_or_else(PoisonError::into_inner) = None;
            if sync.leave_helper() {
                timing::say!(
                    "feed: {} is back on its own event stream ({}); write doors take the \
                     floor until the helper answers again",
                    workspace.display(),
                    ended.unwrap_or("the subscription ended")
                );
            }
            let mut gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
            gate.parked_at = Some(Instant::now());
            while !gate.stop && !gate.retry {
                gate = self.wake.wait(gate).unwrap_or_else(PoisonError::into_inner);
            }
            if gate.stop {
                return;
            }
            gate.retry = false;
            gate.parked_at = None;
        }
    }

    /// One attempt: subscribe, qualify, switch the source, admit records
    /// until the subscription ends. `Some(reason)` when the feed itself ended
    /// it; `None` when the helper was never reached or the stream closed.
    fn subscription(
        &self,
        sync: &FeedSync,
        workspace: &Path,
        sentinel_prefix: &str,
        settled: &mut Option<std::sync::mpsc::Sender<()>>,
    ) -> Option<&'static str> {
        if !trusted(&self.socket) {
            return None;
        }
        let mut stream = fsevents_helper::client::subscribe(&self.socket, workspace).ok()?;
        // The subscription is open, so a directory created from here on is in
        // the stream; the walk covers the ones already there.
        if !matches!(holds_ignored_dir(workspace, None), Some(false)) {
            return None;
        }
        let ask = stream.try_clone().ok()?;
        let _ = ask.set_write_timeout(Some(ASK_TIMEOUT));
        *self.stream.lock().unwrap_or_else(PoisonError::into_inner) = Some(ask);
        if self.stopped() {
            return None;
        }
        sync.join_helper();
        drop(settled.take());
        timing::say!(
            "feed: {} takes its events from the fsevents helper at {}",
            workspace.display(),
            self.socket.display()
        );
        let mut decoder = wire::Decoder::default();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) => return None,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            };
            decoder.push(&buf[..n]);
            while let Some(record) = decoder.next_record() {
                if !admit_record(sync, workspace, sentinel_prefix, &record) {
                    return Some("a directory the kernel reports no events under");
                }
            }
        }
    }
}

impl FeedSync {
    /// Switch the feed's source to the helper, under a missed-event collapse:
    /// whatever the stream had not yet delivered is covered by the floor pass
    /// the collapse orders.
    fn join_helper(&self) {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.helper = true;
        s.collapse(RescanCause::MissedEvent);
        drop(s);
        self.cookie.notify_all();
    }

    /// Switch the feed's source back to its stream, under the same collapse.
    /// `false` when the helper was not the source.
    fn leave_helper(&self) -> bool {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !s.helper {
            return false;
        }
        s.helper = false;
        s.collapse(RescanCause::MissedEvent);
        drop(s);
        self.cookie.notify_all();
        true
    }
}

/// Whether the socket file is one the engine may believe: root's, or the
/// engine's own user's (§2).
fn trusted(socket: &Path) -> bool {
    // SAFETY: `geteuid` takes no argument and cannot fail.
    let me = unsafe { libc::geteuid() };
    std::fs::metadata(socket).is_ok_and(|meta| meta.uid() == 0 || meta.uid() == me)
}

/// Admit one helper record. `false` ends the subscription: the record shows
/// a directory the kernel reports no events under.
fn admit_record(
    sync: &FeedSync,
    workspace: &Path,
    sentinel_prefix: &str,
    record: &wire::Record,
) -> bool {
    if record.kind == fse::DROPPED {
        let mut s = sync.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.collapse(RescanCause::Overflow);
        drop(s);
        sync.cookie.notify_all();
        return true;
    }
    if record.kind == fse::FENCE {
        // Every event raised before the request for this serial is admitted
        // above, or a loss record collapsed the set: the barrier's proof.
        if let Ok(serial) = <[u8; 8]>::try_from(record.path.as_slice()) {
            let mut s = sync.state.lock().unwrap_or_else(PoisonError::into_inner);
            s.cookie_seen = s.cookie_seen.max(u64::from_le_bytes(serial));
            drop(s);
            sync.cookie.notify_all();
        }
        return true;
    }
    let rel = Path::new(std::ffi::OsStr::from_bytes(&record.path));
    if rel.as_os_str().is_empty() {
        return true;
    }
    let hideable = hideable_prefix(rel);
    if hideable && rel.components().any(|c| c.as_os_str() == IGNORED_DIR) {
        return false;
    }
    let abs = workspace.join(rel);
    let whole_directory = hideable
        && matches!(record.kind, fse::RENAME | fse::CLONE)
        && (record.node == wire::Node::Dir || abs.is_dir());
    if whole_directory && !matches!(holds_ignored_dir(&abs, Some(WHOLE_DIR_CAP)), Some(false)) {
        return false;
    }
    let kind = event_kind(record, &abs);
    let s = sync.state.lock().unwrap_or_else(PoisonError::into_inner);
    let s = sync.path_event(s, rel, &abs, kind, sentinel_prefix);
    let doubt = s.doubt.is_some();
    drop(s);
    if doubt {
        sync.cookie.notify_all();
    }
    true
}

/// The watcher event kind a helper record stands for, so both sources reach
/// one admission rule ([`super::FeedState::admit`]).
fn event_kind(record: &wire::Record, abs: &Path) -> EventKind {
    let is_dir = record.node == wire::Node::Dir;
    match record.kind {
        fse::CREATE_DIR => EventKind::Create(CreateKind::Folder),
        fse::CLONE if is_dir || abs.is_dir() => EventKind::Create(CreateKind::Folder),
        fse::CREATE_FILE | fse::CLONE => EventKind::Create(CreateKind::File),
        fse::DELETE => EventKind::Remove(match record.node {
            wire::Node::Dir => RemoveKind::Folder,
            wire::Node::File => RemoveKind::File,
            wire::Node::Unknown | wire::Node::Other => RemoveKind::Any,
        }),
        fse::RENAME => EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
        fse::CONTENT_MODIFIED | fse::EXCHANGE => {
            EventKind::Modify(ModifyKind::Data(DataChange::Any))
        }
        fse::STAT_CHANGED
        | fse::FINDER_INFO_CHANGED
        | fse::CHOWN
        | fse::XATTR_MODIFIED
        | fse::XATTR_REMOVED => EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
        _ => EventKind::Modify(ModifyKind::Any),
    }
}

/// Does the tree under `dir` hold a directory named [`IGNORED_DIR`] on a
/// path that can hold members (no dot-prefixed segment below `dir`)?
/// `None` is "cannot tell": a directory that would not list, or more than
/// `cap` entries. A `dir` already gone holds nothing. Symlinks are not
/// followed.
fn holds_ignored_dir(dir: &Path, cap: Option<usize>) -> Option<bool> {
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let listing = match std::fs::read_dir(&next) {
            Ok(listing) => listing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && next == dir => {
                return Some(false);
            }
            Err(_) => return None,
        };
        for entry in listing {
            let entry = entry.ok()?;
            seen += 1;
            if cap.is_some_and(|cap| seen > cap) {
                return None;
            }
            if !entry.file_type().ok()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if name == IGNORED_DIR {
                return Some(true);
            }
            if name.to_str().is_some_and(fs::domain::dot_segment) {
                continue;
            }
            stack.push(entry.path());
        }
    }
    Some(false)
}

/// Test support: a stand-in for `mrd-fsevents-helper`.
#[cfg(test)]
pub(crate) mod fake {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use fsevents_helper::{fse, wire};

    /// A stand-in helper on a socket in `dir`: it accepts one subscriber at a
    /// time, answers `ok`, sends the records the test queued, and answers a
    /// barrier request with a fence behind them — the order a kernel queue
    /// gives a write followed by a barrier.
    pub(crate) struct FakeHelper {
        pub(crate) socket: PathBuf,
        queue: mpsc::Sender<Vec<u8>>,
        hang_up: mpsc::Sender<()>,
    }

    impl FakeHelper {
        pub(crate) fn start(dir: &Path) -> FakeHelper {
            let socket = dir.join("helper.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let (queue, queued) = mpsc::channel::<Vec<u8>>();
            let (hang_up, ended) = mpsc::channel::<()>();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut hello = [0u8; 8];
                    if stream.read_exact(&mut hello).is_err() || hello != fsevents_helper::HELLO {
                        continue;
                    }
                    if stream.write_all(fsevents_helper::ACK).is_err() {
                        continue;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(5)));
                    let mut asked = Vec::new();
                    let mut buf = [0u8; 64];
                    loop {
                        if ended.try_recv().is_ok() {
                            break;
                        }
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => asked.extend_from_slice(&buf[..n]),
                            Err(_) => {}
                        }
                        let mut out: Vec<u8> = queued.try_iter().flatten().collect();
                        let (serials, _) = asked.as_chunks::<8>();
                        let whole = serials.len() * 8;
                        for serial in serials {
                            wire::encode(fse::FENCE, wire::Node::Unknown, serial, &mut out);
                        }
                        asked.drain(..whole);
                        if !out.is_empty() && stream.write_all(&out).is_err() {
                            break;
                        }
                    }
                }
            });
            FakeHelper {
                socket,
                queue,
                hang_up,
            }
        }

        /// End the open subscription; the next one is accepted.
        pub(crate) fn hang_up(&self) {
            self.hang_up.send(()).unwrap();
        }

        /// Queue one record for the subscriber.
        pub(crate) fn send(&self, kind: u8, node: wire::Node, rel: &str) {
            let mut out = Vec::new();
            wire::encode(kind, node, rel.as_bytes(), &mut out);
            self.queue.send(out).unwrap();
        }
    }

    /// Poll `holds` until it is true; panic naming `what` after 10 s.
    pub(crate) fn until(what: &str, mut holds: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !holds() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CookieOutcome, Pending, WorkspaceFeed};
    use super::fake::{FakeHelper, until};
    use super::*;

    fn feed_on_helper(root: &Path, helper: &FakeHelper) -> WorkspaceFeed {
        let feed = WorkspaceFeed::start(root, fs::stable::FeedGen::default()).expect("watcher");
        feed.attach_helper(root, helper.socket.clone());
        until("the helper to become the source", || feed.on_helper());
        feed
    }

    const WAIT: Duration = Duration::from_secs(10);

    /// §4: the helper is the feed's one source. The switch is a missed-event
    /// collapse; after it a barrier is answered by the helper's fence and
    /// writes no sentinel, a member the helper names is dirty, one only the
    /// kernel watcher saw is not, and a take that consumed something leaves
    /// the engine's own event behind it.
    #[test]
    fn the_helper_is_the_feeds_one_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("ws");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let helper = FakeHelper::start(dir.path());
        let feed = feed_on_helper(&root, &helper);
        assert_eq!(feed.take(), Pending::All(RescanCause::MissedEvent));

        std::fs::write(root.join("notes/unseen.md"), "the watcher alone saw this").unwrap();
        helper.send(fse::CONTENT_MODIFIED, wire::Node::File, "notes/named.md");
        assert_eq!(feed.cookie_barrier(&root, WAIT), CookieOutcome::Seen);
        assert_eq!(feed.cookie_written(), 0, "a sentinel was written");
        // The kernel watcher has long delivered `unseen.md`; it was discarded.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            feed.take(),
            Pending::Paths(vec![PathBuf::from("notes/named.md")])
        );
        assert!(root.join(crate::feed::COOKIE_DIR).join("take").exists());
    }

    /// §4, loss: the helper's loss record is the overflow cause, one floor
    /// pass, and the helper stays the source.
    #[test]
    fn a_loss_record_collapses_once_and_keeps_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        let helper = FakeHelper::start(dir.path());
        let feed = feed_on_helper(&root, &helper);
        let _ = feed.take();

        helper.send(fse::DROPPED, wire::Node::Unknown, "");
        until("the loss to collapse the set", || feed.stats().all_dirty);
        assert_eq!(feed.cookie_barrier(&root, WAIT), CookieOutcome::Unproven);
        assert_eq!(feed.take(), Pending::All(RescanCause::Overflow));
        assert!(feed.on_helper());
        assert_eq!(feed.cookie_barrier(&root, WAIT), CookieOutcome::Seen);
    }

    /// §5: a subscription that ends returns the feed to its stream under a
    /// missed-event collapse, and a door's retry reaches a helper that came
    /// back.
    #[test]
    fn a_lost_helper_returns_the_feed_to_its_stream_until_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        let helper = FakeHelper::start(dir.path());
        let feed = feed_on_helper(&root, &helper);
        let _ = feed.take();

        helper.hang_up();
        until("the feed to leave the helper", || !feed.on_helper());
        assert_eq!(feed.take(), Pending::All(RescanCause::MissedEvent));
        // Its own stream sights the sentinel again.
        assert_eq!(feed.cookie_barrier(&root, WAIT), CookieOutcome::Seen);

        feed.set_helper_retry_after(Duration::ZERO);
        until("a retry to reach the helper", || {
            feed.retry_helper();
            feed.on_helper()
        });
        assert_eq!(feed.take(), Pending::All(RescanCause::MissedEvent));
    }

    /// §4, qualifying: a tree that holds `Backups.backupdb` never takes the
    /// helper, and one that grows it, or receives a directory whole that
    /// holds it, leaves the helper.
    #[test]
    fn a_directory_the_kernel_withholds_keeps_the_feed_off_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("ws");
        std::fs::create_dir_all(root.join("a").join(IGNORED_DIR)).unwrap();
        assert_eq!(holds_ignored_dir(&root, None), Some(true));
        assert_eq!(holds_ignored_dir(&root.join("gone"), None), Some(false));
        std::fs::remove_dir(root.join("a").join(IGNORED_DIR)).unwrap();
        std::fs::create_dir_all(root.join(".hidden").join(IGNORED_DIR)).unwrap();
        assert_eq!(holds_ignored_dir(&root, None), Some(false));
        assert_eq!(holds_ignored_dir(&root, Some(1)), None);

        let helper = FakeHelper::start(&base);
        let feed = feed_on_helper(&root, &helper);
        helper.send(fse::CREATE_DIR, wire::Node::Dir, "a/Backups.backupdb");
        until("the created directory to end the subscription", || {
            !feed.on_helper()
        });

        feed.set_helper_retry_after(Duration::ZERO);
        until("the next attempt to qualify", || {
            feed.retry_helper();
            feed.on_helper()
        });
        std::fs::create_dir_all(root.join("moved").join(IGNORED_DIR)).unwrap();
        helper.send(fse::RENAME, wire::Node::Dir, "moved");
        until("the moved-in directory to end the subscription", || {
            !feed.on_helper()
        });
    }
}
