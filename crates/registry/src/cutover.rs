//! The per-workspace cutover state machine — merkle-spec §4.2.6, merged
//! plan §6 step 7, behind the write-plane exclusion.
//!
//! **Starting a cutover is an explicit act; the daemon only finishes one.**
//! [`start`] crosses the no-return boundary and has exactly one production
//! caller: the operator verb. At workspace open the registry calls
//! [`resume_if_interrupted`], which finishes a build the boundary already
//! committed to and does NOTHING to a workspace with no record — law 1 keeps
//! serving.
//!
//! That split is a correction made on live evidence, and it is worth stating
//! rather than merely doing: the first revision ran the machine at every
//! workspace open, so opening ANY workspace — a user's, a fixture's — carried
//! it irreversibly across the boundary with nobody deciding. A boundary with
//! no rollback needs a named owner and a moment (price paper §9: per-workspace
//! scheduling, canary, explicit recovery ownership), which is what an explicit
//! verb is.
//!
//! A LIVE workspace drains through the same machine ([`run`] with the observed
//! ring instance, so its subscriptions end labeled); a quiet one reaches the
//! drain as the interim-EX acquisition plus an intents-empty proof.
//!
//! What the machine does NOT do, by ruling: activate the B-03 fence
//! (spec §4.2.5 amendment — the record alone crosses the boundary), build a
//! shadow (`B_cutover` answered — pay once), or eagerly rebuild the SQL
//! projection (the store is disposable BY LANDED DESIGN and recreates when
//! its fingerprint prefix differs — `view::store`; rebuilding it inside the
//! pause would manufacture downtime for no correctness gain). The fresh
//! restart checkpoint rides the ordinary best-effort lifecycle after the
//! build; the machine's own checkpoint act is the DISCARD, which is the
//! correctness-critical half (a pre-boundary file must never be adopted
//! past the boundary — a missing checkpoint costs a cold rebuild, never
//! correctness).
//!
//! Every component of the real admission pause is measured and recorded
//! (gates 22/24: never quote the 1.45 s old-law receipt) — a best-effort
//! sidecar `.meridian/cutover.receipt` keeps the numbers where the cutover
//! ran.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fs::cutover::{CutoverRecord, CutoverState, CutoverStatus, ServingLaw};

/// The measured components of one cutover's admission pause, microseconds.
/// `fence` is deliberately absent: the tombstone never activates (spec
/// §4.2.5 amendment); `projection` is deliberately absent: the SQL store
/// rebuilds lazily on first use by landed design, outside the pause.
// The shared `_us` postfix IS the honest unit label on every field —
// renaming to satisfy the lint would trade clarity for silence.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
pub struct Components {
    /// Interim-EX acquisition + the intents-empty proof.
    pub drain_us: u128,
    /// The no-return act: the record durable at `NEW_BUILDING`.
    pub record_begin_us: u128,
    /// Checkpoint discard (the correctness half of the checkpoint step).
    pub checkpoint_discard_us: u128,
    /// The one Markdown walk building the law-2 tree and minting the first
    /// radix-encoded served token.
    pub build_us: u128,
    /// The publication: the record durable at `NEW_COMMITTED`.
    pub commit_us: u128,
    /// End to end, EX taken to EX released.
    pub total_us: u128,
}

/// One completed cutover: the durable record plus the measured pause.
#[derive(Debug)]
pub struct CutoverRun {
    pub record: CutoverRecord,
    pub components: Components,
}

/// How long the drain waits for the interim EX before refusing: a
/// cooperating writer's critical section is milliseconds, so ten seconds of
/// 25 ms probes is a generous bound, not a wait policy.
const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// RESUME an interrupted cutover at workspace open — and never START one.
///
/// **The trigger is an explicit operator act, never ambient** (corrected
/// 2026-08-16 on live evidence): an earlier revision of this module ran the
/// machine whenever a workspace opened without a record, which silently
/// carried EVERY workspace — including throwaway fixtures — across the
/// irreversible no-return boundary on first touch. Crossing a boundary with
/// no rollback is a deliberate act with a named owner (price paper §9:
/// per-workspace scheduling, canary, explicit recovery ownership), so
/// starting a cutover is [`start`]'s job and this function's answer for a
/// record-less workspace is: nothing to do, law 1 keeps serving.
///
/// A `NEW_BUILDING` record means the boundary was ALREADY crossed by such an
/// act and the build did not finish — the engine must never serve the
/// retired law again, so it finishes the build here. A `NEW_COMMITTED`
/// record is nothing to do. A corrupt record propagates its typed refusal —
/// the caller spells `fingerprint_cutover_recovery_required` and serves
/// nothing.
pub(crate) fn resume_if_interrupted(
    root: &fs::WorkspaceRoot,
    cache: &Arc<Mutex<fs::DomainCache>>,
    cache_root: &Path,
) -> io::Result<Option<CutoverRun>> {
    match fs::cutover::status(root)? {
        CutoverStatus::Present(rec) if rec.state == CutoverState::NewBuilding => {
            run(root, cache, cache_root, Resume::Record(rec), None).map(Some)
        }
        CutoverStatus::Present(_) | CutoverStatus::Absent => Ok(None),
    }
}

/// START the one-time cutover on a workspace that has never taken it — the
/// explicit operator act. Refuses a workspace that already carries a record
/// rather than crossing anything twice.
///
/// # Errors
/// `AlreadyExists` when the workspace already has a cutover record; the
/// drain's refusals (a live writer, unsettled intents); any I/O failure.
pub fn start(
    root: &fs::WorkspaceRoot,
    cache: &Arc<Mutex<fs::DomainCache>>,
    cache_root: &Path,
    live: Option<&crate::ring::WorkspaceRing>,
) -> io::Result<CutoverRun> {
    if let CutoverStatus::Present(rec) = fs::cutover::status(root)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "this workspace already carries cutover {} ({}) — the boundary is crossed once",
                rec.cutover_id,
                match rec.state {
                    CutoverState::NewBuilding => "still building",
                    CutoverState::NewCommitted => "committed",
                }
            ),
        ));
    }
    run(
        root,
        cache,
        cache_root,
        Resume::Fresh { old_instance: None },
        live,
    )
}

/// Where the machine starts: a fresh boundary crossing (record absent), or
/// a resume of a crashed build (`NEW_BUILDING` found durable at open).
pub enum Resume {
    Fresh {
        /// The retired ring instance when a LIVE drain observed one — the
        /// record remembers it so a late resumption cursor naming it gets
        /// the labeled `fingerprint_epoch_retired` answer (wire-contract
        /// §4.7). An at-open cutover has none to observe.
        old_instance: Option<String>,
    },
    Record(CutoverRecord),
}

/// Walk the machine: drain → no-return → build → commit. See the module doc
/// for what each phase is and is not. `live` is the workspace's ring when
/// the cutover drains a SERVING workspace: its subscriptions END at the
/// labeled boundary (one terminal `fingerprint_epoch_retired` control each,
/// then close — card step 2: never a silent chain break) and the record
/// remembers its retired instance. At-open cutovers have no ring.
///
/// # Errors
/// The drain's refusals (a writer holds the workspace past the deadline, an
/// unsettled intent), the record's write failures, or any I/O failure in the
/// build.
pub fn run(
    root: &fs::WorkspaceRoot,
    cache: &Arc<Mutex<fs::DomainCache>>,
    cache_root: &Path,
    resume: Resume,
    live: Option<&crate::ring::WorkspaceRing>,
) -> io::Result<CutoverRun> {
    let t_total = Instant::now();

    // DRAINING (in-RAM by design — a crash here leaves no record and the
    // old law serving, the pre-boundary crash row): the interim EX is the
    // cross-process admission close; holding it proves no governed writer
    // is in flight. Subscriptions end HERE, labeled (the paper's drain
    // steps 6–7); a crash after the send and before the boundary closed
    // subscribers spuriously, which is safe — they resync and the old law
    // still serves. Not durable: the record's two states are the only
    // durable ones (merkle-spec §4.2.6).
    let t = Instant::now();
    let _ex = acquire_ex(root)?;
    settled_intents(root)?;
    let domain = fs::domain::Domain::load(root)?.version();
    let cutover_id = match &resume {
        Resume::Fresh { .. } => fs::cutover::mint_cutover_id(),
        Resume::Record(record) => record.cutover_id.clone(),
    };
    let observed_instance = live.map(|ring| {
        ring.retire_epoch(crate::ring::RetiredEpoch {
            cutover_id: cutover_id.clone(),
            from: (model::HASH_LAW_FLAT, domain),
            to: (model::HASH_LAW_RADIX, domain),
        });
        ring.instance()
    });
    let drain_us = t.elapsed().as_micros();

    // The no-return boundary — ONE durable act — or the resumption of a
    // boundary already crossed.
    let t = Instant::now();
    let record = match resume {
        Resume::Fresh { old_instance } => {
            let record = CutoverRecord {
                state: CutoverState::NewBuilding,
                cutover_id,
                from: model::RootVersion::law1(domain),
                to: model::RootVersion::law2(domain),
                old_instance: observed_instance.or(old_instance),
                started_unix_ms: unix_ms(),
                committed_unix_ms: None,
            };
            fs::cutover::begin_new_building(root, &record)?;
            record
        }
        Resume::Record(record) => record,
    };
    let record_begin_us = t.elapsed().as_micros();

    // NEW_BUILDING. First the checkpoint discard — a pre-boundary file must
    // never be adopted past the boundary; idempotent across resumes.
    let t = Instant::now();
    super::checkpoint::discard(cache_root, &root.0);
    let checkpoint_discard_us = t.elapsed().as_micros();

    // The one Markdown walk: the observation builds every leaf and radix
    // vertex under the new identity, and the serve below mints the first
    // radix-encoded token. The flip precedes the build so nothing minted
    // mid-build can carry the old law (the record already refuses serving
    // to every OTHER door via its NEW_BUILDING state).
    let t = Instant::now();
    let serving_law = ServingLaw {
        hash_law: model::HASH_LAW_RADIX,
        law_boundary: record.to.ladder_position(),
    };
    let built = {
        let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.set_serving_law(serving_law);
        cache.root(root)?
    };
    let build_us = t.elapsed().as_micros();

    // NEW_COMMITTED — the durable publication; admission reopens when the
    // EX drops on return.
    let t = Instant::now();
    let mut committed = record.clone();
    committed.state = CutoverState::NewCommitted;
    committed.committed_unix_ms = Some(unix_ms());
    fs::cutover::commit(root, &committed)?;
    let commit_us = t.elapsed().as_micros();

    let components = Components {
        drain_us,
        record_begin_us,
        checkpoint_discard_us,
        build_us,
        commit_us,
        total_us: t_total.elapsed().as_micros(),
    };
    write_receipt(root, &committed, components, &built.0);
    Ok(CutoverRun {
        record: committed,
        components,
    })
}

fn acquire_ex(root: &fs::WorkspaceRoot) -> io::Result<fs::WriteLock> {
    let deadline = Instant::now() + DRAIN_DEADLINE;
    loop {
        match fs::WriteLock::acquire(root) {
            Ok(lock) => return Ok(lock),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "cutover drain: a writer held the workspace write lock past the \
                         drain deadline",
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e),
        }
    }
}

/// The drain's settled-transactions proof: `.meridian/intents/` holds no
/// entries. Outstanding intents are settled by the old law's own recovery
/// BEFORE the boundary (gate 19: no old-law decided transaction completes
/// afterward) — the machine refuses to cross rather than adopt them.
fn settled_intents(root: &fs::WorkspaceRoot) -> io::Result<()> {
    let dir = root.0.join(".meridian").join("intents");
    match std::fs::read_dir(&dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(entries) => {
            let names: Vec<String> = entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            if names.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "cutover drain: {} unsettled write intent(s) in .meridian/intents \
                     ({}) — recover them under the old law first; the boundary is not \
                     crossed over live transactions",
                    names.len(),
                    names.join(", ")
                )))
            }
        }
    }
}

/// Best-effort measurement sidecar — informational, never authority (the
/// record is the authority; this file keeps gate-22/24 numbers where the
/// cutover ran).
fn write_receipt(root: &fs::WorkspaceRoot, record: &CutoverRecord, c: Components, served: &str) {
    let body = format!(
        "cutover {}\nserved_after={served}\ndrain_us={}\nrecord_begin_us={}\n\
         checkpoint_discard_us={}\nbuild_us={}\ncommit_us={}\ntotal_us={}\n\
         fence=never-activates (spec 4.2.5 amendment 2026-08-16)\n\
         projection=lazy-by-design (view store recreates on prefix change, outside the pause)\n\
         checkpoint_fresh=ordinary lifecycle (best-effort by design)\n",
        record.cutover_id,
        c.drain_us,
        c.record_begin_us,
        c.checkpoint_discard_us,
        c.build_us,
        c.commit_us,
        c.total_us,
    );
    let _ = std::fs::write(root.0.join(".meridian").join("cutover.receipt"), body);
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs::cutover::{CutoverState, CutoverStatus};

    fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("plan.md"), "# Plan\n\nship it\n").expect("page");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        let cache_root = tempfile::tempdir().expect("cache root");
        (dir, root, cache_root)
    }

    fn memo() -> Arc<Mutex<fs::DomainCache>> {
        Arc::new(Mutex::new(fs::DomainCache::new()))
    }

    #[test]
    fn the_machine_walks_once_and_ensure_is_then_a_no_op() {
        let (_d, root, cache_root) = ws();
        let cache = memo();
        let run = start(&root, &cache, cache_root.path(), None).expect("the operator act");
        assert_eq!(run.record.state, CutoverState::NewCommitted);
        assert!(run.record.committed_unix_ms.is_some());
        assert!(run.components.total_us > 0);
        // The measurement sidecar landed beside the record.
        let receipt =
            std::fs::read_to_string(root.0.join(".meridian/cutover.receipt")).expect("receipt");
        assert!(receipt.contains("build_us="), "{receipt}");
        assert!(receipt.contains("fence=never-activates"), "{receipt}");
        // The served token is law 2 from the machine's own build.
        let served = cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .overlay_root()
            .expect("served");
        assert!(served.0.starts_with("b3a:"), "{}", served.0);
        // Second entry: nothing to do.
        assert!(
            resume_if_interrupted(&root, &cache, cache_root.path())
                .expect("machine")
                .is_none(),
            "a committed workspace has nothing to resume"
        );
        let err = start(&root, &cache, cache_root.path(), None)
            .expect_err("and the boundary is crossed once");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn a_crashed_build_resumes_and_never_serves_old() {
        let (_d, root, cache_root) = ws();
        // Crash state: the boundary is durable, the build never finished.
        let record = CutoverRecord {
            state: CutoverState::NewBuilding,
            cutover_id: fs::cutover::mint_cutover_id(),
            from: model::RootVersion::law1(0),
            to: model::RootVersion::law2(0),
            old_instance: None,
            started_unix_ms: 1,
            committed_unix_ms: None,
        };
        fs::cutover::begin_new_building(&root, &record).expect("begin");

        let cache = memo();
        let run = resume_if_interrupted(&root, &cache, cache_root.path())
            .expect("machine")
            .expect("a NEW_BUILDING record resumes");
        assert_eq!(run.record.cutover_id, record.cutover_id);
        assert_eq!(run.record.state, CutoverState::NewCommitted);
        match fs::cutover::status(&root).expect("status") {
            CutoverStatus::Present(rec) => assert_eq!(rec.state, CutoverState::NewCommitted),
            CutoverStatus::Absent => panic!("the record survives"),
        }
    }

    #[test]
    fn a_live_drain_ends_the_epoch_labeled_and_remembers_the_instance() {
        let (_d, root, cache_root) = ws();
        let ring = crate::ring::WorkspaceRing::new(&root);
        let instance = ring.instance();
        let cache = memo();
        let outcome = run(
            &root,
            &cache,
            cache_root.path(),
            Resume::Fresh { old_instance: None },
            Some(&ring),
        )
        .expect("live machine");
        assert_eq!(
            outcome.record.old_instance.as_deref(),
            Some(instance.as_str())
        );
        let retired = ring.retired().expect("the epoch ended labeled");
        assert_eq!(retired.cutover_id, outcome.record.cutover_id);
        assert_eq!(retired.from, (model::HASH_LAW_FLAT, 0));
        assert_eq!(retired.to, (model::HASH_LAW_RADIX, 0));
    }

    /// Gate 22/24 driver — run explicitly (`--ignored --nocapture`) on a
    /// measurement host: builds a corpus of `MERIDIAN_CUTOVER_MEASURE_N`
    /// members (default `100_000`) and reports every outage component
    /// separately. The 1.45 s old-law receipt is never quoted; this run IS
    /// the number's source.
    #[test]
    #[ignore = "measurement driver, not a gate — run with --ignored on the measurement host"]
    fn measure_cutover_components() {
        let n: usize = std::env::var("MERIDIAN_CUTOVER_MEASURE_N")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..n {
            let sub = dir.path().join(format!("d{:03}", i % 428));
            std::fs::create_dir_all(&sub).expect("mkdir");
            std::fs::write(
                sub.join(format!("m{i}.md")),
                format!("# M{i}\n\nmember {i} of the measurement corpus\n"),
            )
            .expect("member");
        }
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        let cache_root = tempfile::tempdir().expect("cache root");
        let outcome = start(&root, &memo(), cache_root.path(), None).expect("fresh cutover");
        let c = outcome.components;
        println!(
            "CUTOVER-MEASURE n={n}: drain_us={} record_begin_us={} \
             checkpoint_discard_us={} build_us={} commit_us={} total_us={}",
            c.drain_us,
            c.record_begin_us,
            c.checkpoint_discard_us,
            c.build_us,
            c.commit_us,
            c.total_us
        );
        println!(
            "CUTOVER-MEASURE receipt:\n{}",
            std::fs::read_to_string(root.0.join(".meridian/cutover.receipt")).expect("receipt")
        );
    }

    #[test]
    fn unsettled_intents_hold_the_boundary_closed() {
        let (_d, root, cache_root) = ws();
        let intents = root.0.join(".meridian/intents");
        std::fs::create_dir_all(&intents).expect("mkdir");
        std::fs::write(intents.join("aa.intent"), b"x").expect("intent");

        let err = start(&root, &memo(), cache_root.path(), None).expect_err("the drain refuses");
        assert!(err.to_string().contains("unsettled"), "{err}");
        // And the boundary was NOT crossed: the old law still serves.
        assert_eq!(
            fs::cutover::status(&root).expect("status"),
            CutoverStatus::Absent
        );
    }
}
