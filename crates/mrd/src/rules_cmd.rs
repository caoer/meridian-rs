//! `mrd rules replay` — corpus replay for the extension kernel (decisions/0003 §
//! Testing methodology, the third layer above fixtures).
//!
//! # What it does
//! Walk a workspace's real history as a stream of consecutive corpus states,
//! synthesize the [`rules::ChangeEvent`] stream from the per-file diffs, run a
//! rule set over every event, and report the aggregate: which rules NEVER fired
//! (dead-rule detection), per-rule fire counts, the effect-kind distribution,
//! and the fuel-consumption profile. Fixtures say "my cases pass"; replay says
//! "against my tree's actual traffic, rule X is dead, rule Y fires 400×."
//!
//! # Two state sources, one replay core
//! - **git** (default): `git rev-list --reverse --first-parent` over the
//!   workspace, diffing each commit against its first parent (the root commit
//!   against the empty tree). Renames surface as delete+create — no `-M`, so a
//!   moved doc reads as an honest pair of change facts. Streaming: only one
//!   commit's changed-file bytes live in memory at a time.
//! - **snapshots** (`--snapshots DIR`): an ordered set of snapshot subdirs, each
//!   a full corpus state, diffed consecutively (an empty baseline precedes the
//!   first, so its files are `created` events). Committable and public-safe —
//!   the synthetic-corpus CI lane and the determinism/dead-rule tests use it.
//!
//! Both sources feed the SAME per-file synthesis + aggregation, so a rule's
//! profile is source-independent. The report is a pure function of
//! `(states, rules)` — replaying the same history twice is byte-identical
//! (determinism inherited from the kernel), so no wall-clock stamp enters the
//! report body.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use model::delta::{FileChangeKind, FileDelta, file_delta};
use model::{Document, NodeKind, Ref};
use rules::{ChangeEvent, EvalLimits, Rule};
use serde_json::json;

use crate::{Fail, current_dir};

/// The well-known SHA-1 of git's empty tree — the "before" tree the root commit
/// is diffed against, so its files surface as `created` change facts.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Run `mrd rules <sub>` — currently only `replay`.
///
/// # Errors
/// A missing / unknown subcommand, or the replay itself failing (see
/// [`run_replay`]).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        return Err(Fail::tool("rules needs a subcommand (replay)".to_owned()));
    };
    match sub.as_str() {
        "replay" => run_replay(&args[1..]),
        other => Err(Fail::tool(format!("unknown rules subcommand: {other}"))),
    }
}

/// The parsed tail of `mrd rules replay`.
struct ReplayArgs {
    /// The git workspace to replay (positional; default cwd). Ignored when
    /// `--snapshots` selects the snapshot source.
    path: Option<String>,
    /// An ordered snapshot-dir corpus instead of git history.
    snapshots: Option<String>,
    /// The directory of `.star` rule files to replay (required).
    rules_dir: Option<String>,
    /// Write the report here instead of stdout.
    out: Option<String>,
    /// Emit the machine summary as JSON instead of the markdown report.
    json: bool,
}

impl ReplayArgs {
    /// Parse `argv` after `rules replay`. Value flags take the next token; an
    /// unknown flag or a missing value is a loud exit-2.
    fn parse(tail: &[String]) -> Result<Self, Fail> {
        let mut parsed = ReplayArgs {
            path: None,
            snapshots: None,
            rules_dir: None,
            out: None,
            json: false,
        };
        let mut i = 0;
        while i < tail.len() {
            let arg = tail[i].as_str();
            match arg {
                "--json" => parsed.json = true,
                "--snapshots" => parsed.snapshots = Some(take_value(tail, &mut i, "--snapshots")?),
                "--rules" => parsed.rules_dir = Some(take_value(tail, &mut i, "--rules")?),
                "--out" => parsed.out = Some(take_value(tail, &mut i, "--out")?),
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if parsed.path.is_none() => parsed.path = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
            i += 1;
        }
        Ok(parsed)
    }
}

/// Consume the value token following a value flag, advancing the cursor.
fn take_value(tail: &[String], i: &mut usize, flag: &str) -> Result<String, Fail> {
    *i += 1;
    tail.get(*i)
        .cloned()
        .ok_or_else(|| Fail::tool(format!("{flag} needs a value")))
}

/// Run `mrd rules replay`: load the rule set, drive the selected state source
/// through the replay core, and emit the markdown report (or a JSON summary).
///
/// # Errors
/// The rules dir is missing / empty / unreadable, the state source cannot be
/// resolved or read, or the report cannot be written to `--out`.
fn run_replay(tail: &[String]) -> Result<(), Fail> {
    let args = ReplayArgs::parse(tail)?;
    let rules_dir = args
        .rules_dir
        .as_ref()
        .ok_or_else(|| Fail::tool("rules replay needs --rules DIR".to_owned()))?;
    let rules = load_rules(Path::new(rules_dir))?;

    let source = if let Some(dir) = &args.snapshots {
        Source::Snapshots {
            dir: PathBuf::from(dir),
        }
    } else {
        let cwd = current_dir()?;
        let base = match &args.path {
            Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
            Some(p) => cwd.join(p),
            None => cwd,
        };
        let workspace = workspace::canonicalize(&base).map_err(|e| {
            Fail::tool(format!("cannot resolve workspace {} ({e})", base.display()))
        })?;
        Source::Git { workspace }
    };

    let report = replay(&source, &rules)?;
    let rendered = if args.json {
        report.to_json()
    } else {
        report.to_markdown()
    };

    if let Some(out) = &args.out {
        std::fs::write(out, &rendered)
            .map_err(|e| Fail::tool(format!("cannot write report to {out}: {e}")))?;
        println!(
            "wrote {out} ({} events over {} rules)",
            report.events,
            rules.len()
        );
    } else {
        print!("{rendered}");
    }
    Ok(())
}

/// Load every `.star` rule file directly under `dir`, sorted by file stem (the
/// rule id) so the rule order — and thus the report — is deterministic.
///
/// # Errors
/// The dir is unreadable, holds no `.star` file, or a rule file cannot be read.
fn load_rules(dir: &Path) -> Result<Vec<Rule>, Fail> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| Fail::tool(format!("cannot read rules dir {}: {e}", dir.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("star"))
        })
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Err(Fail::tool(format!(
            "no .star rule files in {}",
            dir.display()
        )));
    }
    let mut rules = Vec::with_capacity(entries.len());
    for path in entries {
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rule")
            .to_owned();
        let source = std::fs::read_to_string(&path)
            .map_err(|e| Fail::tool(format!("cannot read rule {}: {e}", path.display())))?;
        rules.push(Rule::new(id, source));
    }
    Ok(rules)
}

/// One file's before/after bytes at one step of history. `before == None` is a
/// creation, `after == None` a deletion.
struct FileChange {
    path: String,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}

/// The corpus-state source: real git history or an ordered snapshot fixture.
enum Source {
    Git { workspace: PathBuf },
    Snapshots { dir: PathBuf },
}

/// Metadata a source reports once, for the report frontmatter.
struct SourceMeta {
    label: &'static str,
    corpus: String,
    range: String,
}

/// Drive one replay: seed the aggregate from the rule set, stream the source's
/// per-file changes through the synthesis + eval core, and fold into a report.
///
/// # Errors
/// The source cannot be resolved or read (git failures, an unreadable snapshot
/// dir). A non-UTF-8 `.md` file is SKIPPED (counted), never fatal — the real
/// corpus must not be able to abort its own replay.
fn replay(source: &Source, rules: &[Rule]) -> Result<Report, Fail> {
    let limits = EvalLimits::default();
    let mut agg = Aggregate::new(rules);
    let meta = match source {
        Source::Git { workspace } => drive_git(workspace, &mut |fc| {
            process_change(&fc, rules, limits, &mut agg);
        })?,
        Source::Snapshots { dir } => drive_snapshots(dir, &mut |fc| {
            process_change(&fc, rules, limits, &mut agg);
        })?,
    };
    Ok(agg.into_report(meta, rules.len()))
}

/// Process one file change: build the before/after documents, derive the change
/// facts, synthesize the event, and fold every rule's metered outcome into the
/// aggregate. A non-UTF-8 file is counted and skipped; an unchanged pair (equal
/// file revs) is a no-op the delta layer reports as `None`.
fn process_change(fc: &FileChange, rules: &[Rule], limits: EvalLimits, agg: &mut Aggregate) {
    agg.file_changes += 1;
    let Ok(before) = decode_doc(fc.before.as_deref()) else {
        agg.non_utf8 += 1;
        return;
    };
    let Ok(after) = decode_doc(fc.after.as_deref()) else {
        agg.non_utf8 += 1;
        return;
    };
    let Some(delta) = file_delta(before.as_ref(), after.as_ref()) else {
        agg.unchanged += 1;
        return;
    };
    let event = synth_event(&fc.path, &delta, before.as_ref(), after.as_ref());
    agg.events += 1;

    for tel in rules::eval_telemetry(rules, &event, limits) {
        let ra = agg
            .rules
            .get_mut(&tel.rule_id)
            .expect("rule seeded in aggregate");
        ra.fuel_runs += 1;
        ra.fuel_sum = ra.fuel_sum.saturating_add(tel.fuel_used);
        ra.fuel_max = ra.fuel_max.max(tel.fuel_used);
        ra.mem_max = ra.mem_max.max(tel.mem_used);
        match tel.outcome {
            Ok(effects) => {
                if !effects.is_empty() {
                    ra.events_fired += 1;
                    ra.total_effects += effects.len() as u64;
                    for e in effects {
                        *agg.kind_counts.entry(e.kind.as_str()).or_insert(0) += 1;
                    }
                }
            }
            Err(e) => {
                ra.errors += 1;
                if ra.error_sample.is_none() {
                    ra.error_sample = Some(e.to_string());
                }
            }
        }
    }
}

/// Decode optional file bytes into a parsed document. `None` bytes ⇒ `None`
/// document (an absent side). `Err(())` on non-UTF-8 content (the caller skips).
fn decode_doc(bytes: Option<&[u8]>) -> Result<Option<Document>, ()> {
    match bytes {
        None => Ok(None),
        Some(b) => {
            let raw = std::str::from_utf8(b).map_err(|_| ())?.to_owned();
            let nodes = syntax::parse(&raw);
            Ok(Some(model::build(raw, nodes)))
        }
    }
}

/// Synthesize the semantic event for one file change (0003 §3). For a modified
/// file the changed sections/fields come from the node-grain delta; for a
/// created/deleted file — which the delta layer leaves node-less — the whole
/// document's section headings and frontmatter keys are the changed set (every
/// node is new / gone), so a rule keying on fields still sees a new doc's fields.
fn synth_event(
    path: &str,
    delta: &FileDelta,
    before: Option<&Document>,
    after: Option<&Document>,
) -> ChangeEvent {
    let (mut sections, mut fields) = match delta.change {
        FileChangeKind::Created => after.map(doc_inventory).unwrap_or_default(),
        FileChangeKind::Deleted => before.map(doc_inventory).unwrap_or_default(),
        FileChangeKind::Modified => delta_inventory(delta),
    };
    // Deterministic, duplicate-free change sets — the report and eval must not
    // depend on tree-walk order.
    dedup_sorted(&mut sections);
    dedup_sorted(&mut fields);
    ChangeEvent {
        file: path.to_owned(),
        sections_changed: sections,
        fields_changed: fields,
        fingerprint_before: delta
            .file_rev_before
            .as_ref()
            .map(|r| r.0.clone())
            .unwrap_or_default(),
        fingerprint_after: delta
            .file_rev_after
            .as_ref()
            .map(|r| r.0.clone())
            .unwrap_or_default(),
        depth: 0,
    }
}

/// The changed sections and fields of a modified file, from its node deltas: an
/// `hpath` or `anchor` target is a section path, an `fm_key` target is a field.
fn delta_inventory(delta: &FileDelta) -> (Vec<String>, Vec<String>) {
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    // Every node delta — Added / Edited / Removed alike — names a path that
    // changed at this event; the change class itself is not part of the
    // synthesized payload (0003 §3 carries only which sections / fields moved).
    for nd in &delta.nodes {
        match &nd.target {
            Ref::Hpath(segs) => sections.push(render_hpath(segs)),
            Ref::Anchor(id) => sections.push(format!("^{id}")),
            Ref::FmKey(key) => fields.push(key.clone()),
        }
    }
    (sections, fields)
}

/// Every section heading-path and frontmatter key in a document — the change set
/// for a whole-file creation or deletion.
fn doc_inventory(doc: &Document) -> (Vec<String>, Vec<String>) {
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    collect_inventory(&doc.root, &mut sections, &mut fields);
    (sections, fields)
}

fn collect_inventory(node: &model::Node, sections: &mut Vec<String>, fields: &mut Vec<String>) {
    match &node.kind {
        NodeKind::Section { .. } => {
            if let Some(hpath) = &node.hpath {
                sections.push(hpath.join("#"));
            }
        }
        NodeKind::Frontmatter { map } => {
            for key in map.keys() {
                fields.push(key.to_owned());
            }
        }
        _ => {}
    }
    for child in &node.children {
        collect_inventory(child, sections, fields);
    }
}

/// Render a mint-plane hpath as a `#`-joined heading path (`A#B`), appending the
/// 1-based occurrence index when the segment carries one (`A#B%2`).
fn render_hpath(segs: &[model::HpathSeg]) -> String {
    segs.iter()
        .map(|s| match s.n {
            Some(n) => format!("{}%{n}", s.h),
            None => s.h.clone(),
        })
        .collect::<Vec<_>>()
        .join("#")
}

/// Sort then dedup in place — a stable, duplicate-free change set.
fn dedup_sorted(v: &mut Vec<String>) {
    v.sort();
    v.dedup();
}

// ---------------------------------------------------------------------------
// git source
// ---------------------------------------------------------------------------

/// Drive git history: linear first-parent commits oldest→newest, diffing each
/// against its first parent (the root against the empty tree), emitting one
/// [`FileChange`] per changed `.md` path. Renames are not detected (`-M` off),
/// so a move is an honest delete + create pair.
///
/// # Errors
/// `git` is unavailable or the path is not a repository (`rev-list` fails).
fn drive_git(workspace: &Path, emit: &mut dyn FnMut(FileChange)) -> Result<SourceMeta, Fail> {
    let list = git_text(
        workspace,
        &["rev-list", "--reverse", "--first-parent", "HEAD"],
    )?;
    let commits: Vec<&str> = list.lines().filter(|l| !l.is_empty()).collect();
    let range = match (commits.first(), commits.last()) {
        (Some(f), Some(l)) => format!("{}..{}", short(f), short(l)),
        _ => "(no commits)".to_owned(),
    };

    let mut parent = EMPTY_TREE.to_owned();
    for commit in &commits {
        let status = git_text(
            workspace,
            &["diff", "--name-status", "--no-renames", &parent, commit],
        )?;
        for line in status.lines() {
            let mut cols = line.split('\t');
            let Some(code) = cols.next() else { continue };
            let Some(path) = cols.next() else { continue };
            if !is_md(path) {
                continue;
            }
            let tag = code.as_bytes().first().copied().unwrap_or(b'?');
            match tag {
                b'A' => emit(FileChange {
                    path: path.to_owned(),
                    before: None,
                    after: Some(git_bytes(workspace, commit, path)?),
                }),
                b'D' => emit(FileChange {
                    path: path.to_owned(),
                    before: Some(git_bytes(workspace, &parent, path)?),
                    after: None,
                }),
                // M, T (type change) and anything else with both sides present.
                _ => emit(FileChange {
                    path: path.to_owned(),
                    before: Some(git_bytes(workspace, &parent, path)?),
                    after: Some(git_bytes(workspace, commit, path)?),
                }),
            }
        }
        (*commit).clone_into(&mut parent);
    }

    Ok(SourceMeta {
        label: "git",
        corpus: workspace.display().to_string(),
        range,
    })
}

/// Run a git command that yields UTF-8 text (hashes, name-status).
fn git_text(workspace: &Path, args: &[&str]) -> Result<String, Fail> {
    let out = run_git(workspace, args)?;
    String::from_utf8(out).map_err(|e| Fail::tool(format!("git emitted non-UTF-8: {e}")))
}

/// Read one blob's raw bytes at `rev:path`. Bytes, not text — a `.md` file may
/// be non-UTF-8, which the replay core skips downstream.
fn git_bytes(workspace: &Path, rev: &str, path: &str) -> Result<Vec<u8>, Fail> {
    run_git(workspace, &["show", &format!("{rev}:{path}")])
}

/// Run `git -C workspace <args>`, capturing stdout bytes. A non-zero exit is a
/// tool failure carrying git's stderr.
fn run_git(workspace: &Path, args: &[&str]) -> Result<Vec<u8>, Fail> {
    let out = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .map_err(|e| Fail::tool(format!("cannot run git: {e}")))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(Fail::tool(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            stderr.trim()
        )))
    }
}

/// A commit's short form for the report range.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(10)]
}

/// Whether a repo-relative path is a markdown file (the addressable corpus set,
/// matching `fs::walk`).
fn is_md(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

// ---------------------------------------------------------------------------
// snapshot source
// ---------------------------------------------------------------------------

/// Drive an ordered snapshot corpus: each immediate subdir of `dir`, sorted by
/// name, is a full corpus state. An empty baseline precedes the first, so the
/// first state's files are `created` events; consecutive states are diffed
/// per-file (union of paths, byte comparison).
///
/// # Errors
/// `dir` is unreadable, or a snapshot file cannot be read.
fn drive_snapshots(dir: &Path, emit: &mut dyn FnMut(FileChange)) -> Result<SourceMeta, Fail> {
    let mut states: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| Fail::tool(format!("cannot read snapshots dir {}: {e}", dir.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    states.sort();

    let mut prev: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for state in &states {
        let curr = read_state(state)?;
        let paths: BTreeSet<&String> = prev.keys().chain(curr.keys()).collect();
        for path in paths {
            let before = prev.get(path).cloned();
            let after = curr.get(path).cloned();
            if before == after {
                continue;
            }
            emit(FileChange {
                path: path.clone(),
                before,
                after,
            });
        }
        prev = curr;
    }

    Ok(SourceMeta {
        label: "snapshots",
        corpus: dir.display().to_string(),
        range: format!("{} states", states.len()),
    })
}

/// Read one snapshot state: every `.md` file under `state`, keyed by its
/// forward-slashed path relative to `state`.
fn read_state(state: &Path) -> Result<BTreeMap<String, Vec<u8>>, Fail> {
    let mut out = BTreeMap::new();
    read_state_dir(state, state, &mut out)?;
    Ok(out)
}

fn read_state_dir(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), Fail> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| Fail::tool(format!("cannot read snapshot dir {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| Fail::tool(format!("cannot read snapshot entry: {e}")))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| Fail::tool(format!("cannot stat {}: {e}", path.display())))?;
        if file_type.is_dir() {
            read_state_dir(root, &path, out)?;
        } else if is_md(&path.to_string_lossy()) {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path)
                .map_err(|e| Fail::tool(format!("cannot read {}: {e}", path.display())))?;
            out.insert(rel, bytes);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// aggregation + report
// ---------------------------------------------------------------------------

/// Per-rule running totals over the whole replay.
#[derive(Default)]
struct RuleAgg {
    /// Events where the rule emitted at least one effect (it "fired").
    events_fired: u64,
    /// Total effect descriptors emitted across all events.
    total_effects: u64,
    /// Events where the rule faulted (a typed [`rules::EvalError`]).
    errors: u64,
    /// The first fault message seen (for the report's error sample).
    error_sample: Option<String>,
    /// Metered eval count (== events replayed once the rule set is fixed).
    fuel_runs: u64,
    /// Summed Starlark ticks (for the average).
    fuel_sum: u64,
    /// Peak single-event fuel.
    fuel_max: u64,
    /// Peak single-event eval-heap bytes.
    mem_max: u64,
}

/// The whole-replay accumulator, seeded from the rule set so a rule that never
/// runs still appears.
struct Aggregate {
    rules: BTreeMap<String, RuleAgg>,
    kind_counts: BTreeMap<&'static str, u64>,
    events: u64,
    file_changes: u64,
    unchanged: u64,
    non_utf8: u64,
}

impl Aggregate {
    fn new(rules: &[Rule]) -> Self {
        let mut map = BTreeMap::new();
        for r in rules {
            map.insert(r.id.clone(), RuleAgg::default());
        }
        Aggregate {
            rules: map,
            kind_counts: BTreeMap::new(),
            events: 0,
            file_changes: 0,
            unchanged: 0,
            non_utf8: 0,
        }
    }

    fn into_report(self, meta: SourceMeta, rule_count: usize) -> Report {
        let dead: Vec<String> = self
            .rules
            .iter()
            .filter(|(_, a)| a.events_fired == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let errored = self.rules.values().filter(|a| a.errors > 0).count();
        let total_effects: u64 = self.rules.values().map(|a| a.total_effects).sum();
        Report {
            meta,
            rule_count,
            events: self.events,
            file_changes: self.file_changes,
            unchanged: self.unchanged,
            non_utf8: self.non_utf8,
            dead_rules: dead,
            errored_rules: errored,
            total_effects,
            rules: self.rules,
            kind_counts: self.kind_counts,
        }
    }
}

/// The finished replay report — a pure function of `(states, rules)`.
struct Report {
    meta: SourceMeta,
    rule_count: usize,
    events: u64,
    file_changes: u64,
    unchanged: u64,
    non_utf8: u64,
    dead_rules: Vec<String>,
    errored_rules: usize,
    total_effects: u64,
    rules: BTreeMap<String, RuleAgg>,
    kind_counts: BTreeMap<&'static str, u64>,
}

impl Report {
    /// Render the markdown report: a frontmatter summary block then the tables
    /// (dead rules, fire counts, effect-kind distribution, fuel profile, errors).
    /// Deterministic — no wall-clock stamp; every table is sorted by a stable key.
    fn to_markdown(&self) -> String {
        let mut s = String::new();
        let fired = self.rule_count - self.dead_rules.len();
        // Frontmatter summary — filterable, per the house convention.
        let _ = write!(
            s,
            "---\n\
             tool: mrd rules replay\n\
             source: {}\n\
             corpus: {}\n\
             range: {}\n\
             rules: {}\n\
             events: {}\n\
             file_changes: {}\n\
             unchanged: {}\n\
             non_utf8_skipped: {}\n\
             fired_rules: {fired}\n\
             dead_rules: {}\n\
             errored_rules: {}\n\
             total_effects: {}\n\
             ---\n\n",
            self.meta.label,
            self.meta.corpus,
            self.meta.range,
            self.rule_count,
            self.events,
            self.file_changes,
            self.unchanged,
            self.non_utf8,
            self.dead_rules.len(),
            self.errored_rules,
            self.total_effects,
        );

        let _ = write!(
            s,
            "# Rules Replay Report\n\n\
             Replayed **{}** rule(s) over **{}** change event(s) from **{}** file change(s) (`{}` source, `{}`).\n\n",
            self.rule_count, self.events, self.file_changes, self.meta.label, self.meta.range
        );

        // Dead rules — the headline signal.
        s.push_str("## Dead rules (never fired)\n\n");
        if self.dead_rules.is_empty() {
            s.push_str("_none — every rule fired at least once._\n\n");
        } else {
            for id in &self.dead_rules {
                let _ = writeln!(s, "- `{id}`");
            }
            s.push('\n');
        }

        // Per-rule fire counts.
        s.push_str("## Per-rule fire counts\n\n");
        s.push_str("| rule | events_fired | total_effects | errors |\n");
        s.push_str("|------|-------------:|--------------:|-------:|\n");
        for (id, a) in &self.rules {
            let _ = writeln!(
                s,
                "| `{id}` | {} | {} | {} |",
                a.events_fired, a.total_effects, a.errors
            );
        }
        s.push('\n');

        // Effect-kind distribution.
        s.push_str("## Effect-kind distribution\n\n");
        if self.kind_counts.is_empty() {
            s.push_str("_no effects emitted._\n\n");
        } else {
            s.push_str("| kind | count |\n|------|------:|\n");
            for (kind, count) in &self.kind_counts {
                let _ = writeln!(s, "| `{kind}` | {count} |");
            }
            s.push('\n');
        }

        // Fuel-consumption profile.
        s.push_str("## Fuel consumption profile\n\n");
        s.push_str("| rule | runs | fuel_sum | fuel_max | fuel_avg | mem_max_bytes |\n");
        s.push_str("|------|-----:|---------:|---------:|---------:|--------------:|\n");
        for (id, a) in &self.rules {
            let avg = a.fuel_sum.checked_div(a.fuel_runs).unwrap_or(0);
            let _ = writeln!(
                s,
                "| `{id}` | {} | {} | {} | {avg} | {} |",
                a.fuel_runs, a.fuel_sum, a.fuel_max, a.mem_max
            );
        }
        s.push('\n');

        // Errors (only rules that faulted).
        let errored: Vec<(&String, &RuleAgg)> =
            self.rules.iter().filter(|(_, a)| a.errors > 0).collect();
        if !errored.is_empty() {
            s.push_str("## Errors\n\n");
            s.push_str("| rule | count | sample |\n|------|------:|--------|\n");
            for (id, a) in errored {
                let sample = a
                    .error_sample
                    .as_deref()
                    .unwrap_or("")
                    .replace('|', "\\|")
                    .replace('\n', " ");
                let _ = writeln!(s, "| `{id}` | {} | {sample} |", a.errors);
            }
            s.push('\n');
        }

        s
    }

    /// The machine summary: the frontmatter fields plus per-rule fire counts and
    /// the kind distribution, as a JSON object (for `--json` tooling).
    fn to_json(&self) -> String {
        let rules: BTreeMap<&String, serde_json::Value> = self
            .rules
            .iter()
            .map(|(id, a)| {
                (
                    id,
                    json!({
                        "events_fired": a.events_fired,
                        "total_effects": a.total_effects,
                        "errors": a.errors,
                        "fuel_sum": a.fuel_sum,
                        "fuel_max": a.fuel_max,
                        "mem_max": a.mem_max,
                    }),
                )
            })
            .collect();
        let value = json!({
            "source": self.meta.label,
            "corpus": self.meta.corpus,
            "range": self.meta.range,
            "rules": self.rule_count,
            "events": self.events,
            "file_changes": self.file_changes,
            "unchanged": self.unchanged,
            "non_utf8_skipped": self.non_utf8,
            "dead_rules": self.dead_rules,
            "errored_rules": self.errored_rules,
            "total_effects": self.total_effects,
            "effect_kinds": self.kind_counts,
            "per_rule": rules,
        });
        format!("{}\n", serde_json::to_string_pretty(&value).expect("json"))
    }
}
