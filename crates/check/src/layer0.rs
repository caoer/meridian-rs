//! Layer 0 — the rule-free core of `check`.
//!
//! Two pack-free reads over the run plane, neither of which writes a byte:
//! - [`claims_realised`] — observe each claim against the current tree and report the
//!   drifted ones (the realise engine's pure detection, run read-only here);
//! - [`pin_plane`] — the CLAIM plane (`pins:` — did the content drift?) and the RETRIEVAL
//!   plane (each pin's `hash` — is the blob durably anchored?).
//!
//! This layer holds no write-history plane by law: the engine keeps no memory —
//! history is pinned to git at lock, and anything between locks is not history.
//! Layer 0 reads at-rest truth only: does the world still match the pins. An
//! out-of-band edit followed by a governed write is therefore outside the engine's
//! domain by design, not a blind spot. Archaeology lives in git; attribution lives
//! in transcript JSONL.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use fs::WorkspaceRoot;
use model::selector::Color;
use realise::{CheckOutcome, Claim};
use receipt::anchor::{ObjectAnchor, ObjectAnchorFacts};

/// The reason word a grey carries, verbatim on BOTH faces — the human render and
/// `--json`. `grey(unmounted)` and `grey(cannot-assess)` are different refusals
/// that share exit 1, so the word is what tells them apart.
pub const GREY_CANNOT_ASSESS: &str = "grey(cannot-assess)";

/// `check` does not assess write history, and both faces say so WITH the reason
/// — a bare "not-assessed" reads as a gap; the reason reads as the design it is
/// (the engine keeps no memory). Stated because the green narrowed, and a silent
/// narrowing would let a reader carry the old, wider green forward.
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
/// Arrives as a fact rather than being computed here: pin colour is `view`'s
/// chain (corpus index → ref resolution → selector → fingerprint compare), and
/// `check` may not take `view`. The caller reads `view::walk::lock_pin_colors`
/// — the same call `mrd status`'s lock axis makes and the same colouring
/// `mrd walk` lists — so the planes agree by construction: one computer, not
/// three that match today.
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
    /// Carried rather than re-derived: the reason words are spelled once, in
    /// `view`.
    pub label: String,
}

/// One pinned blob that is **ORPHANED**: no ref reaches it, and the file it is
/// the blob OF no longer hashes to it — so no commit of that file will ever
/// anchor it either.
///
/// The refusal is this PAIR, not a bare non-durable state: `never-anchored` is
/// the normal state between `mrd pin` and `git add`, `pending-anchor` the
/// normal state at pre-commit time, so refusing on either refuses every
/// governed commit. The three states stay a READING ([`PinPlane::anchored`] /
/// `pending` / `never`); this pair is the finding.
///
/// No new reason word is minted: the finding cites [`ObjectAnchor::word`] for
/// the non-durable state and states the orphaning as evidence beside it.
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
/// The journal plane and the pin plane fail INDEPENDENTLY: a workspace with a
/// clean journal can still hold a pin whose target drifted (the lock arrived by
/// clone or pull while its source moved) or a blob no ref reaches.
#[derive(Debug)]
pub struct PinPlane {
    /// Pins whose colour is RED — the ledger claims content that is no longer
    /// there.
    pub red: Vec<PinRow>,
    /// Pins whose colour is GREY *for want of a measure* — outside sight, each
    /// carrying its own reason word. These GATE: an unread plane is not a clean
    /// one.
    pub grey: Vec<PinRow>,
    /// Pins whose target the hash domain EXCLUDES — `grey(outside-hash-domain)`.
    /// Reported, never gated, and deliberately not in `grey`.
    ///
    /// The split is a difference in kind, not in severity (`wire-contract.md`
    /// §12.1, verdict-plane clause + enumerator clause). `check` asks *does the
    /// world still match the pins* over the ATTESTED surface; an out-of-domain
    /// target is outside that surface by the workspace's own declaration, so
    /// nothing was owed and nothing went unanswered. Folding it into `grey`
    /// refuses the whole workspace over a pin the engine itself minted at rc=0 —
    /// the P0 this field exists to keep fixed. Named in the render regardless:
    /// an enumerator MAY exclude what its attestation cannot reach, never
    /// silently.
    pub unattested: Vec<PinRow>,
    /// Pinned blobs nothing holds and nothing will — the anchoring FINDING.
    /// See [`OrphanedBlob`] for why the refusal is this pair and not a bare
    /// state.
    pub orphaned: Vec<OrphanedBlob>,
    /// The anchoring three-state, as a READING. Counts, because the three
    /// states are the ordinary lifecycle of a pin and not three verdicts. The
    /// counts also carry the population: an empty `orphaned` list means
    /// different things at zero pinned objects and at fifty.
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
    /// How many pins this corpus DECLARES — the claim plane's population.
    ///
    /// [`PinPlane::red`] and [`PinPlane::grey`] list only non-green pins, so an
    /// empty pair means different things over fifty pins and over none — and
    /// the pin plane is the only thing `--commit-gate` reads. Read by
    /// `--require-pins` (`mrd check`) for callers that want "no coverage ⇒
    /// refuse".
    pub declared: usize,
    /// Pins whose blob lives in ANOTHER root's object store — out of this
    /// read's jurisdiction, skipped and STATED.
    ///
    /// The anchoring plane holds one git handle by design: the ambient root's.
    /// A cross-root pin's blob is not measured here; "outside sight is never
    /// verified" is honoured by stating the sight line, and both faces name
    /// this population. Only blob-ANCHORING scopes — the claim half stays
    /// cross-root.
    pub out_of_jurisdiction: Vec<String>,
}

impl PinPlane {
    /// The pin plane found a finding: a red pin, or an ORPHANED blob.
    ///
    /// An orphan rides the finding leg, not the grey one: git was asked and
    /// replied, so it is an answer, not the absence of one. The bare three
    /// anchoring states are not findings — refusing on a non-durable one
    /// refuses every governed commit.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.red.is_empty() || !self.orphaned.is_empty()
    }

    /// How many pinned blobs git was asked about — the population behind the
    /// three-state reading.
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

    /// Every grey row the plane holds, gating and non-gating alike — the render's
    /// list, never the gate's. Order is stable: gating greys first.
    pub fn every_grey(&self) -> impl Iterator<Item = &PinRow> {
        self.grey.iter().chain(&self.unattested)
    }

    /// Nothing to report and everything asked — the assessed green. Distinct from
    /// "no pins in the corpus" only in that both are true readings; a corpus that
    /// declares no pins genuinely owes nothing.
    #[must_use]
    pub fn is_green(&self) -> bool {
        !self.is_red() && !self.cannot_assess()
    }
}

/// Read the pin plane over `docs`: sort the caller's pin colours, then ask
/// each object store the corpus pins into — THIS root's for bare objects, and
/// every MOUNTED root's for the form-3 objects that name it (cross-root
/// design D-F: one rev-list + cat-file pair per root, only when that root has
/// rows).
///
/// `docs` must be the SAME corpus build the pin colours came from, or the two
/// halves describe two different corpora. `mounted` maps each bound root name
/// to its canonical path; a form-3 object whose root is NOT in the map is
/// skipped and STATED ([`PinPlane::out_of_jurisdiction`]) — unmounted is a
/// sight line, never a silent drop.
///
/// A corpus that pins no blob asks git nothing, so a workspace outside a
/// repository stays green rather than being refused for a question nobody
/// asked. Every unanswerable question is REPORTED, never skipped: a non-oid
/// value, an unreachable store, git absent, no repository — all leave the
/// retrieval plane UNREAD ([`PinPlane::cannot_ask`]), never falsely clean.
#[must_use]
pub fn pin_plane(
    root: &WorkspaceRoot,
    docs: &model::Docs,
    pins: &[PinRow],
    mounted: &BTreeMap<addr::MountName, PathBuf>,
) -> PinPlane {
    let mut plane = PinPlane {
        red: Vec::new(),
        grey: Vec::new(),
        unattested: Vec::new(),
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
            // The one grey that is a policy statement rather than an absence of
            // measure — sorted out here so the gate never sees it.
            Color::Grey(model::selector::GreyReason::OutsideHashDomain { .. }) => {
                plane.unattested.push(pin.clone());
            }
            Color::Grey(_) => plane.grey.push(pin.clone()),
            Color::Green => {}
        }
    }
    match objects_in(docs, mounted) {
        Err(detail) => plane.cannot_ask = Some(detail),
        Ok(grouped) => {
            plane.out_of_jurisdiction = grouped.unmounted;
            if !grouped.ambient.is_empty()
                && let Err(detail) = ask_store(&root.0, &grouped.ambient, &mut plane)
            {
                plane.cannot_ask = Some(detail);
            }
            for (name, objects) in &grouped.foreign {
                let store = &mounted[name];
                if let Err(detail) = ask_store(store, objects, &mut plane) {
                    // One unaskable store leaves the PLANE unread (grey,
                    // never falsely clean), and the refusal names WHICH
                    // root could not answer.
                    plane.cannot_ask = Some(format!("mounted root `{name}`: {detail}"));
                }
            }
        }
    }
    plane
}

/// One pinned blob located in the corpus — the R4 per-pin `hash` and the target
/// it is the blob OF. `key` is the lock object spelling verbatim (rooted for a
/// form-3 row); `rel` is the path half alone — what the live re-hash joins
/// onto the owning root.
#[derive(Debug)]
struct ObjectRef {
    src_path: String,
    key: String,
    rel: String,
    blob_sha: String,
}

/// The corpus's pinned blobs, grouped by the store that must answer for each.
#[derive(Debug, Default)]
struct GroupedObjects {
    /// Bare objects — THIS root's store answers.
    ambient: Vec<ObjectRef>,
    /// Form-3 objects whose root is MOUNTED — that root's store answers.
    foreign: BTreeMap<addr::MountName, Vec<ObjectRef>>,
    /// Form-3 objects whose root is NOT mounted here — skipped and stated.
    unmounted: Vec<String>,
}

/// Every pinned blob the corpus declares, or the reason the plane is unread.
///
/// [`lock::find`] is the parser — the same owner the listing surfaces use. A
/// page whose lock REFUSED contributes nothing: its damage is already named by
/// the grey `lock-refused` pin row the same page projects, and naming one
/// defect twice is how a reader comes to believe there are two.
///
/// The whole-file lock (`path: []` and `properties: []` on one object)
/// declares the same blob twice, so rows are deduped by `(object, hash)` —
/// two rows would ask git one question twice and report one orphan as two
/// findings.
fn objects_in(
    docs: &model::Docs,
    mounted: &BTreeMap<addr::MountName, PathBuf>,
) -> Result<GroupedObjects, String> {
    let mut out = GroupedObjects::default();
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (path, doc) in docs {
        let Ok(Some(found)) = lock::find(doc) else {
            continue;
        };
        for pin in found.lock.pins {
            let key = pin.object;
            // Jurisdiction is decided on STRUCTURE (which root does this
            // object's address name), never on failure — a behavioural skip
            // would let a broken store hide inside the exemption. A form-3
            // object routes to its own root's store when that root is
            // MOUNTED; an unmounted root's rows are skipped and STATED.
            let owner = match addr::Addr::parse(&key) {
                Ok(addr) => match addr.root() {
                    Some(name) if mounted.contains_key(name) => {
                        Some((name.clone(), addr.path().to_owned()))
                    }
                    Some(name) => {
                        out.unmounted
                            .push(format!("`{path}` pin `{key}` (root `{name}`, unmounted)"));
                        continue;
                    }
                    None => None,
                },
                Err(e) => {
                    return Err(format!(
                        "`{path}` pin `{key}` is not an address, so WHICH git to ask is \
                         unknown: {e}"
                    ));
                }
            };
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
            match owner {
                Some((name, rel)) => out.foreign.entry(name).or_default().push(ObjectRef {
                    src_path: path.clone(),
                    key: key.clone(),
                    rel,
                    blob_sha: oid,
                }),
                None => out.ambient.push(ObjectRef {
                    src_path: path.clone(),
                    rel: key.clone(),
                    key,
                    blob_sha: oid,
                }),
            }
        }
    }
    Ok(out)
}

/// Ask ONE object store about every entry, in ONE pass, and classify.
///
/// Two git calls at most, never a call per blob: one `git rev-list --objects
/// --all` for the reachable set and one batched `git cat-file --batch-check`
/// for presence. [`ObjectAnchor::classify`] reads the pair — the same
/// classifier `mrd status`'s gauge runs. Both facts come from THIS handle in
/// THIS pass: a fact pair split across two stores is a wrong answer, not a
/// stale one.
///
/// All three states are counted as a READING; only the ORPHAN is a finding.
/// The orphan check costs one `git hash-object` per non-anchored entry only.
fn ask_store(
    store_root: &std::path::Path,
    objects: &[ObjectRef],
    plane: &mut PinPlane,
) -> Result<(), String> {
    let repo = git::Repo::at(store_root.to_path_buf());
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
        // If the file still hashes to this blob, committing it anchors it — a
        // lifecycle moment, not a defect. `rel` is the object's path half
        // without `.md`, so the extension goes back on before git is asked —
        // joined onto the store that owns the row, never the ambient root.
        let live = match repo.blob_oid(&store_root.join(format!("{}.md", object.rel))) {
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
            card_template: None,
        }
    }

    /// A drifted claim surfaces as a [`ClaimFinding`]; a converged one does not.
    #[test]
    fn claims_realised_reports_only_drifted_claims() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
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
    fn corpus(path: &str, text: &str) -> model::Docs {
        let doc = model::build(text.to_string(), syntax::parse(text));
        BTreeMap::from([(path.to_string(), std::sync::Arc::new(doc))])
    }

    /// A `meridian-lock` block carrying one R4 pin, in the canonical bytes the
    /// engine mints — rendered through [`lock::render`], never hand-written.
    /// `object` is the wiki-link inner text — the target's vault path without
    /// `.md`.
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

    /// The grey bucket splits by KIND, and the split decides whether `check`
    /// refuses the whole workspace.
    ///
    /// `grey(outside-hash-domain)` is a policy statement about the attested
    /// surface, so it is NAMED and gates nothing; every other grey is an absence
    /// of measure and still gates. Both halves are asserted here — a gate that
    /// only checked the new arm would pass a change that stopped refusing on
    /// blindness too, which is the fail-open this split must never become.
    #[test]
    fn the_domain_grey_is_reported_while_a_blindness_grey_still_gates() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let docs = BTreeMap::new();

        let unattested = pin_plane(
            &root,
            &docs,
            &[pin(Color::Grey(
                model::selector::GreyReason::OutsideHashDomain {
                    path: "drafts/tmp.md".to_string(),
                },
            ))],
            &BTreeMap::new(),
        );
        assert!(
            unattested.grey.is_empty() && unattested.unattested.len() == 1,
            "the domain grey is sorted out of the gating bucket"
        );
        assert!(
            !unattested.cannot_assess(),
            "nothing went unanswered: the workspace declared this surface unattested"
        );
        assert!(
            !unattested.is_red() && unattested.is_green(),
            "so `check` must not refuse the workspace over a pin the engine minted at rc=0"
        );
        assert_eq!(
            unattested.every_grey().count(),
            1,
            "and it is still NAMED — an enumerator excludes, never silently"
        );

        let blind = pin_plane(
            &root,
            &docs,
            &[pin(Color::Grey(model::selector::GreyReason::Ambiguous))],
            &BTreeMap::new(),
        );
        assert!(
            blind.cannot_assess() && !blind.is_green(),
            "a grey for want of a measure still gates — an unread plane is not a clean one"
        );
    }

    /// The sort is load-bearing in both directions: a red pin is a FINDING, a
    /// grey pin a refusal-for-want-of-evidence.
    #[test]
    fn a_red_pin_is_a_finding_and_a_grey_pin_is_an_absence_of_evidence() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let docs = BTreeMap::new();

        let red = pin_plane(
            &root,
            &docs,
            &[pin(Color::Red(model::selector::RedReason::Drifted))],
            &BTreeMap::new(),
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
            &BTreeMap::new(),
        );
        assert!(!grey.is_red(), "grey is not red");
        assert!(grey.cannot_assess(), "grey names itself");
        assert!(!grey.is_green(), "and it is never green");

        let green = pin_plane(&root, &docs, &[pin(Color::Green)], &BTreeMap::new());
        assert!(green.is_green(), "a green pin leaves nothing to report");
    }

    /// An ORPHANED blob rides the FINDING leg; the bare three states are a
    /// reading — a plane with `pending` and `never` counts and no orphan is
    /// green.
    #[test]
    fn an_orphaned_blob_is_a_finding_while_the_bare_three_states_are_a_reading() {
        let lifecycle = PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            unattested: Vec::new(),
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

    /// An object store that cannot be asked leaves the retrieval plane UNREAD —
    /// grey, never an empty list a reader could bank as clean.
    #[test]
    fn an_unaskable_object_store_is_grey_never_an_empty_clean_reading() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", &pin_page("source", &"a".repeat(40)));
        let plane = pin_plane(&root, &docs, &[], &BTreeMap::new());
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

    /// A corpus pinning no blob asks git nothing and stays green.
    #[test]
    fn a_corpus_that_references_no_blob_asks_nothing_and_stays_green() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", "# Claim\n\nno lock at all.\n");
        let plane = pin_plane(&root, &docs, &[], &BTreeMap::new());
        assert!(
            plane.cannot_ask.is_none(),
            "git was never asked, so there is nothing it could not answer"
        );
        assert!(plane.is_green(), "and the reading is a true zero");
    }

    /// A cross-root pin is SKIPPED AND STATED — out of this plane's
    /// jurisdiction, not unknown. Skipping alone would be a silent narrowing,
    /// so the population is stated.
    #[test]
    fn a_cross_root_pin_is_skipped_and_stated_never_silently_dropped() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", &pin_page("alpha:source", &"a".repeat(40)));

        let grouped =
            objects_in(&docs, &BTreeMap::new()).expect("a cross-root pin is not an error");
        assert!(
            grouped.ambient.is_empty(),
            "the ambient store is never asked about another root's blob: {:?}",
            grouped.ambient
        );
        assert!(
            grouped.foreign.is_empty(),
            "an UNMOUNTED root has no store to route to: {:?}",
            grouped.foreign.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            grouped.unmounted.len(),
            1,
            "and the skip is counted, not dropped"
        );
        assert!(
            grouped.unmounted[0].contains("alpha") && grouped.unmounted[0].contains("claim.md"),
            "the statement names the root AND the page that declares it: {:?}",
            grouped.unmounted
        );

        // The disclosure reaches the PLANE, which is what the faces render.
        let plane = pin_plane(&root, &docs, &[], &BTreeMap::new());
        assert_eq!(
            plane.out_of_jurisdiction.len(),
            1,
            "the plane carries the population the faces must state"
        );
        // The scoping did not buy a false clean: nothing was asked.
        assert!(
            plane.cannot_ask.is_none(),
            "out-of-jurisdiction is not the same fact as cannot-ask: {:?}",
            plane.cannot_ask
        );
    }

    /// The jurisdiction skip keys on STRUCTURE, never on FAILURE: a broken
    /// ambient store must still grey. Two arms over the same unaskable root —
    /// once ambient, once cross-root — and they must differ.
    #[test]
    fn a_broken_ambient_store_still_greys_and_is_not_swallowed_by_the_exemption() {
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));

        // ARM 1 — AMBIENT pin, unaskable store: greys. Fails CLOSED.
        let ambient = corpus("claim.md", &pin_page("source", &"a".repeat(40)));
        let ambient_plane = pin_plane(&root, &ambient, &[], &BTreeMap::new());
        assert!(
            ambient_plane.cannot_ask.is_some(),
            "a broken AMBIENT store is still grey — the exemption must not cover it"
        );
        assert!(ambient_plane.cannot_assess() && !ambient_plane.is_green());

        // ARM 2 — CROSS-ROOT pin, same unaskable store: scoped out, never asked.
        let cross = corpus("claim.md", &pin_page("alpha:source", &"a".repeat(40)));
        let cross_plane = pin_plane(&root, &cross, &[], &BTreeMap::new());
        assert!(
            cross_plane.cannot_ask.is_none(),
            "the cross-root blob was never this plane's to ask about"
        );

        // The arms differ — that difference IS the discriminator.
        assert_ne!(
            ambient_plane.cannot_ask.is_some(),
            cross_plane.cannot_ask.is_some(),
            "structure decides, not failure"
        );
    }

    /// D-F — a form-3 object whose root is MOUNTED is ASKED of that root's own
    /// store: anchored counts, no `out_of_jurisdiction` row. Before this, no
    /// plane anywhere answered durability for a cross-root blob.
    #[test]
    fn a_mounted_roots_form_3_blob_is_asked_of_that_roots_own_store() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alpha = tmp.path().join("alpha");
        std::fs::create_dir_all(&alpha).expect("mkdir");
        std::fs::write(alpha.join("source.md"), "# S\n\ncontent.\n").expect("target");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "df@example.invalid"],
            vec!["config", "user.name", "df"],
            vec!["add", "."],
            vec!["commit", "-q", "-m", "seed"],
        ] {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&alpha)
                .args(&args)
                .status()
                .expect("git runs")
                .success();
            assert!(ok, "git {args:?}");
        }
        let oid = git::Repo::at(alpha.clone())
            .blob_oid(&alpha.join("source.md"))
            .expect("the committed file has a blob oid");

        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", &pin_page("alpha:source", &oid));
        let mounted = BTreeMap::from([(
            addr::MountName::parse("alpha").expect("a name"),
            alpha.clone(),
        )]);

        let plane = pin_plane(&root, &docs, &[], &mounted);
        assert!(
            plane.out_of_jurisdiction.is_empty(),
            "a MOUNTED root's rows are asked, not stated as outside: {:?}",
            plane.out_of_jurisdiction
        );
        assert_eq!(plane.anchored, 1, "the committed blob reads ANCHORED");
        assert!(
            plane.cannot_ask.is_none() && plane.orphaned.is_empty(),
            "one committed cross-root blob is a clean reading"
        );

        // The counter-arm: the same corpus with the root NOT mounted stays a
        // stated skip — mounting is what converts the sight line into a read.
        let unmounted = pin_plane(&root, &docs, &[], &BTreeMap::new());
        assert_eq!(unmounted.out_of_jurisdiction.len(), 1);
        assert_eq!(unmounted.asked(), 0, "nothing was asked without the mount");

        // And an unaskable MOUNTED store greys the plane NAMING the root —
        // never a silent skip, never a false clean.
        let broken = BTreeMap::from([(
            addr::MountName::parse("alpha").expect("a name"),
            PathBuf::from("/nonexistent-store"),
        )]);
        let grey = pin_plane(&root, &docs, &[], &broken);
        assert!(
            grey.cannot_ask
                .as_deref()
                .is_some_and(|d| d.contains("alpha")),
            "the refusal names WHICH root could not answer: {:?}",
            grey.cannot_ask
        );
    }

    /// A value that is not an object id is REPORTED, never skipped — a read
    /// that dropped it would report a corrupt retrieval plane as a true zero.
    #[test]
    fn a_value_that_is_not_an_object_id_is_reported_never_skipped() {
        let docs = corpus("claim.md", &pin_page("source", "not-an-oid"));
        let detail =
            objects_in(&docs, &BTreeMap::new()).expect_err("git cannot be asked about a non-oid");
        assert!(
            detail.contains("not an object id"),
            "and it says why: {detail}"
        );
    }

    /// A page whose lock REFUSED contributes no objects — its damage is already
    /// named by the grey `lock-refused` pin row the same page projects.
    #[test]
    fn a_refused_lock_contributes_no_objects_because_its_pin_row_already_names_it() {
        // Both blocks are valid R4, so the refusal under test is the
        // two-blocks-on-one-page rule and nothing else.
        let one = pin_page("source", &"a".repeat(40));
        let block = one
            .split_once("\n\n")
            .expect("the fixture is a heading then the block")
            .1;
        let docs = corpus("claim.md", &format!("# P\n\n{block}\n{block}"));
        // The precondition names WHICH refusal: `is_err()` could not
        // distinguish this from the version gate or a malformed row.
        assert_eq!(
            lock::find(docs.get("claim.md").expect("page")),
            Err(lock::LockError::MultipleBlocks),
            "the fixture is a REFUSED lock, and refused for THIS reason — two \
             blocks on one page"
        );
        assert!(
            objects_in(&docs, &BTreeMap::new())
                .expect("a refusal is not this reader's error")
                .ambient
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
        let root = WorkspaceRoot(PathBuf::from("/nonexistent"));
        let claim = Claim {
            selector: "faulty".to_string(),
            rule: None,
            check: Box::new(Faults),
            apply: None,
            retry_budget: 0,
            card_template: None,
        };
        let err = claims_realised(&root, std::slice::from_ref(&claim))
            .expect_err("a faulting observation is an error, not a drift");
        assert_eq!(err.reason, "observation faulted");
    }
}
