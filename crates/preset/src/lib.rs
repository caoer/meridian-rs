//! Presets + session birth (U5.3) — a preset is a def file whose `inputs` pin
//! the convention floor; `unfold` materializes the declared scaffold through the
//! U2.6 guarded create; `new` births one record from the def's `^template` after
//! validating it against the def's `^properties`.
//!
//! # Charter
//! **Owns:** the preset-def grammar (`type: def` + a multi-line `inputs` block
//! sequence pinning the floor + `# Properties`/`^properties`,
//! `# Template`/`^template`, `# Unfold` body sections), its validation, and the
//! two births — [`unfold`] (the whole scaffold set) and [`new_record`] (one
//! `^template` record). Every birth rides the ONE write path
//! ([`wire_serve::write::create`], d2 §2.5 C3 / U2.6): CAS `if_absent`,
//! journaled birth, gate seam — so every scaffold file carries a birth receipt
//! (an `op=create` journal row) and no write dodges the guard.
//!
//! **Never does:** invent a second write path (birth is guarded create, never a
//! raw `fs::write`), mint identity or a clock (`actor`/`now` are caller-supplied,
//! §9), or hold session policy / liveness (that is the customer's — docs/laws.md
//! charter). mrd dials these; it re-implements none of it.
//!
//! # The U2.11 safe grain (d2 §5.5, law)
//! A preset's `inputs` is a multi-line frontmatter block sequence. It is READ
//! through the U2.11 whole-value `fm_key` grain ([`model::resolve`] on
//! [`model::Ref::FmKey`]) — never a line-oriented scan that would stop at the key
//! line — and the preset pin a birth writes into its root record is RENDERED as a
//! whole block-sequence value and written atomically as the file's birth bytes,
//! never a single-line properties upsert (which the U2.11 corruption guard
//! refuses). [`render_block_sequence`] + the grain read are the two halves of
//! that round trip.

use model::{Document, NodeKind, Ref, YamlMap};
use serde::Serialize;
use wire::{ErrorBody, ErrorCode};

/// The default root record a session preset instantiates and pins the preset
/// into (design d3 §6: `mrd unfold` "instantiates the `^template` root record
/// `SESSION.md`"). Overridable by the def frontmatter key `root`.
const DEFAULT_ROOT_RECORD: &str = "SESSION.md";

/// The workspace prefix the convention floor lives under (the U4.4 floor suite:
/// `conventions/<slug>/CHECK.md`). A session preset's `inputs` pin paths here.
const FLOOR_PREFIX: &str = "conventions/";

// ---------------------------------------------------------------------------
// The caller envelope (§9 — the crate mints no identity and no clock)
// ---------------------------------------------------------------------------

/// The caller-supplied birth envelope: the recorded actor and time (both stamped
/// exactly as given, never invented) and the `--dry` switch (everything except
/// disk — no file, no journal row).
#[derive(Debug, Clone, Default)]
pub struct BirthOptions {
    /// The actor recorded on every birth (guarded create §9).
    pub actor: Option<String>,
    /// The time fact stamped on every birth; absent stays absent.
    pub now: Option<String>,
    /// Dry run — the guarded create runs everything except disk, and still
    /// refuses a would-be clobber (the `if_absent` CAS holds on a dry birth).
    pub dry: bool,
}

// ---------------------------------------------------------------------------
// The preset def grammar
// ---------------------------------------------------------------------------

/// One `^properties` rule: a required frontmatter key, optionally pinned to a
/// fixed value. A born record must carry `key` (`value: None`) or carry `key`
/// with exactly `value` (`value: Some`). A rule whose `key` is empty is a
/// malformed def rule (the source line did not parse).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropRule {
    /// The frontmatter key the rule governs.
    pub key: String,
    /// `Some(v)` ⇒ the key must equal `v`; `None` ⇒ the key must be present.
    pub value: Option<String>,
    /// The source list-item text — carried verbatim so a refusal names the rule.
    pub raw: String,
}

/// A parsed preset def (design d3 §6: `type: def`, body = `# Properties`
/// (`^properties`) / `# Template` (`^template`) / `# Unfold`).
#[derive(Debug, Clone)]
pub struct PresetDef {
    /// The def page path (workspace-relative) — the pin target a birth records.
    pub path: String,
    /// The def's whole-file rev — pinned into a born session so the declared
    /// shape is re-derivable forever (d3 §6, "the only cross-time memory").
    pub rev: String,
    /// The `defines:` kind (`session`, `task`, …). The `mrd new` `<kind>`.
    pub defines: String,
    /// The root record the scaffold pins the preset into (`root:` or the
    /// [`DEFAULT_ROOT_RECORD`]).
    pub root_record: String,
    /// The `births:` target-path template for [`new_record`] (`{{id}}`-filled).
    /// `None` ⇒ the default `{{kind}}/{{id}}.md`.
    pub births: Option<String>,
    /// The convention-floor pins — the `inputs` block sequence, read through the
    /// U2.11 whole-value grain (d2 §5.5). Each item is a `path@rev` pin.
    pub inputs: Vec<String>,
    /// The `^properties` rules; `None` ⇒ the def declares no `^properties` block
    /// (a structural def defect [`new_record`] refuses).
    pub properties: Option<Vec<PropRule>>,
    /// The `^template` record body (fence-stripped); `None` ⇒ no `^template`.
    pub template: Option<String>,
    /// The `# Unfold` declared scaffold — the workspace-relative file paths
    /// [`unfold`] materializes, in declared order.
    pub scaffold: Vec<String>,
    /// The `# Ephemeral` declared-disposable allowlist — path globs (`*.lock`)
    /// or exact paths [`reconcile`] MAY prune (the subtractive allowlist, ZT
    /// ruling #3). Empty ⇒ nothing is disposable, so reconcile prunes no file.
    pub ephemeral: Vec<String>,
}

/// A tool-level failure (mrd exit 2): the def could not be loaded or a birth
/// faulted for a reason other than the guarded `if_absent` CAS. Distinct from a
/// [`RefusalReason`] finding (exit 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// The def page could not be read (missing, unreadable).
    Io(String),
    /// The page is not a preset def (`type:` is absent or not `def`).
    NotADef {
        /// The def path the caller named.
        path: String,
        /// The `type:` value found (or `"(absent)"`).
        found: String,
    },
    /// A birth faulted at the write door for a reason other than the CAS (a bad
    /// path in the scaffold, an I/O failure) — the wire error, surfaced.
    Write(String),
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresetError::Io(e) => write!(f, "cannot read the def: {e}"),
            PresetError::NotADef { path, found } => {
                write!(
                    f,
                    "{path} is not a preset def (type: {found}, expected def)"
                )
            }
            PresetError::Write(e) => write!(f, "birth faulted at the write door: {e}"),
        }
    }
}

impl std::error::Error for PresetError {}

/// A refusal finding (mrd exit 1), the same shape `pin`/`attest` mint: the closed
/// §8 `code` + `recovery` class, a teaching message, and — for a `def_invalid` —
/// the named def rule that failed (refusal-amendment row 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusalReason {
    /// The §8 code — `def_invalid` (row 17) or `cas_mismatch` (rows 13/14).
    pub code: &'static str,
    /// The §8 recovery class — `fix` (a `def_invalid`) or `refresh` (a
    /// `cas_mismatch`).
    pub recovery: &'static str,
    /// The teaching message; names the violated def rule for a `def_invalid`.
    pub message: String,
    /// The `^properties` rule that failed (`def_invalid` only) — the `{rule}`
    /// extra the taxonomy row carries.
    pub rule: Option<String>,
}

impl RefusalReason {
    /// A `def_invalid{rule}` refusal (row 17, recovery `fix`): the named
    /// `^properties`/def rule the preset birth violated.
    #[must_use]
    fn def_invalid(rule: impl Into<String>) -> Self {
        let rule = rule.into();
        RefusalReason {
            code: "def_invalid",
            recovery: "fix",
            message: format!("^properties def rule violated: {rule}"),
            rule: Some(rule),
        }
    }

    /// A `cas_mismatch` refusal (rows 13/14, recovery `refresh`): the birth path
    /// is already occupied — the `if_absent` CAS held, no byte landed.
    #[must_use]
    fn cas_mismatch(path: &str) -> Self {
        RefusalReason {
            code: "cas_mismatch",
            recovery: "refresh",
            message: format!("{path} already exists — the if_absent CAS refused the birth"),
            rule: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Loading + parsing a def
// ---------------------------------------------------------------------------

/// Load and parse a preset def page.
///
/// # Errors
/// [`PresetError::Io`] when the page cannot be read; [`PresetError::NotADef`]
/// when it carries no `type: def` frontmatter.
pub fn load_def(root: &fs::WorkspaceRoot, def_path: &str) -> Result<PresetDef, PresetError> {
    let doc = fs::load(root, std::path::Path::new(def_path))
        .map_err(|e| PresetError::Io(e.to_string()))?;
    let ty = fm_scalar(&doc, "type");
    if ty.as_deref() != Some("def") {
        return Err(PresetError::NotADef {
            path: def_path.to_owned(),
            found: ty.unwrap_or_else(|| "(absent)".to_owned()),
        });
    }
    Ok(PresetDef {
        path: def_path.to_owned(),
        rev: doc.root.node_rev.0.clone(),
        defines: fm_scalar(&doc, "defines").unwrap_or_default(),
        root_record: fm_scalar(&doc, "root").unwrap_or_else(|| DEFAULT_ROOT_RECORD.to_owned()),
        births: fm_scalar(&doc, "births"),
        inputs: read_inputs_grain(&doc),
        properties: parse_properties(&doc.raw),
        template: parse_template(&doc.raw),
        scaffold: parse_unfold(&doc.raw),
        ephemeral: parse_ephemeral(&doc.raw),
    })
}

/// Parse the `# Ephemeral` section into the declared-disposable allowlist (each
/// `- <glob-or-path>` list item). Empty ⇒ no `# Ephemeral` section or no items —
/// reconcile then prunes NO file (the allowlist is empty by construction).
fn parse_ephemeral(raw: &str) -> Vec<String> {
    let Some(body) = title_section(raw, "Ephemeral") else {
        return Vec::new();
    };
    body.lines()
        .filter_map(|line| {
            let item = line.trim().strip_prefix("- ")?;
            let pat = item.trim().trim_matches(['"', '\'', '`']);
            (!pat.is_empty()).then(|| pat.to_owned())
        })
        .collect()
}

/// Read a scalar frontmatter value off the parsed frontmatter map (the public
/// [`YamlMap`] surface). Multi-line block values are read through the grain
/// ([`read_inputs_grain`]), never here.
fn fm_scalar(doc: &Document, key: &str) -> Option<String> {
    fn find(node: &model::Node) -> Option<&YamlMap> {
        if let NodeKind::Frontmatter { map } = &node.kind {
            return Some(map);
        }
        node.children.iter().find_map(find)
    }
    find(&doc.root)
        .and_then(|m| m.0.iter().find(|(k, _)| k == key))
        .map(|(_, v)| v.clone())
}

/// Read the `inputs` block sequence through the U2.11 whole-value grain
/// (`resolve(FmKey("inputs"))` spans the key line PLUS every indented item, d2
/// §5.5) and parse each `- "item"` into its pin string. Absent `inputs` ⇒ empty.
fn read_inputs_grain(doc: &Document) -> Vec<String> {
    let Ok(target) = model::resolve(doc, &Ref::FmKey("inputs".to_owned())) else {
        return Vec::new();
    };
    let grain = &doc.raw[target.span.clone()];
    grain
        .lines()
        .filter_map(|line| {
            let item = line.trim().strip_prefix("- ")?;
            Some(item.trim().trim_matches(['"', '\'']).to_owned())
        })
        .collect()
}

/// Parse the `^properties` block into rules. `None` ⇒ no `^properties` section
/// (a structural def defect). Each `- <key> [= <value>]` list item is one rule;
/// a `required` keyword is documentation. A `key`-less item parses to an empty
/// key (a malformed rule the validator names).
fn parse_properties(raw: &str) -> Option<Vec<PropRule>> {
    let body = anchor_section(raw, "properties")?;
    let rules = body
        .lines()
        .filter_map(|line| {
            let item = line.trim().strip_prefix("- ")?;
            Some(parse_prop_rule(item))
        })
        .collect();
    Some(rules)
}

/// Parse one `^properties` list item: `` `key` required `` or `key = value` or
/// `` `key` required = value ``. The key is the first word (backticks stripped);
/// a `= value` tail pins the value (implies required).
fn parse_prop_rule(item: &str) -> PropRule {
    let raw = item.trim().to_owned();
    let (head, value) = match raw.split_once('=') {
        Some((head, val)) => (head, Some(val.trim().trim_matches(['"', '\'']).to_owned())),
        None => (raw.as_str(), None),
    };
    let key = head
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .to_owned();
    PropRule { key, value, raw }
}

/// Extract the `^template` record body — the fenced code block inside the
/// `# Template` (`^template`) section, fences stripped. `None` ⇒ no `^template`.
fn parse_template(raw: &str) -> Option<String> {
    let body = anchor_section(raw, "template")?;
    fenced_block(&body)
}

/// Parse the `# Unfold` section into the declared scaffold file paths (each
/// `- <path>` list item). Empty ⇒ no `# Unfold` section or no items.
fn parse_unfold(raw: &str) -> Vec<String> {
    let Some(body) = title_section(raw, "Unfold") else {
        return Vec::new();
    };
    body.lines()
        .filter_map(|line| {
            let item = line.trim().strip_prefix("- ")?;
            let path = item.trim().trim_matches(['"', '\'', '`']);
            (!path.is_empty()).then(|| path.to_owned())
        })
        .collect()
}

/// The body of the section whose heading line carries the block anchor
/// `^{anchor}` (`# Properties ^properties`), from the heading to the next
/// heading. `None` ⇒ no such section.
fn anchor_section(raw: &str, anchor: &str) -> Option<String> {
    let token = format!("^{anchor}");
    section_body(raw, |heading| heading.contains(&token))
}

/// The body of the section whose heading TEXT's first word equals `title` (the
/// leading `#`s, anchor, and trailing tokens ignored). `None` ⇒ no such section.
fn title_section(raw: &str, title: &str) -> Option<String> {
    section_body(raw, |heading| {
        heading.trim_start_matches('#').split_whitespace().next() == Some(title)
    })
}

/// The body lines of the first section (heading to next heading) whose heading
/// line satisfies `pick`. FENCE-AWARE: a `#` line inside a ```` ``` ```` fenced
/// block (e.g. a `^template` record's own `# {{id}}` heading) is body, never a
/// section boundary. The frontmatter block is skipped so a `---` fence is never
/// mistaken for content.
fn section_body(raw: &str, pick: impl Fn(&str) -> bool) -> Option<String> {
    let body = strip_frontmatter(raw);
    let mut in_section = false;
    let mut in_fence = false;
    let mut out = String::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if in_section {
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        if !in_fence && trimmed.starts_with('#') {
            // A section heading: it closes the current section, or opens the
            // picked one. The heading line itself is never part of the body.
            if in_section {
                return Some(out);
            }
            if pick(trimmed) {
                in_section = true;
            }
            continue;
        }
        if in_section {
            out.push_str(line);
            out.push('\n');
        }
    }
    in_section.then_some(out)
}

/// The document body after the leading `---` frontmatter fence pair. A page with
/// no frontmatter returns unchanged.
fn strip_frontmatter(raw: &str) -> &str {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return raw;
    };
    match rest.find("\n---\n") {
        Some(idx) => &rest[idx + "\n---\n".len()..],
        None => raw,
    }
}

/// The content of the first fenced code block in `body` (lines strictly between
/// the opening ```` ``` ```` and the closing fence), fences excluded. `None` ⇒ no
/// fenced block.
fn fenced_block(body: &str) -> Option<String> {
    let mut lines = body.lines();
    // Advance to the opening fence.
    for line in lines.by_ref() {
        if line.trim_start().starts_with("```") {
            let mut out = String::new();
            for next in lines.by_ref() {
                if next.trim_start().starts_with("```") {
                    // Drop the trailing newline the last push added.
                    if out.ends_with('\n') {
                        out.pop();
                    }
                    return Some(out);
                }
                out.push_str(next);
                out.push('\n');
            }
            return None; // unterminated fence
        }
    }
    None
}

// ---------------------------------------------------------------------------
// def validation + `mrd new`
// ---------------------------------------------------------------------------

/// The outcome of a `new_record` birth: [`Born`](NewOutcome::Born) with the
/// birth receipt, or [`Refused`](NewOutcome::Refused) with the closed-taxonomy
/// reason (a `def_invalid` naming the rule, or a `cas_mismatch`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum NewOutcome {
    /// The record was born (or, on `--dry`, would be born) through the guarded
    /// create.
    Born(NewReport),
    /// The birth refused — `def_invalid` (the def is inconsistent) or
    /// `cas_mismatch` (the target already exists).
    Refused(NewRefusal),
}

/// A successful `new_record` birth.
#[derive(Debug, Clone, Serialize)]
pub struct NewReport {
    /// The born record path (workspace-relative).
    pub target: String,
    /// The born record's whole-file rev (computed from the body — present on a
    /// dry run too).
    pub rev: String,
    /// The birth receipt (`r-NNNNNN` journal anchor); `None` on a dry run.
    pub receipt: Option<String>,
    /// Whether this was a `--dry` rehearsal (nothing landed).
    pub dry: bool,
}

/// A refused `new_record` birth.
#[derive(Debug, Clone, Serialize)]
pub struct NewRefusal {
    /// The record path the birth targeted.
    pub target: String,
    /// The closed-taxonomy reason.
    pub reason: RefusalReason,
}

/// Birth one record from a def's `^template` (`mrd new <kind> <id>`, design d3
/// §1.3/§6): resolve the def, fill the `^template`, validate the filled record
/// against the def's `^properties`, and birth the first rev through the U2.6
/// guarded create — emitting the inline birth receipt.
///
/// A def that declares no `^properties`/`^template`, carries a malformed rule, or
/// whose `^template` cannot satisfy its own `^properties` is an INVALID def: the
/// birth refuses `def_invalid{rule}` (row 17), naming the def rule, and writes
/// nothing. A target that already exists refuses `cas_mismatch` (the `if_absent`
/// CAS). Neither refusal is a [`PresetError`].
///
/// # Errors
/// [`PresetError`] on a tool failure — the def is unreadable / not a def, or a
/// birth faults at the write door for a reason other than the CAS.
pub fn new_record(
    root: &fs::WorkspaceRoot,
    def_path: &str,
    id: &str,
    opts: &BirthOptions,
) -> Result<NewOutcome, PresetError> {
    let def = load_def(root, def_path)?;
    let target = birth_target(&def, id);

    // Structural def checks: the def must declare a ^properties block and a
    // ^template, and every rule must parse. Any failure is an invalid def.
    let Some(properties) = &def.properties else {
        return Ok(refuse_new(
            target,
            RefusalReason::def_invalid("the def declares no ^properties block"),
        ));
    };
    let Some(template) = &def.template else {
        return Ok(refuse_new(
            target,
            RefusalReason::def_invalid("the def declares no ^template block"),
        ));
    };
    if let Some(bad) = properties.iter().find(|r| r.key.is_empty()) {
        return Ok(refuse_new(
            target,
            RefusalReason::def_invalid(format!("malformed rule '{}'", bad.raw)),
        ));
    }

    // Fill the template and parse the record it would birth.
    let body = fill_template(template, id, &def.defines, opts);
    let record = model::build(body.clone(), syntax::parse(&body));

    // Validate the filled record against every ^properties rule. The FIRST
    // violation names the failing def rule (def_invalid, row 17).
    if let Some(rule) = first_violated_rule(properties, &record) {
        return Ok(refuse_new(
            target,
            RefusalReason::def_invalid(rule.raw.clone()),
        ));
    }

    // Birth the first rev through the guarded create (CAS if_absent, journaled).
    match birth(root, &target, &body, opts)? {
        BirthResult::Born(receipt) => Ok(NewOutcome::Born(NewReport {
            target,
            rev: record.root.node_rev.0.clone(),
            receipt,
            dry: opts.dry,
        })),
        BirthResult::Occupied(reason) => Ok(refuse_new(target, reason)),
    }
}

/// Assemble a [`NewOutcome::Refused`] over a target + reason.
fn refuse_new(target: String, reason: RefusalReason) -> NewOutcome {
    NewOutcome::Refused(NewRefusal { target, reason })
}

/// The birth target path: the def's `births:` template (`{{id}}`-filled), else
/// the default `{{kind}}/{{id}}.md`.
fn birth_target(def: &PresetDef, id: &str) -> String {
    match &def.births {
        Some(t) => fill_vars(t, id, &def.defines, None, None),
        None => format!("{}/{}.md", def.defines, id),
    }
}

/// The first `^properties` rule the record violates, or `None` if it satisfies
/// all of them. A rule is violated when its key is absent, or (for a pinned
/// rule) present with the wrong value.
fn first_violated_rule<'a>(rules: &'a [PropRule], record: &Document) -> Option<&'a PropRule> {
    rules.iter().find(|rule| {
        let observed = fm_scalar(record, &rule.key);
        match (&rule.value, observed) {
            (_, None) => true,                           // required key absent
            (Some(expected), Some(v)) => v != *expected, // pinned to the wrong value
            (None, Some(_)) => false,                    // present — satisfied
        }
    })
}

/// Fill a `^template` body: replace `{{id}}`, `{{kind}}`, `{{actor}}`, `{{now}}`.
fn fill_template(template: &str, id: &str, kind: &str, opts: &BirthOptions) -> String {
    fill_vars(
        template,
        id,
        kind,
        opts.actor.as_deref(),
        opts.now.as_deref(),
    )
}

/// Replace the four birth placeholders in `text`.
fn fill_vars(text: &str, id: &str, kind: &str, actor: Option<&str>, now: Option<&str>) -> String {
    text.replace("{{id}}", id)
        .replace("{{kind}}", kind)
        .replace("{{actor}}", actor.unwrap_or(""))
        .replace("{{now}}", now.unwrap_or(""))
}

// ---------------------------------------------------------------------------
// `mrd unfold`
// ---------------------------------------------------------------------------

/// The whole-scaffold materialization report.
#[derive(Debug, Clone, Serialize)]
pub struct UnfoldReport {
    /// The preset def path unfolded.
    pub preset: String,
    /// The convention-floor pins the preset's `inputs` declare (read through the
    /// U2.11 grain) — the law the born session lives under (d2 §5.5).
    pub floor: Vec<String>,
    /// Per-scaffold-file outcome, in declared order.
    pub files: Vec<FileOutcome>,
}

impl UnfoldReport {
    /// Whether every declared scaffold file was born — no file was refused by the
    /// `if_absent` CAS. A clean unfold exits 0; any occupied path is a finding.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.files
            .iter()
            .all(|f| matches!(f, FileOutcome::Born { .. }))
    }

    /// Every born file's `(path, receipt)` — the birth-receipt sweep surface.
    #[must_use]
    pub fn births(&self) -> Vec<(&str, Option<&str>)> {
        self.files
            .iter()
            .filter_map(|f| match f {
                FileOutcome::Born { path, receipt } => Some((path.as_str(), receipt.as_deref())),
                FileOutcome::Occupied { .. } => None,
            })
            .collect()
    }
}

/// One scaffold file's unfold outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FileOutcome {
    /// The file was born (or, on `--dry`, would be) through the guarded create.
    Born {
        /// The scaffold file path.
        path: String,
        /// The birth receipt (`r-NNNNNN`); `None` on a dry run.
        receipt: Option<String>,
    },
    /// The path already exists — the `if_absent` CAS refused the birth (the
    /// guarded-create semantics hold under unfold; nothing was clobbered).
    Occupied {
        /// The scaffold file path.
        path: String,
        /// The `cas_mismatch` reason.
        reason: RefusalReason,
    },
}

/// Materialize a preset's declared scaffold (`mrd unfold <preset>`, design d3 §6:
/// reconcile toward the declared scaffold). Every `# Unfold` file is born through
/// the U2.6 guarded create, so each carries a birth receipt (an `op=create`
/// journal row); a path that already exists refuses via the `if_absent` CAS and
/// is left byte-untouched. The root record is born pinning the preset (an
/// `inputs` block sequence rendered through the U2.11 safe grain).
///
/// # Errors
/// [`PresetError`] on a tool failure — the preset is unreadable / not a def, or a
/// birth faults at the write door for a reason other than the CAS.
pub fn unfold(
    root: &fs::WorkspaceRoot,
    preset_path: &str,
    opts: &BirthOptions,
) -> Result<UnfoldReport, PresetError> {
    let def = load_def(root, preset_path)?;
    let mut files = Vec::with_capacity(def.scaffold.len());
    for path in &def.scaffold {
        let body = if *path == def.root_record {
            render_root_record(&def, path, opts.now.as_deref())
        } else {
            render_stub(&def, path, opts.now.as_deref())
        };
        let outcome = match birth(root, path, &body, opts)? {
            BirthResult::Born(receipt) => FileOutcome::Born {
                path: path.clone(),
                receipt,
            },
            BirthResult::Occupied(reason) => FileOutcome::Occupied {
                path: path.clone(),
                reason,
            },
        };
        files.push(outcome);
    }
    Ok(UnfoldReport {
        preset: preset_path.to_owned(),
        floor: def.inputs.clone(),
        files,
    })
}

// ---------------------------------------------------------------------------
// reconcile-toward-scaffold (ZT ruling #3 — the asymmetric reconcile law)
// ---------------------------------------------------------------------------

/// The asymmetric reconcile plan — the ZT ruling #3 fold computed as a PURE
/// function of the declared scaffold, the declared-ephemeral allowlist, and the
/// live tree. The law, verbatim (design-3 §6; plan §4 Block 3 U3.5b):
///
/// - **Materialize (additive): ALL missing declared paths** (set-difference of
///   declared scaffold − live tree).
/// - **Remove (subtractive): ONLY declared-ephemeral files + empty-undeclared
///   directories** — an ALLOWLIST, never set-difference. (The empty-dir half is
///   computed by the apply, which has the tree; this plan carries the file half.)
/// - **Everything else — undeclared content files — renders as [`findings`], never
///   prune actions.** "Unused = no dependents" is a report, never an auto-delete.
///
/// [`findings`](ReconcilePlan::findings): reconcile is asymmetric — additive by
/// set-difference, subtractive by allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ReconcilePlan {
    /// Missing declared scaffold paths → materialize (guarded create). The
    /// additive half: set-difference of declared − live.
    pub materialize: Vec<String>,
    /// Declared-ephemeral files present in the tree → prune (guarded remove).
    /// The subtractive allowlist half — ONLY paths the def marks disposable.
    pub prune: Vec<String>,
    /// Undeclared content files present in the tree → check-plane FINDINGS,
    /// NEVER prune actions. The load-bearing asymmetry (E&R map: "undeclared
    /// non-empty path → never pruned → finding, not deletion").
    pub findings: Vec<String>,
}

/// Compute the asymmetric reconcile plan (PURE — the fold, no I/O). `declared` is
/// the def scaffold, `ephemeral` the declared-disposable allowlist (globs or exact
/// paths), `live_files` every live file path in the reconcile scope.
///
/// A live file is: converged if declared; pruned if it matches the ephemeral
/// allowlist; a FINDING otherwise (undeclared content is never pruned — the
/// allowlist, never set-difference, governs subtraction). A declared path absent
/// from the tree is materialized.
#[must_use]
pub fn reconcile_plan(
    declared: &[String],
    ephemeral: &[String],
    live_files: &[String],
) -> ReconcilePlan {
    use std::collections::BTreeSet;
    let declared_set: BTreeSet<&str> = declared.iter().map(String::as_str).collect();
    let live_set: BTreeSet<&str> = live_files.iter().map(String::as_str).collect();

    let materialize = declared
        .iter()
        .filter(|d| !live_set.contains(d.as_str()))
        .cloned()
        .collect();

    let mut prune = Vec::new();
    let mut findings = Vec::new();
    for file in live_files {
        if declared_set.contains(file.as_str()) {
            continue; // declared and present → converged, left byte-untouched
        }
        if ephemeral.iter().any(|pat| ephemeral_match(pat, file)) {
            prune.push(file.clone()); // declared-ephemeral → the prune allowlist
        } else {
            findings.push(file.clone()); // undeclared content → finding, NEVER pruned
        }
    }
    ReconcilePlan {
        materialize,
        prune,
        findings,
    }
}

/// Whether an ephemeral allowlist pattern matches a live path. `*.ext` matches any
/// path whose basename ends `.ext`; a pattern with no `*` matches an exact
/// workspace-relative path OR a bare basename. Deliberately minimal — the
/// allowlist is small and author-declared, never a broad glob engine.
fn ephemeral_match(pattern: &str, path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    if let Some(suffix) = pattern.strip_prefix('*') {
        base.ends_with(suffix)
    } else {
        pattern == path || pattern == base
    }
}

/// One reconcile outcome per acted-on path (materialize / prune), plus the
/// findings the check plane renders.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileReport {
    /// The reconciled preset def path.
    pub preset: String,
    /// Materialize outcomes — one per missing declared path (born, or occupied).
    pub materialized: Vec<FileOutcome>,
    /// Prune outcomes — one per declared-ephemeral file the reconcile removed.
    pub pruned: Vec<PruneOutcome>,
    /// Empty-undeclared directories the reconcile removed (rmdir; the dir half of
    /// the allowlist). Empty unless `--prune`.
    pub pruned_dirs: Vec<String>,
    /// Undeclared content files rendered as findings — reported, NEVER pruned.
    pub findings: Vec<String>,
}

impl ReconcileReport {
    /// Whether reconcile converged the tree to the scaffold with no residual
    /// finding — every declared path materialized (or already present) and no
    /// undeclared content file left drifting.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
            && self
                .materialized
                .iter()
                .all(|f| !matches!(f, FileOutcome::Occupied { .. }))
    }
}

/// One declared-ephemeral file's prune outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PruneOutcome {
    /// The file was removed (or, on `--dry`, would be) through the guarded remove.
    Removed {
        /// The removed file path.
        path: String,
        /// The death receipt (`r-NNNNNN`); `None` on a dry run.
        receipt: Option<String>,
    },
    /// The removal refused at the guarded door (a CAS/root drift or reserved
    /// path) — the file is left byte-untouched, the reason carried.
    Refused {
        /// The file path the prune targeted.
        path: String,
        /// The wire refusal, surfaced.
        reason: String,
    },
}

/// Reconcile the live tree toward a preset's declared scaffold (`mrd reconcile
/// <preset> [--prune]`; ZT ruling #3). Materializes ALL missing declared paths
/// through the U2.6 guarded create (each carries a birth receipt); with `prune`,
/// removes ONLY declared-ephemeral files (guarded remove) and empty-undeclared
/// directories (rmdir) — the subtractive allowlist. Undeclared content files are
/// rendered as findings, NEVER pruned. Every write rides the ONE guarded path.
///
/// The reconcile scope is the set of directories the declared scaffold occupies —
/// reconcile never scans or prunes outside the shape's own territory.
///
/// # Errors
/// [`PresetError`] on a tool failure — the preset is unreadable / not a def, or a
/// birth/death faults at the write door for a reason other than the guarded CAS.
pub fn reconcile(
    root: &fs::WorkspaceRoot,
    preset_path: &str,
    prune: bool,
    opts: &BirthOptions,
) -> Result<ReconcileReport, PresetError> {
    let def = load_def(root, preset_path)?;
    let live_files = scan_scope(root, &def.scaffold);
    let plan = reconcile_plan(&def.scaffold, &def.ephemeral, &live_files);

    // Additive: materialize every missing declared path (guarded create, byte
    // -untouched on an occupied path — same door as unfold).
    let mut materialized = Vec::with_capacity(plan.materialize.len());
    for path in &plan.materialize {
        let body = if *path == def.root_record {
            render_root_record(&def, path, opts.now.as_deref())
        } else {
            render_stub(&def, path, opts.now.as_deref())
        };
        let outcome = match birth(root, path, &body, opts)? {
            BirthResult::Born(receipt) => FileOutcome::Born {
                path: path.clone(),
                receipt,
            },
            BirthResult::Occupied(reason) => FileOutcome::Occupied {
                path: path.clone(),
                reason,
            },
        };
        materialized.push(outcome);
    }

    // Subtractive (allowlist, ONLY under `--prune`): declared-ephemeral files
    // through the guarded remove, then empty-undeclared directories (rmdir).
    let mut pruned = Vec::new();
    let mut pruned_dirs = Vec::new();
    if prune {
        for path in &plan.prune {
            pruned.push(prune_file(root, path, opts)?);
        }
        pruned_dirs = prune_empty_dirs(root, &def.scaffold, &plan.findings, opts.dry);
    }

    Ok(ReconcileReport {
        preset: preset_path.to_owned(),
        materialized,
        pruned_dirs,
        pruned,
        findings: plan.findings,
    })
}

/// Remove one declared-ephemeral file through the U2.6 guarded remove
/// (remove-what-you-read: read the live rev, then delete under that CAS). A
/// guarded refusal is carried as a [`PruneOutcome::Refused`], never a tool fault.
fn prune_file(
    root: &fs::WorkspaceRoot,
    path: &str,
    opts: &BirthOptions,
) -> Result<PruneOutcome, PresetError> {
    let doc =
        fs::load(root, std::path::Path::new(path)).map_err(|e| PresetError::Io(e.to_string()))?;
    let if_file_rev = wire::NodeRev(doc.root.node_rev.0.clone());
    let args = wire_serve::write::RemoveArgs {
        id: None,
        path: wire::Path(path.to_owned()),
        if_file_rev,
        actor: opts.actor.clone(),
        now: opts.now.clone(),
        if_root: None,
        dry: opts.dry,
    };
    match wire_serve::write::remove(root, 0, &args, &[]) {
        Ok(out) => Ok(PruneOutcome::Removed {
            path: path.to_owned(),
            receipt: out.journal_anchor,
        }),
        Err(e) => Ok(PruneOutcome::Refused {
            path: path.to_owned(),
            reason: format!(
                "{:?}: {}",
                e.code,
                e.message.as_deref().unwrap_or("guarded remove refused")
            ),
        }),
    }
}

/// Remove empty-undeclared directories in the scaffold scope (the dir half of the
/// prune allowlist). A directory is prunable iff it is EMPTY, is not a prefix of a
/// declared path, and holds no finding. Deepest-first so a nest of empty dirs
/// collapses in one pass. Raw `rmdir` (a directory carries no governed rev); dry
/// runs report without removing.
fn prune_empty_dirs(
    root: &fs::WorkspaceRoot,
    declared: &[String],
    findings: &[String],
    dry: bool,
) -> Vec<String> {
    let mut dirs: Vec<String> = scope_dirs(declared).into_iter().collect();
    // Deepest-first: more path separators = deeper.
    dirs.sort_by(|a, b| {
        b.matches('/')
            .count()
            .cmp(&a.matches('/').count())
            .then(b.cmp(a))
    });
    let declared_prefixes: std::collections::BTreeSet<&str> =
        declared.iter().flat_map(|d| ancestors_of(d)).collect();
    let mut removed = Vec::new();
    for dir in dirs {
        if declared_prefixes.contains(dir.as_str()) {
            continue; // a declared path lives under here — keep it
        }
        if findings.iter().any(|f| f.starts_with(&format!("{dir}/"))) {
            continue; // holds undeclared content — never pruned
        }
        let abs = root.0.join(&dir);
        let empty = std::fs::read_dir(&abs).is_ok_and(|mut it| it.next().is_none());
        if empty {
            if !dry {
                let _ = std::fs::remove_dir(&abs);
            }
            removed.push(dir);
        }
    }
    removed
}

/// Every workspace-relative ancestor directory of a path (`a/b/c.md` → `a`,
/// `a/b`), the set a directory must avoid to be "undeclared".
fn ancestors_of(path: &str) -> Vec<&str> {
    path.char_indices()
        .filter(|(_, ch)| *ch == '/')
        .map(|(i, _)| &path[..i])
        .collect()
}

/// The directories the scaffold occupies, walked live — the reconcile scope. Only
/// these dirs are scanned for undeclared content and empty-dir prune.
fn scope_dirs(declared: &[String]) -> std::collections::BTreeSet<String> {
    declared
        .iter()
        .flat_map(|d| ancestors_of(d))
        .map(str::to_owned)
        .collect()
}

/// The direct parent directory of a workspace-relative path (`a/b/c.md` → `a/b`,
/// `top.md` → `""` for the workspace root).
fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[..idx],
        None => "",
    }
}

/// Scan the reconcile scope for every live file path (workspace-relative,
/// forward-slashed). The scope is the set of directories that DIRECTLY hold a
/// declared scaffold file (including the workspace root for a top-level file) —
/// reconcile never reaches outside the shape's own territory.
fn scan_scope(root: &fs::WorkspaceRoot, declared: &[String]) -> Vec<String> {
    let scan_dirs: std::collections::BTreeSet<&str> =
        declared.iter().map(|d| parent_dir(d)).collect();
    let mut live = Vec::new();
    for dir in scan_dirs {
        let abs = if dir.is_empty() {
            root.0.clone()
        } else {
            root.0.join(dir)
        };
        let Ok(entries) = std::fs::read_dir(&abs) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // Skip engine/system files — a dotfile (`.meridian.toml`) or the
            // reserved journal is never "undeclared content" to reconcile.
            if name.starts_with('.') {
                continue;
            }
            let rel = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            if rel == fs::domain::RESERVED_JOURNAL_PATH {
                continue;
            }
            live.push(rel);
        }
    }
    live.sort();
    live.dedup();
    live
}

/// Render the session root record's birth bytes: it pins the PRESET (`inputs:
/// [preset@rev]`) so the declared shape is re-derivable forever (d3 §6). The pin
/// is a multi-line block sequence rendered through the U2.11 safe grain and
/// written atomically as the birth bytes — never a single-line properties patch.
fn render_root_record(def: &PresetDef, preset_path: &str, now: Option<&str>) -> String {
    let created = now.unwrap_or("");
    let inputs = render_block_sequence("inputs", &[format!("{}@{}", def.path, def.rev)]);
    format!(
        "---\ntype: {kind}\npreset: {preset_path}\ncreated: {created}\n{inputs}\n---\n\n# {kind} — born from {preset_path}\n",
        kind = def.defines,
    )
}

/// Render a non-root scaffold stub's birth bytes: a stamped record naming the
/// preset it was scaffolded from and its own path (a governed page, birth bytes
/// that carry a span so the birth is receiptable inline).
fn render_stub(def: &PresetDef, path: &str, now: Option<&str>) -> String {
    let created = now.unwrap_or("");
    format!(
        "---\ntype: scaffold\npreset: {preset}\npath: {path}\ncreated: {created}\n---\n\n# {path}\n",
        preset = def.path,
    )
}

/// Render a multi-line frontmatter block sequence (the U2.11 safe shape, d2
/// §5.5): the key line, then one indented `  - "item"` line per item. Reading it
/// back through the whole-value `fm_key` grain recovers exactly these items.
#[must_use]
pub fn render_block_sequence(key: &str, items: &[String]) -> String {
    use std::fmt::Write as _;
    let mut out = format!("{key}:");
    for item in items {
        let _ = write!(out, "\n  - \"{item}\"");
    }
    out
}

// ---------------------------------------------------------------------------
// The shared guarded birth
// ---------------------------------------------------------------------------

/// One guarded birth's result: [`Born`](BirthResult::Born) with the receipt, or
/// [`Occupied`](BirthResult::Occupied) when the `if_absent` CAS refused.
enum BirthResult {
    Born(Option<String>),
    Occupied(RefusalReason),
}

/// Birth one file through the U2.6 guarded create ([`wire_serve::write::create`]):
/// CAS `if_absent`, journaled birth, the gate seam over the bare commit (`&[]`).
/// The `if_absent` CAS is the single no-clobber guard — an occupied path returns
/// `cas_mismatch`, mapped to [`BirthResult::Occupied`], never a clobber.
///
/// # Errors
/// [`PresetError::Write`] on any write-door refusal OTHER than the CAS (a bad
/// path, an I/O fault) — the wire error, surfaced.
fn birth(
    root: &fs::WorkspaceRoot,
    path: &str,
    body: &str,
    opts: &BirthOptions,
) -> Result<BirthResult, PresetError> {
    let args = wire_serve::write::CreateArgs {
        id: None,
        path: wire::Path(path.to_owned()),
        body: body.to_owned(),
        actor: opts.actor.clone(),
        now: opts.now.clone(),
        if_root: None,
        dry: opts.dry,
    };
    match wire_serve::write::create(root, 0, &args, &[]) {
        Ok(out) => Ok(BirthResult::Born(out.journal_anchor)),
        Err(e) if is_cas_mismatch(&e) => {
            Ok(BirthResult::Occupied(RefusalReason::cas_mismatch(path)))
        }
        Err(e) => Err(PresetError::Write(format!("{e:?}"))),
    }
}

/// Whether a guarded-create refusal is the `if_absent` CAS mismatch (the path is
/// occupied) — the one refusal a birth treats as an occupancy finding, never a
/// tool fault.
fn is_cas_mismatch(err: &ErrorBody) -> bool {
    err.code == ErrorCode::CasMismatch
}

/// Whether a session preset's `inputs` pin the convention floor (the U4.4 floor
/// suite under `conventions/`) — non-empty and every pin under [`FLOOR_PREFIX`].
/// A read-only check over the parsed def (d2 §5.5, "inputs pins the pack floor").
#[must_use]
pub fn pins_floor(def: &PresetDef) -> bool {
    !def.inputs.is_empty() && def.inputs.iter().all(|pin| pin.starts_with(FLOOR_PREFIX))
}
