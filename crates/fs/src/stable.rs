//! The stable-read protocol (`docs/node-rev-merkle-spec.md` §6.2) — the
//! watermark trust close that makes the resident tree at least as strict as
//! both fresh instruments on every path (merged plan §4.1; fable F-11,
//! adopted).
//!
//! `StatKey` alone can miss a same-instant, same-size, in-place write: every
//! stamp the kernel assigns is truncated to the backend's timestamp quantum,
//! so a write landing in the SAME quantum as the stat that vouched for a
//! digest leaves the whole key — ctime included — byte-identical. The close
//! is git's racy-clean rule, adopted as law:
//!
//! 1. Every memo entry records the **observation watermark** it was captured
//!    under. A digest is trusted only while its stamps sit at least one full
//!    calibrated granule BEFORE that watermark ([`racy`]); a racy entry is
//!    re-read before its digest is trusted — its record is deliberately
//!    spoiled (`seen: None`), never silently served.
//! 2. The **granule is measured, never assumed** ([`calibrate`]): a probe at
//!    workspace open rewrites a file under `.meridian/` (outside the §12.1
//!    hash domain, the cookie's own precedent) and takes the smallest
//!    observed stamp movement as the comparison unit — an upper bound on the
//!    backend's quantum, so over-estimation only ever costs re-reads.
//! 3. The **watermark is a filesystem stamp, not a system-clock read**
//!    ([`watermark`]): the probe file is touched and its own mtime taken, so
//!    the compare never crosses clock domains (a skewed NFS server clock
//!    cannot un-racy a fresh write).
//! 4. Byte reads run **settled** ([`read_settled`]): open without following
//!    links, fstat before AND after the bytes — identity moved mid-read ⇒
//!    discard and re-read; identity still moving after the retry budget ⇒
//!    the leaf is a SUSPECT (a still-open in-place writer), served this pass
//!    but recorded spoiled.
//! 5. An **event-generation fence** ([`FeedGen::bracket`]) brackets each
//!    read: a feed event landing inside the bracket re-classifies the read
//!    (recorded spoiled) rather than letting a torn observation into the
//!    memo.
//! 6. **Unknown capability or event loss is LOUD, never silent trust**: a
//!    probe that cannot run leaves [`Calibration::Unavailable`] (no stat
//!    reuse at all — every pass re-reads), and [`FeedGen::note_loss`] drops
//!    guard currency to [`GuardCurrency::Untrusted`] until a full
//!    observation re-baselines.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::StatKey;

/// The probe file the calibration and every per-pass watermark touch. Lives
/// under `.meridian/` — outside the §12.1 hash domain by the standing
/// dot-segment floor (the currency cookie's own law), so touching it can
/// never move a root or break a held token.
pub const PROBE_FILE: &str = "stable-read.probe";

/// Rewrites the burst phase runs back-to-back; more samples tighten the
/// measured granule on backends whose stamp clock is cached (ext4's
/// jiffy-grained `current_time` answers identical stamps to rapid writes).
const BURST_WRITES: u32 = 16;

/// Escalation sleeps after an all-equal burst, longest last. A backend that
/// shows no stamp movement across the whole ladder is classified into the
/// coarsest known stamp class ([`COARSE_GRANULE_NS`]) — still measured
/// ("no tick within the ladder"), never assumed fine.
const ESCALATION_SLEEPS_MS: [u64; 7] = [1, 2, 4, 8, 16, 32, 64];

/// The coarsest known real filesystem stamp quantum (FAT's 2 s). Declared
/// when the probe observes no movement inside the escalation ladder.
const COARSE_GRANULE_NS: u64 = 2_000_000_000;

/// How many times a read whose fd identity moved mid-read is retried before
/// the leaf is classified SUSPECT (a still-open in-place writer).
const SETTLE_RETRIES: u32 = 4;

/// The measured timestamp-granularity calibration of one workspace backend
/// (spec §6.2 row 2). Probed once per workspace open; queryable for the
/// whole cache life ([`crate::DomainCache::calibration`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Calibration {
    /// The watermark's comparison unit, in nanoseconds — an upper bound on
    /// the backend's stamp quantum.
    Measured {
        /// Smallest observed stamp movement (ns), or the coarse-class
        /// ceiling when the ladder saw none.
        granule_ns: u64,
    },
    /// The probe could not run (unwritable `.meridian/`, alien I/O failure):
    /// unknown capability. No stat identity is trusted — every observation
    /// re-reads every member — and guard currency reports untrusted. LOUD at
    /// probe time, never silent.
    Unavailable {
        /// What failed, in words an operator acts on.
        reason: String,
    },
}

/// Guard currency as the resident cache can vouch for it right now
/// (spec §6.2 row 6, §6.3's fast-path precondition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardCurrency {
    /// A full observation has landed, the backend is calibrated, and no
    /// event loss is unabsorbed.
    Trusted,
    /// No baseline, unknown capability, or reported event loss. Consumers
    /// must take the §6.2-governed extent refresh — never a vouched answer.
    Untrusted {
        /// Which fact dropped the currency.
        reason: String,
    },
}

/// The shared feed-generation cell: the engine-owned watcher (merged plan §6
/// step 4, the event-feed card) advances it per delivered event and reports
/// loss on overflow; the observation path brackets every byte read with it
/// and consults the loss count for guard currency. Cloning shares the cell.
#[derive(Debug, Clone, Default)]
pub struct FeedGen(Arc<FeedGenCell>);

#[derive(Debug, Default)]
struct FeedGenCell {
    generation: AtomicU64,
    losses: AtomicU64,
}

/// One read's fence bracket: the generation captured before the bytes.
#[derive(Debug, Clone, Copy)]
pub struct FenceToken(u64);

impl FeedGen {
    /// Record one delivered feed event (the watcher's verb). Events landing
    /// inside an open [`FenceToken`] re-classify that read.
    pub fn advance(&self) {
        self.0.generation.fetch_add(1, Ordering::Release);
    }

    /// Report event loss (kernel queue overflow, watcher gap) — LOUD at the
    /// moment of loss, spec §6.2 row 6: guard currency reports untrusted
    /// until a full observation re-baselines past this loss.
    pub fn note_loss(&self, reason: &str) {
        self.0.losses.fetch_add(1, Ordering::Release);
        eprintln!(
            "merkle: event loss reported ({reason}) — guard currency is UNTRUSTED until a \
             full observation re-baselines (merkle-spec 6.2)"
        );
    }

    /// The current generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }

    /// Losses reported over the cell's life (monotonic).
    #[must_use]
    pub fn losses(&self) -> u64 {
        self.0.losses.load(Ordering::Acquire)
    }

    /// Open a fence bracket around one byte read.
    #[must_use]
    pub fn bracket(&self) -> FenceToken {
        FenceToken(self.generation())
    }

    /// Did the bracket close clean — no feed event landed inside it?
    #[must_use]
    pub fn clean(&self, token: FenceToken) -> bool {
        self.generation() == token.0
    }
}

/// One filesystem stamp as `stat` reports it: `(seconds, nanoseconds)`.
pub(crate) type FsStamp = (i64, i64);

/// A stamp as one signed nanosecond count (i128: no epoch-range or
/// subtraction overflow anywhere in the compare).
fn stamp_ns(stamp: FsStamp) -> i128 {
    i128::from(stamp.0) * 1_000_000_000 + i128::from(stamp.1)
}

/// The trust context one observation pass runs under: the calibrated granule
/// and the pass watermark. Either absent ⇒ nothing is trusted and fresh
/// records are spoiled (`seen: None`) — the never-silent-trust floor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrustCtx {
    /// Measured granule, ns. `None` under [`Calibration::Unavailable`].
    pub(crate) granule_ns: Option<u64>,
    /// This pass's watermark (the probe file's own mtime — filesystem clock,
    /// truncated like every stamp). `None` when capture failed.
    pub(crate) watermark: Option<FsStamp>,
}

impl TrustCtx {
    /// The untrusted floor: reuse nothing, spoil every record.
    pub(crate) fn untrusted() -> TrustCtx {
        TrustCtx {
            granule_ns: None,
            watermark: None,
        }
    }

    /// May a memo entry recorded under watermark `seen` with identity `key`
    /// be trusted without a byte read? Requires a calibrated granule, an
    /// unspoiled record, and stamps strictly older than one granule before
    /// the record watermark ([`racy`]).
    pub(crate) fn trusts(&self, key: &StatKey, seen: Option<FsStamp>) -> bool {
        match (self.granule_ns, seen) {
            (Some(granule), Some(seen)) => !racy(key, seen, granule),
            _ => false,
        }
    }

    /// The watermark fresh records ride: `None` (spoiled) when the pass has
    /// no trustworthy watermark.
    pub(crate) fn record_seen(&self) -> Option<FsStamp> {
        match (self.granule_ns, self.watermark) {
            (Some(_), Some(w)) => Some(w),
            _ => None,
        }
    }
}

/// Git's racy-clean predicate over the full key: an identity whose mtime OR
/// ctime is at or after `watermark - granule` may share the watermark's own
/// stamp quantum with a later write, so its digest is never trusted from
/// stat equality alone. Strict boundary: `stamp == watermark - granule` is
/// racy (a write at the watermark instant can truncate to exactly that
/// stamp).
pub(crate) fn racy(key: &StatKey, watermark: FsStamp, granule_ns: u64) -> bool {
    let (_, _, _, mtime, ctime) = key.raw_parts();
    let floor = stamp_ns(watermark) - i128::from(granule_ns);
    stamp_ns(mtime) >= floor || stamp_ns(ctime) >= floor
}

/// Measure the backend's stamp granularity at `meridian_dir` (created if
/// absent): burst-rewrite the probe file sampling each write's mtime, then
/// escalate with short sleeps if the burst never saw a tick. The smallest
/// observed movement is the granule — an upper bound on the backend quantum
/// (distinct stamps differ by at least one quantum), so measurement error
/// only ever widens the racy window. A ladder with no movement classifies
/// the backend into the coarsest known class ([`COARSE_GRANULE_NS`]).
///
/// Failure is [`Calibration::Unavailable`] and LOUD here — the caller keeps
/// serving, re-reading everything (never silent trust).
pub(crate) fn calibrate(meridian_dir: &Path) -> Calibration {
    match measure_granule(meridian_dir) {
        Ok(granule_ns) => Calibration::Measured { granule_ns },
        Err(e) => {
            let reason = format!("timestamp calibration probe failed: {e}");
            eprintln!(
                "merkle: {reason} — stat identity is UNTRUSTED at this workspace; every \
                 observation re-reads every member (merkle-spec 6.2, never silent trust)"
            );
            Calibration::Unavailable { reason }
        }
    }
}

fn measure_granule(meridian_dir: &Path) -> io::Result<u64> {
    std::fs::create_dir_all(meridian_dir)?;
    let probe = meridian_dir.join(PROBE_FILE);
    let mut stamps: Vec<FsStamp> = Vec::with_capacity(BURST_WRITES as usize + 8);
    for _ in 0..BURST_WRITES {
        stamps.push(touch(&probe)?);
    }
    if smallest_tick(&stamps).is_none() {
        for sleep_ms in ESCALATION_SLEEPS_MS {
            std::thread::sleep(Duration::from_millis(sleep_ms));
            stamps.push(touch(&probe)?);
            if smallest_tick(&stamps).is_some() {
                break;
            }
        }
    }
    match smallest_tick(&stamps) {
        Some(tick) => Ok(u64::try_from(tick).unwrap_or(COARSE_GRANULE_NS)),
        None => Ok(COARSE_GRANULE_NS),
    }
}

/// The smallest positive movement between consecutive samples, ns.
fn smallest_tick(stamps: &[FsStamp]) -> Option<i128> {
    stamps
        .windows(2)
        .filter_map(|w| {
            let d = stamp_ns(w[1]) - stamp_ns(w[0]);
            (d > 0).then_some(d)
        })
        .min()
}

/// Rewrite the probe file and answer the mtime the write was assigned — one
/// filesystem-clock sample.
fn touch(probe: &Path) -> io::Result<FsStamp> {
    let mut f = File::create(probe)?;
    f.write_all(b"stable-read probe\n")?;
    let (_, _, _, mtime, _) = StatKey::of(&f.metadata()?).raw_parts();
    Ok(mtime)
}

/// Capture this pass's observation watermark: touch the probe file and take
/// its own mtime — "now" on the filesystem's stamp clock, truncated exactly
/// as member stamps are. Captured BEFORE any member is statted or read, so
/// the racy compare is conservative for the whole pass.
pub(crate) fn watermark(meridian_dir: &Path) -> io::Result<FsStamp> {
    touch(&meridian_dir.join(PROBE_FILE))
}

/// One settled byte read (spec §6.2 rows 3–5): opened without following
/// links, fd identity fstatted before and after the bytes.
pub(crate) struct SettledRead {
    /// The bytes of the last attempt.
    pub(crate) bytes: Vec<u8>,
    /// The fd-true identity the bytes were read under (the before-fstat of
    /// the last attempt) — the ONLY honest key to record for this digest;
    /// a path stat taken outside the read can be raced by the write that
    /// minted it.
    pub(crate) key: StatKey,
    /// `false` ⇒ identity was still moving after the retry budget: a
    /// still-open in-place writer — the leaf is SUSPECT, its record must be
    /// spoiled.
    pub(crate) settled: bool,
}

/// Read `abs` under the stable-read law: `O_NOFOLLOW` open, fstat before and
/// after the bytes, retry while identity moves mid-read, SUSPECT past the
/// retry budget. Symlink refusal (`ELOOP`, classified via `symlink_metadata`)
/// is the caller's to spell per observation law.
///
/// # Errors
/// Any open/read/fstat failure; the caller maps refusal shapes.
pub(crate) fn read_settled(abs: &Path) -> io::Result<SettledRead> {
    let mut last: Option<(Vec<u8>, StatKey)> = None;
    for _ in 0..=SETTLE_RETRIES {
        let mut f = crate::open_nofollow(abs)?;
        let before = StatKey::of(&f.metadata()?);
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut f, &mut bytes)?;
        let after = StatKey::of(&f.metadata()?);
        if before == after {
            return Ok(SettledRead {
                bytes,
                key: before,
                settled: true,
            });
        }
        last = Some((bytes, before));
    }
    let (bytes, key) = last.expect("at least one attempt ran");
    Ok(SettledRead {
        bytes,
        key,
        settled: false,
    })
}

/// The `.meridian` directory of `root` — where the probe file lives.
pub(crate) fn meridian_dir(root: &crate::WorkspaceRoot) -> PathBuf {
    root.0.join(".meridian")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(mtime: FsStamp, ctime: FsStamp) -> StatKey {
        StatKey::from_raw_parts(1, 2, 3, mtime, ctime)
    }

    /// The racy boundary is strict: one full granule before the watermark is
    /// still racy; one nanosecond older is trusted — and ctime alone trips
    /// it (a forged mtime never un-racies a write).
    #[test]
    fn racy_boundary_is_strict_and_covers_ctime() {
        let w: FsStamp = (100, 0);
        let g = 1_000_000_000u64; // 1 s
        // stamp == watermark - granule → racy.
        assert!(racy(&key((99, 0), (0, 0)), w, g));
        // one ns older → trusted.
        assert!(!racy(&key((98, 999_999_999), (0, 0)), w, g));
        // ctime alone at the boundary → racy.
        assert!(racy(&key((0, 0), (99, 0)), w, g));
        // both comfortably old → trusted.
        assert!(!racy(&key((10, 0), (10, 0)), w, g));
        // at or after the watermark itself → racy.
        assert!(racy(&key((100, 0), (0, 0)), w, g));
        assert!(racy(&key((250, 0), (0, 0)), w, g));
    }

    /// The trust context: no granule or no record watermark ⇒ nothing is
    /// trusted and fresh records are spoiled.
    #[test]
    fn untrusted_context_trusts_nothing_and_spoils_records() {
        let k = key((10, 0), (10, 0));
        let ctx = TrustCtx::untrusted();
        assert!(!ctx.trusts(&k, Some((100, 0))));
        assert_eq!(ctx.record_seen(), None);

        let calibrated = TrustCtx {
            granule_ns: Some(1),
            watermark: Some((100, 0)),
        };
        assert!(calibrated.trusts(&k, Some((100, 0))));
        assert!(
            !calibrated.trusts(&k, None),
            "a spoiled record never trusts"
        );
        assert_eq!(calibrated.record_seen(), Some((100, 0)));
    }

    /// The fence bracket: an event landing inside the bracket dirties it; a
    /// quiet bracket closes clean.
    #[test]
    fn fence_bracket_catches_an_event_landing_inside() {
        let feed = FeedGen::default();
        let clean = feed.bracket();
        assert!(feed.clean(clean));
        let dirty = feed.bracket();
        feed.advance();
        assert!(!feed.clean(dirty));
        assert_eq!(feed.generation(), 1);
    }

    /// Loss is monotonic and visible.
    #[test]
    fn losses_count_monotonically() {
        let feed = FeedGen::default();
        assert_eq!(feed.losses(), 0);
        feed.note_loss("fixture");
        feed.note_loss("fixture");
        assert_eq!(feed.losses(), 2);
        // A clone shares the cell — the watcher's handle and the cache's
        // handle are one instrument.
        let shared = feed.clone();
        shared.note_loss("fixture");
        assert_eq!(feed.losses(), 3);
    }

    /// The probe measures a real backend: a granule comes back, positive and
    /// no coarser than the declared ceiling. The measured value is printed —
    /// the per-filesystem row of the test matrix (card stable-read-protocol
    /// gate 3; run with `--nocapture` to record it).
    #[test]
    fn calibration_probe_measures_this_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let calibration = calibrate(&tmp.path().join(".meridian"));
        let Calibration::Measured { granule_ns } = calibration else {
            panic!("probe failed on a writable tempdir: {calibration:?}");
        };
        eprintln!("calibration: granule_ns={granule_ns} (test-matrix row)");
        assert!(granule_ns > 0);
        assert!(granule_ns <= COARSE_GRANULE_NS);
        // The watermark rides the same file and the same clock.
        let w = watermark(&tmp.path().join(".meridian")).expect("watermark");
        assert!(stamp_ns(w) > 0);
    }

    /// A settled read over a quiet file answers the fd-true identity.
    #[test]
    fn settled_read_answers_fd_true_identity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("x.md");
        std::fs::write(&p, b"stable\n").expect("write");
        let read = read_settled(&p).expect("read");
        assert!(read.settled);
        assert_eq!(read.bytes, b"stable\n");
        let disk = StatKey::of_path(&p).expect("stat").expect("present");
        assert_eq!(read.key, disk);
    }
}
