//! `mrd status` — the bare, pure-local drift + freshness summary.
//!
//! ```text
//! mrd status [--json] [--cwd PATH]
//! ```
//!
//! # What bare `status` is
//! `status = freshness` (distinct from `check = validity`): what is armed, what
//! drifted, and how fresh the origin knowledge is — from frozen facts only.
//!
//! - **pure-local** — no daemon, no network, no fetch;
//! - **O(armed) + O(pins) + O(objects)** — one index read
//!   (`conventions/INDEX.md`) for the armed set, one re-hash per armed
//!   convention's `CHECK.md`, and the git refs. The `meridian-lock` planes add
//!   one corpus build of this workspace, plus one per mount root that this
//!   corpus's own lock addresses actually name ([`lock_addressed_roots`]). The
//!   vibe-debt gauge costs at most two git calls per object store (one
//!   `rev-list`, one batched `cat-file`), never a call per blob;
//! - **fetch-less** — the anchor axis is therefore never `verified` and never
//!   renders a bare `at-tip` (W-C1);
//! - **predicate-free** — it never evaluates a `check:`. Drift here is a
//!   mechanical rev compare, never a starlark run.
//!
//! # The composed legend — four axes on one surface (U6.2)
//! The orthogonal axes render side by side, never merged, each rolled up
//! worst-of independently:
//!
//! - **pin color** — the armed set's evidence drift: `green` (every armed row's
//!   live rule page rev still equals the rev the artifact pinned) or
//!   `red content-drifted`.
//! - **lock color** — the `meridian-lock` pins' fingerprint verdicts
//!   ([`LockAxis`]), rolled up red > grey > green. A different source and a
//!   different compare from the armed-set `pin` axis, so neither subsumes the
//!   other and neither changes the other's roll-up.
//! - **anchor state** — the origin-freshness qualifier: `as-known` /
//!   `unverified`, never `verified` (status cannot fetch). This is where origin
//!   tip-compare currency lives, for every axis on the line.
//! - **armed mode** — the worst armed mode (`off` / `warn` / `block` / `armed`),
//!   and one violation row per `--force`-escaped skip.
//! - **vibe debt** — the quantity axis ([`VibeDebt`]): how many lock-referenced
//!   blobs git holds that no commit reaches, and how many bytes they are. A
//!   meter, never a gate: it never enters the exit triad.
//!
//! # The armed summary line
//! `<A> armed · <D> drifted · forced-since-realise: not tracked`:
//! - `armed` — the row count of the attested armed-rules artifact
//!   ([`fs::domain::ARMED_RULES_PATH`]);
//! - `drifted` — armed rows whose live page rev ≠ the rev the artifact pinned;
//! - `forced-since-realise` — rendered as explicitly not tracked, with the
//!   reason: the engine keeps no memory, so a forced write between two locks is
//!   not a thing `status` can observe.
//!
//! # Exit triad
//! - **0** — clean: nothing armed drifted and the armed law is readable.
//! - **1** — a finding: an armed rule drifted, or the armed-rules artifact
//!   faulted.
//! - **2** — bad invocation, or an unresolvable / unreadable workspace.

use std::collections::{BTreeMap, HashSet};
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

/// Run `mrd status [--json] [--cwd PATH]`. Errors [`Fail`] exit 2 on a bad invocation or an
/// unresolvable / unreadable workspace; exit 1 when the summary carries a finding (drift, or a
/// faulted INDEX).
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

/// Parse `[--json] [--cwd PATH]` — the bare form only (no page positional).
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

/// What the forced-since-realise axis says. One constant, used by both faces, so
/// the human line and the `--json` face cannot disagree about what was checked.
const FORCED_NOT_TRACKED: &str = "not-tracked";

/// Why it is not tracked — rendered, not implied.
const FORCED_NOT_TRACKED_WHY: &str = "the engine keeps no memory by design — \
                                      a forced write between two locks is not history; look in git";

/// The `meridian-lock` axis (U6.2) — the corpus's lock pins rolled up worst-of.
/// Orthogonal to [`StatusReport::pin_rollup`], never merged: `pin` rolls up the
/// armed set's evidence drift; `lock` rolls up the fingerprint verdicts of every
/// `meridian-lock` pin in the corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LockAxis {
    /// Lock rows colored — pins plus any page-level lock-refusal row.
    rows: usize,
    /// The worst-of color across those rows — `None` when there are none. Not a
    /// color: a vault with no lock pins has nothing to verify, and green there
    /// would claim an attestation nobody made.
    rollup: Option<Color>,
    /// Set when the corpus itself could not be read — reported as such rather
    /// than as an empty (falsely clean) axis.
    unreadable: Option<String>,
}

impl LockAxis {
    /// Roll up `colors` worst-of: red (a measured defect) over grey (never
    /// measured) over green. Grey above green is load-bearing — one unverifiable
    /// pin must not hide inside a green fleet.
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

    /// The axis word: the worst-of label plus the row count, or the empty /
    /// unreadable case. The count is bracketed because a reason already carries
    /// its own parenthesized detail.
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

/// The vibe-debt gauge (U6.2) — how much of the corpus's retrieval plane is held
/// by nothing but this machine. A `--vibe` pin writes its blob eagerly
/// (`git hash-object -w`) so the pin can be verified before the file is
/// committed. That blob is reachable from no ref, so it survives only until
/// `git gc` ages it past the repository's local `gc.pruneExpire`
/// ([`receipt::anchor::PENDING_ANCHOR_TTL`]).
#[derive(Debug, Clone, PartialEq, Eq)]
struct VibeDebt {
    /// Distinct lock-referenced blobs present in the object database and
    /// reachable from no commit.
    blobs: usize,
    /// Their total size, git's own byte count.
    bytes: u64,
    /// Set when reachability could not be measured (no git, not a repository, an
    /// unreadable corpus, or a root whose object store cannot be named or
    /// asked): the gauge reports unknown, never a false `0`.
    unknown: Option<String>,
}

impl VibeDebt {
    /// Nothing owed. Renders and serializes exactly like any other reading — a
    /// gauge that hides at zero is not a gauge.
    fn clear() -> VibeDebt {
        VibeDebt {
            blobs: 0,
            bytes: 0,
            unknown: None,
        }
    }

    /// Unmeasurable — the reachability question could not be asked. Never
    /// collapsed into `0`.
    fn unknown(detail: String) -> VibeDebt {
        VibeDebt {
            blobs: 0,
            bytes: 0,
            unknown: Some(detail),
        }
    }

    /// The gauge word: the count, the bytes, or the unknown case.
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
    /// `ephemeral`.
    source: String,
    /// Count of `[x]` rows in the attested INDEX (the armed set).
    armed: usize,
    /// Count of armed conventions whose live `CHECK.md` rev ≠ pinned `armed_rev`.
    drifted: usize,
    /// The armed-rules artifact's fault detail: a corrupt artifact, or an
    /// artifact missing on a workspace the once-armed marker says has been
    /// armed. Absent artifact and absent marker is genesis — not a fault.
    artifact_fault: Option<String>,
    /// The pin-color axis roll-up — `Green` all-fresh, `Red(Drifted)` any-drift.
    pin_rollup: Color,
    /// The meridian-lock axis — the corpus's lock pins, rolled up worst-of.
    lock: LockAxis,
    /// The vibe-debt gauge — lock-referenced blobs no commit reaches.
    vibe_debt: VibeDebt,
    /// The armed-mode axis roll-up — the worst armed mode.
    mode_rollup: Mode,
    /// The rendered anchor-qualified tip axis — never a bare `at-tip`.
    anchor_axis: String,
    /// The as-known-ageless nudge hint, when present.
    nudge: Option<&'static str>,
}

impl StatusReport {
    /// The exit-1 predicate: armed evidence drifted, or the armed-rules artifact
    /// faulted. A forced write is not in it — nothing observes one.
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
    /// anchor state · armed mode · vibe debt, side by side, never merged.
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

    /// The `--json` shape: the axes as fields, the counts, and the disclosure.
    fn json(&self) -> String {
        let doc = json!({
            "workspace": self.workspace,
            "source": self.source,
            "armed_rules": {
                "armed": self.armed,
                "drifted": self.drifted,
                // A string, not a count: a `0` would read as "checked, none
                // found" about a property nothing observes.
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
                // Always emitted: an empty vault reports 0 pins and a null color,
                // never an absent field a reader could mistake for "not checked".
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
                // Always emitted: a corpus with nothing owed reports 0 blobs and
                // 0 bytes — a gauge that hides at zero is not a gauge.
                "vibe_debt": {
                    "blobs": self.vibe_debt.blobs,
                    "bytes": self.vibe_debt.bytes,
                    "label": self.vibe_debt.render(),
                    "unknown": self.vibe_debt.unknown,
                },
            },
            "nudge": self.nudge,
            // No `violations` key: an empty array would read as "looked, found
            // none".
            "findings": self.has_findings(),
        });
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
    }
}

/// Gather the status summary over a canonical workspace path. The impure edge:
/// it reads the armed-rules artifact, the once-armed marker, the armed rule
/// pages, and the git refs — every read frozen, none re-evaluated.
fn gather(workspace: &Path, source: &str) -> StatusReport {
    let (armed, artifact_fault) = read_armed(workspace);

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

    let (anchor_axis, nudge) = anchor_axis(workspace);

    // One corpus build answers both the pin colors (claim plane) and the
    // vibe-debt gauge (retrieval plane).
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

/// Read both `meridian-lock` planes of the workspace over one corpus build: the
/// claim plane's pin colors rolled up worst-of ([`LockAxis`]) and the retrieval
/// plane's unreachable blobs ([`VibeDebt`]). The one place status leaves the
/// O(armed) set — the same builder `mrd walk` uses, so the two cannot disagree.
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
    // The real mount table, through the one loader `mrd walk` and `mrd check`
    // use, so the planes agree by construction. The corpora are narrowed to the
    // roots this corpus's own lock addresses name; the table itself is not.
    let mounts = crate::walk_cmd::load_mounts_for(&crate::walk_cmd::lock_addressed_roots(&docs));
    let corpus = mounts.rooted(&docs);
    let colors: Vec<Color> = view::walk::lock_pin_colors_rooted(&corpus, mounts.set())
        .into_iter()
        .map(|p| p.color)
        .collect();
    (LockAxis::roll_up(&colors), vibe_debt(workspace, &docs))
}

/// Which object store one pinned blob belongs to: the ambient workspace
/// (`None`), or a named root (`Some`) — the blob-anchoring check runs against
/// that root's git repo.
type StoreKey = Option<addr::MountName>;

/// Measure the vibe debt: the lock-referenced blobs git holds that no commit
/// reaches, counted and summed in bytes.
///
/// Two git calls at most per store, never a call per blob: one
/// `git rev-list --objects --all` into the reachable set and one batched
/// `git cat-file --batch-check`. Only `ObjectAnchor::PendingAnchor` is debt.
/// Entries are grouped by the root their key names, and each group is asked of
/// that root's repository ([`StoreKey`]). A value that is not an object id, or a
/// key that names no store, is unknown, never skipped — a dropped entry would
/// read a corrupt retrieval plane as a true zero.
fn vibe_debt(workspace: &Path, docs: &BTreeMap<String, Document>) -> VibeDebt {
    // Dedupe is keyed by store, never globally: the same oid under two roots is
    // two objects in two databases.
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
        // The refusal is carried, never swallowed — falling back to the ambient
        // store would ask the wrong database and answer confidently.
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

    // The mount table is read once, and only when a key actually names a root: a
    // corpus whose every key is ambient asks the config plane nothing.
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
        // One handle per root, so a store's reachable set can never be read
        // against another store's presence answer.
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

/// The local path of the git repository backing one named root. A mount lookup
/// and nothing more: the table maps canonical name → local path.
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
    // Declared but unusable is a different cause with a different fix. The mount
    // plane's own sentence is carried verbatim, so this gauge and `mrd config`
    // say the same thing about the same root.
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

/// The bound mount table, or `None` when this machine has none to read. Absence
/// is not a failure of the gauge — [`store_path`] then says so in words.
fn load_mount_table() -> Option<config::mount::MountTable> {
    let resolution = config::resolve(&config::Env::from_process()).ok()?;
    let cfg = resolution.config()?;
    config::mount::bind(cfg).ok()
}

/// A git failure while asking one store, with the root named: "not a git
/// repository" is only actionable once the reader knows which repo was asked.
fn store_fail(store: &StoreKey, fail: &git::GitFail) -> String {
    match store {
        None => fail.to_string(),
        Some(name) => format!("root `{name}`: {fail}"),
    }
}

/// The `unknown` detail for pinned blobs git cannot be asked about — the count,
/// the reason clause, and the first offender's page and key. `None` when there
/// are no offenders.
fn cannot_ask_detail(offenders: &[String], because: &str) -> Option<String> {
    let first = offenders.first()?;
    let n = offenders.len();
    let unit = if n == 1 { "entry" } else { "entries" };
    Some(format!("{n} pinned {unit} {because} (first: {first})"))
}

/// One armed row read from the artifact: which page was attested, at which rev,
/// and in which mode. Flattened out of [`policy::armed::ArmedRow`] because status
/// keeps no policy value alive past the read — it renders counts, not law.
struct ArmedPage {
    page: String,
    mode: Mode,
    attested_rev: String,
}

/// Read the attested armed-rules artifact into the armed set. The genesis
/// reading pivots on the marker, never on the artifact: absent artifact and
/// absent marker is a never-armed workspace — nothing armed, no fault, clean.
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

/// Whether an armed row's live page rev differs from the rev the artifact pinned.
/// A missing page counts as drift (fail-closed: the armed law can no longer be
/// verified at its rev).
fn page_drifted(workspace: &Path, page: &str, attested_rev: &str) -> bool {
    match std::fs::read_to_string(workspace.join(page)) {
        Ok(text) => page_rev(&text) != attested_rev,
        Err(_) => true,
    }
}

/// Render the anchor-qualified tip axis for the workspace, plus any nudge hint.
/// Fetch-less: `run_observed` is always false, so the state is never `verified`
/// and never renders a bare `at-tip` (the W-C1 invariant). Renders "no origin
/// ref" when the remote-tracking ref is absent, rather than a bare position.
fn anchor_axis(workspace: &Path) -> (String, Option<&'static str>) {
    let branch = git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let head = git(workspace, &["rev-parse", "HEAD"]);
    let origin_ref = format!("refs/remotes/origin/{branch}");
    let origin = git(
        workspace,
        &["rev-parse", "--verify", "--quiet", &origin_ref],
    );

    let now_unix = now_unix();
    // No dated observation exists to carry: `None` classifies to `AsKnownAgeless`,
    // which keeps its nudge.
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
        // No remote-tracking ref (or no HEAD): no tip to compare, so render the
        // anchor state alone rather than a fabricated position.
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
/// on any failure (missing ref, no repo, non-zero exit) — a git failure renders
/// `unverified`, never a fabricated position.
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

    #[test]
    fn composed_line_green_clean() {
        let line = compose(Color::Green, "at-tip (anchor as-known)", Mode::Off);
        assert_eq!(
            line,
            "pin green · lock none · anchor at-tip (anchor as-known) · armed off · vibe-debt 0 blobs (0 bytes)"
        );
    }

    /// Compose the axes without an IO edge.
    fn compose(pin: Color, anchor_axis: &str, mode: Mode) -> String {
        report(pin, LockAxis::roll_up(&[]), anchor_axis, mode).composed_line()
    }

    /// A pure [`StatusReport`] with no IO edge.
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
        // Orthogonal: a grey lock axis never repaints the armed-set pin axis.
        assert_eq!(drifted["composed"]["pin_color"], json!("green"));
    }

    // ── S11: the vibe-debt gauge (U6.2, a quantity not a color) ─────────────

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
        // Orthogonal: debt repaints no color axis.
        assert_eq!(v["composed"]["pin_color"], json!("green"));
        assert_eq!(v["composed"]["lock"]["color"], json!("green"));
    }

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

    // ── the forced-since-realise DISCLOSURE ──────────────────────────────────

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
