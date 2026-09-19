//! Parking the daemon: the primitives that let every resident thread sleep in
//! the kernel until one of the events it serves arrives, instead of ticking.
//!
//! Three shapes, one syscall (`poll(2)`):
//!
//! - [`wait_readable`] parks on a set of descriptors with an optional
//!   deadline.
//! - [`Doorbell`] is a one-shot broadcast (shutdown, the idle-exit request):
//!   ring it once and every waiter, present and future, sees it. It is the
//!   read half of a socketpair whose write half is shut down by the ring, so
//!   the descriptor reads EOF forever after.
//! - [`ChangeWakers`] and [`Waker`] are a many-shot, per-waiter signal (a
//!   frame recorded on a ring, a kernel event on a workspace). Each waiter
//!   owns its own socketpair; a wake writes one byte to each registered
//!   waiter that has not drained the previous one, so a burst coalesces and
//!   no buffer ever fills.
//!
//! Why descriptors and not a condvar: the push plane already parks on a
//! socket (the peer's EOF is the one death signal a quiet workspace
//! produces), and a thread can wait on a descriptor and a condvar at once
//! only by ticking between them. With every signal a descriptor, one `poll`
//! waits on all of them and the tick goes away. `docs/status.md` § The daemon
//! at rest states what the daemon wakes for.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

/// Park until at least one of `fds` is readable (data, EOF, or an error
/// condition — anything a read would now answer without blocking), or until
/// `timeout` passes; `None` is no deadline and `Some(Duration::ZERO)` is a
/// probe that never parks. Returns one flag per descriptor, in order; every
/// flag `false` means nothing was readable by the deadline. The kernel is
/// always asked at least once, so a descriptor that is readable NOW is
/// reported even when the deadline has already lapsed. `EINTR` re-enters the
/// wait against the same deadline.
///
/// # Errors
/// `poll(2)` failed for a reason other than a signal.
pub fn wait_readable(fds: &[BorrowedFd<'_>], timeout: Option<Duration>) -> io::Result<Vec<bool>> {
    let mut polled: Vec<libc::pollfd> = fds
        .iter()
        .map(|fd| libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let count = libc::nfds_t::try_from(polled.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many descriptors"))?;
    let deadline = timeout.map(|t| Instant::now() + t);
    loop {
        let wait_ms = match deadline {
            None => -1,
            Some(deadline) => {
                // Round UP, never down: a sub-millisecond remainder must park
                // for a millisecond, not spin on `poll(…, 0)` until it lapses.
                // A lapsed deadline polls once with no wait, so a descriptor
                // that is already readable is still reported. A remainder
                // past `i32::MAX` parks in slices; the loop re-checks the
                // deadline after each.
                let left = deadline.saturating_duration_since(Instant::now());
                libc::c_int::try_from(left.as_micros().div_ceil(1_000)).unwrap_or(libc::c_int::MAX)
            }
        };
        for entry in &mut polled {
            entry.revents = 0;
        }
        // SAFETY: `polled` is a live, initialized `pollfd` array of exactly
        // `count` entries, and every descriptor in it is borrowed for the
        // duration of the call.
        let ready = unsafe { libc::poll(polled.as_mut_ptr(), count, wait_ms) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            // Timed out — or a clamped slice lapsed short of the deadline.
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(vec![false; polled.len()]);
            }
            continue;
        }
        return Ok(polled
            .iter()
            .map(|entry| {
                entry.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
            })
            .collect());
    }
}

/// How long a waiter falls back to sleeping when `poll` itself fails — a
/// condition no production path has produced, guarded so a broken wait
/// degrades to a slow tick and never to a spin.
const WAIT_FAILURE_BACKOFF: Duration = Duration::from_millis(200);

/// A one-shot broadcast: rung once, seen by every waiter forever after.
///
/// The read half of a socketpair. Ringing shuts the write half down, so the
/// read half is readable (EOF) from then on — a level-triggered fact any
/// number of threads can `poll` on without consuming it. The flag mirrors
/// the same fact for the cheap in-thread check.
#[derive(Debug)]
pub struct Doorbell {
    rung: AtomicBool,
    rx: UnixStream,
    tx: Mutex<Option<UnixStream>>,
}

impl Doorbell {
    /// A silent bell.
    ///
    /// # Errors
    /// The socketpair could not be created.
    pub fn new() -> io::Result<Self> {
        let (tx, rx) = UnixStream::pair()?;
        Ok(Doorbell {
            rung: AtomicBool::new(false),
            rx,
            tx: Mutex::new(Some(tx)),
        })
    }

    /// Ring it. Returns whether it had ALREADY been rung, so a caller that
    /// must act exactly once can tell (the `swap` shape of the flag it
    /// replaces).
    pub fn ring(&self) -> bool {
        let already = self.rung.swap(true, Ordering::SeqCst);
        if let Some(tx) = self
            .tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // EOF on the read half is the wake; the flag above is the fact.
            let _ = tx.shutdown(Shutdown::Both);
        }
        already
    }

    /// Has it been rung?
    #[must_use]
    pub fn is_rung(&self) -> bool {
        self.rung.load(Ordering::SeqCst)
    }

    /// The descriptor to `poll` on: readable once rung.
    #[must_use]
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.rx.as_fd()
    }

    /// Park for `timeout` unless rung first. Returns `true` when rung — the
    /// caller's signal to stop — and `false` when the interval elapsed.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> bool {
        if self.is_rung() {
            return true;
        }
        if wait_readable(&[self.fd()], Some(timeout)).is_err() {
            thread::sleep(timeout.min(WAIT_FAILURE_BACKOFF));
        }
        self.is_rung()
    }
}

/// One registered waiter's write half plus its coalescing flag.
#[derive(Debug)]
struct WakerSlot {
    tx: UnixStream,
    /// Set by a wake that wrote its byte, cleared by the waiter's drain: a
    /// second wake before the drain writes nothing, so a burst is one byte.
    pending: AtomicBool,
}

/// The many-shot signal for one workspace: everyone parked on its ring or
/// its feed. Registered waiters are held weakly, so a waiter that drops
/// (its push loop ended) needs no unregister call and is pruned on the next
/// registration.
#[derive(Debug, Default)]
pub struct ChangeWakers {
    slots: Mutex<Vec<Weak<WakerSlot>>>,
}

impl ChangeWakers {
    /// Register a waiter: its own socketpair, both halves non-blocking (the
    /// wake never parks a producer; the drain never parks the waiter).
    ///
    /// # Errors
    /// The socketpair could not be created or configured.
    pub fn register(&self) -> io::Result<Waker> {
        let (tx, rx) = UnixStream::pair()?;
        tx.set_nonblocking(true)?;
        rx.set_nonblocking(true)?;
        let slot = Arc::new(WakerSlot {
            tx,
            pending: AtomicBool::new(false),
        });
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.retain(|weak| weak.strong_count() > 0);
        slots.push(Arc::downgrade(&slot));
        Ok(Waker { slot, rx })
    }

    /// Wake every registered waiter that is not already pending a wake.
    pub fn wake_all(&self) {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        for slot in slots.iter().filter_map(Weak::upgrade) {
            if !slot.pending.swap(true, Ordering::SeqCst) {
                // Non-blocking, one byte; a failure leaves `pending` set and
                // the waiter still wakes on whatever the buffer holds.
                let _ = (&slot.tx).write(&[1]);
            }
        }
    }

    /// How many waiters are registered and alive.
    #[must_use]
    pub fn waiters(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }
}

/// One waiter's read half: `poll` on [`Waker::fd`], then [`Waker::drain`]
/// BEFORE re-reading the state the wake announced, so a wake that lands
/// during the read is not lost.
#[derive(Debug)]
pub struct Waker {
    slot: Arc<WakerSlot>,
    rx: UnixStream,
}

impl Waker {
    /// The descriptor to `poll` on: readable while a wake is pending.
    #[must_use]
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.rx.as_fd()
    }

    /// Consume the pending wake so the next one lands. Clears the flag first
    /// and reads second: a wake arriving between the two writes a byte that
    /// the read consumes, and the caller's state check that follows sees its
    /// cause either way.
    pub fn drain(&self) {
        self.slot.pending.store(false, Ordering::SeqCst);
        let mut sink = [0u8; 64];
        while matches!((&self.rx).read(&mut sink), Ok(n) if n > 0) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_doorbell_wakes_a_parked_waiter_and_stays_rung() {
        let bell = Arc::new(Doorbell::new().unwrap());
        let parked = Arc::clone(&bell);
        let waiter = thread::spawn(move || {
            let started = Instant::now();
            let rung = parked.wait(Duration::from_secs(10));
            (rung, started.elapsed())
        });
        thread::sleep(Duration::from_millis(50));
        assert!(!bell.ring(), "first ring reports it was silent");
        let (rung, elapsed) = waiter.join().unwrap();
        assert!(rung, "the wait ends on the ring, not the deadline");
        assert!(
            elapsed < Duration::from_secs(5),
            "the ring woke the waiter; it did not sleep the interval out ({elapsed:?})"
        );
        assert!(bell.ring(), "a second ring reports it was already rung");
        assert!(bell.is_rung());
        assert!(
            bell.wait(Duration::from_secs(10)),
            "a rung bell answers every later wait at once"
        );
    }

    #[test]
    fn a_silent_doorbell_lets_the_interval_elapse() {
        let bell = Doorbell::new().unwrap();
        let started = Instant::now();
        assert!(!bell.wait(Duration::from_millis(30)));
        assert!(started.elapsed() >= Duration::from_millis(30));
        assert!(!bell.is_rung());
    }

    #[test]
    fn a_sub_millisecond_deadline_parks_instead_of_spinning() {
        let bell = Doorbell::new().unwrap();
        let ready = wait_readable(&[bell.fd()], Some(Duration::from_micros(300))).unwrap();
        assert_eq!(ready, vec![false]);
    }

    #[test]
    fn wakers_wake_every_registered_waiter_once_per_drain() {
        let wakers = ChangeWakers::default();
        let a = wakers.register().unwrap();
        let b = wakers.register().unwrap();
        assert_eq!(wakers.waiters(), 2);
        let idle = wait_readable(&[a.fd(), b.fd()], Some(Duration::ZERO)).unwrap();
        assert_eq!(idle, vec![false, false], "nothing pending before a wake");

        wakers.wake_all();
        wakers.wake_all();
        let ready = wait_readable(&[a.fd(), b.fd()], Some(Duration::from_secs(1))).unwrap();
        assert_eq!(ready, vec![true, true], "one wake reaches every waiter");
        a.drain();
        let after = wait_readable(&[a.fd(), b.fd()], Some(Duration::ZERO)).unwrap();
        assert_eq!(
            after,
            vec![false, true],
            "draining consumes the burst for that waiter alone"
        );
        wakers.wake_all();
        let again = wait_readable(&[a.fd()], Some(Duration::from_secs(1))).unwrap();
        assert_eq!(again, vec![true], "a drained waiter wakes on the next wake");

        drop(b);
        assert_eq!(wakers.waiters(), 1, "a dropped waiter is no longer counted");
        wakers.wake_all();
        let c = wakers.register().unwrap();
        assert_eq!(wakers.waiters(), 2, "registration prunes the dead slot");
        let fresh = wait_readable(&[c.fd()], Some(Duration::ZERO)).unwrap();
        assert_eq!(
            fresh,
            vec![false],
            "a new waiter owes nothing for earlier wakes"
        );
    }
}
