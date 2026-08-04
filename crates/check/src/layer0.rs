//! Layer 0 — the rule-free core of `check`.
//!
//! Two pack-free reads over the run plane, neither of which writes a byte:
//! - [`claims_realised`] — observe each claim against the current tree and report
//!   the drifted ones (the realise engine's pure detection, run read-only here);
//! - [`pin_plane`] — the CLAIM plane (`pins:` — did the content drift?) and the
//!   RETRIEVAL plane (each pin's `hash` — is the blob durably anchored?).
//!
//! # Why this layer holds no write-history plane — the LAW, not a gap
//! ZT, ruling 2026-08-03, verbatim: *"Engine does not have memory. It should not
//! have. History is pin to git when we lock. Anything between locks is not
//! history."*
//!
//! So layer 0 reads **at-rest / at-touch truth only: does the world still match the
//! pins.** The receipt journal is deleted and its chain continuity, baseline dating
//! and interval accounting are **never rebuilt as check memory** — not because they
//! were lost, but because they were never this engine's to carry. Archaeology lives
//! in git; attribution lives in transcript JSONL.
//!
//! That is why an out-of-band edit followed by a governed write is not detected
//! here. It is **outside the engine's domain by design**, not a blind spot the
//! engine tolerates. A detector for it would be this layer reaching back into
//! memory it is ruled not to keep.

use std::collections::{BTreeMap, BTreeSet};

use fs::WorkspaceRoot;
use model::Document;
use model::selector::Color;
use realise::{CheckOutcome, Claim};
use receipt::anchor::{ObjectAnchor, ObjectAnchorFacts};

/// The reason word a grey carries, verbatim on BOTH faces — the human render and
/// `--json` — per S3-R6. The colour alone is not the answer: `grey(unmounted)` and
/// `grey(cannot-assess)` are different refusals that share exit 1, so the word is
/// what tells them apart.
pub const GREY_CANNOT_ASSESS: &str = "grey(cannot-assess)";

/// **`check` does not assess write history**, and both faces say so WITH THE REASON
/// — a bare "not-assessed" reads as a gap, while the reason reads as the design it
/// is: the engine keeps no memory, history is pinned to git at lock, and anything
/// between locks is not history.
///
/// Stated because the green NARROWED: a corpus that once read grey ("I cannot date
/// your write history") now reads green ("the world still matches the pins"). A
/// silent narrowing would let a reader carry the old, wider green forward.
pub const WRITE_HISTORY_NOT_ASSESSED: &str = "not-assessed";

/// One claim that is NOT realised: its selector and why the observation drifted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFinding {
    /// The claim selector (the board-card key; d2 §5.4).
    pub selector: String,
    /// Why the check failed — the observed-vs-expected detail from the drift.
    pub detail: String,
}

/// Claims-realised (d2 §3): observe each claim against the current tree and
/// report the ones that did not converge. A pure read — no apply, no cap (the
/// `check` verb never converges; the realise loop does). Reuses the realise
/// engine's [`Check`] detection rather than forking it, so `check` reads exactly
/// what `realise` would converge.
///
/// # Errors
/// [`realise::CheckError`] when a claim's observation itself faults (page load /
/// I/O) — distinct from a clean [`CheckOutcome::Drifted`], which is a finding, not
/// an error.
pub fn claims_realised(
    root: &WorkspaceRoot,
    claims: &[Claim],
) -> Result<Vec<ClaimFinding>, realise::CheckError> {
    let mut drifted = Vec::new();
    for claim in claims {
        match claim.check.observe(root)? {
            CheckOutcome::Converged => {}
            CheckOutcome::Drifted { detail } => drifted.push(ClaimFinding {
                selector: claim.selector.clone(),
                detail,
            }),
        }
    }
    Ok(drifted)
}

// ---------------------------------------------------------------------------
// U14 — the PIN PLANE: what the journal plane cannot see
// ---------------------------------------------------------------------------

/// One pin row as the ONE pin computer coloured it, handed to [`pin_plane`].
///
/// # Why this arrives as a FACT rather than being computed here
/// The colour of a pin is `view`'s to compute — it is the end of a chain that
/// runs corpus index → ref resolution → selector → fingerprint compare, and a
/// SECOND copy of that chain is exactly how the pin plane and the decoration
/// plane once came to hash two different documents for one ref. `check` may not
/// take `view` (S3-R3 authorized the `git` and `lock` leaves, and nothing wider),
/// so the caller reads `view::walk::lock_pin_colors` — **the same call
/// `mrd status`'s lock axis makes and the same colouring `mrd walk` lists** — and
/// hands the rows here.
///
/// That is the fact/classify split this workspace already runs on
/// ([`receipt::anchor::ObjectAnchorFacts`]: *"the caller asks git; this crate
/// never shells out"*), and it is what makes the three planes agree **by
/// construction** rather than by assertion: there is one computer, not three that
/// match today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRow {
    /// The page whose `meridian-lock` block declares this pin.
    pub src_path: String,
    /// The declared ref, verbatim — empty on a lock-refusal row, which declares
    /// no ref and carries its refusal in [`PinRow::color`].
    pub declared_ref: String,
    /// The colour the ONE pin computer gave it — what decides red from grey.
    pub color: Color,
    /// The colour RENDERED by that same computer (`view::walk::color_label`):
    /// `red content-drifted`, `grey unmounted (root 'x')`, and so on.
    ///
    /// Carried rather than re-derived because the reason words are spelled ONCE,
    /// in `view`, and a second speller in this crate would be two literals that
    /// agree only until someone edits one of them (S3-R59). Same shape as
    /// [`ClaimFinding::detail`], which likewise arrives pre-rendered from the
    /// engine that owns the words.
    pub label: String,
}

/// One pinned blob that is **ORPHANED**: no ref reaches it, and the
/// file it is the blob OF no longer hashes to it — so no commit of that file will
/// ever anchor it either. The pin's evidence is held by nothing and is on its way
/// to nothing.
///
/// # Why the refusal is this pair and not a state on its own (measured, S3-R23(1))
/// *"Measure the ground truth BEFORE you write the check."* Measured across the
/// ordinary governed lifecycle of one pin:
///
/// | moment | `ObjectAnchor` | why |
/// |---|---|---|
/// | after `mrd pin`, before `git add` | **never-anchored** | a non-vibe pin hashes without `-w`, so the object is not in the database at all |
/// | at pre-commit hook time (staged) | **pending-anchor** | `git add` wrote the blob; the commit that would anchor it is the one being fenced |
/// | after the commit | **anchored** | a ref reaches it |
///
/// **So a refusal on `pending-anchor` refuses EVERY governed commit, and a refusal
/// on `never-anchored` refuses every workspace between `pin` and `git add`.**
/// Verified: wiring the refusal to `pending-anchor` reddened all three legs of
/// `u35_journaled_doors.rs`, which drive a REAL `git commit` through the RATIFIED
/// fence over fully governed corpora. That is the "fence refuses every governed
/// commit" failure S3-R8 migrated a whole unit to prevent, arriving from the other
/// side.
///
/// The state that is a defect in every one of those moments is the ORPHAN: the
/// blob is unreachable **and** the file's current bytes no longer produce it. The
/// three states stay a READING ([`PinPlane::anchored`] / `pending` / `never`); this
/// pair is the finding.
///
/// **No new reason word is minted** (S3-R49, convention over invention): the
/// finding cites [`ObjectAnchor::word`] for whichever of the two non-durable states
/// it is, and states the orphaning as EVIDENCE beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanedBlob {
    /// The page whose `meridian-lock` block declares this object.
    pub src_path: String,
    /// The pin's `object`, verbatim (what the blob is FOR).
    pub key: String,
    /// The blob sha, verbatim — an object id in git's world, not the engine's.
    pub blob_sha: String,
    /// Which non-durable state git reports it in — the EXISTING word, reused.
    pub state: ObjectAnchor,
    /// What the file hashes to now, or why it could not be hashed. The evidence
    /// that no commit will anchor the recorded blob.
    pub live: String,
}

/// The pin plane's reading over one corpus: what `check` can say about the pins
/// that the receipt journal will never carry.
///
/// **The journal plane and the pin plane fail INDEPENDENTLY, and that is the
/// whole finding.** A workspace whose last receipt accounts for the live tree and
/// whose chain is continuous — a journal plane with nothing at all to report — can
/// still hold a pin whose target drifted (the lock arrived by clone or pull while
/// its source moved) or a blob no ref reaches. Measured on this tree: `check`
/// green / `walk` `red content-drifted` / `status` `lock red content-drifted`, one
/// corpus, one run.
#[derive(Debug)]
pub struct PinPlane {
    /// Pins whose colour is RED — the ledger claims content that is no longer
    /// there.
    pub red: Vec<PinRow>,
    /// Pins whose colour is GREY — outside sight, each carrying its own reason
    /// word (S3-R6: one vocabulary, distinct words for distinct causes).
    pub grey: Vec<PinRow>,
    /// Pinned blobs nothing holds and nothing will — the
    /// anchoring FINDING. See [`OrphanedBlob`] for why the refusal is this pair
    /// and not a bare state.
    pub orphaned: Vec<OrphanedBlob>,
    /// **The anchoring three-state, as a READING** (GAP A: the verb was
    /// structurally blind to it; now it is not). Counts, because the three states
    /// are the ordinary lifecycle of a pin and not three verdicts.
    ///
    /// The counts also carry the POPULATION (S3-R23(5)): an empty `orphaned` list
    /// means two completely different things at zero pinned objects and at fifty,
    /// and *"a gate whose population empties is the quietest way for coverage to
    /// disappear"*.
    pub anchored: usize,
    /// Blobs git holds that no ref reaches — durable only until `gc.pruneExpire`
    /// ([`receipt::anchor::PENDING_ANCHOR_TTL`]). The normal state of anything
    /// staged, so a reading and never a gate here.
    pub pending: usize,
    /// Blobs git does not hold at all. The normal state of a non-vibe pin before
    /// `git add`, so likewise a reading.
    pub never: usize,
    /// Why the object store could not be asked at all, when it could not be.
    /// `Some` ⇒ the retrieval plane is UNREAD, never read clean.
    pub cannot_ask: Option<String>,
    /// **How many pins this corpus DECLARES — the CLAIM plane's population**
    /// (S3-R23(5), the same doctrine [`PinPlane::asked`] carries for the retrieval
    /// plane).
    ///
    /// [`PinPlane::red`] and [`PinPlane::grey`] list only pins that are not green,
    /// so an empty pair means two entirely different things over fifty pins and
    /// over none — *"a gate whose population empties is the quietest way for
    /// coverage to disappear"*. Before the journal died the difference was
    /// academic; it is now load-bearing, because the pin plane is the ONLY thing
    /// `--commit-gate` reads, and zero declared pins is the case where it gates on
    /// nothing at all.
    ///
    /// Read by `--require-pins` (`mrd check`), which is how a caller that wants
    /// "no coverage ⇒ refuse" says so.
    pub declared: usize,
    /// **Pins whose blob lives in ANOTHER root's object store — out of this
    /// read's jurisdiction, skipped and STATED** (ruling 2026-08-04).
    ///
    /// The anchoring plane holds ONE git handle by design: the ambient root's.
    /// A cross-root pin's blob is in a repository this read cannot ask, so it is
    /// not measured here — and *"outside sight is never verified"* is honoured by
    /// STATING THE SIGHT LINE, not by pretending the blob was seen. Both the
    /// human face and `--json` name this population; a silent narrowing would be
    /// a lie, and this one is spoken.
    ///
    /// # Why this is a scoping and not a gap
    /// Pre-R4 the retrieval plane was the separate `objects:` table, which in
    /// practice carried no cross-root rows — so ambient-only is what this plane
    /// has ALWAYS measured. R4 moved the blob hash onto every pin row and thereby
    /// made the implicit population VISIBLE; it did not widen the jurisdiction.
    /// Left unscoped, every corpus holding one cross-root pin became ungovernable,
    /// because the fence refused every commit.
    ///
    /// **Only blob-ANCHORING scopes.** The CLAIM half stays cross-root — that is
    /// U11's proven capability and it is untouched here.
    pub out_of_jurisdiction: Vec<String>,
}

impl PinPlane {
    /// The pin plane found a finding: a red pin, or an ORPHANED blob.
    ///
    /// **An orphan rides the finding leg, not the grey one**, because it is an
    /// ANSWER and not the absence of one: git was asked, git replied, and the reply
    /// is that the pin's evidence is held by nothing and no commit will anchor it.
    /// Grey is reserved for the question that could not be put.
    ///
    /// **The three anchoring states themselves are NOT findings** — they are the
    /// ordinary lifecycle of a pin, and refusing on either non-durable one refuses
    /// every governed commit. [`OrphanedBlob`] carries the measurement.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.red.is_empty() || !self.orphaned.is_empty()
    }

    /// How many pinned blobs git was asked about — the population behind the
    /// three-state reading (S3-R23(5)).
    #[must_use]
    pub fn asked(&self) -> usize {
        self.anchored + self.pending + self.never
    }

    /// The pin plane could not assess something: a grey pin, or an object store
    /// it could not ask. Never green, never red.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        !self.grey.is_empty() || self.cannot_ask.is_some()
    }

    /// Nothing to report and everything asked — the assessed green. Distinct from
    /// "no pins in the corpus" only in that both are true readings; a corpus that
    /// declares no pins genuinely owes nothing.
    #[must_use]
    pub fn is_green(&self) -> bool {
        !self.is_red() && !self.cannot_assess()
    }
}

/// Read the pin plane over `docs`: sort the caller's pin colours, then ask THIS
/// root's object store about every blob the corpus pins.
///
/// `docs` must be the SAME corpus build the pin colours came from — a second
/// build would let the two halves describe two different corpora, which is the
/// trap `mrd status`'s two axes are explicitly built to avoid.
///
/// # It asks nothing it does not have to
/// A corpus that pins no blob asks git nothing: nothing is
/// referenced, so nothing can be owed, and a workspace outside a repository stays
/// green rather than being refused for a question nobody asked.
///
/// # Every unanswerable question is REPORTED, never skipped
/// A value that is not an object id, a key naming a root whose store this read
/// cannot reach, git absent, the directory not a repository — all four leave the
/// retrieval plane UNREAD, and an unread plane is [`PinPlane::cannot_ask`], never
/// an empty (falsely clean) one. *Outside sight is never verified.*
#[must_use]
pub fn pin_plane(
    root: &WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    pins: &[PinRow],
) -> PinPlane {
    let mut plane = PinPlane {
        red: Vec::new(),
        grey: Vec::new(),
        orphaned: Vec::new(),
        anchored: 0,
        pending: 0,
        never: 0,
        cannot_ask: None,
        declared: pins.len(),
        out_of_jurisdiction: Vec::new(),
    };
    for pin in pins {
        match pin.color {
            Color::Red(_) => plane.red.push(pin.clone()),
            Color::Grey(_) => plane.grey.push(pin.clone()),
            Color::Green => {}
        }
    }
    match objects_in(docs) {
        Err(detail) => plane.cannot_ask = Some(detail),
        Ok((objects, outside)) => {
            plane.out_of_jurisdiction = outside;
            if !objects.is_empty()
                && let Err(detail) = ask_store(root, &objects, &mut plane)
            {
                plane.cannot_ask = Some(detail);
            }
        }
    }
    plane
}

/// One pinned blob located in the corpus — the R4 per-pin `hash` and the target
/// it is the blob OF.
#[derive(Debug)]
struct ObjectRef {
    src_path: String,
    key: String,
    blob_sha: String,
}

/// Every pinned blob the corpus declares, or the reason the plane is unread.
///
/// [`lock::find`] is the parser — the SAME owner `view::walk::lock_objects` uses,
/// so a page's pinned blobs can never be read here by a second reader that
/// disagrees with the listing surfaces about what the page declares.
///
/// A page whose lock REFUSED contributes nothing, exactly as it contributes
/// none to the listing planes: its damage is already named by the grey
/// `lock-refused` PIN row the same page projects, and naming one defect twice is
/// how a reader comes to believe there are two.
///
/// # One blob, one question (R4)
/// R4 retired the shared `objects:` table and moved the git blob hash onto the
/// pin row itself, so the whole-file lock — `path: []` and `properties: []` on
/// one object — declares the SAME blob twice. They are deduped by `(object,
/// hash)` here for the reason stated above: two rows naming one blob would ask
/// git one question twice and report a single orphan as two findings.
fn objects_in(docs: &BTreeMap<String, Document>) -> Result<(Vec<ObjectRef>, Vec<String>), String> {
    let mut out: Vec<ObjectRef> = Vec::new();
    let mut outside: Vec<String> = Vec::new();
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (path, doc) in docs {
        let Ok(Some(found)) = lock::find(doc) else {
            continue;
        };
        for pin in found.lock.pins {
            let key = pin.object;
            // ── JURISDICTION, decided on STRUCTURE and decided FIRST ──────────
            //
            // The discriminator is "is this object's root the ambient one", read
            // off the ADDRESS — never "did asking the store fail". That ordering
            // is the whole safety property: a behavioural skip would let a
            // genuinely broken ambient store hide inside the exemption, which is
            // a real defect wearing a jurisdiction badge. A broken AMBIENT store
            // still greys and still fails closed, below.
            //
            // An object naming another root names another object store, and this
            // read holds one handle. It is not measured here and it is not
            // pretended to be measured: the caller STATES the population
            // ([`PinPlane::out_of_jurisdiction`]).
            //
            // WHO OWNS THE QUESTION INSTEAD: cross-root blob durability is
            // `u13_per_root_anchoring`'s surface — the per-root anchoring read
            // that holds the right handle. This scoping narrows THIS plane; it
            // does not make the question disappear.
            match addr::Addr::parse(&key) {
                Ok(addr) => {
                    if let Some(name) = addr.root() {
                        outside.push(format!("`{path}` pin `{key}` (root `{name}`)"));
                        continue;
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "`{path}` pin `{key}` is not an address, so WHICH git to ask is \
                         unknown: {e}"
                    ));
                }
            }
            let oid = pin.hash.to_ascii_lowercase();
            if !git::is_oid(&oid) {
                return Err(format!(
                    "`{path}` pin `{key}` has a hash that is not an object id, so git cannot be \
                     asked about it"
                ));
            }
            if !seen.insert((path.clone(), key.clone(), oid.clone())) {
                continue;
            }
            out.push(ObjectRef {
                src_path: path.clone(),
                key,
                blob_sha: oid,
            });
        }
    }
    Ok((out, outside))
}

/// Ask ONE object store about every entry, in ONE pass, and classify.
///
/// Two git calls at most, never a call per blob: ONE `git rev-list --objects
/// --all` into the reachable set (O(1) membership) and ONE batched `git cat-file
/// --batch-check` for presence. [`ObjectAnchor::classify`] then reads the pair —
/// the same classifier `mrd status`'s gauge runs, so the two surfaces cannot
/// disagree about what a blob's state is called.
///
/// **Both facts come from THIS handle in THIS pass** — the per-root law's own
/// warning (U13): *a fact pair split across two stores is a wrong answer, not a
/// stale one.*
///
/// **All three states are counted as a READING; only the ORPHAN is a finding.**
/// The three states are the ordinary lifecycle of one pin (never-anchored →
/// pending-anchor → anchored, measured in [`OrphanedBlob`]'s table), so refusing on
/// either non-durable one refuses every governed commit. A blob is orphaned when it
/// is unreachable **and** the file it is the blob OF no longer hashes to it: nothing
/// holds it and no commit will.
///
/// That second question costs one `git hash-object` per NON-ANCHORED entry only —
/// an anchored blob is durable and needs no follow-up — so the common case stays at
/// the two batched calls above.
fn ask_store(
    root: &WorkspaceRoot,
    objects: &[ObjectRef],
    plane: &mut PinPlane,
) -> Result<(), String> {
    let repo = git::Repo::at(root.0.clone());
    let reachable = repo
        .reachable_objects()
        .map_err(|e| format!("the object store could not be asked: {e}"))?;
    let oids: Vec<&str> = objects.iter().map(|o| o.blob_sha.as_str()).collect();
    let info = repo
        .object_info(&oids)
        .map_err(|e| format!("the object store could not be asked: {e}"))?;

    for (object, present) in objects.iter().zip(info) {
        let facts = ObjectAnchorFacts {
            object_present: present.is_some(),
            reachable_from_commit: reachable.contains(&object.blob_sha),
        };
        let state = ObjectAnchor::classify(&facts);
        match state {
            ObjectAnchor::Anchored => {
                plane.anchored += 1;
                continue;
            }
            ObjectAnchor::PendingAnchor => plane.pending += 1,
            ObjectAnchor::NeverAnchored => plane.never += 1,
        }
        // Does the file still hash to this blob? If it does, committing the file
        // anchors it and this is a lifecycle moment, not a defect. If it does not,
        // nothing holds the blob and nothing will.
        // R4's `object` is the target's wiki-link inner text — the vault path
        // WITHOUT `.md` (the spelling U8's `pin_row` mints). The blob is the blob
        // of the FILE, so the extension goes back on before git is asked.
        let live = match repo.blob_oid(&root.0.join(format!("{}.md", object.key))) {
            Ok(oid) if oid.eq_ignore_ascii_case(&object.blob_sha) => continue,
            Ok(oid) => oid,
            Err(e) => format!("the file could not be hashed ({e})"),
        };
        plane.orphaned.push(OrphanedBlob {
            src_path: object.src_path.clone(),
            key: object.key.clone(),
            blob_sha: object.blob_sha.clone(),
            state,
            live,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use realise::Check;

    use super::*;

    /// A read-only claim (no apply) over a fixed [`CheckOutcome`] — the pure-read
    /// half of a realise [`Claim`], for the claims-realised tests.
    fn fixed_claim(selector: &str, outcome: CheckOutcome) -> Claim {
        struct Fixed(CheckOutcome);
        impl Check for Fixed {
            fn observe(&self, _root: &WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
                Ok(self.0.clone())
            }
        }
        Claim {
            selector: selector.to_string(),
            rule: None,
            check: Box::new(Fixed(outcome)),
            apply: None,
            retry_budget: 0,
        }
    }

    /// A drifted claim surfaces as a [`ClaimFinding`]; a converged one does not.
    #[test]
    fn claims_realised_reports_only_drifted_claims() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let claims = [
            fixed_claim("green", CheckOutcome::Converged),
            fixed_claim(
                "red",
                CheckOutcome::Drifted {
                    detail: "status: 'open' is 'closed'".to_string(),
                },
            ),
        ];
        let drifted = claims_realised(&root, &claims).expect("clean observations");
        assert_eq!(drifted.len(), 1, "only the drifted claim surfaces");
        assert_eq!(drifted[0].selector, "red");
        assert_eq!(drifted[0].detail, "status: 'open' is 'closed'");
    }

    // ── U14: the pin plane ──────────────────────────────────────────────────

    /// A corpus of one page, built the way `fs::build_corpus` builds one.
    fn corpus(path: &str, text: &str) -> BTreeMap<String, Document> {
        let doc = model::build(text.to_string(), syntax::parse(text));
        BTreeMap::from([(path.to_string(), doc)])
    }

    /// A `meridian-lock` block carrying one R4 pin, in the canonical bytes the
    /// engine mints.
    ///
    /// Rendered through [`lock::render`] rather than hand-written: the pre-R4
    /// fixtures here were hand-written `objects:` tables, and a hand-written
    /// fixture is exactly what let a retired schema keep compiling after the type
    /// it described was gone. `object` is the wiki-link inner text — the target's
    /// vault path WITHOUT `.md`.
    fn pin_page(object: &str, sha: &str) -> String {
        let mut lock = lock::Lock::new();
        lock.upsert_pin(lock::PinEntry::new(
            object,
            sha,
            lock::Selector::Path(vec!["S".to_string()]),
            "fp1.span2.b3.a8222f5a",
        ));
        format!("# P\n\n{}\n", lock::render(&lock))
    }

    fn pin(color: Color) -> PinRow {
        PinRow {
            src_path: "claim.md".to_string(),
            declared_ref: "source.md#S/G".to_string(),
            label: "rendered by the ONE computer".to_string(),
            color,
        }
    }

    /// The sort is load-bearing in BOTH directions: a red pin is a FINDING and a
    /// grey pin is a refusal-for-want-of-evidence, and collapsing either into the
    /// other is what S3-R6's separate reason words exist to prevent. A sort that
    /// always said red fails the grey arm; one that always said grey fails the red.
    #[test]
    fn a_red_pin_is_a_finding_and_a_grey_pin_is_an_absence_of_evidence() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = BTreeMap::new();

        let red = pin_plane(
            &root,
            &docs,
            &[pin(Color::Red(model::selector::RedReason::Drifted))],
        );
        assert!(red.is_red(), "a drifted pin is a lie the ledger is telling");
        assert!(
            !red.cannot_assess(),
            "it was assessed — that is why it is red"
        );

        let grey = pin_plane(
            &root,
            &docs,
            &[pin(Color::Grey(model::selector::GreyReason::Ambiguous))],
        );
        assert!(!grey.is_red(), "grey is not red");
        assert!(grey.cannot_assess(), "grey names itself");
        assert!(!grey.is_green(), "and it is never green");

        let green = pin_plane(&root, &docs, &[pin(Color::Green)]);
        assert!(green.is_green(), "a green pin leaves nothing to report");
    }

    /// **An ORPHANED blob rides the FINDING leg; the bare three states do not.**
    ///
    /// The orphan is an ANSWER — git was asked and replied — not a question that
    /// could not be put, so grey is wrong for it. And the three states on their own
    /// are the ordinary lifecycle of a pin: a plane holding a `pending` and a
    /// `never` count with no orphan among them is **green**, because refusing there
    /// would refuse every governed commit (measured — see [`OrphanedBlob`]).
    #[test]
    fn an_orphaned_blob_is_a_finding_while_the_bare_three_states_are_a_reading() {
        let lifecycle = PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            orphaned: Vec::new(),
            anchored: 1,
            pending: 1,
            never: 1,
            cannot_ask: None,
            declared: 3,
            out_of_jurisdiction: Vec::new(),
        };
        assert_eq!(lifecycle.asked(), 3, "the population is the three states");
        assert!(
            lifecycle.is_green(),
            "staged and not-yet-added blobs are lifecycle moments, not defects — \
             refusing here refuses every governed commit"
        );

        let orphan = PinPlane {
            orphaned: vec![OrphanedBlob {
                src_path: "claim.md".to_string(),
                key: "source.md".to_string(),
                blob_sha: "a".repeat(40),
                state: ObjectAnchor::PendingAnchor,
                live: "b".repeat(40),
            }],
            ..lifecycle
        };
        assert!(orphan.is_red(), "the fence must refuse an orphan");
        assert!(
            !orphan.cannot_assess(),
            "and it must not be filed as an absence of evidence"
        );
    }

    /// **Honest degradation, in the direction that matters.** An object store that
    /// cannot be asked leaves the retrieval plane UNREAD — grey, never an empty
    /// `pending_anchor` list a reader could bank as clean. A `cannot_ask` that
    /// reported green would be the false green this whole unit exists to close,
    /// rebuilt one plane over.
    #[test]
    fn an_unaskable_object_store_is_grey_never_an_empty_clean_reading() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", &pin_page("source", &"a".repeat(40)));
        let plane = pin_plane(&root, &docs, &[]);
        assert!(
            plane.cannot_ask.is_some(),
            "there is no repository at /nonexistent — the question could not be put"
        );
        assert!(plane.cannot_assess(), "so the plane is grey");
        assert!(!plane.is_green(), "and it is not clean");
        assert!(
            plane.asked() == 0 && plane.orphaned.is_empty() && !plane.is_red(),
            "the empty counts must NOT be readable as a verdict — `cannot_ask` is \
             what carries the meaning here"
        );
    }

    /// A corpus pinning no blob asks git NOTHING: nothing is
    /// referenced, so nothing can be owed. Without this arm every workspace
    /// outside a repository would refuse, and a guard that blocks everything is
    /// indistinguishable from one that guards nothing (S3-R8(c)).
    #[test]
    fn a_corpus_that_references_no_blob_asks_nothing_and_stays_green() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", "# Claim\n\nno lock at all.\n");
        let plane = pin_plane(&root, &docs, &[]);
        assert!(
            plane.cannot_ask.is_none(),
            "git was never asked, so there is nothing it could not answer"
        );
        assert!(plane.is_green(), "and the reading is a true zero");
    }

    /// **A cross-root pin is SKIPPED AND STATED — it is out of this plane's
    /// jurisdiction, not unknown** (ruling 2026-08-04).
    ///
    /// # This test's subject was REPLACED by a ruling, not repaired
    /// It was `a_rooted_objects_key_refuses_rather_than_asking_the_wrong_store`
    /// and it asserted a REFUSAL. That refusal was correct on its own terms and
    /// unchanged since v1 — but R4 made `hash` mandatory on every pin, so what
    /// used to reach it (cross-root `objects:` rows, which nobody wrote) became
    /// EVERY cross-root pin. Same rule, different population, inverted outcome:
    /// the anchoring plane greyed, the commit-gate greyed, and **the fence
    /// refused every commit in any repo holding one cross-root pin**.
    ///
    /// The ruling scoped the plane to the ambient root explicitly, restoring the
    /// ratified pre-R4 behaviour that `f6_check_sees_the_mount_table`'s
    /// acceptance criterion has always encoded.
    ///
    /// # What is asserted, and why BOTH halves are load-bearing
    /// Skipping alone would be a SILENT narrowing — the exact false clean this
    /// plane exists to prevent. So the population is STATED: *"outside sight is
    /// never verified"* is honoured by naming the sight line, not by pretending
    /// the blob was seen.
    #[test]
    fn a_cross_root_pin_is_skipped_and_stated_never_silently_dropped() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", &pin_page("alpha:source", &"a".repeat(40)));

        let (asked, outside) = objects_in(&docs).expect("a cross-root pin is not an error");
        assert!(
            asked.is_empty(),
            "the ambient store is never asked about another root's blob: {asked:?}"
        );
        assert_eq!(outside.len(), 1, "and the skip is counted, not dropped");
        assert!(
            outside[0].contains("alpha") && outside[0].contains("claim.md"),
            "the statement names the root AND the page that declares it: {outside:?}"
        );

        // The disclosure reaches the PLANE, which is what the faces render.
        let plane = pin_plane(&root, &docs, &[]);
        assert_eq!(
            plane.out_of_jurisdiction.len(),
            1,
            "the plane carries the population the faces must state"
        );
        // And the scoping did NOT buy a false clean by greying: nothing was
        // asked, so nothing is unanswerable.
        assert!(
            plane.cannot_ask.is_none(),
            "out-of-jurisdiction is not the same fact as cannot-ask: {:?}",
            plane.cannot_ask
        );
    }

    /// **The jurisdiction skip keys on STRUCTURE, never on FAILURE** — the
    /// safety property that keeps the exemption from becoming camouflage.
    ///
    /// The discriminator is "is this object's root the ambient one", read off the
    /// address BEFORE any store is asked. Were it behavioural ("did asking
    /// fail"), a genuinely broken AMBIENT store could hide inside the
    /// jurisdiction exemption — a real defect wearing a jurisdiction badge.
    ///
    /// Two arms, and they must DIFFER or the discriminator is a decoration: the
    /// same unaskable `/nonexistent` root, once with an ambient pin and once with
    /// a cross-root pin.
    #[test]
    fn a_broken_ambient_store_still_greys_and_is_not_swallowed_by_the_exemption() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));

        // ARM 1 — AMBIENT pin, unaskable store: greys. Fails CLOSED.
        let ambient = corpus("claim.md", &pin_page("source", &"a".repeat(40)));
        let ambient_plane = pin_plane(&root, &ambient, &[]);
        assert!(
            ambient_plane.cannot_ask.is_some(),
            "a broken AMBIENT store is still grey — the exemption must not cover it"
        );
        assert!(ambient_plane.cannot_assess() && !ambient_plane.is_green());

        // ARM 2 — CROSS-ROOT pin, same unaskable store: scoped out, never asked.
        let cross = corpus("claim.md", &pin_page("alpha:source", &"a".repeat(40)));
        let cross_plane = pin_plane(&root, &cross, &[]);
        assert!(
            cross_plane.cannot_ask.is_none(),
            "the cross-root blob was never this plane's to ask about"
        );

        // The arms differ. That difference IS the discriminator; if these two
        // ever agree, the skip has stopped keying on structure.
        assert_ne!(
            ambient_plane.cannot_ask.is_some(),
            cross_plane.cannot_ask.is_some(),
            "structure decides, not failure"
        );
    }

    /// A value that is not an object id is REPORTED, never skipped: git cannot be
    /// asked about it, so its anchoring is unmeasurable — and a read that dropped
    /// it would report a corrupt retrieval plane as a true zero.
    #[test]
    fn a_value_that_is_not_an_object_id_is_reported_never_skipped() {
        let docs = corpus("claim.md", &pin_page("source", "not-an-oid"));
        let detail = objects_in(&docs).expect_err("git cannot be asked about a non-oid");
        assert!(
            detail.contains("not an object id"),
            "and it says why: {detail}"
        );
    }

    /// A page whose lock REFUSED contributes no objects — exactly as it
    /// contributes none to the listing planes. Its damage is already named by the
    /// grey `lock-refused` PIN row the same page projects, and naming one defect
    /// twice is how a reader comes to believe there are two.
    #[test]
    fn a_refused_lock_contributes_no_objects_because_its_pin_row_already_names_it() {
        // BOTH blocks are valid R4 — so the refusal under test is the
        // two-blocks-on-one-page rule and nothing else. The pre-R4 fixture here
        // was a `version: 1` `objects:` table, which this reader would have
        // refused on the VERSION first: a test that passed for a reason other
        // than its own subject.
        let one = pin_page("source", &"a".repeat(40));
        let block = one
            .split_once("\n\n")
            .expect("the fixture is a heading then the block")
            .1;
        let docs = corpus("claim.md", &format!("# P\n\n{block}\n{block}"));
        // The precondition names WHICH refusal, not merely that one happened.
        // `is_err()` cannot distinguish two-blocks-on-a-page from the version
        // gate or a malformed row, so a fixture that drifted would still satisfy
        // it and the assertion below would then measure a refusal nobody named.
        // The typed variant is stronger than a string: nothing to spell, nothing
        // to drift, and the compiler checks it.
        assert_eq!(
            lock::find(docs.get("claim.md").expect("page")),
            Err(lock::LockError::MultipleBlocks),
            "the fixture is a REFUSED lock, and refused for THIS reason — two \
             blocks on one page"
        );
        assert!(
            objects_in(&docs)
                .expect("a refusal is not this reader's error")
                .0
                .is_empty(),
            "so it declares no askable object here"
        );
    }

    /// A claim whose observation faults propagates the error (fail-loud), never a
    /// false green.
    #[test]
    fn claims_realised_propagates_an_observation_fault() {
        struct Faults;
        impl Check for Faults {
            fn observe(&self, _root: &WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
                Err(realise::CheckError {
                    selector: "faulty".to_string(),
                    reason: "observation faulted".to_string(),
                })
            }
        }
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let claim = Claim {
            selector: "faulty".to_string(),
            rule: None,
            check: Box::new(Faults),
            apply: None,
            retry_budget: 0,
        };
        let err = claims_realised(&root, std::slice::from_ref(&claim))
            .expect_err("a faulting observation is an error, not a drift");
        assert_eq!(err.reason, "observation faulted");
    }
}
