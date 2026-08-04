//! `mrd status` — the bare, pure-local drift + freshness summary (U3.6, the LAST
//! leg of the bridge; cutover order law: status cuts last).
//!
//! ```text
//! mrd status [--json] [--cwd PATH]
//! ```
//!
//! # What bare `status` is (plan §4 Block 3 U3.6; d2 §6.2)
//! `status = freshness` (d2 §3; distinct from `check = validity`). Bare `status`
//! answers "what is armed, what drifted, and how fresh is my origin knowledge" —
//! from FROZEN facts only. It is:
//!
//! - **pure-local** — no daemon, no network, no fetch (cap-free);
//! - **O(armed) + O(pins) + O(objects)** — it reads ONE index file
//!   (`conventions/INDEX.md`) for the armed set, re-hashes each armed
//!   convention's `CHECK.md` (O(armed) small reads), and reads the git refs.
//!   The `meridian-lock` planes add ONE corpus
//!   build of THIS workspace (the lock lives in the corpus's pages, so nothing
//!   smaller can see it), plus one corpus build per mount root **that this
//!   corpus's own lock addresses actually name** — usually none. That last
//!   clause is load-bearing and was once absent, in the doc and in the code:
//!   `walk_cmd::load_mounts` built EVERY declared root eagerly, so a bare
//!   `status` walked every tree in the machine's `~/MERIDIAN.md` to colour a
//!   handful of ambient pins. Measured on a four-root machine: 271 MB and 43,524
//!   directories per invocation, 80.6% of the run, at 0 armed rules
//!   (`crates/mrd/src/walk_cmd.rs` § `load_mounts_for`). The narrowing is
//!   [`lock_addressed_roots`];
//!   shared by both planes: the pin colors are O(pins) and the vibe-debt gauge is
//!   O(objects) plus at most TWO git calls PER OBJECT STORE (one `rev-list`, one
//!   batched `cat-file`) — never O(corpus) and never a call per blob, so the
//!   3k-corpus wall-time stays sub-second — a bound over the WORKSPACE corpus,
//!   which holds only because the roots are no longer walked unconditionally. A
//!   corpus whose pinned objects are
//!   all ambient has exactly ONE store and so exactly the two calls it always
//!   had; a key naming a root adds that root's store, because the anchoring
//!   check runs against THAT root's git repo (U13, ratified cross-root
//!   addressing §4 — six roots, six object stores, one law);
//! - **fetch-less** — the anchor axis is therefore NEVER `verified` and never
//!   renders a bare `at-tip` (W-C1, U2.7; the colors amendment § anchor axis);
//! - **predicate-free** — it never evaluates a `check:` (the <1s budget holds;
//!   the retired passenger-registry amendment; R4 makes an unrecognised lock key
//!   engine-ignored by definition). Drift here is a mechanical rev compare, never
//!   a starlark run.
//!
//! # The composed legend — four axes on one surface (U6.2)
//! `status` renders the orthogonal axes side by side, never merged, each rolled
//! up worst-of INDEPENDENTLY (colors amendment § composed legend):
//!
//! - **pin color** — the armed set's evidence drift: `green` (every armed row's
//!   live rule PAGE rev still equals the rev the artifact pinned) or
//!   `red content-drifted` (some armed evidence drifted). The named greys of the
//!   full color law are the render's CAPABILITY — the composed line carries them
//!   whichever it is handed — and the armed set itself reaches none of them.
//!   Their enumeration lives in the colors amendment, and which variants exist is
//!   owned by `view::walk::color_reason`'s exhaustive match, never counted here.
//! - **lock color** — the `meridian-lock` pins' FINGERPRINT verdicts
//!   ([`LockAxis`]), rolled up red > grey > green. A different source and a
//!   different compare from the armed-set `pin` axis, so neither subsumes the
//!   other and neither changes the other's roll-up.
//! - **anchor state** — the origin-freshness qualifier (U2.7): `as-known` /
//!   `unverified`, NEVER `verified` (status cannot fetch). This is where origin
//!   tip-compare CURRENCY lives, for every axis on the line — see [`LockAxis`]
//!   for why a repo-level currency fact never enters a per-pin color.
//! - **armed mode** — the worst armed mode (`off` / `warn` / `block` / `armed`),
//!   and one violation row per `--force`-escaped skip.
//! - **vibe debt** — the quantity axis ([`VibeDebt`]): how many lock-referenced
//!   blobs git holds that no commit reaches, and how many bytes they are. A
//!   METER, never a gate: it never enters the exit triad.
//!
//! # The armed summary line
//! `<A> armed · <D> drifted · forced-since-realise: not tracked`:
//! - `armed` — the row count of the attested armed-rules artifact
//!   ([`fs::domain::ARMED_RULES_PATH`]);
//! - `drifted` — armed rows whose live PAGE rev ≠ the rev the artifact pinned
//!   (the arming drift gate, read-only);
//! - `forced-since-realise` — **rendered as explicitly not tracked, and the line
//!   says why.** It used to count `op=force` rows in the receipt journal since
//!   the last realise apply. ZT ruled the ledger out of existence (2026-08-03:
//!   *"Engine does not have memory. It should not have. History is pin to git
//!   when we lock. Anything between locks is not history."*), so a forced write
//!   between two locks is not a thing `status` can observe — by design, not by
//!   accident.
//!
//! # Why the axis is DISCLOSED and not deleted
//! Deleting the line would be a silent narrowing, which is the one move both
//! this docket's rulings forbid. Two facts make it worse here than in a
//! read-only report:
//!
//! 1. **The exit code moved.** A forced write used to make `status` exit 1. A
//!    workspace that exits 0 today may be one a forced write is live in. A
//!    reader who is not told reads the same green as before and concludes
//!    something `status` never checked.
//! 2. **The question is still good.** "Has anyone forced past an armed rule?"
//!    did not stop mattering; it stopped being answerable HERE. The line names
//!    where the answer lives now — git — so the reader's next move is one hop
//!    away instead of a wrong conclusion.
//!
//! It is the same shape `check` was ruled into for its sibling question: state,
//! on both faces, that the property is not assessed. A count is not re-derivable
//! from git (a forced splice and an ordinary one land as the same commit), so
//! the honest surface is a disclosure, never a re-derivation and never silence.
//!
//! # Exit triad (§4 preamble)
//! - **0** — clean: nothing armed drifted and the armed law is readable. NOTE
//!   what 0 no longer covers: a forced write cannot move this code, because it
//!   is not observed (see the disclosure above).
//! - **1** — a finding: an armed rule drifted, or the armed-rules artifact
//!   faulted. Field-equivalent to `md status`'s red (drifted) exit at the
//!   semantic class.
//! - **2** — bad invocation, or an unresolvable / unreadable workspace.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use model::Document;
use model::selector::{Color, RedReason};
use policy::armed::{Mode, parse_artifact};
use policy::page_rev;
use receipt::anchor::{AnchorFacts, AnchorState, ObjectAnchor, ObjectAnchorFacts, TipPosition};
use serde_json::json;
use view::walk::{color_detail, color_label, color_reason, color_tone};

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, but the summary
/// carries a live drift or a faulted armed-rules artifact.
const EXIT_FINDING: u8 = 1;

/// Run `mrd status [--json] [--cwd PATH]`.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation or an unresolvable / unreadable workspace;
/// exit 1 when the summary carries a finding (drift, or a faulted INDEX).
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

    // The resolution is reported, never assumed: the ruling requires every
    // answer to name which rung answered and which root it named. `status` used
    // to print the path alone, which is exactly the silence being retired. The
    // label is the ladder's own word (`resolve::Source`), so this surface cannot
    // drift from the tier vocabulary.
    let report = gather(&workspace, resolved.source.label());

    match format {
        Format::Json => println!("{}", report.json()),
        Format::Human => print!("{}", report.render_human()),
    }

    if report.has_findings() {
        return Err(Fail::with_code(EXIT_FINDING, report.finding_summary()));
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

/// What the forced-since-realise axis says now that nothing observes it. One
/// constant, used by BOTH faces, so the human line and the `--json` face cannot
/// drift into disagreeing about what was checked.
const FORCED_NOT_TRACKED: &str = "not-tracked";

/// Why it is not tracked, in the words of the law that made it so. Rendered, not
/// implied: a reader meeting a missing axis needs the reason in front of them,
/// not in a changelog.
const FORCED_NOT_TRACKED_WHY: &str = "the engine keeps no memory by design — \
                                      a forced write between two locks is not history; look in git";

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
    /// an unreadable corpus, or a root whose object store cannot be named or
    /// asked): the gauge reports unknown, never a false `0`.
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
    /// How that workspace was resolved — the tier word, or `daemon-adopted` /
    /// `ephemeral`. Rendered beside the path so no reader has to assume it.
    source: String,
    /// Count of `[x]` rows in the attested INDEX (the armed set).
    armed: usize,
    /// Count of armed conventions whose live `CHECK.md` rev ≠ pinned `armed_rev`.
    drifted: usize,
    /// The armed-rules artifact's fault detail, when the workspace's armed law
    /// cannot be trusted: a corrupt artifact, or an artifact missing on a
    /// workspace the once-armed marker says HAS been armed. Absent artifact AND
    /// absent marker is genesis — unarmed, and not a fault.
    artifact_fault: Option<String>,
    /// The pin-color axis roll-up — `Green` all-fresh, `Red(Drifted)` any-drift.
    pin_rollup: Color,
    /// The meridian-lock axis — the corpus's lock pins, rolled up worst-of.
    lock: LockAxis,
    /// The vibe-debt gauge — lock-referenced blobs no commit reaches.
    vibe_debt: VibeDebt,
    /// The armed-mode axis roll-up — the worst armed mode.
    mode_rollup: Mode,
    /// The rendered anchor-qualified tip axis (U2.7) — never a bare `at-tip`.
    anchor_axis: String,
    /// The as-known-ageless nudge hint, when present.
    nudge: Option<&'static str>,
}

impl StatusReport {
    /// A finding is live when armed evidence drifted or the armed-rules artifact
    /// faulted — the exit-1 predicate.
    ///
    /// A forced write is NOT in it any more, and that is the disclosure's whole
    /// point: the axis left the exit code with the data source, so the surface
    /// says so rather than letting a quieter 0 pass for a cleaner one.
    fn has_findings(&self) -> bool {
        self.drifted > 0 || self.artifact_fault.is_some()
    }

    /// The one-line stderr summary that rides the exit-1 `Fail`.
    fn finding_summary(&self) -> String {
        if let Some(detail) = &self.artifact_fault {
            return format!("armed-rules fault: {detail}");
        }
        format!("{} drifted", self.drifted)
    }

    /// The composed multi-axis line — armed-pin color · meridian-lock color ·
    /// anchor state · armed mode · vibe debt, side by side, never
    /// merged (U6.2 composed legend). `lock` sits beside `anchor` on purpose:
    /// the pin verdict and the currency qualifier that reads it are one glance
    /// apart. `vibe-debt` is the fifth question — not a color and not a verdict,
    /// but a quantity — so it rides the tail rather than splitting that pair.
    fn composed_line(&self) -> String {
        format!(
            "pin {} · lock {} · anchor {} · armed {} · vibe-debt {}",
            color_label(&self.pin_rollup),
            self.lock.render(),
            self.anchor_axis,
            self.mode_rollup.as_str(),
            self.vibe_debt.render(),
        )
    }

    /// Render the human summary block.
    fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "status  {} ({})", self.workspace, self.source);
        let _ = writeln!(
            out,
            "  armed-rules: {} armed · {} drifted · forced-since-realise: {FORCED_NOT_TRACKED} \
             ({FORCED_NOT_TRACKED_WHY})",
            self.armed, self.drifted,
        );
        let _ = writeln!(out, "  {}", self.composed_line());
        if let Some(nudge) = self.nudge {
            let _ = writeln!(out, "  hint: {nudge}");
        }
        if let Some(detail) = &self.artifact_fault {
            let _ = writeln!(out, "  armed-rules fault: {detail}");
        }
        out
    }

    /// The `--json` shape: the three axes as fields, the counts, the boundary, and
    /// the violation rows.
    fn json(&self) -> String {
        let doc = json!({
            "workspace": self.workspace,
            "source": self.source,
            "armed_rules": {
                "armed": self.armed,
                "drifted": self.drifted,
                // The count key and its `boundary` are REMOVED, not zeroed and
                // not nulled: a 0 reads as "checked, none found" and a null as
                // "checked, no answer", and both are lies about a property
                // nothing observes. What replaces them is a string that cannot
                // be mistaken for a count, carrying its own reason.
                "forced_since_realise": {
                    "tracked": false,
                    "state": FORCED_NOT_TRACKED,
                    "why": FORCED_NOT_TRACKED_WHY,
                },
                "fault": self.artifact_fault,
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
                "armed_mode": self.mode_rollup.as_str(),
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
            // `violations` is REMOVED for the same reason: an empty array reads
            // as "looked, found none".
            "findings": self.has_findings(),
        });
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
    }
}

/// Gather the status summary over a canonical workspace path. The impure edge: it
/// reads the armed-rules artifact, the once-armed marker, the armed rule PAGES,
/// and the git refs — every read frozen, none
/// re-evaluated. It never fails: every
/// absent / unreadable frozen fact degrades to its honest empty case (genesis,
/// unverified), so status always renders a summary.
///
/// `source` is how the caller resolved `workspace` — carried in rather than
/// re-derived, because this half is pure over a path and the ladder's answer is
/// the caller's fact. It is a parameter, not a settable field, so no render can
/// reach a report whose provenance was never filled in.
fn gather(workspace: &Path, source: &str) -> StatusReport {
    // 1. The armed set — ONE artifact read plus the marker probe (O(armed)).
    let (armed, artifact_fault) = read_armed(workspace);

    // 2. Per-armed drift — re-hash each armed PAGE (O(armed) small reads).
    let mut drifted = 0usize;
    for a in &armed {
        if page_drifted(workspace, &a.page, &a.attested_rev) {
            drifted += 1;
        }
    }
    let pin_rollup = if drifted > 0 {
        Color::Red(RedReason::Drifted)
    } else {
        Color::Green
    };
    let mode_rollup = armed.iter().map(|a| a.mode).max().unwrap_or(Mode::Off);

    // 3. The anchor axis — git refs, fetch-less, never verified (U2.7).
    let (anchor_axis, nudge) = anchor_axis(workspace);

    // 4. The meridian-lock planes — ONE corpus build answering both the pin
    //    colors (claim plane) and the vibe-debt gauge (retrieval plane).
    let (lock, vibe_debt) = lock_planes(workspace);

    StatusReport {
        workspace: workspace.display().to_string(),
        source: source.to_owned(),
        armed: armed.len(),
        drifted,
        artifact_fault,
        pin_rollup,
        lock,
        vibe_debt,
        mode_rollup,
        anchor_axis,
        nudge,
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
    // U11/F6 — the REAL mount table, through the one loader `mrd walk` and
    // `mrd check` use. `lock_pin_colors(&docs)` resolved against an EMPTY table,
    // so this axis could not vary on the cross-root axis at all: a bound root's
    // pin rolled up `grey(unmounted)` matched, drifted or restored. Fixing only
    // `check` would have made these two planes disagree — the guarantee at
    // `check/src/layer0.rs` is that they agree BY CONSTRUCTION, and it stays true
    // because both are fed the same corpus and the same table.
    //
    // The table is loaded with the corpora NARROWED to the roots this corpus's
    // own lock addresses name ([`lock_addressed_roots`]). The table itself is
    // unnarrowed, so this stays the same table `walk` and `check` are fed and the
    // by-construction agreement above is untouched — see
    // [`crate::walk_cmd::load_mounts_for`] for why the corpora may differ and the
    // verdicts may not.
    let mounts = crate::walk_cmd::load_mounts_for(&lock_addressed_roots(&docs));
    let corpus = mounts.rooted(&docs);
    let colors: Vec<Color> = view::walk::lock_pin_colors_rooted(&corpus, mounts.set())
        .into_iter()
        .map(|p| p.color)
        .collect();
    (LockAxis::roll_up(&colors), vibe_debt(workspace, &docs))
}

/// Every mount root the corpus's `meridian-lock` addresses NAME — the exact set
/// of roots whose pages this run can read, and so the exact set worth building.
///
/// # Why the set is knowable before any root is loaded
/// A pin's root is a property of its ADDRESS, not of the tree the address points
/// into: `sessions:notes/plan.md` names `sessions` whether or not `sessions` is
/// declared, readable, or holds that page. Reading the name therefore costs the
/// ambient corpus that is already in memory and no root corpus at all.
///
/// The root is read off [`view::read_face::LockItem::declared_addr`] — the
/// parsed address, which that field's contract names **the structural owner**
/// (U10): *"Every consumer that needs the root, the path or the selector reads
/// THIS; nothing re-splits `declared_ref`."* A second spelling of the address
/// grammar here is exactly the drift that field exists to prevent, so this
/// function contains none.
///
/// A row with no address — a lock refusal, or a spelling outside the grammar —
/// contributes no root, which is correct rather than lossy: it resolves into no
/// root, so no root's pages can answer for it.
fn lock_addressed_roots(docs: &BTreeMap<String, Document>) -> BTreeSet<addr::MountName> {
    let mut roots = BTreeSet::new();
    for doc in docs.values() {
        for item in view::read_face::page_lock_items(doc) {
            if let Some(root) = item.declared_addr.as_ref().and_then(addr::Addr::root) {
                roots.insert(root.clone());
            }
        }
    }
    roots
}

/// Which object store one pinned blob belongs to: the ambient
/// workspace (`None`), or a named root (`Some`).
///
/// **U13 — per-root anchoring, ratified `2026-07-24-cross-root-addressing.md`
/// §4:** *"the blob-anchoring check runs against THAT root's git repo — six
/// roots, six object stores, one law."* The pin's `object` is an agent-plane
/// address (§2: lock addresses use the canonical `root:` form),
/// so its root names the repository whose object database holds the blob. The
/// write path already carries the prefix through untouched (`wire-serve`'s
/// `set_object` — "the key is the target's path spelling VERBATIM … so a later
/// `root:` prefix rides through"); this is the reader that honours it.
type StoreKey = Option<addr::MountName>;

/// Measure the vibe debt: the lock-referenced blobs git holds that no commit
/// reaches, counted and summed in bytes.
///
/// Two git calls at most PER STORE, never a call per blob: ONE
/// `git rev-list --objects --all` into the reachable set (S5's `ReachableSet`,
/// O(1) membership) and ONE batched `git cat-file --batch-check` for presence
/// and size. `receipt::anchor::ObjectAnchor` classifies the gathered facts — the
/// same fact/classify split the origin-anchor axis uses — and only its
/// `PendingAnchor` state is debt.
///
/// **The law is one; the store is per root ([`StoreKey`]).** Entries are grouped
/// by the root their key names, and each group is asked of THAT root's
/// repository — `git::Repo` is a handle and never a singleton (seam rule D12),
/// so six roots are six handles running one unchanged classification. An
/// ambient-keyed corpus takes exactly the pre-U13 path: one group, one handle,
/// the workspace.
///
/// A corpus that references no blobs asks git nothing: nothing is referenced, so
/// nothing can be owed, and the gauge reads a true `0` even outside a repository.
///
/// A value that is not an object id at all is UNKNOWN, never skipped: git cannot
/// be asked about it, so the entry's debt is unmeasurable, and a gauge that
/// dropped it read a corrupt retrieval plane as a true zero — the same false
/// clean the `unknown` slot exists to prevent. A KEY that names no store is the
/// same class for the same reason: the question is *which* git to ask, and an
/// unanswerable one is reported, never guessed.
fn vibe_debt(workspace: &Path, docs: &BTreeMap<String, Document>) -> VibeDebt {
    // Distinct blob ids PER STORE, first-sighting order: one blob referenced by
    // two pages is ONE object on disk, and counting it twice would double its
    // bytes — but the same oid under two roots is TWO objects in two databases,
    // so the dedupe is keyed by store and never globally.
    let mut seen: HashSet<(StoreKey, String)> = HashSet::new();
    let mut stores: Vec<(StoreKey, Vec<String>)> = Vec::new();
    let mut malformed: Vec<String> = Vec::new();
    let mut unaddressable: Vec<String> = Vec::new();
    for object in view::walk::lock_objects(docs) {
        let oid = object.blob_sha.to_ascii_lowercase();
        if !git::is_oid(&oid) {
            malformed.push(format!("{} pin `{}`", object.src_path, object.key));
            continue;
        }
        // `Addr::parse` REFUSES a malformed root rather than reading it as a
        // literal path, and that refusal is carried here rather than swallowed:
        // falling back to the ambient store would ask the WRONG database and
        // answer confidently — a wrong SUCCESS, which is the one shape this
        // gauge must never produce.
        let Ok(addr) = addr::Addr::parse(&object.key) else {
            unaddressable.push(format!("{} pin `{}`", object.src_path, object.key));
            continue;
        };
        let store: StoreKey = addr.root().cloned();
        if seen.insert((store.clone(), oid.clone())) {
            match stores.iter_mut().find(|(k, _)| *k == store) {
                Some((_, oids)) => oids.push(oid),
                None => stores.push((store, vec![oid])),
            }
        }
    }
    if let Some(detail) = cannot_ask_detail(&malformed, "not an object id, so git cannot be asked")
    {
        return VibeDebt::unknown(detail);
    }
    if let Some(detail) = cannot_ask_detail(
        &unaddressable,
        "with a key that is not an address, so WHICH git to ask is unknown",
    ) {
        return VibeDebt::unknown(detail);
    }
    if stores.is_empty() {
        return VibeDebt::clear();
    }

    // The mount table is read ONCE, and only when a key actually names a root:
    // a corpus whose every key is ambient asks the config plane nothing, so a
    // single-root machine's gauge is byte-for-byte what it was before U13.
    let table = if stores.iter().any(|(store, _)| store.is_some()) {
        load_mount_table()
    } else {
        None
    };

    let mut debt = VibeDebt::clear();
    for (store, oids) in &stores {
        let root = match store {
            None => workspace.to_path_buf(),
            Some(name) => match store_path(name, table.as_ref()) {
                Ok(path) => path,
                Err(detail) => return VibeDebt::unknown(detail),
            },
        };
        // ONE handle per root. Both facts below come from THIS handle, in this
        // iteration, so a store's reachable set can never be read against
        // another store's presence answer.
        let repo = git::Repo::at(root);
        let reachable = match repo.reachable_objects() {
            Ok(set) => set,
            Err(fail) => return VibeDebt::unknown(store_fail(store, &fail)),
        };
        let refs: Vec<&str> = oids.iter().map(String::as_str).collect();
        let info = match repo.object_info(&refs) {
            Ok(info) => info,
            Err(fail) => return VibeDebt::unknown(store_fail(store, &fail)),
        };

        for (oid, info) in oids.iter().zip(info) {
            let facts = ObjectAnchorFacts {
                object_present: info.is_some(),
                reachable_from_commit: reachable.contains(oid),
            };
            // `PendingAnchor` alone is debt: present, reachable from nothing.
            // The size is git's own byte count, so the sum costs no second git
            // call.
            if ObjectAnchor::classify(&facts) == ObjectAnchor::PendingAnchor
                && let Some(present) = info
            {
                debt.blobs += 1;
                debt.bytes += present.size;
            }
        }
    }
    debt
}

/// The local path of the git repository backing ONE named root — the ratified
/// §4 lookup, and it is a MOUNT LOOKUP and nothing more (U11's settlement of
/// D12): the table maps canonical name → local path, and `git::Repo::at` takes
/// it from there.
///
/// Every failure arm is an honest degradation naming the root — never a
/// fabricated sha and never a silent fall back to the ambient store, which would
/// answer a different repository's question in this one's name.
fn store_path(
    name: &addr::MountName,
    table: Option<&config::mount::MountTable>,
) -> Result<PathBuf, String> {
    let Some(table) = table else {
        return Err(format!(
            "`{name}` names a root, but no mount table could be read here, so its object store cannot be asked"
        ));
    };
    let Some(mount) = table.by_name(name.as_str()) else {
        return Err(format!(
            "root `{name}` is not mounted here, so its object store cannot be asked. Fix: declare it in MERIDIAN.md"
        ));
    };
    // DECLARED but unusable is a DIFFERENT CAUSE with a different fix, and
    // telling an operator to declare a root they have already declared is the
    // false teaching S3-R43 removed. The mount plane's own sentence is carried
    // verbatim, so this gauge and `mrd config` say the same thing about the same
    // root rather than two spellings of it.
    if mount.state().refuses() {
        return Err(format!(
            "root `{name}` is declared but its object store cannot be asked: {}",
            mount.state().detail()
        ));
    }
    let Some(path) = mount.canonical_path() else {
        return Err(format!(
            "root `{name}` binds no readable path here, so its object store cannot be asked"
        ));
    };
    Ok(path.to_path_buf())
}

/// The bound mount table, or `None` when this machine has none to read.
///
/// Absence is the topology working as designed (§8 M6) and never a failure of
/// the gauge: a machine with no `MERIDIAN.md` binds no roots, so a rooted key
/// has no store to ask — which [`store_path`] then says in words. The same
/// never-fail shape `mrd walk`'s loader uses, for the same reason.
fn load_mount_table() -> Option<config::mount::MountTable> {
    let resolution = config::resolve(&config::Env::from_process()).ok()?;
    let cfg = resolution.config()?;
    config::mount::bind(cfg).ok()
}

/// A git failure while asking ONE store, with the root named.
///
/// The ambient arm keeps the pre-U13 wording byte-for-byte — a single-root
/// machine's `unknown` detail did not become a different sentence because the
/// engine grew roots. A named root prefixes its own name, because "not a git
/// repository" is only actionable once the reader knows WHICH repository was
/// asked (§5's per-root row: honest degradation, never a fabricated sha).
fn store_fail(store: &StoreKey, fail: &git::GitFail) -> String {
    match store {
        None => fail.to_string(),
        Some(name) => format!("root `{name}`: {fail}"),
    }
}

/// The `unknown` detail for pinned blobs git cannot be asked about — the
/// count, the reason clause, and the first offender's page and key, so the
/// reading names WHERE the retrieval plane is damaged instead of just refusing
/// to answer. `None` when there are no offenders.
///
/// One helper for both causes on purpose: an entry whose VALUE is not an object
/// id and one whose KEY names no store are the same reading — the question
/// cannot be put to git — and two spellings of one reading is how a reader comes
/// to believe they are two different states.
fn cannot_ask_detail(offenders: &[String], because: &str) -> Option<String> {
    let first = offenders.first()?;
    let n = offenders.len();
    let unit = if n == 1 { "entry" } else { "entries" };
    Some(format!("{n} pinned {unit} {because} (first: {first})"))
}

/// One armed row read from the artifact: which PAGE was attested, at which rev,
/// and in which mode. Flattened out of [`policy::armed::ArmedRow`] because status
/// keeps no policy value alive past the read — it renders counts, not law.
struct ArmedPage {
    page: String,
    mode: Mode,
    attested_rev: String,
}

/// Read the attested armed-rules artifact into the armed set.
///
/// The GENESIS reading is the whole subtlety, and it pivots on the MARKER, never
/// on the artifact: absent artifact AND absent marker is a never-armed workspace —
/// nothing armed, no fault, clean. An absent artifact on a workspace that HAS been
/// armed is the silent-disarm attack, so it fails CLOSED to a named fault; and a
/// present-but-corrupt artifact does too, because a page that will not parse must
/// never read as an empty, gate-disabling armed set.
///
/// Both reads come from [`wire_serve::armed_disk`], which is the same pair the
/// write door and the reaction feeder use — a workspace that disagreed with
/// itself about whether it is armed is exactly what one reader prevents.
fn read_armed(workspace: &Path) -> (Vec<ArmedPage>, Option<String>) {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let ever_armed = wire_serve::armed_disk::once_armed(&root);
    let Some(text) = wire_serve::armed_disk::read_artifact(&root) else {
        if ever_armed {
            return (
                Vec::new(),
                Some(format!(
                    "{} is absent on a workspace that has been armed ({} is present) — \
                     the armed law cannot be read",
                    fs::domain::ARMED_RULES_PATH,
                    fs::domain::ATTESTED_MARKER_PATH,
                )),
            );
        }
        return (Vec::new(), None);
    };
    match parse_artifact(&text) {
        Ok(artifact) => {
            let armed = artifact
                .rows()
                .iter()
                .map(|row| ArmedPage {
                    page: row.page().to_owned(),
                    mode: row.mode(),
                    attested_rev: row.rev().to_owned(),
                })
                .collect();
            (armed, None)
        }
        Err(corrupt) => (Vec::new(), Some(corrupt.detail)),
    }
}

/// Whether an armed row's live PAGE rev differs from the rev the artifact pinned
/// (the arming drift gate, read-only). A missing page — the pinned evidence
/// vanished — counts as drift (fail-closed: the armed law can no longer be
/// verified at its rev).
fn page_drifted(workspace: &Path, page: &str, attested_rev: &str) -> bool {
    match std::fs::read_to_string(workspace.join(page)) {
        Ok(text) => page_rev(&text) != attested_rev,
        Err(_) => true,
    }
}

/// Render the anchor-qualified tip axis (U2.7) for the workspace, plus any nudge
/// hint. Fetch-less: `run_observed` is ALWAYS false, so the state is never
/// `verified` and never renders a bare `at-tip` (the W-C1 invariant). Returns a
/// custom "no origin ref" render when the remote-tracking ref is absent (no tip to
/// compare) rather than a misleading bare position.
fn anchor_axis(workspace: &Path) -> (String, Option<&'static str>) {
    let branch = git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let head = git(workspace, &["rev-parse", "HEAD"]);
    let origin_ref = format!("refs/remotes/origin/{branch}");
    let origin = git(
        workspace,
        &["rev-parse", "--verify", "--quiet", &origin_ref],
    );

    let now_unix = now_unix();
    // No dated observation exists to carry: the row extractor that supplied one
    // died with the engine's memory (U4; ZT 2026-08-03), and status does not
    // fetch, so it has observed nothing itself. `None` classifies to
    // `AsKnownAgeless` — "the ref is here, I cannot say how current it is" —
    // which is the true state and the one that keeps its nudge. `classify` is
    // untouched: a caller that CAN date an observation still classifies exactly
    // as before.
    let facts = AnchorFacts {
        run_observed: false,
        last_observation: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use model::selector::GreyReason;
    use serde_json::Value;

    // ── the grey render fixtures (U6.2 full color law) ───────────────────────

    /// A grey renders through the composed line's pin-color axis carrying its
    /// contract label (colors amendment § Colors), not a bare tone. The render
    /// function carries the FULL color law, not just the armed set's green/red.
    ///
    /// **This test asserts no COUNT, and its name must not acquire one.** Whether
    /// every `GreyReason` variant HAS a label is a different question, and it is
    /// already owned by the compiler: `view::walk::color_reason` is an exhaustive
    /// match with no wildcard arm, so a new variant cannot land without breaking
    /// the build there. A second exhaustive match here would be a COPY of that
    /// guarantee, which `view::walk`'s own doc comment names as the defect —
    /// *"two `match`es over one enum is how a board and a walk start
    /// disagreeing"*. The cases below are a sample of the render path, not a
    /// census of the enum.
    #[test]
    fn a_grey_renders_through_the_composed_line_with_its_contract_label() {
        let cases = [
            (GreyReason::ImmutableRoot, "grey immutable-root"),
            (GreyReason::Ambiguous, "grey ambiguous"),
        ];
        for (reason, expected) in cases {
            let line = compose(
                Color::Grey(reason),
                "at-tip (anchor unverified)",
                Mode::Block,
            );
            assert!(
                line.contains(expected),
                "grey variant renders {expected}: {line}"
            );
        }
    }

    /// The composed multi-axis line renders every axis side by side, never
    /// merged — armed-pin color, meridian-lock color, anchor state, armed
    /// mode (U6.2).
    #[test]
    fn composed_line_shows_every_axis_side_by_side() {
        let line = compose(
            Color::Red(RedReason::Drifted),
            "behind (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d)",
            Mode::Warn,
        );
        assert_eq!(
            line,
            "pin red content-drifted · lock none · anchor behind (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d) · armed warn · vibe-debt 0 blobs (0 bytes)"
        );
    }

    /// The green, off, at-tip clean composition.
    #[test]
    fn composed_line_green_clean() {
        let line = compose(Color::Green, "at-tip (anchor as-known)", Mode::Off);
        assert_eq!(
            line,
            "pin green · lock none · anchor at-tip (anchor as-known) · armed off · vibe-debt 0 blobs (0 bytes)"
        );
    }

    /// Compose the axes without an IO edge — the pure render under test by the
    /// fixtures.
    fn compose(pin: Color, anchor_axis: &str, mode: Mode) -> String {
        report(pin, LockAxis::roll_up(&[]), anchor_axis, mode).composed_line()
    }

    /// A pure [`StatusReport`] with no IO edge — the render fixtures' subject.
    fn report(pin: Color, lock: LockAxis, anchor_axis: &str, mode: Mode) -> StatusReport {
        report_with_debt(pin, lock, anchor_axis, mode, VibeDebt::clear())
    }

    /// The same pure report with a chosen vibe-debt reading.
    fn report_with_debt(
        pin: Color,
        lock: LockAxis,
        anchor_axis: &str,
        mode: Mode,
        vibe_debt: VibeDebt,
    ) -> StatusReport {
        StatusReport {
            workspace: "/ws".to_owned(),
            source: "git-root".to_owned(),
            armed: 0,
            drifted: 0,
            artifact_fault: None,
            pin_rollup: pin,
            lock,
            vibe_debt,
            mode_rollup: mode,
            anchor_axis: anchor_axis.to_owned(),
            nudge: None,
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
                Mode::Off,
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
                Mode::Off,
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
            Mode::Off,
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
            Mode::Off,
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
                Mode::Off,
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
    /// unchanged by any reading, and `findings` stays false. The armed mode is not
    /// in this stage at all.
    #[test]
    fn vibe_debt_is_never_a_finding() {
        let owed = report_with_debt(
            Color::Green,
            LockAxis::roll_up(&[]),
            "at-tip (anchor as-known)",
            Mode::Block,
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

    // ── the forced-since-realise DISCLOSURE (ZT 2026-08-03) ──────────────────

    /// **The disclosure constants are WELL-FORMED PROSE, asserted.** They ship
    /// verbatim to both faces, so a stray run of whitespace is not cosmetic — it
    /// is malformed output on the one line the advisor ruled mandatory, and it
    /// reads as carelessness about exactly the claim being made carefully.
    ///
    /// This exists because it happened: the `why` constant shipped with a
    /// six-space run in the middle of a sentence, in both the human line and the
    /// JSON value, and neither a compiler, `-D warnings`, nor a reviewer reading
    /// the source caught it — a broken string LOOKS fine in source and only
    /// shows up rendered.
    #[test]
    fn the_disclosure_constants_are_well_formed() {
        for (name, text) in [
            ("FORCED_NOT_TRACKED", FORCED_NOT_TRACKED),
            ("FORCED_NOT_TRACKED_WHY", FORCED_NOT_TRACKED_WHY),
        ] {
            assert!(
                !text.contains("  "),
                "{name} carries a run of whitespace and ships to both faces: {text:?}"
            );
            assert_eq!(
                text.trim(),
                text,
                "{name} has stray edge whitespace: {text:?}"
            );
            assert!(
                !text.contains('\n'),
                "{name} is a single rendered line: {text:?}"
            );
        }
    }

    /// The human line states the axis is not tracked AND why. A reader who used
    /// to read a count must not be able to read this line as a clean zero.
    #[test]
    fn the_human_line_discloses_that_forced_writes_are_not_tracked() {
        let report = report(Color::Green, LockAxis::roll_up(&[]), "at-tip", Mode::Off);
        let out = report.render_human();
        assert!(
            out.contains("forced-since-realise: not-tracked"),
            "the axis is named and disclosed, not dropped: {out}"
        );
        assert!(
            out.contains("the engine keeps no memory by design"),
            "and the line carries the reason: {out}"
        );
        assert!(
            !out.contains("0 forced-since-realise"),
            "it must never render as a count: {out}"
        );
    }

    /// The `--json` face carries the same disclosure, and carries NO count and
    /// NO violations array. An absent key reads as "not checked"; a `0` or an
    /// empty array would read as "checked, none found", which is the lie.
    #[test]
    fn the_json_face_discloses_rather_than_zeroing() {
        let report = report(Color::Green, LockAxis::roll_up(&[]), "at-tip", Mode::Off);
        let v: Value = serde_json::from_str(&report.json()).expect("json");
        let forced = &v["armed_rules"]["forced_since_realise"];
        assert_eq!(forced["tracked"], serde_json::json!(false));
        assert_eq!(forced["state"], serde_json::json!("not-tracked"));
        assert!(
            forced["why"].as_str().is_some_and(|w| w.contains("git")),
            "the reason names where the answer lives now: {forced}"
        );
        assert!(
            !forced.is_number(),
            "the key must not be a count any more: {forced}"
        );
        assert!(
            v["armed_rules"].get("boundary").is_none(),
            "the receipts boundary bounded a count that no longer exists: {v}"
        );
        assert!(
            v.get("violations").is_none(),
            "an empty violations array would read as 'looked, found none': {v}"
        );
    }

    /// A forced write cannot move the exit code any more, because nothing
    /// observes one. The predicate is drift and artifact fault only — asserted
    /// so a later edit cannot quietly re-add a source nothing feeds.
    #[test]
    fn the_finding_predicate_is_drift_and_fault_only() {
        let mut report = report(Color::Green, LockAxis::roll_up(&[]), "at-tip", Mode::Off);
        assert!(!report.has_findings(), "a clean workspace exits 0");
        report.drifted = 1;
        assert!(report.has_findings(), "drift is still a finding");
        report.drifted = 0;
        report.artifact_fault = Some("corrupt".to_owned());
        assert!(
            report.has_findings(),
            "a faulted artifact is still a finding"
        );
    }
}
