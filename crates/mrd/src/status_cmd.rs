//! `mrd status` — the bare, pure-local drift + freshness summary (U3.6, the LAST
//! leg of the bridge; cutover order law: status cuts last).
//!
//! ```text
//! mrd status [--json] [--cwd PATH]
//! ```
//!
//! # What bare `status` is (plan §4 Block 3 U3.6; d2 §6.2)
//! `status = freshness` (d2 §3; distinct from `check = validity`). Bare `status`
//! answers "what is armed, what drifted, what was forced, and how fresh is my
//! origin knowledge" — from FROZEN facts only. It is:
//!
//! - **pure-local** — no daemon, no network, no fetch (cap-free);
//! - **O(armed) + O(pins) + O(objects)** — it reads ONE index file
//!   (`conventions/INDEX.md`) for the armed set, re-hashes each armed
//!   convention's `CHECK.md` (O(armed) small reads), scans the bounded receipt
//!   journal, and reads the git refs. The `meridian-lock` planes add ONE corpus
//!   build (the lock lives in the corpus's pages, so nothing smaller can see
//!   it), shared by both: the pin colors are O(pins) and the vibe-debt gauge is
//!   O(objects) plus at most TWO git calls (one `rev-list`, one batched
//!   `cat-file`) — never O(corpus) and never a call per blob, so the 3k-corpus
//!   wall-time stays sub-second;
//! - **fetch-less** — the anchor axis is therefore NEVER `verified` and never
//!   renders a bare `at-tip` (W-C1, U2.7; the colors amendment § anchor axis);
//! - **predicate-free** — it never evaluates a `check:` (the <1s budget holds;
//!   passenger-registry amendment). Drift here is a mechanical rev compare, never
//!   a starlark run.
//!
//! # The composed legend — four axes on one surface (U6.2)
//! `status` renders the orthogonal axes side by side, never merged, each rolled
//! up worst-of INDEPENDENTLY (colors amendment § composed legend):
//!
//! - **pin color** — the armed set's evidence drift: `green` (every armed
//!   convention's live `CHECK.md` rev still equals its pinned `armed_rev`) or
//!   `red content-drifted` (some armed evidence drifted). The four named greys of
//!   the full color law are the render's capability (fixtures pin each), and are
//!   reached only by pinned `^inputs` edges, not the armed set.
//! - **lock color** — the `meridian-lock` pins' FINGERPRINT verdicts
//!   ([`LockAxis`]), rolled up red > grey > green. A different source and a
//!   different compare from the armed-set `pin` axis, so neither subsumes the
//!   other and neither changes the other's roll-up.
//! - **anchor state** — the origin-freshness qualifier (U2.7): `as-known` /
//!   `unverified`, NEVER `verified` (status cannot fetch). This is where origin
//!   tip-compare CURRENCY lives, for every axis on the line — see [`LockAxis`]
//!   for why a repo-level currency fact never enters a per-pin color.
//! - **convention severity** — the worst armed severity (`off` / `warn` /
//!   `block`), and one violation row per `--force`-escaped skip.
//! - **vibe debt** — the quantity axis ([`VibeDebt`]): how many lock-referenced
//!   blobs git holds that no commit reaches, and how many bytes they are. A
//!   METER, never a gate: it never enters the exit triad.
//!
//! # The INDEX summary line
//! `<A> armed · <D> drifted · <F> forced-since-realise (receipts boundary)`:
//! - `armed` — the count of `[x]` rows in the attested INDEX (U1.4);
//! - `drifted` — armed conventions whose live `CHECK.md` rev ≠ the pinned
//!   `armed_rev` (the arming drift gate, read-only);
//! - `forced-since-realise` — the count of `op=force` journal rows (U4.3) newer
//!   than the last realise APPLY. The boundary is the last `now` in
//!   `receipts/realise.md` (the realise receipt ledger — realise applies leave NO
//!   reserved-journal marker, so this is a receipts boundary, not a
//!   journal-anchored guarantee). It resolves toward VISIBILITY: no realise
//!   receipt ⇒ genesis (count all forced writes), and a tied/unordered `now`
//!   COUNTS — the counter over-reports violations, never under-reports.
//!
//! # Exit triad (§4 preamble)
//! - **0** — clean: nothing armed drifted and nothing was forced.
//! - **1** — a finding: an armed convention drifted, a forced write is live, or
//!   the INDEX is a convention-fault. Field-equivalent to `md status`'s red
//!   (drifted) exit at the semantic class.
//! - **2** — bad invocation, or an unresolvable / unreadable workspace.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use model::Document;
use model::selector::{Color, RedReason};
use policy::{Enforcement, evidence_rev, parse_index_strict};
use receipt::anchor::{
    AnchorFacts, AnchorState, ObjectAnchor, ObjectAnchorFacts, Observed, TipPosition,
};
use receipt::journal::parse_rows;
use serde_json::{Value, json};
use view::walk::{color_detail, color_label, color_reason, color_tone};

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, but the summary
/// carries a live drift, a forced write, or a faulted INDEX.
const EXIT_FINDING: u8 = 1;

/// The attested INDEX page — the ONE file the armed-set read opens (byte-equal to
/// `policy::binding::RESERVED_INDEX_PATH` / `fs::domain`).
const CONVENTIONS_INDEX: &str = "conventions/INDEX.md";

/// The realise receipt ledger — the `forced-since-realise` boundary source. NOT
/// the reserved journal: realise applies append `- run {json} ^r-NNNNNN` lines
/// here, never an `op=force`-carrying journal row.
const REALISE_RECEIPTS: &str = "receipts/realise.md";

/// Run `mrd status [--json] [--cwd PATH]`.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation or an unresolvable / unreadable workspace;
/// exit 1 when the summary carries a finding (drift, a forced write, or a faulted
/// INDEX).
pub(crate) fn run(tail: &[String]) -> Result<(), Fail> {
    let (format, cwd_arg) = parse(tail)?;
    let cwd = match cwd_arg {
        Some(p) => p,
        None => current_dir()?,
    };
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let workspace = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;

    let report = gather(&workspace);

    match format {
        Format::Json => println!("{}", report.json()),
        Format::Human => print!("{}", report.render_human()),
    }

    if report.has_findings() {
        return Err(Fail {
            code: EXIT_FINDING,
            message: report.finding_summary(),
        });
    }
    Ok(())
}

/// Parse `[--json] [--cwd PATH]` — the bare form only (no page positional; a page
/// query is not this verb, U3.6).
fn parse(tail: &[String]) -> Result<(Format, Option<PathBuf>), Fail> {
    let mut json = false;
    let mut cwd: Option<PathBuf> = None;
    let mut i = 0;
    while i < tail.len() {
        let arg = tail[i].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v.to_owned())),
            None => (arg, None),
        };
        match flag {
            "--json" => json = true,
            "--cwd" => {
                let v = if let Some(v) = inline {
                    v
                } else {
                    i += 1;
                    tail.get(i)
                        .cloned()
                        .ok_or_else(|| Fail::tool("--cwd needs a value".to_owned()))?
                };
                cwd = Some(PathBuf::from(v));
            }
            other => return Err(Fail::tool(format!("unknown flag or argument: {other}"))),
        }
        i += 1;
    }
    let format = if json { Format::Json } else { Format::Human };
    Ok((format, cwd))
}

/// One `--force`-escaped skip surfaced as a violation row (U4.3 §11.1): the
/// bypassed rule, the journal anchor, and the recorded `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    /// The bypassed convention / binding-break (the `forced_rule=` token).
    rule: String,
    /// The journal row anchor (`r-NNNNNN`) — the permanent record of the skip.
    anchor: String,
    /// The recorded timestamp of the forced write, verbatim (never invented).
    now: Option<String>,
}

/// Where the `forced-since-realise` count is anchored — named so the render can
/// state precisely that this is a receipts boundary, not a journal guarantee.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Boundary {
    /// No realise receipt exists — count ALL forced writes (visibility fallback).
    Genesis,
    /// The last realise-apply `now` from `receipts/realise.md`.
    LastApply(String),
}

/// The `meridian-lock` axis (U6.2) — the corpus's lock pins rolled up worst-of.
///
/// **Its relationship to [`StatusReport::pin_rollup`] is ORTHOGONAL, never
/// merged.** The two read different sources and answer different questions:
/// `pin` rolls up the ARMED SET's evidence drift (each armed convention's live
/// `CHECK.md` rev vs its pinned `armed_rev`, from `conventions/INDEX.md`);
/// `lock` rolls up the FINGERPRINT verdicts of every `meridian-lock` pin in the
/// corpus. Neither can subsume the other, `pin_rollup`'s own worst-of
/// (red-if-any-drifted, else green) is unchanged by this axis, and a green on
/// one axis never colors the other — the U6.2 composed legend renders them side
/// by side, each rolled up independently.
///
/// **Currency is NOT folded into the tone.** Origin tip-compare is a
/// REPOSITORY-level fact, and a lock verdict is per-pin and content-addressed
/// (D12) — folding a repo fact into a per-pin color would both merge two axes
/// and re-root a root-independent computation. Currency therefore stays on the
/// `anchor` axis this line already renders: `lock` says whether the pinned
/// content still matches the working copy, `anchor` says how current that
/// working copy is against origin's tip. Read together, never multiplied.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LockAxis {
    /// Lock rows colored — pins plus any page-level lock-refusal row.
    rows: usize,
    /// The worst-of color across those rows — `None` when there are none. Not a
    /// color: a vault with no lock pins has nothing to verify, and rendering
    /// green there would claim an attestation nobody made.
    rollup: Option<Color>,
    /// Set when the corpus itself could not be read — the lock plane is out of
    /// sight, reported as such rather than as an empty (falsely clean) axis.
    unreadable: Option<String>,
}

impl LockAxis {
    /// Roll up `colors` worst-of: red (a measured defect) over grey (never
    /// measured) over green. **Grey above green is load-bearing** — a roll-up
    /// that let one unverifiable pin hide inside a green fleet would render the
    /// exact false green the color law forbids.
    fn roll_up(colors: &[Color]) -> LockAxis {
        let rollup = colors.iter().max_by_key(|c| Self::severity(c)).cloned();
        LockAxis {
            rows: colors.len(),
            rollup,
            unreadable: None,
        }
    }

    /// The worst-of rank of one color: green 0 < grey 1 < red 2.
    fn severity(color: &Color) -> u8 {
        match color {
            Color::Green => 0,
            Color::Grey(_) => 1,
            Color::Red(_) => 2,
        }
    }

    /// The axis word: the worst-of label plus the row count, or the honest empty
    /// / unreadable case. Never a bare tone. The count is bracketed because a
    /// reason already carries its own parenthesized detail.
    fn render(&self) -> String {
        if let Some(detail) = &self.unreadable {
            return format!("unreadable ({detail})");
        }
        let Some(color) = &self.rollup else {
            return "none".to_owned();
        };
        let unit = if self.rows == 1 { "pin" } else { "pins" };
        format!("{} [{} {unit}]", color_label(color), self.rows)
    }
}

/// The vibe-debt gauge (U6.2) — how much of the corpus's retrieval plane is
/// held by nothing but this machine.
///
/// A `--vibe` pin writes its blob eagerly (`git hash-object -w`) so the pin can
/// be verified before the file is committed. That blob is reachable from no ref,
/// so it survives only until `git gc` ages it past the repository's local
/// `gc.pruneExpire` ([`receipt::anchor::PENDING_ANCHOR_TTL`], named residual
/// G1). The debt is exactly that population: the lock-referenced blobs git HAS
/// but no commit reaches, counted and summed in bytes.
///
/// **It is a METER, never a gate.** Debt never enters
/// [`StatusReport::has_findings`], never refuses a write, and never warns as an
/// error — the gauge reports the size of the window, it does not shorten it.
///
/// **What it does NOT count:** a blob absent from the object database
/// (`never-anchored` — pruned past the TTL, or a fresh clone) is not debt but
/// past debt: nothing local can pay it, and its bytes no longer exist to sum. A
/// blob a commit reaches is not debt at all.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VibeDebt {
    /// Distinct lock-referenced blobs present in the object database and
    /// reachable from no commit.
    blobs: usize,
    /// Their total size, git's own byte count.
    bytes: u64,
    /// Set when reachability could not be measured (no git, not a repository,
    /// an unreadable corpus): the gauge reports unknown, never a false `0`.
    unknown: Option<String>,
}

impl VibeDebt {
    /// Nothing owed — the honest zero. A gauge that hides at zero is not a
    /// gauge, so this renders and serializes exactly like any other reading.
    fn clear() -> VibeDebt {
        VibeDebt {
            blobs: 0,
            bytes: 0,
            unknown: None,
        }
    }

    /// Unmeasurable — the reachability question could not be asked. Never
    /// collapsed into `0`: a false clean is the one reading this gauge exists
    /// to prevent.
    fn unknown(detail: String) -> VibeDebt {
        VibeDebt {
            blobs: 0,
            bytes: 0,
            unknown: Some(detail),
        }
    }

    /// The gauge word: the count, the bytes, or the honest unknown case.
    fn render(&self) -> String {
        if let Some(detail) = &self.unknown {
            return format!("unknown ({detail})");
        }
        let unit = if self.blobs == 1 { "blob" } else { "blobs" };
        format!("{} {unit} ({} bytes)", self.blobs, self.bytes)
    }
}

/// The gathered, render-ready status summary — the three composed axes, the INDEX
/// counts, and the forced-write violation rows.
struct StatusReport {
    workspace: String,
    /// Count of `[x]` rows in the attested INDEX (the armed set).
    armed: usize,
    /// Count of armed conventions whose live `CHECK.md` rev ≠ pinned `armed_rev`.
    drifted: usize,
    /// Count of `op=force` journal rows newer than the realise boundary.
    forced: usize,
    /// Where the forced count is anchored.
    boundary: Boundary,
    /// The INDEX convention-fault detail, when the page is present but corrupt.
    index_fault: Option<String>,
    /// The pin-color axis roll-up — `Green` all-fresh, `Red(Drifted)` any-drift.
    pin_rollup: Color,
    /// The meridian-lock axis — the corpus's lock pins, rolled up worst-of.
    lock: LockAxis,
    /// The vibe-debt gauge — lock-referenced blobs no commit reaches.
    vibe_debt: VibeDebt,
    /// The convention-severity axis roll-up — the worst armed severity.
    severity_rollup: Enforcement,
    /// The rendered anchor-qualified tip axis (U2.7) — never a bare `at-tip`.
    anchor_axis: String,
    /// The as-known-ageless nudge hint, when present.
    nudge: Option<&'static str>,
    /// The `--force`-escaped skips since the boundary.
    violations: Vec<Violation>,
}

impl StatusReport {
    /// A finding is live when armed evidence drifted, a forced write is unresolved,
    /// or the INDEX faulted — the exit-1 predicate.
    fn has_findings(&self) -> bool {
        self.drifted > 0 || self.forced > 0 || self.index_fault.is_some()
    }

    /// The one-line stderr summary that rides the exit-1 `Fail`.
    fn finding_summary(&self) -> String {
        if let Some(detail) = &self.index_fault {
            return format!("INDEX convention-fault: {detail}");
        }
        format!(
            "{} drifted, {} forced-since-realise",
            self.drifted, self.forced
        )
    }

    /// The composed multi-axis line — armed-pin color · meridian-lock color ·
    /// anchor state · convention severity · vibe debt, side by side, never
    /// merged (U6.2 composed legend). `lock` sits beside `anchor` on purpose:
    /// the pin verdict and the currency qualifier that reads it are one glance
    /// apart. `vibe-debt` is the fifth question — not a color and not a verdict,
    /// but a quantity — so it rides the tail rather than splitting that pair.
    fn composed_line(&self) -> String {
        format!(
            "pin {} · lock {} · anchor {} · convention {} · vibe-debt {}",
            color_label(&self.pin_rollup),
            self.lock.render(),
            self.anchor_axis,
            self.severity_rollup.as_str(),
            self.vibe_debt.render(),
        )
    }

    /// Render the human summary block.
    fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "status  {}", self.workspace);
        let boundary = match &self.boundary {
            Boundary::Genesis => "genesis".to_owned(),
            Boundary::LastApply(now) => format!("since {now}"),
        };
        let _ = writeln!(
            out,
            "  INDEX: {} armed · {} drifted · {} forced-since-realise (receipts boundary: {boundary})",
            self.armed, self.drifted, self.forced,
        );
        let _ = writeln!(out, "  {}", self.composed_line());
        if let Some(nudge) = self.nudge {
            let _ = writeln!(out, "  hint: {nudge}");
        }
        if let Some(detail) = &self.index_fault {
            let _ = writeln!(out, "  INDEX fault: {detail}");
        }
        for v in &self.violations {
            let _ = writeln!(out, "  {}", render_violation_row(v));
        }
        out
    }

    /// The `--json` shape: the three axes as fields, the counts, the boundary, and
    /// the violation rows.
    fn json(&self) -> String {
        let violations: Vec<Value> = self
            .violations
            .iter()
            .map(|v| {
                json!({
                    "rule": v.rule,
                    "anchor": v.anchor,
                    "now": v.now,
                })
            })
            .collect();
        let boundary = match &self.boundary {
            Boundary::Genesis => json!({ "kind": "genesis" }),
            Boundary::LastApply(now) => json!({ "kind": "receipts", "since": now }),
        };
        let doc = json!({
            "workspace": self.workspace,
            "index": {
                "armed": self.armed,
                "drifted": self.drifted,
                "forced_since_realise": self.forced,
                "boundary": boundary,
                "fault": self.index_fault,
            },
            "composed": {
                "pin_color": color_tone(&self.pin_rollup),
                "pin_label": color_label(&self.pin_rollup),
                // The meridian-lock axis, ALWAYS emitted (no conditional omit):
                // an empty vault reports 0 pins and a null color, never an
                // absent field a reader could mistake for "not checked".
                "lock": {
                    "pins": self.lock.rows,
                    "color": self.lock.rollup.as_ref().map(color_tone),
                    "reason": self.lock.rollup.as_ref().and_then(color_reason),
                    "detail": self.lock.rollup.as_ref().and_then(color_detail),
                    "label": self.lock.render(),
                    "unreadable": self.lock.unreadable,
                },
                "anchor": self.anchor_axis,
                "convention_severity": self.severity_rollup.as_str(),
                // The vibe-debt gauge, ALWAYS emitted (no conditional omit): a
                // corpus with nothing owed reports 0 blobs and 0 bytes, because
                // a gauge that hides at zero is not a gauge — an absent field
                // reads as "not measured", which is the `unknown` case instead.
                "vibe_debt": {
                    "blobs": self.vibe_debt.blobs,
                    "bytes": self.vibe_debt.bytes,
                    "label": self.vibe_debt.render(),
                    "unknown": self.vibe_debt.unknown,
                },
            },
            "nudge": self.nudge,
            "violations": violations,
            "findings": self.has_findings(),
        });
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
    }
}

/// Render one forced-write violation row (U4.3 §11.1) — the visible, permanent
/// record of a `--force`-escaped armed refusal.
fn render_violation_row(v: &Violation) -> String {
    let now = v.now.as_deref().unwrap_or("(no timestamp)");
    format!(
        "violation: forced past `{}` · convention block · ^{} ({now})",
        v.rule, v.anchor,
    )
}

/// Gather the status summary over a canonical workspace path. The impure edge: it
/// reads the INDEX, the armed `CHECK.md` files, the journal, the realise receipts,
/// and the git refs — every read frozen, none re-evaluated. It never fails: every
/// absent / unreadable frozen fact degrades to its honest empty case (genesis,
/// unverified), so status always renders a summary.
fn gather(workspace: &Path) -> StatusReport {
    // 1. The armed set — ONE index-file read (O(armed)).
    let (armed, index_fault) = read_armed(workspace);

    // 2. Per-armed drift — re-hash each CHECK.md (O(armed) small reads).
    let mut drifted = 0usize;
    for a in &armed {
        if convention_drifted(workspace, &a.slug, &a.armed_rev) {
            drifted += 1;
        }
    }
    let pin_rollup = if drifted > 0 {
        Color::Red(RedReason::Drifted)
    } else {
        Color::Green
    };
    let severity_rollup = armed
        .iter()
        .map(|a| a.enforcement)
        .max()
        .unwrap_or(Enforcement::Off);

    // 3. The journal + the realise boundary — the forced-since-realise count.
    let journal_text = read_optional(workspace, receipt_journal_rel());
    let rows = parse_rows(&journal_text);
    let boundary = realise_boundary(workspace);
    let (forced, violations) = forced_since(&journal_text, &rows, &boundary);

    // 4. The anchor axis — git refs, fetch-less, never verified (U2.7).
    let (anchor_axis, nudge) = anchor_axis(workspace, &rows);

    // 5. The meridian-lock planes — ONE corpus build answering both the pin
    //    colors (claim plane) and the vibe-debt gauge (retrieval plane).
    let (lock, vibe_debt) = lock_planes(workspace);

    StatusReport {
        workspace: workspace.display().to_string(),
        armed: armed.len(),
        drifted,
        forced,
        boundary,
        index_fault,
        pin_rollup,
        lock,
        vibe_debt,
        severity_rollup,
        anchor_axis,
        nudge,
        violations,
    }
}

/// Read both `meridian-lock` planes of the workspace over ONE corpus build: the
/// claim plane's pin colors rolled up worst-of ([`LockAxis`]) and the retrieval
/// plane's unreachable blobs ([`VibeDebt`]).
///
/// This is the ONE place status leaves the O(armed) set: the lock lives in the
/// corpus's pages, so it costs one corpus build (`fs::domain_snapshot` +
/// `fs::build_corpus` — the same builder `mrd walk` uses, so status and walk can
/// never disagree about a pin). Both planes read that ONE build; a second build
/// would let the two axes describe two different corpora. It stays sub-second on
/// the 3k-doc corpus: the build reads and parses bytes without resolving
/// anything, the coloring is O(pins) and the gauge O(objects), never O(corpus).
///
/// Honest degradation, like every other frozen-fact read here: an unreadable
/// corpus reports `unreadable` / `unknown`, never an empty (falsely clean) axis.
fn lock_planes(workspace: &Path) -> (LockAxis, VibeDebt) {
    let docs = match crate::walk_cmd::build_docs(workspace) {
        Ok(docs) => docs,
        Err(fail) => {
            let axis = LockAxis {
                rows: 0,
                rollup: None,
                unreadable: Some(fail.message.clone()),
            };
            return (axis, VibeDebt::unknown(fail.message));
        }
    };
    let colors: Vec<Color> = view::walk::lock_pin_colors(&docs)
        .into_iter()
        .map(|p| p.color)
        .collect();
    (LockAxis::roll_up(&colors), vibe_debt(workspace, &docs))
}

/// Measure the vibe debt: the lock-referenced blobs git holds that no commit
/// reaches, counted and summed in bytes.
///
/// Two git calls at most, never a call per blob: ONE
/// `git rev-list --objects --all` into the reachable set (S5's `ReachableSet`,
/// O(1) membership) and ONE batched `git cat-file --batch-check` for presence
/// and size. `receipt::anchor::ObjectAnchor` classifies the gathered facts — the
/// same fact/classify split the origin-anchor axis uses — and only its
/// `PendingAnchor` state is debt.
///
/// A corpus that references no blobs asks git nothing: nothing is referenced, so
/// nothing can be owed, and the gauge reads a true `0` even outside a repository.
///
/// A value that is not an object id at all is UNKNOWN, never skipped: git cannot
/// be asked about it, so the entry's debt is unmeasurable, and a gauge that
/// dropped it read a corrupt retrieval plane as a true zero — the same false
/// clean the `unknown` slot exists to prevent.
fn vibe_debt(workspace: &Path, docs: &BTreeMap<String, Document>) -> VibeDebt {
    // Distinct blob ids, first-sighting order: one blob referenced by two pages
    // is ONE object on disk, and counting it twice would double its bytes.
    let mut seen: HashSet<String> = HashSet::new();
    let mut oids: Vec<String> = Vec::new();
    let mut malformed: Vec<String> = Vec::new();
    for object in view::walk::lock_objects(docs) {
        let oid = object.blob_sha.to_ascii_lowercase();
        if !git::is_oid(&oid) {
            malformed.push(format!("{} objects.{}", object.src_path, object.key));
            continue;
        }
        if seen.insert(oid.clone()) {
            oids.push(oid);
        }
    }
    if let Some(detail) = malformed_detail(&malformed) {
        return VibeDebt::unknown(detail);
    }
    if oids.is_empty() {
        return VibeDebt::clear();
    }

    let repo = git::Repo::at(workspace);
    let reachable = match repo.reachable_objects() {
        Ok(set) => set,
        Err(fail) => return VibeDebt::unknown(fail.to_string()),
    };
    let refs: Vec<&str> = oids.iter().map(String::as_str).collect();
    let info = match repo.object_info(&refs) {
        Ok(info) => info,
        Err(fail) => return VibeDebt::unknown(fail.to_string()),
    };

    let mut debt = VibeDebt::clear();
    for (oid, info) in oids.iter().zip(info) {
        let facts = ObjectAnchorFacts {
            object_present: info.is_some(),
            reachable_from_commit: reachable.contains(oid),
        };
        // `PendingAnchor` alone is debt: present, reachable from nothing. The
        // size is git's own byte count, so the sum costs no second git call.
        if ObjectAnchor::classify(&facts) == ObjectAnchor::PendingAnchor
            && let Some(present) = info
        {
            debt.blobs += 1;
            debt.bytes += present.size;
        }
    }
    debt
}

/// The `unknown` detail for `objects:` entries whose value is not an object id —
/// the count plus the first offender's page and key, so the reading names WHERE
/// the retrieval plane is damaged instead of just refusing to answer. `None`
/// when every entry is well-formed.
fn malformed_detail(malformed: &[String]) -> Option<String> {
    let first = malformed.first()?;
    let n = malformed.len();
    let unit = if n == 1 { "entry" } else { "entries" };
    Some(format!(
        "{n} `objects:` {unit} not an object id, so git cannot be asked (first: {first})"
    ))
}

/// One armed convention read from the INDEX: the pinned `armed_rev` per slug, plus
/// its severity.
struct ArmedConv {
    slug: String,
    enforcement: Enforcement,
    armed_rev: String,
}

/// Read the attested INDEX into the armed set. An absent page is a genesis
/// workspace (nothing armed, no fault); a present-but-corrupt page fails CLOSED to
/// a named fault (never read as an empty, gate-disabling armed set).
fn read_armed(workspace: &Path) -> (Vec<ArmedConv>, Option<String>) {
    let text = read_optional(workspace, CONVENTIONS_INDEX);
    if text.is_empty() {
        return (Vec::new(), None);
    }
    match parse_index_strict(&text) {
        Ok(refs) => {
            let armed = refs
                .into_iter()
                .map(|r| ArmedConv {
                    slug: r.slug,
                    enforcement: r.enforcement,
                    armed_rev: r.armed_rev,
                })
                .collect();
            (armed, None)
        }
        Err(corrupt) => (Vec::new(), Some(corrupt.detail)),
    }
}

/// Whether an armed convention's live `CHECK.md` rev differs from its pinned
/// `armed_rev` (the arming drift gate, read-only). A missing `CHECK.md` — the
/// pinned evidence vanished — counts as drift (fail-closed: the armed law can no
/// longer be verified at its rev).
fn convention_drifted(workspace: &Path, slug: &str, armed_rev: &str) -> bool {
    let rel = format!("conventions/{slug}/CHECK.md");
    match std::fs::read_to_string(workspace.join(&rel)) {
        Ok(text) => evidence_rev(&text) != armed_rev,
        Err(_) => true,
    }
}

/// The reserved receipt-journal path, workspace-relative.
fn receipt_journal_rel() -> &'static str {
    fs::domain::RESERVED_JOURNAL_PATH
}

/// Read a workspace-relative page, treating any read error (absent, unreadable) as
/// the empty string — the genesis / tolerated case for the frozen-fact reads.
fn read_optional(workspace: &Path, rel: &str) -> String {
    std::fs::read_to_string(workspace.join(rel)).unwrap_or_default()
}

/// Resolve the `forced-since-realise` boundary from the realise receipt ledger:
/// the LAST realise-apply `now` (`- run {json} ^r-NNNNNN` lines), or [`Boundary::Genesis`]
/// when the ledger is absent / has no dated apply.
fn realise_boundary(workspace: &Path) -> Boundary {
    let text = read_optional(workspace, REALISE_RECEIPTS);
    match last_receipt_now(&text) {
        Some(now) => Boundary::LastApply(now),
        None => Boundary::Genesis,
    }
}

/// The `now` field of the LAST `- run {json} ^r-NNNNNN` receipt line, or `None`
/// when no dated apply is present. Parses only the JSON envelope — never a
/// predicate.
fn last_receipt_now(receipts: &str) -> Option<String> {
    receipts
        .lines()
        .rev()
        .filter_map(|line| line.trim().strip_prefix("- run "))
        .find_map(|rest| {
            // Strip the trailing ` ^r-NNNNNN` anchor, parse the JSON envelope.
            let json = rest.rsplit_once(" ^").map_or(rest, |(head, _)| head);
            serde_json::from_str::<Value>(json.trim())
                .ok()
                .and_then(|v| v.get("now").and_then(Value::as_str).map(str::to_owned))
        })
}

/// Count the `op=force` journal rows newer than the realise boundary, and collect
/// them as violation rows. Resolves toward VISIBILITY (leader ruling): a forced
/// write with no `now`, or a `now` that ties / is unordered relative to the
/// boundary, COUNTS — only a row strictly older than the boundary is excluded.
fn forced_since(
    journal_text: &str,
    rows: &[receipt::journal::ParsedRow],
    boundary: &Boundary,
) -> (usize, Vec<Violation>) {
    let lines: Vec<&str> = journal_text.lines().collect();
    let mut violations = Vec::new();
    for row in rows.iter().filter(|r| r.op == "force") {
        if !counts_since(row.now.as_deref(), boundary) {
            continue;
        }
        let rule = lines
            .get(row.line_no.saturating_sub(1))
            .and_then(|line| forced_rule(line))
            .unwrap_or_else(|| "(unnamed)".to_owned());
        violations.push(Violation {
            rule,
            anchor: row.anchor.clone(),
            now: row.now.clone(),
        });
    }
    (violations.len(), violations)
}

/// Whether a forced row's `now` counts against the boundary. Over-reports: only a
/// dated row STRICTLY older than a dated boundary is excluded; genesis, an undated
/// row, and a tie all count.
fn counts_since(row_now: Option<&str>, boundary: &Boundary) -> bool {
    // Exclude ONLY a dated row strictly older than a dated boundary; genesis, an
    // undated row, and a tie all fall through to `true` (over-report).
    match (boundary, row_now) {
        (Boundary::LastApply(b), Some(n)) => n >= b.as_str(),
        _ => true,
    }
}

/// Extract the `forced_rule=<rule>` token from a raw `op=force` journal line. The
/// token is whitespace-collapsed at write time (one token, no split), so a plain
/// whitespace tokenize recovers it. `ParsedRow` does not model it.
fn forced_rule(line: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix("forced_rule="))
        .map(str::to_owned)
}

/// Render the anchor-qualified tip axis (U2.7) for the workspace, plus any nudge
/// hint. Fetch-less: `run_observed` is ALWAYS false, so the state is never
/// `verified` and never renders a bare `at-tip` (the W-C1 invariant). Returns a
/// custom "no origin ref" render when the remote-tracking ref is absent (no tip to
/// compare) rather than a misleading bare position.
fn anchor_axis(
    workspace: &Path,
    rows: &[receipt::journal::ParsedRow],
) -> (String, Option<&'static str>) {
    let branch = git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let head = git(workspace, &["rev-parse", "HEAD"]);
    let origin_ref = format!("refs/remotes/origin/{branch}");
    let origin = git(
        workspace,
        &["rev-parse", "--verify", "--quiet", &origin_ref],
    );

    let now_unix = now_unix();
    let last_observation = receipt::anchor::last_observation(rows).and_then(|now| {
        parse_rfc3339(now).map(|unix| Observed {
            now: now.to_owned(),
            unix,
        })
    });
    let facts = AnchorFacts {
        run_observed: false,
        last_observation,
        origin_ref_present: origin.is_some(),
    };
    let state = AnchorState::classify(&facts, now_unix);
    let nudge = receipt::anchor::nudge_hint(&state);

    if let (Some(h), Some(o)) = (head, origin) {
        let tip = if h == o {
            TipPosition::AtTip
        } else {
            TipPosition::Behind
        };
        (receipt::anchor::render_tip_axis(tip, &state), nudge)
    } else {
        // No remote-tracking ref (or no HEAD): there is no tip to compare. Render
        // the anchor state alone, never a fabricated position.
        let word = if branch.is_empty() {
            "no origin ref".to_owned()
        } else {
            format!("no origin/{branch} ref")
        };
        (
            format!("{word} (anchor {})", anchor_state_word(&state)),
            nudge,
        )
    }
}

/// The bare state word for the no-origin-ref render (the qualifier without a tip
/// position, which does not exist when there is no ref to compare).
fn anchor_state_word(state: &AnchorState) -> &'static str {
    match state {
        AnchorState::Verified => "verified",
        AnchorState::AsKnownAged { .. } | AnchorState::AsKnownAgeless => "as-known",
        AnchorState::Unverified => "unverified",
    }
}

/// Current wall-clock as epoch seconds — the run's one anchor moment.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Run `git -C workspace <args>` and return trimmed stdout on success, or `None`
/// on any failure (missing ref, no repo, non-zero exit). The tip axis degrades
/// honestly — a git failure renders `unverified`, never a fabricated position.
fn git(workspace: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Parse a canonical RFC3339 UTC token (`YYYY-MM-DDTHH:MM:SSZ`) to epoch seconds,
/// or `None` when it is not that exact shape. The anchor's journaled observations
/// are minted in this form; a non-canonical token is undatable, so the anchor
/// degrades to as-known-AGELESS (never a wrong age). No date crate — the civil
/// arithmetic is exact and dependency-free.
fn parse_rfc3339(token: &str) -> Option<i64> {
    let bytes = token.as_bytes();
    // YYYY-MM-DDTHH:MM:SSZ is exactly 20 bytes with fixed separators.
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return None;
    }
    let num = |lo: usize, hi: usize| token.get(lo..hi)?.parse::<i64>().ok();
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let min = num(14, 16)?;
    let sec = num(17, 19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + min) * 60 + sec)
}

/// Days since the Unix epoch (1970-01-01) for a civil date, by Howard Hinnant's
/// `days_from_civil` algorithm — exact for the proleptic Gregorian calendar, no
/// lookup tables, no dependency.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::selector::GreyReason;

    // ── the four-grey render fixtures (U6.2 full color law) ──────────────────

    /// Each of the four named greys renders through the composed line's pin-color
    /// axis with its contract label (colors amendment § Colors / D2-F4). The
    /// render function carries the FULL color law, not just the armed set's
    /// green/red.
    #[test]
    fn four_greys_render_each_with_its_contract_label() {
        let cases = [
            (GreyReason::DeclaredUnpinned, "grey declared-unpinned"),
            (GreyReason::SupersededAlgo, "grey superseded-algo"),
            (GreyReason::ImmutableRoot, "grey immutable-root"),
            // `Ambiguous` is the code variant for the selector-ambiguity grey; the
            // contract's fourth named grey `unmanaged` shares the DeclaredUnpinned
            // variant (labeled `unmanaged` only on the pin surface). The composed
            // line renders whichever variant it is handed.
            (GreyReason::Ambiguous, "grey ambiguous"),
        ];
        for (reason, expected) in cases {
            let line = compose(
                Color::Grey(reason),
                "at-tip (anchor unverified)",
                Enforcement::Block,
            );
            assert!(
                line.contains(expected),
                "grey variant renders {expected}: {line}"
            );
        }
    }

    /// The composed multi-axis line renders every axis side by side, never
    /// merged — armed-pin color, meridian-lock color, anchor state, convention
    /// severity (U6.2).
    #[test]
    fn composed_line_shows_every_axis_side_by_side() {
        let line = compose(
            Color::Red(RedReason::Drifted),
            "behind (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d)",
            Enforcement::Warn,
        );
        assert_eq!(
            line,
            "pin red content-drifted · lock none · anchor behind (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d) · convention warn · vibe-debt 0 blobs (0 bytes)"
        );
    }

    /// The green, off, at-tip clean composition.
    #[test]
    fn composed_line_green_clean() {
        let line = compose(Color::Green, "at-tip (anchor as-known)", Enforcement::Off);
        assert_eq!(
            line,
            "pin green · lock none · anchor at-tip (anchor as-known) · convention off · vibe-debt 0 blobs (0 bytes)"
        );
    }

    /// Compose the axes without an IO edge — the pure render under test by the
    /// fixtures.
    fn compose(pin: Color, anchor_axis: &str, severity: Enforcement) -> String {
        report(pin, LockAxis::roll_up(&[]), anchor_axis, severity).composed_line()
    }

    /// A pure [`StatusReport`] with no IO edge — the render fixtures' subject.
    fn report(
        pin: Color,
        lock: LockAxis,
        anchor_axis: &str,
        severity: Enforcement,
    ) -> StatusReport {
        report_with_debt(pin, lock, anchor_axis, severity, VibeDebt::clear())
    }

    /// The same pure report with a chosen vibe-debt reading.
    fn report_with_debt(
        pin: Color,
        lock: LockAxis,
        anchor_axis: &str,
        severity: Enforcement,
        vibe_debt: VibeDebt,
    ) -> StatusReport {
        StatusReport {
            workspace: "/ws".to_owned(),
            armed: 0,
            drifted: 0,
            forced: 0,
            boundary: Boundary::Genesis,
            index_fault: None,
            pin_rollup: pin,
            lock,
            vibe_debt,
            severity_rollup: severity,
            anchor_axis: anchor_axis.to_owned(),
            nudge: None,
            violations: Vec::new(),
        }
    }

    // ── S9: the meridian-lock axis (U6.2 never-merged) ───────────────────────

    /// The lock roll-up is worst-of RED > GREY > GREEN, and grey-over-green is
    /// load-bearing: one unverifiable pin must not hide inside a green fleet.
    #[test]
    fn the_lock_rollup_is_worst_of_with_grey_above_green() {
        let grey = Color::Grey(GreyReason::UnverifiableFingerprint {
            unknown: vec!["version"],
        });
        let red = Color::Red(RedReason::Drifted);

        assert_eq!(
            LockAxis::roll_up(&[]).rollup,
            None,
            "no pins is not a color"
        );
        assert_eq!(
            LockAxis::roll_up(&[Color::Green, Color::Green]).rollup,
            Some(Color::Green)
        );
        assert_eq!(
            LockAxis::roll_up(&[Color::Green, grey.clone()]).rollup,
            Some(grey.clone()),
            "a grey pin must not roll up green",
        );
        assert_eq!(
            LockAxis::roll_up(&[Color::Green, grey, red.clone()]).rollup,
            Some(red),
            "a measured red outranks an unmeasured grey",
        );
        assert_eq!(
            LockAxis::roll_up(&[Color::Green, Color::Green, Color::Green]).rows,
            3
        );
    }

    /// The axis renders its worst-of label, its count, and the honest empty /
    /// unreadable cases — never a bare tone and never a silent absence.
    #[test]
    fn the_lock_axis_renders_every_case() {
        assert_eq!(LockAxis::roll_up(&[]).render(), "none");
        assert_eq!(
            LockAxis::roll_up(&[Color::Green, Color::Green]).render(),
            "green [2 pins]"
        );
        assert_eq!(
            LockAxis::roll_up(&[Color::Grey(GreyReason::LockRefused {
                reason: "more than one meridian-lock block on the page".to_owned(),
            })])
            .render(),
            "grey lock-refused (more than one meridian-lock block on the page) [1 pin]",
        );
        assert_eq!(
            LockAxis {
                rows: 0,
                rollup: None,
                unreadable: Some("cannot read the corpus: boom".to_owned()),
            }
            .render(),
            "unreadable (cannot read the corpus: boom)",
        );
    }

    /// `json()` ALWAYS emits the lock fields — an empty vault reports 0 pins and
    /// a null color, never an absent field a reader could mistake for
    /// "not checked". The armed-set `pin_*` fields are untouched beside them.
    #[test]
    fn json_always_emits_the_lock_axis_beside_the_untouched_pin_rollup() {
        let empty: Value = serde_json::from_str(
            &report(
                Color::Green,
                LockAxis::roll_up(&[]),
                "at-tip (anchor as-known)",
                Enforcement::Off,
            )
            .json(),
        )
        .expect("json");
        let lock = &empty["composed"]["lock"];
        assert_eq!(lock["pins"], json!(0));
        assert_eq!(lock["color"], Value::Null, "no pins is not a color");
        assert_eq!(lock["label"], json!("none"));
        assert_eq!(lock["unreadable"], Value::Null);
        // The armed-set axis is unchanged and still its own worst-of.
        assert_eq!(empty["composed"]["pin_color"], json!("green"));
        assert_eq!(empty["composed"]["pin_label"], json!("green"));

        let drifted: Value = serde_json::from_str(
            &report(
                Color::Green,
                LockAxis::roll_up(&[Color::Grey(GreyReason::UnverifiableFingerprint {
                    unknown: vec!["version"],
                })]),
                "at-tip (anchor as-known)",
                Enforcement::Off,
            )
            .json(),
        )
        .expect("json");
        let lock = &drifted["composed"]["lock"];
        assert_eq!(lock["pins"], json!(1));
        assert_eq!(lock["color"], json!("grey"));
        assert_eq!(lock["reason"], json!("unverifiable-fingerprint"));
        assert_eq!(lock["detail"], json!("unknown version"));
        // ORTHOGONAL: a grey lock axis never repaints the armed-set pin axis.
        assert_eq!(drifted["composed"]["pin_color"], json!("green"));
    }

    // ── S11: the vibe-debt gauge (U6.2, a quantity not a color) ─────────────

    /// ZERO RENDERS — the half of the gate that is easiest to miss. With nothing
    /// owed the gauge still occupies its segment on the human line and still
    /// emits its fields in `--json`: `0`, never an absent field a reader could
    /// mistake for "not measured".
    #[test]
    fn the_vibe_debt_gauge_renders_zero_as_a_reading_not_a_silence() {
        assert_eq!(VibeDebt::clear().render(), "0 blobs (0 bytes)");

        let clean = report(
            Color::Green,
            LockAxis::roll_up(&[]),
            "at-tip (anchor as-known)",
            Enforcement::Off,
        );
        assert!(
            clean
                .composed_line()
                .ends_with("· vibe-debt 0 blobs (0 bytes)"),
            "zero holds its segment: {}",
            clean.composed_line(),
        );

        let v: Value = serde_json::from_str(&clean.json()).expect("json");
        let debt = &v["composed"]["vibe_debt"];
        assert_eq!(debt["blobs"], json!(0));
        assert_eq!(debt["bytes"], json!(0));
        assert_eq!(debt["label"], json!("0 blobs (0 bytes)"));
        assert_eq!(debt["unknown"], Value::Null);
        assert!(
            !debt.is_null(),
            "the gauge is a field at zero, never an omission"
        );
    }

    /// A reading renders count AND bytes (singular at one), and `--json` carries
    /// both as numbers beside the unchanged axes.
    #[test]
    fn the_vibe_debt_gauge_renders_count_and_bytes() {
        let one = VibeDebt {
            blobs: 1,
            bytes: 512,
            unknown: None,
        };
        assert_eq!(one.render(), "1 blob (512 bytes)");
        assert_eq!(
            VibeDebt {
                blobs: 3,
                bytes: 4096,
                unknown: None,
            }
            .render(),
            "3 blobs (4096 bytes)"
        );

        let owed = report_with_debt(
            Color::Green,
            LockAxis::roll_up(&[Color::Green]),
            "at-tip (anchor as-known)",
            Enforcement::Off,
            one,
        );
        assert!(
            owed.composed_line()
                .contains("· vibe-debt 1 blob (512 bytes)"),
            "{}",
            owed.composed_line()
        );
        let v: Value = serde_json::from_str(&owed.json()).expect("json");
        assert_eq!(v["composed"]["vibe_debt"]["blobs"], json!(1));
        assert_eq!(v["composed"]["vibe_debt"]["bytes"], json!(512));
        // ORTHOGONAL (U6.2): debt repaints no color axis.
        assert_eq!(v["composed"]["pin_color"], json!("green"));
        assert_eq!(v["composed"]["lock"]["color"], json!("green"));
    }

    /// Unmeasurable is its own reading, never a false `0`: no git, no
    /// repository, or an unreadable corpus says so in both renders.
    #[test]
    fn the_vibe_debt_gauge_reports_unknown_rather_than_a_false_zero() {
        let unknown = VibeDebt::unknown("not a git repository: /ws".to_owned());
        assert_eq!(unknown.render(), "unknown (not a git repository: /ws)");

        let v: Value = serde_json::from_str(
            &report_with_debt(
                Color::Green,
                LockAxis::roll_up(&[]),
                "at-tip (anchor as-known)",
                Enforcement::Off,
                unknown,
            )
            .json(),
        )
        .expect("json");
        let debt = &v["composed"]["vibe_debt"];
        assert_eq!(debt["blobs"], json!(0));
        assert_eq!(
            debt["unknown"],
            json!("not a git repository: /ws"),
            "unknown names why it could not be measured",
        );
        assert_eq!(debt["label"], json!("unknown (not a git repository: /ws)"));
    }

    /// METER, NOT A GATE — debt never becomes a finding: the exit triad is
    /// unchanged by any reading, and `findings` stays false. Enforcement is not
    /// in this stage at all.
    #[test]
    fn vibe_debt_is_never_a_finding() {
        let owed = report_with_debt(
            Color::Green,
            LockAxis::roll_up(&[]),
            "at-tip (anchor as-known)",
            Enforcement::Block,
            VibeDebt {
                blobs: 9,
                bytes: 1_048_576,
                unknown: None,
            },
        );
        assert!(
            !owed.has_findings(),
            "the gauge reports; it never refuses or exits 1"
        );
        let v: Value = serde_json::from_str(&owed.json()).expect("json");
        assert_eq!(v["findings"], json!(false));
    }

    // ── the forced-write violation row fixture (U4.3 §11.1) ──────────────────

    /// The forced-write violation row renders the bypassed rule, the block
    /// severity, and the permanent journal anchor.
    #[test]
    fn forced_write_renders_the_violation_row() {
        let v = Violation {
            rule: "reviewer-not-owner".to_owned(),
            anchor: "r-000042".to_owned(),
            now: Some("2026-07-23T10:00:00Z".to_owned()),
        };
        assert_eq!(
            render_violation_row(&v),
            "violation: forced past `reviewer-not-owner` · convention block · ^r-000042 (2026-07-23T10:00:00Z)"
        );
    }

    /// `forced_rule=` is recovered from a raw `op=force` journal line (it is not a
    /// `ParsedRow` field).
    #[test]
    fn forced_rule_reads_the_token() {
        let line = "- op=force path=tasks/fix.md actor=agent:a root_before=b3:1 root_after=b3:2 edits=0 forced_rule=reviewer-not-owner ^r-000042";
        assert_eq!(forced_rule(line).as_deref(), Some("reviewer-not-owner"));
        assert_eq!(forced_rule("- op=splice ^r-000001"), None);
    }

    // ── the forced-since-realise boundary (leader ruling: over-report) ───────

    /// Genesis (no realise receipt) counts every forced write.
    #[test]
    fn genesis_boundary_counts_all_forced() {
        assert!(counts_since(
            Some("2020-01-01T00:00:00Z"),
            &Boundary::Genesis
        ));
        assert!(counts_since(None, &Boundary::Genesis));
    }

    /// A dated boundary excludes ONLY a strictly-older dated row; a newer row, a
    /// tie, and an undated row all count (visibility).
    #[test]
    fn dated_boundary_over_reports() {
        let b = Boundary::LastApply("2026-07-23T00:00:00Z".to_owned());
        assert!(
            !counts_since(Some("2026-07-22T23:59:59Z"), &b),
            "strictly older is excluded"
        );
        assert!(
            counts_since(Some("2026-07-23T00:00:00Z"), &b),
            "a tie counts"
        );
        assert!(
            counts_since(Some("2026-07-23T00:00:01Z"), &b),
            "newer counts"
        );
        assert!(counts_since(None, &b), "an undated row counts");
    }

    /// The last realise-apply `now` is read from the receipt ledger's last
    /// `- run {json}` line; an absent / undated ledger is genesis.
    #[test]
    fn last_receipt_now_reads_the_latest_apply() {
        let ledger = "# realise receipts\n\
            - run {\"page\":\"a.md\",\"now\":\"2026-07-23T09:00:00Z\"} ^r-000001\n\
            - run {\"page\":\"b.md\",\"now\":\"2026-07-23T10:00:00Z\"} ^r-000002\n";
        assert_eq!(
            last_receipt_now(ledger).as_deref(),
            Some("2026-07-23T10:00:00Z")
        );
        assert_eq!(last_receipt_now(""), None);
        assert_eq!(last_receipt_now("# empty\n"), None);
    }

    /// End-to-end forced count over a journal: two force rows after the boundary,
    /// one before — the boundary excludes only the strictly-older one.
    #[test]
    fn forced_since_counts_rows_after_boundary() {
        let journal = "# journal\n\
            - op=splice path=a.md root_before=b3:0 root_after=b3:1 edits=1 ^r-000001\n\
            - op=force path=x.md actor=agent:a root_before=b3:1 root_after=b3:2 edits=0 forced_rule=rule-old ^r-000002\n\
            - op=force path=y.md actor=agent:a root_before=b3:2 root_after=b3:3 edits=0 forced_rule=rule-new ^r-000003\n";
        // Manually stamp `now` onto the force rows by re-parsing after injecting.
        let journal = journal
            .replace(
                "forced_rule=rule-old ^r-000002",
                "now=2026-07-22T00:00:00Z forced_rule=rule-old ^r-000002",
            )
            .replace(
                "forced_rule=rule-new ^r-000003",
                "now=2026-07-24T00:00:00Z forced_rule=rule-new ^r-000003",
            );
        let rows = parse_rows(&journal);
        let boundary = Boundary::LastApply("2026-07-23T00:00:00Z".to_owned());
        let (count, violations) = forced_since(&journal, &rows, &boundary);
        assert_eq!(count, 1, "only the post-boundary force row counts");
        assert_eq!(violations[0].rule, "rule-new");
        assert_eq!(violations[0].anchor, "r-000003");

        // Genesis counts both.
        let (all, _) = forced_since(&journal, &rows, &Boundary::Genesis);
        assert_eq!(all, 2, "genesis counts every forced write");
    }

    // ── the RFC3339 → epoch parser (anchor age arithmetic) ───────────────────

    /// Known epoch anchors round-trip; a non-canonical token is undatable
    /// (`None`), so the anchor degrades to ageless rather than mis-dating.
    #[test]
    fn rfc3339_parses_canonical_utc_only() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2000-01-01T00:00:00Z"), Some(946_684_800));
        assert_eq!(parse_rfc3339("2026-07-23T00:00:00Z"), Some(1_784_764_800));
        // Non-canonical shapes are undatable.
        assert_eq!(parse_rfc3339("2026-07-23"), None);
        assert_eq!(parse_rfc3339("2026-07-23T00:00:00+00:00"), None);
        assert_eq!(parse_rfc3339("not-a-date"), None);
    }
}
