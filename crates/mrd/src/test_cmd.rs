//! `mrd test` — the tier-1 scenario runner (U1.2): calibrate conventions and the
//! write plane against literate scenario files, with fidelity as the point.
//!
//! # What a scenario is
//! A scenario is a markdown file: YAML frontmatter (`scenario`, `convention_rev`,
//! `cas`, `pairs`) plus fenced blocks —
//!
//! - sibling `base/` files, or an inline ` ```base <path> ` block, mount into a
//!   REAL tmpdir as the scenario's before-world;
//! - ` ```json ^put ` (legacy ` ```put ` also accepted) routes the exact JSON
//!   operation through production `wire_serve::write`;
//! - ` ```starlark ^expect ` (legacy ` ```expect ` also accepted) asserts over
//!   `t.result` / `t.doc(path)` / `t.journal`. For a sibling HOOK convention,
//!   `t.result.effects` carries the production-shaped armed intents.
//!
//! # Fire / pass, observed — and the pairing lint
//! A scenario FIRES when its `^put` sequence is refused (the first refusal is
//! `t.result`, and the sequence stops); it PASSES when the writes commit. Fire /
//! pass is OBSERVED from the run, never declared. A firing scenario MUST wikilink
//! a passing sibling (`pairs: "[[…]]"`) — proof the same convention admits valid
//! input — or the harness fails with a HARD ERROR (exit 2), never a soft finding.
//!
//! # Mount confinement (load-bearing, taxonomy row 22)
//! Every `^put` target and every `t.doc(path)` is canonicalized within the mount:
//! an absolute path or a `..` / `.` segment is REFUSED (`bad_path`), never
//! allowed to escape the scenario's tmpdir.
//!
//! # Output + exit codes (§4 preamble law, `docs/status.md`)
//! JSON under `--json`, a human table otherwise. Exit 0 (every `^expect` held),
//! 1 (a scenario's `^expect` failed — findings), 2 (a malformed scenario or a
//! pairing hard error — tool failure).

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use fs::WorkspaceRoot;
use model::{Document, NodeKind};
use policy::{
    ChangeOp, CheckLimits, Convention, ConventionFiles, Invocation, derive_change, derive_event,
    evaluate_hooks_for_test, load_convention,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use wire::ResponseBody;
use wire::{
    Edit, EditShape, ErrorBody, ErrorCode, NodeRev, Op, Path as WirePath, PutAt, Root, SecRef,
};
use wire_serve::ambient_root;
use wire_serve::rev::Rev;
use wire_serve::write::{CreateArgs, RemoveArgs, SpliceArgs, create, remove, splice};

use crate::expect::run_expect;
use crate::{Fail, Format};

/// Run `mrd test <PATH> [--json]`: load the scenarios under `PATH` (a dir of
/// `*.md`, or a single file), run each, lint the pairings, and render the report.
///
/// # Errors
/// [`Fail`] — exit 2 (bad usage, unreadable path, a malformed scenario, or a
/// pairing hard error) or exit 1 (one or more `^expect` blocks did not hold).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    // The history tier is a distinct mode: `--history` reconstructs journal rows
    // against git and runs a convention's `check_change`, never the scenario path.
    if args.iter().any(|a| a == "--history") {
        return crate::history_cmd::dispatch(args);
    }

    let mut path: Option<String> = None;
    let mut json = false;
    let mut corpus = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--corpus" => corpus = true,
            flag if flag.starts_with('-') => {
                return Err(Fail::tool(format!("unknown flag: {flag}")));
            }
            value if path.is_none() => path = Some(value.to_owned()),
            value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
        }
    }
    let format = if json { Format::Json } else { Format::Human };

    // Tier 2 — the corpus runner: the positional is a corpus-test SPEC file.
    if corpus {
        let spec = path.ok_or_else(|| Fail::tool("test --corpus needs a SPEC file".to_owned()))?;
        return crate::corpus_tier::run(&spec, format);
    }

    let path = path.ok_or_else(|| Fail::tool("test needs a scenario DIR or FILE".to_owned()))?;

    let report = run_suite(Path::new(&path))?;
    match format {
        Format::Json => println!("{}", report.to_json()),
        Format::Human => print!("{}", report.to_human()),
    }

    if report.has_hard_error() {
        return Err(Fail::tool(report.hard_error_summary()));
    }
    if report.failed_count() > 0 {
        return Err(Fail::findings(format!(
            "{} scenario(s) failed their ^expect",
            report.failed_count()
        )));
    }
    Ok(())
}

/// Load + run every scenario under `path`, then lint the pairings.
fn run_suite(path: &Path) -> Result<SuiteReport, Fail> {
    let files = collect_scenarios(path)?;
    let mut results = Vec::with_capacity(files.len());
    for file in &files {
        let text = std::fs::read_to_string(file)
            .map_err(|e| Fail::tool(format!("cannot read scenario {}: {e}", file.display())))?;
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("scenario")
            .to_owned();
        match parse_scenario(&stem, &text).and_then(|mut scenario| {
            hydrate_sibling_convention(file, &mut scenario).map(|()| scenario)
        }) {
            Ok(scenario) => results.push(run_scenario(&scenario)),
            Err(msg) => results.push(ScenarioResult {
                name: stem,
                convention_rev: String::new(),
                pairs: None,
                kind: ResultKind::Malformed(msg),
            }),
        }
    }
    let hard_errors = pairing_lint(&results);
    Ok(SuiteReport {
        results,
        hard_errors,
    })
}

/// Resolve `path` to the scenario file list: a single `*.md` file, or every
/// `*.md` directly under a directory (sorted, so the report is deterministic).
///
/// # Errors
/// The path does not exist, a directory holds no `*.md`, or it is unreadable.
fn collect_scenarios(path: &Path) -> Result<Vec<PathBuf>, Fail> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if path.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| Fail::tool(format!("cannot read scenario dir {}: {e}", path.display())))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")))
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(Fail::tool(format!(
                "no *.md scenarios in {}",
                path.display()
            )));
        }
        return Ok(files);
    }
    Err(Fail::tool(format!(
        "scenario path {} is neither a file nor a directory",
        path.display()
    )))
}

// ── mount confinement (load-bearing, taxonomy row 22) ────────────────────────

/// Canonicalize `path` WITHIN a scenario mount: the §1 path law the strict writer
/// enforces — no absolute path, no `.` / `..` / empty segment — so a `^put` or a
/// `t.doc(path)` can never escape the tmpdir via `mount.join`. Returns the
/// workspace-relative path; a violation is the caller's `bad_path`.
pub(crate) fn confine(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("empty path escapes the mount".to_owned());
    }
    if path.starts_with('/') {
        return Err(format!("absolute path {path:?} escapes the mount"));
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(format!(
                "path {path:?} has an illegal segment {seg:?} (escapes the mount)"
            ));
        }
    }
    Ok(PathBuf::from(path))
}

// ── scenario model + parse ───────────────────────────────────────────────────

/// One parsed scenario.
struct Scenario {
    name: String,
    convention_rev: String,
    cas: bool,
    /// The passing-sibling wikilink target (frontmatter `pairs`), unbracketed.
    pairs: Option<String>,
    /// Files mounted into the tmpdir: `(relative path, content)`.
    base: Vec<(String, String)>,
    /// The write actions, applied in order. `^put` is a strictly decoded wire
    /// request; the unanchored legacy `put` fence retains its compatibility shape.
    puts: Vec<ScenarioWrite>,
    /// The fenced `def expect(t)` starlark, if present.
    expect: Option<String>,
    /// A sibling convention loaded from `<convention>/scenarios/`, when this page
    /// uses Doc 1's folder shape. Inline scenarios leave this empty.
    conventions: Vec<Convention>,
}

enum ScenarioWrite {
    /// The Doc 1 surface: exact production request, strict-decoded once at load.
    Wire { id: Option<u64>, op: Op },
    /// Compatibility for the older unanchored ` ```put ` fixture grammar.
    Legacy(PutJson),
}

/// One legacy unanchored ` ```put ` action. Flat + forgiving: each op reads the
/// fields it needs. `if_root` / `if_node_rev` are honored only when the scenario
/// declares `cas: true`. New `^put` blocks never enter this translation.
#[derive(Deserialize)]
struct PutJson {
    /// `splice` | `create` | `remove`.
    op: String,
    /// The target file path (mount-confined).
    path: String,
    /// `splice`: the §2.1 target ref (`{"hpath":…}` / `{"anchor":…}` / `{"fm_key":…}`).
    #[serde(default)]
    target: Option<SecRef>,
    /// `splice` put: `all` | `content` | `end` | `upsert`.
    #[serde(default)]
    at: Option<String>,
    /// `splice` put: the text to write.
    #[serde(default)]
    text: Option<String>,
    /// `splice` match: the exact `old` slice to replace.
    #[serde(default)]
    old: Option<String>,
    /// `splice` match: its replacement.
    #[serde(default)]
    new: Option<String>,
    /// CAS node-grain guard (honored only under `cas: true`).
    #[serde(default)]
    if_node_rev: Option<String>,
    /// CAS world guard (honored only under `cas: true`): a full root, or absent
    /// to have the harness supply the current ambient root.
    #[serde(default)]
    if_root: Option<String>,
    /// `create`: the new file's full body.
    #[serde(default)]
    body: Option<String>,
    /// `remove`: the whole-file rev the caller read (remove-what-you-read).
    #[serde(default)]
    if_file_rev: Option<String>,
}

/// Parse one scenario's frontmatter + fenced blocks. A `put` block whose JSON
/// will not parse, or a `base` block with no path, is a malformed scenario.
fn parse_scenario(stem: &str, text: &str) -> Result<Scenario, String> {
    let fm = parse_frontmatter(text);
    let name = fm
        .get("scenario")
        .cloned()
        .unwrap_or_else(|| stem.to_owned());
    let convention_rev = fm.get("convention_rev").cloned().unwrap_or_default();
    let cas = fm.get("cas").is_some_and(|v| v == "true");
    let pairs = fm.get("pairs").map(|v| unwikilink(v));

    let mut base = Vec::new();
    let mut puts = Vec::new();
    let mut expect = None;
    for (info, body) in scan_blocks(text) {
        let tokens: Vec<&str> = info.split_whitespace().collect();
        match tokens.first().copied() {
            Some("base") => {
                let path = tokens
                    .get(1)
                    .ok_or_else(|| "```base needs a path (```base notes/todo.md)".to_owned())?;
                base.push(((*path).to_owned(), body));
            }
            Some("put") => {
                let pj: PutJson = serde_json::from_str(&body)
                    .map_err(|e| format!("```put JSON did not parse: {e}"))?;
                puts.push(ScenarioWrite::Legacy(pj));
            }
            Some("expect") => expect = Some(body),
            _ if tokens.contains(&"^put") => {
                puts.push(parse_wire_put(&body)?);
            }
            _ if tokens.contains(&"^expect") => expect = Some(body),
            _ => {}
        }
    }
    Ok(Scenario {
        name,
        convention_rev,
        cas,
        pairs,
        base,
        puts,
        expect,
        conventions: Vec::new(),
    })
}

fn parse_wire_put(body: &str) -> Result<ScenarioWrite, String> {
    let mut object: Map<String, Value> = serde_json::from_str(body)
        .map_err(|error| format!("^put JSON did not parse as a request object: {error}"))?;
    let id = match object.get("id") {
        None => None,
        Some(value) => {
            let id = value
                .as_u64()
                .ok_or_else(|| "^put `id` must be an integer in [0, 2^53)".to_owned())?;
            if id >= (1_u64 << 53) {
                return Err("^put `id` must be an integer in [0, 2^53)".to_owned());
            }
            Some(id)
        }
    };
    wire_serve::rev::rename_request(&mut object);
    let op = wire_serve::decode::decode(&object, Rev::V3).map_err(|error| {
        format!(
            "^put wire decode refused `{}`: {}",
            code_str(error.code),
            error.message.as_deref().unwrap_or_default()
        )
    })?;
    Ok(ScenarioWrite::Wire { id, op })
}

struct DirConventionFiles<'a> {
    dir: &'a Path,
}

impl ConventionFiles for DirConventionFiles<'_> {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir.join(rel_path))
    }

    fn exists(&self, rel_path: &str) -> bool {
        self.dir.join(rel_path).exists()
    }
}

/// Apply Doc 1's convention-folder shape when `file` lives directly under a
/// `scenarios/` directory: mount every sibling `base/` file at its relative path
/// and load the convention capability page beside it. Existing inline `base <path>`
/// blocks remain valid and cannot be silently shadowed by a sibling fixture.
fn hydrate_sibling_convention(file: &Path, scenario: &mut Scenario) -> Result<(), String> {
    let Some(scenarios_dir) = file.parent() else {
        return Ok(());
    };
    if scenarios_dir.file_name().and_then(|name| name.to_str()) != Some("scenarios") {
        return Ok(());
    }
    let convention_dir = scenarios_dir
        .parent()
        .ok_or_else(|| "scenario directory has no convention parent".to_owned())?;

    let base_dir = convention_dir.join("base");
    if base_dir.is_dir() {
        let mut files = Vec::new();
        collect_base_files(&base_dir, &base_dir, &mut files)?;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, content) in files {
            if scenario.base.iter().any(|(existing, _)| existing == &path) {
                return Err(format!(
                    "base path {path:?} is declared both in sibling base/ and an inline block"
                ));
            }
            scenario.base.push((path, content));
        }
    }

    let declares_capability =
        convention_dir.join("CHECK.md").is_file() || convention_dir.join("HOOK.md").is_file();
    if !declares_capability {
        return Ok(());
    }
    let slug = convention_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "convention folder has no UTF-8 slug".to_owned())?;
    let files = DirConventionFiles {
        dir: convention_dir,
    };
    let convention = load_convention(slug, &files, CheckLimits::default())
        .map_err(|error| format!("convention `{slug}` failed to load: {error}"))?;
    scenario.conventions.push(convention);
    Ok(())
}

fn collect_base_files(
    base: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read sibling base dir {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read base entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_base_files(base, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .expect("base descendant")
                .to_string_lossy()
                .replace('\\', "/");
            confine(&rel).map_err(|error| format!("sibling base file {rel:?}: {error}"))?;
            let content = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read sibling base file {rel:?}: {error}"))?;
            out.push((rel, content));
        }
    }
    Ok(())
}

/// Parse the leading `--- … ---` YAML frontmatter as flat `key: value` pairs
/// (quotes stripped). Enough for the scenario keys; not a general YAML parser.
/// Shared with the `--corpus` tier ([`crate::corpus_tier`]).
pub(crate) fn parse_frontmatter(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return map;
    }
    for line in lines {
        if line.trim_end() == "---" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_owned();
            let val = v.trim().trim_matches(['"', '\'']).to_owned();
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    map
}

/// Every fenced block as `(info-string, body)`. The body preserves inner lines
/// verbatim (each with its trailing newline) — that is the mounted file content /
/// the JSON / the starlark. Frontmatter carries no fence, so scanning the whole
/// text is safe. Shared with the `--corpus` tier ([`crate::corpus_tier`]).
pub(crate) fn scan_blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            let info = info.trim().to_owned();
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            out.push((info, body));
        }
    }
    out
}

/// Strip a `[[…]]` wikilink to its bare target (`[[foo]]` → `foo`); a non-wikilink
/// value is returned as-is.
fn unwikilink(v: &str) -> String {
    v.trim()
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(v)
        .trim()
        .to_owned()
}

// ── running one scenario ─────────────────────────────────────────────────────

/// One armed-not-delivered reaction on the existing `t.result` data surface.
/// Delivery state is deliberately absent: the scenario observes only what the
/// landed write armed.
#[derive(Debug, Clone, allocative::Allocative)]
pub(crate) struct ScenarioEffect {
    pub rule_id: String,
    pub seq: u32,
    pub action: String,
    pub target: Option<String>,
    pub severity: Option<String>,
    pub payload: Option<String>,
    pub receipt: String,
}

impl From<policy::Intent> for ScenarioEffect {
    fn from(intent: policy::Intent) -> Self {
        Self {
            rule_id: intent.rule_id,
            seq: intent.seq,
            action: intent.action,
            target: intent.target,
            severity: intent.severity,
            payload: intent.payload,
            receipt: intent.receipt,
        }
    }
}

/// The outcome of the `^put` sequence — the `t.result` surface the `^expect`
/// block asserts over.
#[derive(Debug, Clone)]
pub(crate) struct PutOutcome {
    /// The writes committed (no refusal).
    pub ok: bool,
    /// The refusal code (`bad_path`, `cas_mismatch`, …) when refused, else empty.
    pub code: String,
    /// The refusal message when refused, else empty.
    pub message: String,
    /// The verdict rule names the write raised (empty on the bare write path).
    pub verdicts: Vec<String>,
    /// The HOOK intents the committed write armed, with no delivery claim.
    pub effects: Vec<ScenarioEffect>,
}

impl PutOutcome {
    fn committed() -> Self {
        Self {
            ok: true,
            code: String::new(),
            message: String::new(),
            verdicts: Vec::new(),
            effects: Vec::new(),
        }
    }

    fn refused(code: &str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: code.to_owned(),
            message: message.into(),
            verdicts: Vec::new(),
            effects: Vec::new(),
        }
    }
}

/// Mount the scenario's `base/` into a fresh tmpdir, apply the `^put` sequence
/// through the production write path, then run its `^expect`.
fn run_scenario(sc: &Scenario) -> ScenarioResult {
    let base_result = |kind| ScenarioResult {
        name: sc.name.clone(),
        convention_rev: sc.convention_rev.clone(),
        pairs: sc.pairs.clone(),
        kind,
    };

    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => return base_result(ResultKind::Malformed(format!("cannot create tmpdir: {e}"))),
    };
    let root = WorkspaceRoot(dir.path().to_path_buf());

    // Mount base/ (confined — a base path escape is a scenario authoring fault).
    for (path, content) in &sc.base {
        let rel = match confine(path) {
            Ok(r) => r,
            Err(m) => {
                return base_result(ResultKind::Malformed(format!("base file {path:?}: {m}")));
            }
        };
        let full = dir.path().join(&rel);
        if let Some(parent) = full.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return base_result(ResultKind::Malformed(format!(
                "cannot create base dir for {path:?}: {e}"
            )));
        }
        if let Err(e) = std::fs::write(&full, content) {
            return base_result(ResultKind::Malformed(format!(
                "cannot write base {path:?}: {e}"
            )));
        }
    }

    // Apply the puts in order; the FIRST refusal is `t.result` and stops the run.
    let mut result = PutOutcome::committed();
    let mut effects = Vec::new();
    for pj in &sc.puts {
        match apply_put(&root, sc.cas, pj, &sc.conventions) {
            Ok(mut outcome) => {
                let refused = !outcome.ok;
                effects.append(&mut outcome.effects);
                result = outcome;
                if refused {
                    break;
                }
            }
            Err(m) => return base_result(ResultKind::Malformed(format!("```put: {m}"))),
        }
    }
    result.effects = effects;

    let journal = std::fs::read_to_string(dir.path().join(fs::domain::RESERVED_JOURNAL_PATH))
        .unwrap_or_default();

    let fired = !result.ok;
    let expect = match &sc.expect {
        Some(src) => run_expect(src, dir.path(), &result, &journal).map_err(|e| e.to_string()),
        None => Err("scenario has no ```expect block".to_owned()),
    };

    base_result(ResultKind::Ran {
        fired,
        result_code: result.code.clone(),
        result_message: result.message.clone(),
        expect,
    })
}

/// Route one scenario write through the production write path. Anchored `^put`
/// requests were already strict-decoded into [`Op`] and are forwarded without a
/// second request grammar. Only the legacy unanchored fence uses `PutJson`.
fn apply_put(
    root: &WorkspaceRoot,
    cas: bool,
    write: &ScenarioWrite,
    conventions: &[Convention],
) -> Result<PutOutcome, String> {
    match write {
        ScenarioWrite::Wire { id, op } => dispatch_wire_write(root, *id, op, conventions),
        ScenarioWrite::Legacy(put) => apply_legacy_put(root, cas, put, conventions),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "exhaustive decoded write router; splitting the two typed arms adds indirection"
)]
fn dispatch_wire_write(
    root: &WorkspaceRoot,
    id: Option<u64>,
    op: &Op,
    conventions: &[Convention],
) -> Result<PutOutcome, String> {
    match op.clone() {
        Op::Splice {
            path,
            actor,
            now,
            receipt,
            if_root,
            dry,
            force,
            edits,
            plan_edits,
            pin,
        } => {
            if let Err(message) = confine(&path.0) {
                return Ok(PutOutcome::refused("bad_path", message));
            }
            let rel = PathBuf::from(&path.0);
            let before_md = std::fs::read_to_string(root.0.join(&rel)).unwrap_or_default();
            let dry = dry.unwrap_or(false);
            let force = force.unwrap_or(false);
            let args = SpliceArgs {
                id,
                path: path.clone(),
                actor: actor.clone(),
                now,
                receipt,
                if_root,
                dry,
                force,
                edits,
                plan_edits,
                pin,
            };
            let mut outcome = match splice(root, 0, &args, &[], None) {
                Ok(out) => {
                    let verdicts = match &out.body {
                        ResponseBody::Splice { verdicts, .. } => verdicts
                            .iter()
                            .map(|verdict| verdict.rule.clone())
                            .collect(),
                        _ => Vec::new(),
                    };
                    PutOutcome {
                        verdicts,
                        ..PutOutcome::committed()
                    }
                }
                Err(error) => refusal(&error),
            };
            if outcome.ok && !dry && !conventions.is_empty() {
                let after_md = std::fs::read_to_string(root.0.join(&rel)).unwrap_or_default();
                outcome.effects = scenario_effects(
                    conventions,
                    &path.0,
                    &before_md,
                    &after_md,
                    ChangeOp::Splice,
                    actor.as_deref(),
                    force,
                )?;
            }
            Ok(outcome)
        }
        Op::Create {
            path,
            body,
            actor,
            now,
            if_root,
            dry,
        } => {
            if let Err(message) = confine(&path.0) {
                return Ok(PutOutcome::refused("bad_path", message));
            }
            let rel = PathBuf::from(&path.0);
            let dry = dry.unwrap_or(false);
            let args = CreateArgs {
                id,
                path: path.clone(),
                body,
                actor: actor.clone(),
                now,
                if_root,
                dry,
            };
            let mut outcome = match create(root, 0, &args, &[]) {
                Ok(out) => PutOutcome {
                    verdicts: out
                        .verdicts
                        .iter()
                        .map(|verdict| verdict.rule.clone())
                        .collect(),
                    ..PutOutcome::committed()
                },
                Err(error) => refusal(&error),
            };
            if outcome.ok && !dry && !conventions.is_empty() {
                let after_md = std::fs::read_to_string(root.0.join(&rel)).unwrap_or_default();
                outcome.effects = scenario_effects(
                    conventions,
                    &path.0,
                    "",
                    &after_md,
                    ChangeOp::Create,
                    actor.as_deref(),
                    false,
                )?;
            }
            Ok(outcome)
        }
        other => Err(format!(
            "^put decoded a non-write operation {other:?}; use `splice` or `create`"
        )),
    }
}

/// Route one legacy translated write through the production path. The
/// mount-confinement refusal (`bad_path`) fires before the production call.
fn apply_legacy_put(
    root: &WorkspaceRoot,
    cas: bool,
    p: &PutJson,
    conventions: &[Convention],
) -> Result<PutOutcome, String> {
    if let Err(msg) = confine(&p.path) {
        return Ok(PutOutcome::refused("bad_path", msg));
    }
    let rel = PathBuf::from(&p.path);
    let before_md = std::fs::read_to_string(root.0.join(&rel)).unwrap_or_default();
    let path = WirePath(p.path.clone());

    // CAS is omitted unless the scenario declares it (explicit frontmatter flag):
    // then the harness supplies an explicit `if_root`, or the live ambient root.
    let cas_root: Option<Root> = if cas {
        match &p.if_root {
            Some(r) => Some(Root(r.clone())),
            None => Some(
                ambient_root(root)
                    .map_err(|e| format!("cannot read ambient root: {:?}", e.code))?,
            ),
        }
    } else {
        None
    };

    let mut outcome = match p.op.as_str() {
        "splice" => splice_put(root, path, cas, cas_root, p),
        "create" => create_put(root, path, cas_root, p),
        "remove" => remove_put(root, path, cas_root, p),
        other => Err(format!(
            "unknown put op {other:?} (want splice|create|remove)"
        )),
    }?;

    if outcome.ok && !conventions.is_empty() {
        let after_md = std::fs::read_to_string(root.0.join(&rel)).unwrap_or_default();
        let op = match p.op.as_str() {
            "splice" => ChangeOp::Splice,
            "create" => ChangeOp::Create,
            "remove" => ChangeOp::Remove,
            _ => unreachable!("validated above"),
        };
        outcome.effects = scenario_effects(
            conventions,
            &p.path,
            &before_md,
            &after_md,
            op,
            Some("mrd-test"),
            false,
        )?;
    }
    Ok(outcome)
}

fn scenario_effects(
    conventions: &[Convention],
    path: &str,
    before_md: &str,
    after_md: &str,
    op: ChangeOp,
    actor: Option<&str>,
    force: bool,
) -> Result<Vec<ScenarioEffect>, String> {
    let before = scenario_doc(path, before_md);
    let after = scenario_doc(path, after_md);
    let change = derive_change(
        &before,
        &after,
        &[],
        Invocation { op, actor, force },
        &[],
        &|_: &str| None,
    );
    let event = derive_event(&change, &before.root.node_rev.0, &after.root.node_rev.0, 0);
    let outcomes = evaluate_hooks_for_test(conventions, &event)
        .map_err(|error| format!("HOOK evaluation failed: {error}"))?;
    let mut effects = Vec::new();
    for outcome in outcomes {
        if let Some(finding) = outcome.findings.first() {
            return Err(format!("HOOK evaluation finding: {finding:?}"));
        }
        effects.extend(outcome.intents.into_iter().map(ScenarioEffect::from));
    }
    Ok(effects)
}

fn scenario_doc(path: &str, md: &str) -> Document {
    let nodes = syntax::parse(md);
    let mut doc = model::build(md.to_owned(), nodes);
    if let NodeKind::Document { path: doc_path, .. } = &mut doc.root.kind {
        path.clone_into(doc_path);
    }
    doc
}

/// The `splice` arm: one edit (a `put` position or a `match`), routed through the
/// production splice choke-point. `if_node_rev` is honored only under `cas`.
fn splice_put(
    root: &WorkspaceRoot,
    path: WirePath,
    cas: bool,
    cas_root: Option<Root>,
    p: &PutJson,
) -> Result<PutOutcome, String> {
    let target = p
        .target
        .clone()
        .ok_or_else(|| "splice needs a `target`".to_owned())?;
    let shape = if let Some(at) = &p.at {
        EditShape::Put {
            at: parse_at(at)?,
            text: p.text.clone().unwrap_or_default(),
        }
    } else if let (Some(old), Some(new)) = (&p.old, &p.new) {
        EditShape::Match {
            old: old.clone(),
            new: new.clone(),
        }
    } else {
        return Err("splice needs either `at`+`text` (put) or `old`+`new` (match)".to_owned());
    };
    let if_node_rev = if cas {
        p.if_node_rev.clone().map(NodeRev)
    } else {
        None
    };
    let args = SpliceArgs {
        id: None,
        path,
        actor: Some("mrd-test".to_owned()),
        now: None,
        receipt: None,
        if_root: cas_root,
        dry: false,
        force: false,
        edits: vec![Edit {
            target,
            edit: shape,
            if_node_rev,
        }],
        plan_edits: Vec::new(),
        // A scenario `^put` is a content write; the pin verb has its own path.
        pin: None,
    };
    Ok(match splice(root, 0, &args, &[], None) {
        Ok(out) => {
            let verdicts = match &out.body {
                ResponseBody::Splice { verdicts, .. } => {
                    verdicts.iter().map(|v| v.rule.clone()).collect()
                }
                _ => Vec::new(),
            };
            PutOutcome {
                verdicts,
                ..PutOutcome::committed()
            }
        }
        Err(e) => refusal(&e),
    })
}

/// The `create` arm: guarded file birth through the production choke-point.
fn create_put(
    root: &WorkspaceRoot,
    path: WirePath,
    cas_root: Option<Root>,
    p: &PutJson,
) -> Result<PutOutcome, String> {
    let body = p
        .body
        .clone()
        .ok_or_else(|| "create needs a `body`".to_owned())?;
    let args = CreateArgs {
        id: None,
        path,
        body,
        actor: Some("mrd-test".to_owned()),
        now: None,
        if_root: cas_root,
        dry: false,
    };
    Ok(match create(root, 0, &args, &[]) {
        Ok(out) => PutOutcome {
            verdicts: out.verdicts.iter().map(|v| v.rule.clone()).collect(),
            ..PutOutcome::committed()
        },
        Err(e) => refusal(&e),
    })
}

/// The `remove` arm: guarded file death (remove-what-you-read) through the
/// production choke-point.
fn remove_put(
    root: &WorkspaceRoot,
    path: WirePath,
    cas_root: Option<Root>,
    p: &PutJson,
) -> Result<PutOutcome, String> {
    let rev = p
        .if_file_rev
        .clone()
        .ok_or_else(|| "remove needs an `if_file_rev` (remove-what-you-read)".to_owned())?;
    let args = RemoveArgs {
        id: None,
        path,
        if_file_rev: NodeRev(rev),
        actor: Some("mrd-test".to_owned()),
        now: None,
        if_root: cas_root,
        dry: false,
    };
    Ok(match remove(root, 0, &args, &[]) {
        Ok(out) => PutOutcome {
            verdicts: out.verdicts.iter().map(|v| v.rule.clone()).collect(),
            ..PutOutcome::committed()
        },
        Err(e) => refusal(&e),
    })
}

/// A production-write refusal → the `t.result` refusal surface.
fn refusal(e: &ErrorBody) -> PutOutcome {
    PutOutcome::refused(&code_str(e.code), e.message.clone().unwrap_or_default())
}

/// The wire string form of an error code (`bad_path`, `root_mismatch`, …).
fn code_str(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Parse a `^put` put position.
fn parse_at(at: &str) -> Result<PutAt, String> {
    match at {
        "all" => Ok(PutAt::All),
        "content" => Ok(PutAt::Content),
        "end" => Ok(PutAt::End),
        "upsert" => Ok(PutAt::Upsert),
        other => Err(format!(
            "unknown put position {other:?} (want all|content|end|upsert)"
        )),
    }
}

// ── results, pairing lint, report ────────────────────────────────────────────

/// One scenario's outcome.
struct ScenarioResult {
    name: String,
    convention_rev: String,
    pairs: Option<String>,
    kind: ResultKind,
}

enum ResultKind {
    /// The scenario could not be parsed / mounted / driven — a structural fault.
    Malformed(String),
    /// The scenario ran: it fired (write refused) or passed (committed), and its
    /// `^expect` either held (`Ok`) or did not (`Err(message)`).
    Ran {
        fired: bool,
        result_code: String,
        result_message: String,
        expect: Result<(), String>,
    },
}

/// The pairing lint (a HARD error, not a soft finding): every FIRING scenario must
/// wikilink a PASSING sibling (one that did not fire) present in the suite.
fn pairing_lint(results: &[ScenarioResult]) -> Vec<String> {
    // name → Some(fired) for a scenario that ran, None for a malformed one.
    let mut fired_by: HashMap<&str, Option<bool>> = HashMap::new();
    for r in results {
        let f = match &r.kind {
            ResultKind::Ran { fired, .. } => Some(*fired),
            ResultKind::Malformed(_) => None,
        };
        fired_by.insert(r.name.as_str(), f);
    }

    let mut errs = Vec::new();
    for r in results {
        if !matches!(&r.kind, ResultKind::Ran { fired: true, .. }) {
            continue;
        }
        match &r.pairs {
            None => errs.push(format!(
                "pairing lint: firing scenario `{}` declares no passing sibling \
                 (add `pairs: \"[[sibling]]\"`)",
                r.name
            )),
            Some(sib) => match fired_by.get(sib.as_str()) {
                None => errs.push(format!(
                    "pairing lint: firing scenario `{}` pairs `[[{sib}]]`, which is not a \
                     scenario in the suite",
                    r.name
                )),
                Some(None) => errs.push(format!(
                    "pairing lint: firing scenario `{}` pairs `[[{sib}]]`, which is malformed \
                     (not a passing sibling)",
                    r.name
                )),
                Some(Some(true)) => errs.push(format!(
                    "pairing lint: firing scenario `{}` pairs `[[{sib}]]`, which ALSO fires — a \
                     firing scenario must pair with a PASSING sibling",
                    r.name
                )),
                Some(Some(false)) => {}
            },
        }
    }
    errs
}

/// The finished suite report.
struct SuiteReport {
    results: Vec<ScenarioResult>,
    hard_errors: Vec<String>,
}

impl SuiteReport {
    /// Scenarios whose `^expect` did not hold (exit-1 findings).
    fn failed_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(&r.kind, ResultKind::Ran { expect: Err(_), .. }))
            .count()
    }

    fn malformed_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(&r.kind, ResultKind::Malformed(_)))
            .count()
    }

    /// A hard error (exit 2): a malformed scenario or a broken pairing — the suite
    /// cannot render a trustworthy verdict.
    fn has_hard_error(&self) -> bool {
        !self.hard_errors.is_empty() || self.malformed_count() > 0
    }

    fn hard_error_summary(&self) -> String {
        let malformed = self.malformed_count();
        let mut parts = Vec::new();
        if malformed > 0 {
            parts.push(format!("{malformed} malformed scenario(s)"));
        }
        if !self.hard_errors.is_empty() {
            parts.push(format!("{} pairing hard error(s)", self.hard_errors.len()));
        }
        parts.join(", ")
    }

    /// The set of convention revs the suite exercised — recorded in the report.
    fn convention_revs(&self) -> Vec<String> {
        let mut set: BTreeSet<String> = BTreeSet::new();
        for r in &self.results {
            if !r.convention_rev.is_empty() {
                set.insert(r.convention_rev.clone());
            }
        }
        set.into_iter().collect()
    }

    fn to_human(&self) -> String {
        let mut s = String::new();
        s.push_str("# mrd test — scenario suite\n\n");
        s.push_str("| scenario | outcome | ^expect | convention_rev |\n");
        s.push_str("|----------|---------|---------|----------------|\n");
        for r in &self.results {
            let (outcome, expect, rev) = match &r.kind {
                ResultKind::Malformed(_) => ("malformed", "—".to_owned(), r.convention_rev.clone()),
                ResultKind::Ran { fired, expect, .. } => (
                    if *fired { "fired" } else { "passed" },
                    match expect {
                        Ok(()) => "ok".to_owned(),
                        Err(_) => "FAIL".to_owned(),
                    },
                    r.convention_rev.clone(),
                ),
            };
            let rev = if rev.is_empty() {
                "—".to_owned()
            } else {
                rev
            };
            let _ = writeln!(s, "| `{}` | {outcome} | {expect} | {rev} |", r.name);
        }
        s.push('\n');

        // Failure and malformed detail.
        for r in &self.results {
            match &r.kind {
                ResultKind::Malformed(msg) => {
                    let _ = writeln!(s, "- **malformed** `{}`: {msg}", r.name);
                }
                ResultKind::Ran {
                    fired,
                    result_code,
                    expect: Err(msg),
                    ..
                } => {
                    let fire = if *fired {
                        format!(" (fired: `{result_code}`)")
                    } else {
                        String::new()
                    };
                    let _ = writeln!(s, "- **^expect FAILED** `{}`{fire}: {msg}", r.name);
                }
                ResultKind::Ran { .. } => {}
            }
        }
        for e in &self.hard_errors {
            let _ = writeln!(s, "- **HARD ERROR** {e}");
        }

        let passed = self
            .results
            .iter()
            .filter(|r| matches!(&r.kind, ResultKind::Ran { expect: Ok(()), .. }))
            .count();
        let revs = self.convention_revs();
        let revs = if revs.is_empty() {
            "(none)".to_owned()
        } else {
            revs.join(", ")
        };
        let _ = write!(
            s,
            "\n{} scenario(s): {passed} passed, {} failed, {} malformed, {} pairing hard error(s).\n\
             convention revs tested: {revs}\n",
            self.results.len(),
            self.failed_count(),
            self.malformed_count(),
            self.hard_errors.len(),
        );
        s
    }

    fn to_json(&self) -> String {
        let scenarios: Vec<Value> = self
            .results
            .iter()
            .map(|r| match &r.kind {
                ResultKind::Malformed(msg) => json!({
                    "name": r.name,
                    "outcome": "malformed",
                    "error": msg,
                    "convention_rev": r.convention_rev,
                    "pairs": r.pairs,
                }),
                ResultKind::Ran {
                    fired,
                    result_code,
                    result_message,
                    expect,
                } => json!({
                    "name": r.name,
                    "outcome": if *fired { "fired" } else { "passed" },
                    "result_code": result_code,
                    "result_message": result_message,
                    "expect_ok": expect.is_ok(),
                    "expect_error": expect.as_ref().err(),
                    "convention_rev": r.convention_rev,
                    "pairs": r.pairs,
                }),
            })
            .collect();
        let value = json!({
            "scenarios": scenarios,
            "hard_errors": self.hard_errors,
            "summary": {
                "total": self.results.len(),
                "passed": self.results.iter()
                    .filter(|r| matches!(&r.kind, ResultKind::Ran { expect: Ok(()), .. }))
                    .count(),
                "failed": self.failed_count(),
                "malformed": self.malformed_count(),
                "pairing_hard_errors": self.hard_errors.len(),
                "convention_revs": self.convention_revs(),
            },
        });
        serde_json::to_string_pretty(&value).expect("json")
    }
}

#[cfg(test)]
mod tests {
    use super::{ScenarioWrite, confine, parse_at, parse_wire_put, unwikilink};
    use wire::{Op, PutAt};

    #[test]
    fn confine_rejects_absolute_and_dotdot_and_dot_and_empty() {
        assert!(confine("/etc/passwd").is_err(), "absolute rejected");
        assert!(confine("../secret.md").is_err(), ".. rejected");
        assert!(confine("a/../b.md").is_err(), "interior .. rejected");
        assert!(confine("a/./b.md").is_err(), ". rejected");
        assert!(confine("").is_err(), "empty rejected");
        assert!(confine("a//b.md").is_err(), "empty segment rejected");
    }

    #[test]
    fn confine_admits_a_plain_relative_path() {
        assert_eq!(
            confine("notes/todo.md").unwrap().to_str(),
            Some("notes/todo.md")
        );
    }

    #[test]
    fn parse_at_maps_the_four_positions() {
        assert!(matches!(parse_at("all").unwrap(), PutAt::All));
        assert!(matches!(parse_at("content").unwrap(), PutAt::Content));
        assert!(matches!(parse_at("end").unwrap(), PutAt::End));
        assert!(matches!(parse_at("upsert").unwrap(), PutAt::Upsert));
        assert!(parse_at("nope").is_err());
    }

    #[test]
    fn anchored_put_uses_the_strict_wire_batch_shape() {
        let request = r#"{
            "op":"splice",
            "path":"tasks/card.md",
            "edits":[
                {"target":{"fm_key":"status"},"edit":{"put":{"at":"upsert","text":"review"}}},
                {"target":{"fm_key":"reviewer"},"edit":{"put":{"at":"upsert","text":"r"}}}
            ]
        }"#;
        let ScenarioWrite::Wire {
            op: Op::Splice { edits, .. },
            ..
        } = parse_wire_put(request).expect("the real wire batch decodes")
        else {
            panic!("the request must remain a typed splice")
        };
        assert_eq!(edits.len(), 2, "both typed edits survive strict decode");

        let flattened = r#"{
            "op":"splice",
            "path":"tasks/card.md",
            "target":{"fm_key":"status"},
            "at":"upsert",
            "text":"review"
        }"#;
        let Err(error) = parse_wire_put(flattened) else {
            panic!("the old flattened lookalike must hit the field wall")
        };
        assert!(
            error.contains("unknown request field")
                && ["target", "at", "text"]
                    .iter()
                    .any(|field| error.contains(field)),
            "the strict decoder names one flattened lookalike field: {error}"
        );
    }

    #[test]
    fn unwikilink_strips_brackets() {
        assert_eq!(unwikilink("[[foo]]"), "foo");
        assert_eq!(unwikilink("  [[bar-baz]] "), "bar-baz");
        assert_eq!(unwikilink("plain"), "plain");
    }
}
