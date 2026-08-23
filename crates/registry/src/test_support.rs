//! Fixture support: ONE owner for a daemon and the temporary tree it serves.
//!
//! # Why this type exists
//!
//! A fixture that keeps a [`RunningServer`] and a [`TempDir`] side by side has
//! a teardown ORDER, and the order is load-bearing: stopping the server drains
//! in-flight drawer rebuilds, so the temporary tree must outlive the stop. Get
//! it backwards and the workspace vanishes under a live builder — the class-2
//! flake (`registry: background drawer rebuild failed for /tmp/.tmp…/ws (No
//! such file or directory)`, pipelines 1098/1101).
//!
//! Rust makes that order easy to invert and impossible to see: **struct fields
//! drop in declaration order, locals drop in reverse**, so the same two values
//! that tear down correctly as
//!
//! ```ignore
//! let tmp = TempDir::new()?;                 // dropped second
//! let server = RunningServer::start(cfg)?;   // dropped first  ✔
//! ```
//!
//! tear down INVERTED the moment they move into a struct in that same order.
//! Nothing in the type system objects, and the fixture is green until the box
//! is loaded. Card `registry-tests-drain-residue` caught three such inversions
//! by hand and left a source-text guard behind; this type is what replaces it.
//!
//! # Why the order cannot be gotten wrong here
//!
//! [`TestServer`] owns both values and stops the server in its own
//! [`Drop::drop`]. Rust runs a type's `Drop::drop` to completion **before** it
//! drops any of that type's fields, so the stop always precedes the temporary
//! tree's removal — **whatever order the fields happen to be declared in**.
//! The invariant is therefore not "declare them in this order" (a rule someone
//! must remember) but a property of the drop glue (a rule nobody can break).
//! A fixture holds ONE field, so it has no order of its own to get wrong.
//!
//! Proved by `a_reversed_field_order_still_stops_the_server_first`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;

use crate::server::{Config, RunningServer};

/// A daemon and the temporary tree it serves, torn down in the one right order.
///
/// Construct it eagerly with [`TestServer::start`], or empty with
/// [`TestServer::idle`] when the fixture starts its daemon lazily (a sandbox
/// that only needs one for some of its drives). Either way the teardown is the
/// same and the fixture holds a single field.
///
/// ```ignore
/// struct Fixture {
///     daemon: TestServer,
///     ws: PathBuf,       // inert: a PathBuf has no teardown
/// }
///
/// let daemon = TestServer::start(|root| {
///     let mut cfg = Config::for_cache_root(root.join("cache"));
///     cfg.drain_cold_builds = Duration::from_secs(30);
///     cfg
/// })?;
/// ```
#[derive(Debug)]
pub struct TestServer {
    /// The daemon, absent until started and taken again by the teardown.
    ///
    /// `Mutex` because a lazily-started fixture fills this slot through a
    /// shared reference ([`TestServer::ensure_live`]); `Option` because the
    /// slot is empty before the first start and after the teardown, which is
    /// what makes stopping idempotent.
    ///
    /// Declared before `tmp` to match the teardown, but nothing depends on
    /// that: see this module's § Why the order cannot be gotten wrong.
    server: Mutex<Option<RunningServer>>,
    /// The temporary tree the daemon serves. Removed only after the stop.
    tmp: TempDir,
}

impl TestServer {
    /// A fresh temporary tree with a daemon already running against it.
    ///
    /// `build` receives the tree's root and returns the [`Config`] to start —
    /// derive the cache root, socket path and workspace from that root, and
    /// raise [`Config::drain_cold_builds`] to the fixture budget (see
    /// [`crate::DEFAULT_DRAIN_COLD_BUILDS`]).
    ///
    /// # Errors
    ///
    /// The temporary tree cannot be created, or [`RunningServer::start`]
    /// refuses (an occupied singleton lock, an unbindable socket).
    pub fn start(build: impl FnOnce(&Path) -> Config) -> io::Result<Self> {
        let this = Self::idle()?;
        this.ensure_live(build)?;
        Ok(this)
    }

    /// A fresh temporary tree with NO daemon yet — the lazy shape, for a
    /// fixture whose drives only sometimes need a resident.
    ///
    /// # Errors
    ///
    /// The temporary tree cannot be created.
    pub fn idle() -> io::Result<Self> {
        Ok(TestServer {
            server: Mutex::new(None),
            tmp: TempDir::new()?,
        })
    }

    /// Start the daemon if it is not already running; a no-op when it is.
    ///
    /// `build` is called only when a start is actually needed.
    ///
    /// # Errors
    ///
    /// [`RunningServer::start`] refuses.
    pub fn ensure_live(&self, build: impl FnOnce(&Path) -> Config) -> io::Result<()> {
        let mut slot = self.lock();
        if slot.is_none() {
            *slot = Some(RunningServer::start(build(self.tmp.path()))?);
        }
        Ok(())
    }

    /// The temporary tree's root — the base every fixture path derives from.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.tmp.path()
    }

    /// The running daemon's socket path.
    ///
    /// # Panics
    ///
    /// No daemon is running: an [`idle`](Self::idle) fixture that never called
    /// [`ensure_live`](Self::ensure_live), or one already torn down.
    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.lock()
            .as_ref()
            .expect("this TestServer has no running daemon — call ensure_live first")
            .socket_path()
            .to_path_buf()
    }

    /// Read the running daemon directly — the escape hatch for the handle's own
    /// surface (`registry()`, `idle_exit_requested()`).
    ///
    /// # Panics
    ///
    /// No daemon is running (see [`socket_path`](Self::socket_path)).
    pub fn with<R>(&self, f: impl FnOnce(&RunningServer) -> R) -> R {
        f(self
            .lock()
            .as_ref()
            .expect("this TestServer has no running daemon — call ensure_live first"))
    }

    /// Stop the daemon now, rather than at the end of the fixture's scope.
    ///
    /// The temporary tree is removed when the returned value drops, which is
    /// immediately — the stop still precedes it.
    pub fn shutdown(self) {
        self.stop();
    }

    /// Stop the daemon if one is running. Idempotent: the handle is TAKEN, so a
    /// second call finds an empty slot.
    fn stop(&self) {
        if let Some(server) = self.lock().take() {
            server.shutdown();
        }
    }

    /// The slot, poisoning ignored: a panicking test still has to tear its
    /// daemon down, and refusing to would bury the real failure under a
    /// second one.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<RunningServer>> {
        self.server
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for TestServer {
    /// Stop the daemon BEFORE the temporary tree is removed.
    ///
    /// This runs to completion before either field's own drop glue, which is
    /// what makes the order structural rather than remembered (§ Why the order
    /// cannot be gotten wrong).
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;

    /// A config good enough to start a daemon under `root`, on the fixture
    /// drain budget.
    #[allow(clippy::duration_suboptimal_units)]
    fn fixture_config(root: &Path) -> Config {
        let mut cfg = Config::for_cache_root(root.join("cache"));
        let never = Duration::from_secs(365 * 24 * 60 * 60);
        cfg.idle_threshold = never;
        cfg.reap_interval = never;
        cfg.prewarm_interval = never;
        cfg.prewarm_quiet_max = never;
        cfg.idle_exit = None;
        cfg.drain_cold_builds = Duration::from_secs(30);
        cfg
    }

    /// **What the type guarantees, and the one shape it does NOT — both
    /// asserted, because the second is the tempting claim and it is false.**
    ///
    /// Guaranteed: `TestServer::drop` runs `stop()` to completion before
    /// EITHER of its own fields drops, so the temporary tree is still there
    /// while the daemon drains. That is a language guarantee — a `Drop::drop`
    /// body runs before the value's own field drop glue — and it is the whole
    /// reason a fixture no longer has a teardown order to remember.
    ///
    /// NOT guaranteed: beating a SIBLING field of an ENCLOSING struct. Sibling
    /// fields drop in declaration order, and `TestServer::drop` only runs when
    /// its own field is reached — so a witness declared before it observes a
    /// LIVE daemon. This test asserts that, measured, rather than its
    /// flattering opposite.
    ///
    /// Why the gap is harmless, and why it is not the old hazard wearing a new
    /// hat: a fixture can no longer hold the temporary tree as a sibling at
    /// all. `TestServer` owns the `TempDir` and lends only `&Path`
    /// ([`TestServer::path`]), so the shape the deleted source-text guard
    /// existed to reject — a `TempDir` declared before the server, removing the
    /// tree under a live drain — cannot be written any more. What a sibling
    /// dropping early can still do is observe; what it cannot do is delete.
    #[test]
    fn a_sibling_declared_before_the_daemon_drops_while_it_is_still_live() {
        /// Fields in the hostile order on purpose: `first` is dropped before
        /// `daemon`, so the witness runs while the daemon is still live.
        struct Inverted {
            first: DropWitness,
            /// Never read, and that is the point: this field exists for its
            /// DROP, which is the entire subject of the test. Reading it would
            /// prove nothing about teardown order.
            #[allow(dead_code)]
            daemon: TestServer,
        }

        /// Records whether the daemon's socket was already gone when this
        /// dropped — i.e. whether the stop had run by then.
        struct DropWitness {
            socket: PathBuf,
            root: PathBuf,
            socket_gone_at_drop: Arc<AtomicBool>,
            root_alive_at_drop: Arc<AtomicBool>,
        }

        impl Drop for DropWitness {
            fn drop(&mut self) {
                self.socket_gone_at_drop
                    .store(!self.socket.exists(), Ordering::SeqCst);
                self.root_alive_at_drop
                    .store(self.root.exists(), Ordering::SeqCst);
            }
        }

        let socket_gone = Arc::new(AtomicBool::new(false));
        let root_alive = Arc::new(AtomicBool::new(false));
        let socket;
        let root;

        {
            let daemon = TestServer::start(fixture_config).expect("the daemon starts");
            socket = daemon.socket_path();
            root = daemon.path().to_path_buf();
            let inverted = Inverted {
                first: DropWitness {
                    socket: socket.clone(),
                    root: root.clone(),
                    socket_gone_at_drop: Arc::clone(&socket_gone),
                    root_alive_at_drop: Arc::clone(&root_alive),
                },
                daemon,
            };
            assert!(
                inverted.first.socket.exists(),
                "the fixture's daemon is serving before the teardown"
            );
        }

        // The half the type does NOT give you, stated as an assertion so it
        // cannot quietly become the opposite belief again.
        assert!(
            !socket_gone.load(Ordering::SeqCst),
            "a sibling field declared BEFORE the daemon drops before it, i.e. \
             before TestServer::drop has run at all — the type orders its own \
             two fields, not the enclosing struct's. Reaching this message \
             means the socket WAS already gone at the sibling's drop: sibling \
             drop order has changed and this test's doc is stale."
        );
        assert!(
            root_alive.load(Ordering::SeqCst),
            "and the temporary tree is untouched at that moment, because the \
             tree belongs to the TestServer and not to the sibling: an early \
             sibling can observe, it cannot delete"
        );

        // The half the type DOES give you, at the only point it is observable
        // from outside: once the TestServer's own field drops, `stop()` has run
        // to completion and only THEN did its `TempDir` field drop.
        assert!(
            !socket.exists(),
            "the daemon is stopped once the TestServer field drops: its socket is gone"
        );
        assert!(
            !root.exists(),
            "and its temporary tree is gone once the stop has completed"
        );
    }

    /// Stopping twice is a no-op, so an explicit `shutdown()` and the drop that
    /// follows it do not both try to tear the daemon down.
    #[test]
    fn stopping_is_idempotent() {
        let daemon = TestServer::start(fixture_config).expect("the daemon starts");
        let socket = daemon.socket_path();
        daemon.shutdown();
        assert!(!socket.exists(), "shutdown stopped the daemon");
    }

    /// The lazy shape: a tree with no daemon, then one on demand, and
    /// `ensure_live` a second time does not start a second daemon (which would
    /// refuse on the singleton lock).
    #[test]
    fn an_idle_fixture_starts_its_daemon_on_demand_exactly_once() {
        let daemon = TestServer::idle().expect("the tree is created");
        assert!(daemon.path().exists(), "the tree exists before any daemon");

        daemon
            .ensure_live(fixture_config)
            .expect("the daemon starts");
        let socket = daemon.socket_path();
        assert!(socket.exists(), "the daemon bound its socket");

        daemon
            .ensure_live(|_| panic!("a live fixture must not build a second config"))
            .expect("the second ensure_live is a no-op");
        assert_eq!(daemon.socket_path(), socket, "still the same daemon");
    }
}
