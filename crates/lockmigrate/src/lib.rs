//! **SELF-RETIRING (U9b, plan decision P4).** The field migration of
//! `meridian-lock` blocks from the v1 shape to R4 v2.
//!
//! # Why this crate exists at all
//!
//! `crates/lock` reads **v2 only** and fails loud on v1. Landing that crate
//! without this tool and an EXECUTED sweep would lock every vault in the field
//! out of its own locks for an unbounded window — the finding that made the
//! lock work a merge train instead of a sequence of independent merges. So the
//! crate, the [governed door](wire_serve::write::lock_migrate), this tool, and
//! the sweep land together.
//!
//! The v1 grammar is spelled in exactly one place, [`v1`], and it is spelled
//! HERE rather than in the reader so that no engine reader can drift back into
//! interpreting an old shape as a new one.
//!
//! # Three disciplines, and one that is new
//!
//! - **dry-run first** — [`Options::dry`] reports every rewrite and writes
//!   nothing. Assert it on a fixture before believing it on a vault.
//! - **idempotent** — a page whose lock is already v2 is not a target, so a
//!   second run over a swept vault rewrites nothing and says so with a count.
//! - **resumable** — each page migrates independently under its own
//!   write-what-you-read CAS, so an interrupted run resumes by re-running: the
//!   done pages are v2 and skip, the remainder writes.
//! - **discriminating** — and this one is not optional bookkeeping. See below.
//!
//! # The discrimination rule, and why a blind sweep is wrong
//!
//! A v1 lock block in a file is not evidence that the engine wrote it. Measured
//! on ZT's live vaults before any code was written: **19 v1 blocks across 9
//! files, of which only 2 are engine-minted page locks.** The other 17 are
//! *illustrations* — a decision record showing the schema it ratified, a design
//! doc's "the current block, for the record", and ZT's own verbatim session
//! traces. Those pages are immutable source material. A sweep that rewrote them
//! would corrupt the historical record while reporting success.
//!
//! So a page is a target only when it passes the engine's OWN laws, and each
//! test is one the engine already states somewhere else:
//!
//! 1. **The LAST block is the page's terminal content** — the placement law
//!    (`wire_serve::write::lock_write` § Placement law) births a lock at EOF.
//!    Prose after the closing fence means a human wrote the block into the
//!    middle of a document, which the engine never does.
//! 2. **There is exactly one block** — sole-writer (#8 §3) mints one;
//!    `lock::find` calls two `MultipleBlocks` corruption. Two or more is
//!    reported and REFUSED, never guessed through.
//! 3. **It parses as v1** — via the quarantined [`v1`] reader.
//!
//! **Placement is tested before arity, and the order is the correctness.**
//! Placement answers the prior question — does this page carry an engine-minted
//! lock at all — and arity then guards the lock it found. Reversed, a document
//! illustrating the schema six times is called `MultipleBlocks` corruption and
//! REFUSED forever for being documentation. That is not hypothetical: the two
//! verbatim ZT session traces in the live corpus have exactly that shape.
//!
//! A page carrying a v1 block that fails (1) is reported as
//! [`PageVerdict::NotEnginePlaced`] and left alone. **That list is for human
//! eyes and the runbook says so** — it is the one place this tool's judgment
//! could be wrong in the safe direction, so it is printed rather than hidden.
//!
//! # The expected drift, named IN ADVANCE (security finding S7)
//!
//! Lock-is-content (#8 §5): the block sits inside the page's own span, so the
//! page's fingerprint covers its lock. Rewriting the block therefore MOVES the
//! page fingerprint — every time, by construction. Any full-body (`path: []`)
//! pin naming a rewritten page drifts once, and [`MigrationReport::expected_drift`]
//! enumerates exactly those rows before the sweep runs. A drift wave nobody
//! predicted reads as corruption; a predicted one is a migration working.

pub mod v1;

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Options for a migration run.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Report every rewrite and write nothing (dry-run-first).
    pub dry: bool,
    /// The recorded actor (§9: recorded exactly as given, never invented).
    pub actor: Option<String>,
    /// The recorded timestamp (§9: recorded exactly as given, never invented).
    pub now: Option<String>,
    /// **The §5.1 WORLD GUARD, armed** (`mrd lock migrate --expect-root`).
    ///
    /// The ambient Merkle root the operator states the vault is at. Every
    /// per-page rewrite carries it, and the door REFUSES with `root_mismatch`
    /// if the vault's ambient root is anything else.
    ///
    /// # Why a sweep needs it and a single write does not
    /// A sweep is many writes over one quiesced vault. The per-page CAS already
    /// proves each PAGE did not move, but it cannot notice that the VAULT is
    /// not the world the operator inspected — a dry run read on one tree and a
    /// real run landing on another passes every per-page check, because each
    /// page is individually consistent. This is the guard that makes the
    /// operator's *"I looked at this vault"* mean the vault they looked at.
    ///
    /// `None` leaves it UNARMED, which is what this tool shipped with (U9b) and
    /// what the runbook wrongly described as armed — the flag exists so the
    /// claim and the code agree.
    pub expect_root: Option<wire::Root>,
}

/// A tool failure — the migration could not run to a verdict. Distinct from a
/// per-page refusal, which IS a reported outcome (the run completed).
#[derive(Debug)]
pub enum SweepError {
    /// **The vault is not a git repository, so no restore point can exist.**
    ///
    /// The sweep's only undo is a pre-sweep commit IN THE VAULT — reverting
    /// meridian-rs restores nothing, because the bytes this changes are the
    /// vault's, not the engine's. A vault with no git is therefore refused
    /// before anything moves, and the runbook says ASK rather than proceed.
    NotAGitRepo { vault: String, detail: String },
    /// The vault could not be read or parsed.
    Corpus(String),
    /// The governed door refused or failed for `path`.
    Door { path: String, detail: String },
}

impl std::fmt::Display for SweepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SweepError::NotAGitRepo { vault, detail } => write!(
                f,
                "REFUSED: `{vault}` is not a git repository ({detail}), so the sweep has \
                 no restore point. The only undo for a lock rewrite is a pre-sweep commit \
                 in the vault itself. Put the vault under git, or ask before proceeding."
            ),
            SweepError::Corpus(m) => write!(f, "cannot read the vault: {m}"),
            SweepError::Door { path, detail } => {
                write!(f, "the lock-migrate door refused for {path}: {detail}")
            }
        }
    }
}

impl std::error::Error for SweepError {}

/// Why one v1 pin row could not be expressed in R4 v2.
///
/// **There is no unconvertible CLASS of row** — anchor rows convert cleanly
/// (`ref: "page#^claim"` → `path: ["^claim"]`, sole `^id` element, block
/// grain), and free-form keys ride along verbatim. Every variant here is a
/// DAMAGED row: bytes that were already outside the v1 contract when the sweep
/// found them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// The `ref` is not a well-formed agent-plane address.
    BadRef { line: usize, found: String },
    /// The `ref` carries an `@fp` render-face decoration. That token is minted
    /// on READ and never stored, so its presence in a file means these bytes did
    /// not come from the engine.
    RefCarriesFp { line: usize, found: String },
    /// No `objects:` entry records the blob sha for this pin's target. R4 puts
    /// the hash ON the row and never omits it ("if hash is missing, we lost the
    /// explicit target meaning"), so there is nothing to synthesize.
    MissingBlob { line: usize, target: String },
    /// A free-form key on the row shadows an R4 reserved field, so carrying it
    /// verbatim would forge a different pin.
    ExtraShadowsReserved { line: usize, key: String },
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::BadRef { line, found } => {
                write!(f, "line {line}: `{found}` is not a well-formed address")
            }
            ConvertError::RefCarriesFp { line, found } => write!(
                f,
                "line {line}: `{found}` carries an `@fp` decoration — that token is \
                 minted on read and never stored"
            ),
            ConvertError::MissingBlob { line, target } => write!(
                f,
                "line {line}: no `objects:` entry records a blob sha for `{target}` — \
                 R4 never omits the hash and this tool never invents one"
            ),
            ConvertError::ExtraShadowsReserved { line, key } => write!(
                f,
                "line {line}: the free-form key `{key}` shadows an R4 reserved field"
            ),
        }
    }
}

impl std::error::Error for ConvertError {}

/// **Convert a parsed v1 lock into the R4 v2 value.**
///
/// The whole schema change, in one function:
///
/// - the `objects:` retrieval plane DISSOLVES — each pin's blob sha moves onto
///   its own row, where it cannot outlive the claim it was written for;
/// - `ref: "root:page.md#A/B"` splits into `object: "[[root:page.md]]"` plus
///   `path: ["A", "B"]` — the address owner does the split, so the migration
///   never re-derives the grammar;
/// - a ref with **no** selector becomes `path: []`, the whole body;
/// - an ANCHOR selector becomes the sole element: `#^claim` → `path: ["^claim"]`;
/// - every unknown key rides across **byte-identically**.
///
/// # The one lossy edge, stated rather than hidden
/// The v1 `ref` joined selector segments with `/`, so a heading whose own text
/// contained a `/` was already ambiguous ON DISK before this tool existed — the
/// design record calls it "the lossy sanitized join". Splitting on `/` is the
/// only available inverse. The fingerprint rides across verbatim, so such a row
/// migrates to a claim that still names its own recorded fingerprint; the U22
/// repair that follows the sweep is what re-derives it.
///
/// # Errors
/// [`ConvertError`] naming the damaged row.
pub fn convert(v1: &v1::LockV1) -> Result<lock::Lock, ConvertError> {
    let mut out = lock::Lock::new();
    for pin in &v1.pins {
        let parsed = addr::Addr::parse(&pin.declared_ref).map_err(|_| ConvertError::BadRef {
            line: pin.line,
            found: pin.declared_ref.clone(),
        })?;
        if parsed.fp().is_some() {
            return Err(ConvertError::RefCarriesFp {
                line: pin.line,
                found: pin.declared_ref.clone(),
            });
        }
        let target = parsed.target();
        let hash = v1
            .blob_of(&target)
            .ok_or_else(|| ConvertError::MissingBlob {
                line: pin.line,
                target: target.clone(),
            })?;

        // v1 addressed the BODY and had no frontmatter arm at all, so every
        // converted row lands on `path`. `properties` pins are v2-native and
        // cannot appear here.
        let selector = match parsed.selector() {
            "" => lock::Selector::Path(Vec::new()),
            sel => lock::Selector::Path(sel.split('/').map(str::to_string).collect()),
        };

        let mut entry = lock::PinEntry::new(&target, hash, selector, &pin.fingerprint);
        for (key, value) in &pin.extra {
            if lock::PinEntry::RESERVED_KEYS.contains(&key.as_str()) {
                return Err(ConvertError::ExtraShadowsReserved {
                    line: pin.line,
                    key: key.clone(),
                });
            }
            entry.extra.insert(key.clone(), value.clone());
        }
        out.upsert_pin(entry);
    }
    Ok(out)
}

/// What the sweep did — or refused to do — at one page.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum PageVerdict {
    /// The page's v1 lock became a v2 lock through the governed door. On a dry
    /// run the revs are still computed — they are facts about the spec, not the
    /// disk — and nothing was written.
    Migrated {
        path: String,
        /// The page's whole-file rev before. **`rev_after` always differs**:
        /// lock-is-content, so rewriting the block moves the page (S7).
        rev_before: String,
        rev_after: String,
        /// How many pin rows crossed.
        pins: usize,
        /// Unknown legacy keys carried verbatim, as `pin-index:key`.
        carried_keys: Vec<String>,
    },
    /// The page's lock is already v2 — the idempotence arm. Out of scope, and
    /// what makes a second run a no-op.
    AlreadyV2 { path: String },
    /// A v1 block that the ENGINE did not place: content follows its closing
    /// fence, so it is an illustration inside a document, not a page lock.
    /// **Left alone, and printed for human eyes** (see the module docs).
    NotEnginePlaced {
        path: String,
        /// How many `meridian-lock` blocks the page carries, all told.
        blocks: usize,
    },
    /// Two or more lock blocks where sole-writer mints one. Corruption by the
    /// engine's own reading; refused, never guessed through.
    MultipleBlocks { path: String, blocks: usize },
    /// A v1 block whose bytes are outside the v1 contract.
    Unparseable { path: String, detail: String },
    /// A v1 block that parsed but carries a damaged row.
    Unconvertible { path: String, detail: String },
}

impl PageVerdict {
    /// The page this verdict is about.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            PageVerdict::Migrated { path, .. }
            | PageVerdict::AlreadyV2 { path }
            | PageVerdict::NotEnginePlaced { path, .. }
            | PageVerdict::MultipleBlocks { path, .. }
            | PageVerdict::Unparseable { path, .. }
            | PageVerdict::Unconvertible { path, .. } => path,
        }
    }

    /// Is this a REFUSAL — a page the sweep could not complete? These are what
    /// make the gate refuse completion; `NotEnginePlaced` is not one of them
    /// (it is deliberately out of scope, not damaged).
    #[must_use]
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            PageVerdict::MultipleBlocks { .. }
                | PageVerdict::Unparseable { .. }
                | PageVerdict::Unconvertible { .. }
        )
    }
}

/// One predicted fingerprint drift (S7): a full-body pin that names a page
/// whose lock block this sweep rewrites.
#[derive(Debug, Clone, Serialize)]
pub struct DriftRow {
    /// The page whose lock carries the pin that will drift.
    pub pinning_page: String,
    /// The pinned object — a page this sweep rewrote.
    pub object: String,
    /// The fingerprint recorded today, which the rewrite invalidates.
    pub stale_fingerprint: String,
}

/// The report of a migration run over one vault.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    /// The vault this ran over.
    pub vault: String,
    /// Nothing was written.
    pub dry: bool,
    /// Per-page verdicts, in corpus (path-sorted) order.
    pub pages: Vec<PageVerdict>,
    /// **The expected, one-time drift, named in advance** (S7).
    pub expected_drift: Vec<DriftRow>,
}

impl MigrationReport {
    /// Pages migrated (or, on a dry run, that would be).
    #[must_use]
    pub fn migrated(&self) -> usize {
        self.count(|p| matches!(p, PageVerdict::Migrated { .. }))
    }

    /// Pages already on v2 — the idempotence count.
    #[must_use]
    pub fn already_v2(&self) -> usize {
        self.count(|p| matches!(p, PageVerdict::AlreadyV2 { .. }))
    }

    /// Pages carrying a v1 block the engine did not place — for human review.
    #[must_use]
    pub fn not_engine_placed(&self) -> usize {
        self.count(|p| matches!(p, PageVerdict::NotEnginePlaced { .. }))
    }

    /// Refusals. **Non-zero means the migration is NOT complete** and the gate
    /// must not pass.
    #[must_use]
    pub fn refusals(&self) -> usize {
        self.count(PageVerdict::is_refusal)
    }

    fn count(&self, f: impl Fn(&PageVerdict) -> bool) -> usize {
        self.pages.iter().filter(|p| f(p)).count()
    }

    /// A human-readable report — the artifact the runbook files.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let mode = if self.dry {
            "DRY RUN — nothing written"
        } else {
            "EXECUTED"
        };
        let _ = write!(s, "# lock v1→v2 migration — {}\n\n{mode}\n\n", self.vault);
        let _ = write!(
            s,
            "migrated: {} · already v2: {} · not engine-placed: {} · refusals: {}\n\n",
            self.migrated(),
            self.already_v2(),
            self.not_engine_placed(),
            self.refusals()
        );
        section(&mut s, "Rewritten pages", self.rewritten_rows());
        section(
            &mut s,
            "EXPECTED fingerprint drift (S7 — one-time, not corruption)",
            self.drift_rows(),
        );
        section(
            &mut s,
            "NOT engine-placed — LEFT ALONE, review by hand",
            self.excluded_rows(),
        );
        section(
            &mut s,
            "REFUSED — the migration is not complete while these stand",
            self.refusal_rows(),
        );
        s
    }

    fn rewritten_rows(&self) -> Vec<String> {
        self.pages
            .iter()
            .filter_map(|p| match p {
                PageVerdict::Migrated {
                    path,
                    rev_before,
                    rev_after,
                    pins,
                    carried_keys,
                } => {
                    let keys = if carried_keys.is_empty() {
                        String::new()
                    } else {
                        format!(" · carried verbatim: {}", carried_keys.join(", "))
                    };
                    Some(format!(
                        "`{path}` — {pins} pin(s), rev {rev_before} → {rev_after}{keys}"
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn drift_rows(&self) -> Vec<String> {
        self.expected_drift
            .iter()
            .map(|d| {
                format!(
                    "`{}` pins `{}` full-body; recorded `{}` goes stale when the \
                     lock block is rewritten. Lock-is-content (#8 §5): the page's \
                     fingerprint covers its own lock. U22 repair re-derives it.",
                    d.pinning_page, d.object, d.stale_fingerprint
                )
            })
            .collect()
    }

    fn excluded_rows(&self) -> Vec<String> {
        self.pages
            .iter()
            .filter_map(|p| match p {
                PageVerdict::NotEnginePlaced { path, blocks } => Some(format!(
                    "`{path}` — {blocks} lock block(s), content follows the closing \
                     fence, so this is an illustration inside a document, not a page lock"
                )),
                _ => None,
            })
            .collect()
    }

    fn refusal_rows(&self) -> Vec<String> {
        self.pages
            .iter()
            .filter(|p| p.is_refusal())
            .map(|p| match p {
                PageVerdict::MultipleBlocks { path, blocks } => {
                    format!("`{path}` — {blocks} lock blocks; sole-writer mints one")
                }
                PageVerdict::Unparseable { path, detail } => {
                    format!("`{path}` — unparseable v1: {detail}")
                }
                PageVerdict::Unconvertible { path, detail } => format!("`{path}` — {detail}"),
                _ => unreachable!("filtered to refusals"),
            })
            .collect()
    }
}

/// Append a `## title` section listing `rows`, or nothing when `rows` is empty —
/// an empty heading in a migration report reads as "checked, none found" only if
/// you already know the tool checked.
fn section(s: &mut String, title: &str, rows: Vec<String>) {
    use std::fmt::Write as _;
    if rows.is_empty() {
        return;
    }
    let _ = write!(s, "## {title}\n\n");
    for r in rows {
        let _ = writeln!(s, "- {r}");
    }
    s.push('\n');
}

/// What the discrimination rule says about one page, before any grammar is read.
enum Candidate {
    /// The page carries no `meridian-lock` block at all — out of scope.
    NoLock,
    /// The page is decided already; build the verdict from its path.
    Verdict(VerdictFor),
    /// The page carries ONE page-terminal block: the migration candidate.
    TerminalBlock(model::ByteSpan),
}

/// Apply rules 1 and 2 of the discrimination rule (module docs) — placement,
/// then arity. **The order is the correctness**, and the reason lives here
/// rather than at the call site because this function IS the rule.
///
/// The engine births a lock at EOF (`lock_write` § Placement law), so "is there
/// content after the LAST block" answers a prior question to "how many are
/// there": whether this page carries an engine-minted lock AT ALL. A document
/// that illustrates the schema six times over — a session trace, a decision
/// record — has six blocks and no page lock. Asking about arity first would
/// call it `MultipleBlocks` corruption and REFUSE it, permanently, for being
/// documentation. Measured on the live corpus: the two verbatim ZT session
/// traces carry exactly that shape.
fn classify(doc: &model::Document) -> Candidate {
    let spans = lock::block_spans(doc);
    let Some(last) = spans.last().cloned() else {
        return Candidate::NoLock;
    };
    let blocks = spans.len();

    // Rule 1 — PLACEMENT.
    if !doc.raw[last.end..].trim().is_empty() {
        return Candidate::Verdict(Box::new(move |path| PageVerdict::NotEnginePlaced {
            path,
            blocks,
        }));
    }
    // Rule 2 — ARITY. The last block is page-terminal, so this page does claim
    // an engine-minted lock; a second block makes it genuinely ambiguous, and
    // `lock::find` refuses it too — the engine cannot read this page today.
    if blocks > 1 {
        return Candidate::Verdict(Box::new(move |path| PageVerdict::MultipleBlocks {
            path,
            blocks,
        }));
    }
    Candidate::TerminalBlock(last)
}

/// Rule 3 of the discrimination rule: read the terminal block through the
/// quarantined v1 grammar and convert it.
///
/// `Ok(Some(v2))` is a migration candidate; `Ok(None)` means the block is
/// already v2 (the idempotence arm); `Err` carries the refusal, still needing a
/// path to name.
type VerdictFor = Box<dyn Fn(String) -> PageVerdict>;

fn read_v1(slice: &str) -> Result<Option<lock::Lock>, VerdictFor> {
    let unparseable = |detail: String| -> VerdictFor {
        Box::new(move |path| PageVerdict::Unparseable {
            path,
            detail: detail.clone(),
        })
    };
    match v1::peek_version(slice) {
        Ok(v1::V1) => {}
        Ok(lock::VERSION) => return Ok(None),
        Ok(other) => return Err(unparseable(format!("version {other} is neither v1 nor v2"))),
        Err(e) => return Err(unparseable(e.to_string())),
    }
    let parsed = v1::parse(slice).map_err(|e| unparseable(e.to_string()))?;
    let converted = convert(&parsed).map_err(|e| -> VerdictFor {
        let detail = e.to_string();
        Box::new(move |path| PageVerdict::Unconvertible {
            path,
            detail: detail.clone(),
        })
    })?;
    Ok(Some(converted))
}

/// **Run the v1 → v2 lock migration over one vault.**
///
/// Discovery is a scan of the whole corpus; the ACTION at each page is decided
/// by the discrimination rule in the module docs. Every rewrite goes through
/// [`wire_serve::write::lock_migrate`] — this function lands no bytes itself,
/// which is what makes the sweep governed rather than a script with a text
/// editor in it.
///
/// # Errors
/// [`SweepError`] on a tool failure. A per-page refusal is NOT an error: it is a
/// reported [`PageVerdict`], and [`MigrationReport::refusals`] is what a gate
/// reads.
pub fn sweep(root: &fs::WorkspaceRoot, opts: &Options) -> Result<MigrationReport, SweepError> {
    // THE RESTORE-POINT PRECONDITION, first, before anything is read or moved.
    // A dry run checks it too: the dry run's whole job is to tell you whether the
    // real run may proceed, and "you have no undo" is the first way the answer is
    // no.
    git::Repo::at(&root.0)
        .top_level()
        .map_err(|e| SweepError::NotAGitRepo {
            vault: root.0.display().to_string(),
            detail: e.to_string(),
        })?;

    let (files, _) = fs::domain_snapshot(root).map_err(|e| SweepError::Corpus(e.to_string()))?;
    let (_index, docs) = fs::build_corpus(files).map_err(|e| SweepError::Corpus(e.to_string()))?;

    let mut pages = Vec::new();
    // The v2 locks this run knows about — converted ones plus already-v2 ones.
    // Drift is computed off this map, so a DRY run predicts exactly what a real
    // run would produce.
    let mut v2_locks: BTreeMap<String, lock::Lock> = BTreeMap::new();
    let mut migrated: BTreeSet<String> = BTreeSet::new();

    // BTreeMap iteration is path-sorted → a deterministic, resumable order.
    for (path, doc) in &docs {
        let (span, slice) = match classify(doc) {
            Candidate::NoLock => continue,
            Candidate::Verdict(make) => {
                pages.push(make(path.clone()));
                continue;
            }
            Candidate::TerminalBlock(span) => {
                let Some(slice) = doc.raw.get(span.clone()) else {
                    pages.push(PageVerdict::Unparseable {
                        path: path.clone(),
                        detail: "lock span out of bounds".into(),
                    });
                    continue;
                };
                (span, slice)
            }
        };
        let _ = &span;

        let converted = match read_v1(slice) {
            Ok(Some(v2)) => v2,
            // Already v2. Record its pins so drift prediction sees pages swept
            // on an earlier, interrupted run.
            Ok(None) => {
                if let Ok(parsed) = lock::parse(slice) {
                    v2_locks.insert(path.clone(), parsed);
                }
                pages.push(PageVerdict::AlreadyV2 { path: path.clone() });
                continue;
            }
            Err(make) => {
                pages.push(make(path.clone()));
                continue;
            }
        };

        let carried_keys: Vec<String> = converted
            .pins
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.extra.keys().map(move |k| format!("{i}:{k}")))
            .collect();

        let args = wire_serve::write::LockMigrateArgs {
            id: None,
            path: wire::Path(path.clone()),
            if_block: slice.to_string(),
            lock: converted.clone(),
            actor: opts.actor.clone(),
            now: opts.now.clone(),
            if_root: opts.expect_root.clone(),
            if_file_rev: wire::NodeRev(doc.root.node_rev.0.clone()),
            dry: opts.dry,
        };
        let outcome =
            wire_serve::write::lock_migrate(root, 0, &args).map_err(|e| SweepError::Door {
                path: path.clone(),
                detail: wire_err(&e),
            })?;

        pages.push(PageVerdict::Migrated {
            path: path.clone(),
            rev_before: outcome.file_rev_before.0.clone(),
            rev_after: outcome.file_rev_after.0.clone(),
            pins: converted.pins.len(),
            carried_keys,
        });
        migrated.insert(path.clone());
        v2_locks.insert(path.clone(), converted);
    }

    Ok(MigrationReport {
        vault: root.0.display().to_string(),
        dry: opts.dry,
        expected_drift: predict_drift(&v2_locks, &migrated),
        pages,
    })
}

/// **The S7 drift prediction**: every full-body (`path: []`) pin that names a
/// page this sweep rewrites.
///
/// Such a pin's fingerprint covers the whole body of its target, and the target's
/// body CONTAINS the lock block being rewritten (lock-is-content, #8 §5). So the
/// recorded fingerprint goes stale the moment the rewrite lands — once, by
/// construction, for reasons that are not corruption.
///
/// Objects are matched by exact path. A v1 `ref` recorded a literal
/// corpus-relative path, so that is an identity for every row this migration
/// produces; wiki-link resolution belongs to the walk plane and is deliberately
/// not re-derived here.
fn predict_drift(
    v2_locks: &BTreeMap<String, lock::Lock>,
    migrated: &BTreeSet<String>,
) -> Vec<DriftRow> {
    let mut rows = Vec::new();
    for (pinning_page, l) in v2_locks {
        for pin in &l.pins {
            let full_body = matches!(&pin.selector, lock::Selector::Path(p) if p.is_empty());
            if full_body && migrated.contains(&pin.object) {
                rows.push(DriftRow {
                    pinning_page: pinning_page.clone(),
                    object: pin.object.clone(),
                    stale_fingerprint: pin.fingerprint.clone(),
                });
            }
        }
    }
    rows
}

/// A one-line diagnostic for a wire error body, mirroring the pin crate's
/// renderer.
fn wire_err(e: &wire::ErrorBody) -> String {
    let code = serde_json::to_value(e.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "error".to_owned());
    match &e.message {
        Some(m) => format!("{code}: {m}"),
        None => code,
    }
}
