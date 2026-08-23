//! Presets + session birth (U5.3) — a preset is a def file whose `inputs` pin
//! the convention floor; [`unfold`] materializes the declared scaffold;
//! [`new_record`] births one record from the def's `^template` after validating
//! it against the def's `^properties`.
//!
//! Owns the preset-def grammar (`type: def`, `inputs` block sequence,
//! `^properties` / `^template` / `# Unfold` sections), its validation, and the
//! two births. Every birth rides the one write path
//! ([`wire_serve::write::create`], U2.6): CAS `if_absent`, journaled birth,
//! gate seam. Never invents a second write path, mints identity or a clock
//! (`actor`/`now` are caller-supplied, §9), or holds session policy.
//!
//! `inputs` is read through the U2.11 whole-value `fm_key` grain (d2 §5.5) —
//! never a line-oriented scan — and the preset pin a birth writes is rendered
//! as a whole block-sequence value ([`render_block_sequence`]) written
//! atomically as birth bytes, never a single-line properties upsert.

use model::{Document, Ref};
use serde::Serialize;
use std::collections::BTreeMap;

/// The default root record a session preset instantiates and pins the preset
/// into (d3 §6). Overridable by the def frontmatter key `root`.
const DEFAULT_ROOT_RECORD: &str = "SESSION.md";

/// The DEFAULT workspace prefix the convention floor lives under (the U4.4 floor
/// suite: `conventions/<slug>/CHECK.md`). A fallback, never a validity predicate
/// — the def's own `floor:` key answers first (run-plane.md § 6, Law 6.3; the
/// no-hard-coded-flow amendment, laws.md).
const DEFAULT_FLOOR_PREFIX: &str = "conventions/";

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
    /// shape is re-derivable forever (d3 §6).
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
    /// The workspace prefix this def's floor pins live under (`floor:` or the
    /// [`DEFAULT_FLOOR_PREFIX`]) — what [`pins_floor`] measures the pins against.
    pub floor_prefix: String,
    /// The `^properties` rules; `None` ⇒ the def declares no `^properties` block
    /// (a structural def defect [`new_record`] refuses).
    pub properties: Option<Vec<PropRule>>,
    /// The `^template` record body (fence-stripped); `None` ⇒ no `^template`.
    pub template: Option<String>,
    /// The `# Unfold` declared scaffold — the workspace-relative file paths
    /// [`unfold`] materializes, in declared order.
    pub scaffold: Vec<String>,
    /// The `# Ephemeral` declared-disposable allowlist — path globs (`*.lock`)
    /// or exact paths [`reconcile`] MAY prune (ruling #3). Empty ⇒ nothing is
    /// disposable, so reconcile prunes no file.
    pub ephemeral: Vec<String>,
    /// A `# Properties` heading stands in the body but carries no `^properties`
    /// id, so the anchor-driven loader found no block. Measured at load, because
    /// this is the ONE shape where "declares no ^properties block" reads as
    /// false to the author staring at their visible heading.
    pub anchorless_properties: bool,
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

    /// A `bad_request` refusal (recovery `fix`): the CALLER's value cannot be
    /// written where the template puts it. Distinct from `def_invalid`, which
    /// blames the def — here the def is well-formed and the birth envelope is
    /// what the door cannot represent, so the message must not name a def rule.
    #[must_use]
    fn bad_request(message: String) -> Self {
        RefusalReason {
            code: "bad_request",
            recovery: "fix",
            message,
            rule: None,
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
    let doc = fs::load(root, std::path::Path::new(def_path)).map_err(|e| {
        PresetError::Io(if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "no def page at {def_path} — searched exactly one path: {}. A bare \
                 kind resolves to `presets/<kind>.md` under the workspace root; a \
                 token carrying `/` or a `.md` suffix names its def page verbatim.",
                root.0.join(def_path).display()
            )
        } else {
            format!("{def_path}: {e}")
        })
    })?;
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
        floor_prefix: fm_scalar(&doc, "floor").unwrap_or_else(|| DEFAULT_FLOOR_PREFIX.to_owned()),
        properties: parse_properties(&doc.raw),
        anchorless_properties: parse_properties(&doc.raw).is_none()
            && title_section(&doc.raw, "Properties").is_some(),
        template: parse_template(&doc.raw),
        scaffold: parse_unfold(&doc.raw),
        ephemeral: parse_ephemeral(&doc.raw),
    })
}

/// The `^template` record body of a page — the fenced code block inside the
/// section whose heading line carries the `^template` anchor, fences stripped
/// (the same extraction [`load_def`] serves defs with). `None` ⇒ the page
/// declares no `^template`.
///
/// Public for doors that take a template page WITHOUT the full def contract —
/// the realise card mint reads its user-supplied card page through this one
/// extractor — so "where a template lives on a page" has exactly one owner.
#[must_use]
pub fn template_of(raw: &str) -> Option<String> {
    parse_template(raw)
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

/// Read a scalar frontmatter value off a parsed page, PUBLISHED through the one
/// value owner ([`model::fm_doc_publish`], wire-contract § A.6.1 + § A.6.1a).
/// This is a value plane: the `^properties` rule check compares what it returns
/// against a def-supplied string, so a fleet-canonical `status: "done"` read raw
/// would compare false against `done` and the face would render a
/// legitimate-looking "no violation" — § A.6's read-half defect, in a checker
/// instead of a script.
///
/// **A block scalar reaches here, and the key is not ours to bound.** The
/// earlier wording claimed multi-line block values were read through the grain
/// ([`read_inputs_grain`]) and "never here". That was false: the grain handles
/// `inputs` only, while [`first_violated_rule`] calls this with `rule.key` —
/// whatever key a preset page's `^properties` rule happens to declare. `status`,
/// `description` and `manifest` all carry block scalars on live pages today, so
/// the only thing keeping the class dormant is that nobody has written a
/// `type: preset` page yet (card
/// `scalar-text-trims-config-key-block-scalars`). Publishing through the seam
/// is what makes that safe; the doc comment asserting a property the code did
/// not enforce is what would have let the next reader skip the check.
fn fm_scalar(doc: &Document, key: &str) -> Option<String> {
    model::fm_doc_publish(doc, key)
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
            // A heading closes the current section or opens the picked one.
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

/// The missing-`^properties` refusal, which states the anchor rule (run-plane
/// § presets): the loader finds the block by its `^` id ON the heading line, so
/// a def carrying a visually complete `# Properties` section with no id declares
/// no block at all. Without the rule, the refusal tells an author that a heading
/// they are looking at does not exist.
///
/// The anchor-less heading clause rides only where the heading was MEASURED —
/// a def with no `# Properties` text anywhere gets the rule, not a diagnosis of
/// a heading it does not have.
fn missing_properties_refusal(def: &PresetDef) -> String {
    let mut out = String::from(
        "the def declares no ^properties block — the loader finds it by the `^properties` id \
         ON the heading line (`# Properties ^properties`), never by the heading text",
    );
    if def.anchorless_properties {
        out.push_str(
            ". This def HAS a `# Properties` heading; the missing byte is its anchor id. Fix: \
             add `^properties` to that heading line",
        );
    }
    out
}

/// Birth one record from a def's `^template` (`mrd new <kind> <id>`, d3
/// §1.3/§6): resolve the def, fill the `^template`, validate the filled record
/// against the def's `^properties`, and birth the first rev through the U2.6
/// guarded create.
///
/// A def that declares no `^properties`/`^template`, carries a malformed rule,
/// or whose `^template` cannot satisfy its own `^properties` refuses
/// `def_invalid{rule}` (row 17), naming the def rule, and writes nothing. A
/// target that already exists refuses `cas_mismatch`. Neither refusal is a
/// [`PresetError`].
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

    // Structural def checks — any failure is an invalid def.
    let Some(properties) = &def.properties else {
        return Ok(refuse_new(
            target,
            RefusalReason::def_invalid(missing_properties_refusal(&def)),
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

    let body = match fill_template(template, id, &def.defines, opts) {
        Ok(body) => body,
        Err(reason) => return Ok(refuse_new(target, reason)),
    };
    let record = model::build(body.clone(), syntax::parse(&body));

    // The FIRST violation names the failing def rule (def_invalid, row 17).
    if let Some(rule) = first_violated_rule(properties, &record) {
        return Ok(refuse_new(
            target,
            RefusalReason::def_invalid(rule.raw.clone()),
        ));
    }

    match birth(root, &target, &body, opts)? {
        BirthResult::Born => Ok(NewOutcome::Born(NewReport {
            target,
            rev: record.root.node_rev.0.clone(),
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

/// Fill a `^template` body with the four birth slots (`{{id}}`, `{{kind}}`,
/// `{{actor}}`, `{{now}}`) — `mrd new`'s door over [`fill_slots`], which owns
/// the fm-aware § A.6.3a encoding walk.
///
/// # Errors
/// A [`RefusalReason`] (`bad_request`/`fix`) when a filled frontmatter value
/// carries a newline ([`birth_newline_refusal`]'s wording of the
/// [`SlotNewline`] fact): the birth is REFUSED — never sanitized, because
/// §3.4 stamps `actor`/`now` exactly as given and a door that trims the
/// caller's identity falsifies the provenance it records.
fn fill_template(
    template: &str,
    id: &str,
    kind: &str,
    opts: &BirthOptions,
) -> Result<String, RefusalReason> {
    let actor = opts.actor.as_deref();
    let now = opts.now.as_deref();
    fill_slots(template, &birth_vars(id, kind, actor, now)).map_err(|e| birth_newline_refusal(&e))
}

/// A slot fill the frontmatter plane cannot carry — the MECHANISM half of the
/// § A.6.3a newline refusal: the value substituted for `placeholder` carried a
/// newline into the template's frontmatter block, on the line declaring `key`.
/// Door-neutral by design: `mrd new` words it as its §3.4 `bad_request`
/// ([`fill_template`]), the realise card mint as a card-mint fault — the
/// sentence each speaks is the door's, the fact is this struct's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotNewline {
    /// The frontmatter key whose line composed the newline (empty when the
    /// line declares no key — the placeholder then names the site).
    pub key: String,
    /// The placeholder (`{{actor}}`, `{{detail}}`, …) whose value carried it.
    pub placeholder: String,
}

/// Fill a `^template` body with a caller-supplied slot table — the fill half
/// of the one template mechanism ([`template_of`] is the extraction half).
///
/// The template's BODY fills verbatim. Inside its FRONTMATTER BLOCK the same
/// substitution is a **value-plane write** (wire-contract § A.6.3a), so it
/// goes through the one encoder every other value-plane door uses: the
/// emitted value is the plain form when the plain form decodes back to
/// exactly the composed string, and a double-quoted scalar otherwise. Without
/// this the door interpolated source bytes, and a value carrying `: ` or a
/// newline minted a SECOND key line — one key twice, which § A.3's
/// first-occurrence model cannot represent.
///
/// `mrd new` calls this with the four birth slots; the realise card mint with
/// the slots its engine owns. One mechanism, per-door slot tables — a second
/// fill implementation is how the § A.6.3a hazards come back.
///
/// # Errors
/// [`SlotNewline`] when a filled frontmatter value carries a newline — a
/// single-line YAML scalar cannot hold one and an escaped-scalar workaround
/// leaks (§ A.6.3), so the caller REFUSES in its own door's vocabulary,
/// never sanitizes.
pub fn fill_slots(template: &str, vars: &[(&str, &str)]) -> Result<String, SlotNewline> {
    let Some(fm_end) = frontmatter_block_end(template) else {
        return Ok(fill_all(template, vars));
    };

    let mut out = String::with_capacity(template.len());
    // The key a value line belongs to — carried across a block sequence, whose
    // `  - item` lines hold values for the key line above them.
    let mut key = String::new();
    for line in template[..fm_end].split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let eol = &line[text.len()..];
        if let Some(k) = line_key(text) {
            key = k.to_string();
        }
        out.push_str(&fill_fm_line(text, &key, vars)?);
        out.push_str(eol);
    }
    out.push_str(&fill_all(&template[fm_end..], vars));
    Ok(out)
}

/// The four birth placeholders paired with the values that replace them.
fn birth_vars<'a>(
    id: &'a str,
    kind: &'a str,
    actor: Option<&'a str>,
    now: Option<&'a str>,
) -> [(&'static str, &'a str); 4] {
    [
        ("{{id}}", id),
        ("{{kind}}", kind),
        ("{{actor}}", actor.unwrap_or("")),
        ("{{now}}", now.unwrap_or("")),
    ]
}

/// The byte just past the template's leading frontmatter block (its closing
/// fence line and newline included), or `None` when the template opens no block
/// or never closes one — in which case there is no frontmatter plane to govern
/// and the whole text fills as body.
fn frontmatter_block_end(template: &str) -> Option<usize> {
    let mut offset = template.strip_prefix("---\n").map(|_| 4)?;
    while offset < template.len() {
        let line = template[offset..].split_inclusive('\n').next()?;
        if line.trim_end_matches(['\n', '\r']) == "---" {
            return Some(offset + line.len());
        }
        offset += line.len();
    }
    None
}

/// The key a frontmatter line declares (`key:` at the line start), or `None` for
/// a sequence item, a continuation, or a comment.
fn line_key(text: &str) -> Option<&str> {
    if text.starts_with([' ', '\t', '-', '#']) {
        return None;
    }
    let key = text.split_once(':')?.0;
    (!key.is_empty()).then_some(key)
}

/// Fill one frontmatter line. A placeholder standing in a VALUE position is
/// composed and then encoded (§ A.6.3); anything else fills verbatim, with the
/// newline refusal still standing — a newline is illegal anywhere in this block.
fn fill_fm_line(text: &str, key: &str, vars: &[(&str, &str)]) -> Result<String, SlotNewline> {
    if !text.contains("{{") {
        return Ok(text.to_string());
    }
    if let Some(split) = value_region(text)
        && text[split..].contains("{{")
    {
        let (prefix, region) = text.split_at(split);
        let core = quoted_lone_placeholder(region).unwrap_or_else(|| region.trim_end());
        let composed = fill_all(core, vars);
        let encoded =
            policy::defs::yaml_safe_value(&composed).map_err(|_| slot_newline(key, core, vars))?;
        return Ok(format!("{prefix}{encoded}"));
    }
    // No value region (a placeholder in the KEY position, or a shape this door
    // does not model): the fill stays verbatim, but a newline still cannot ride
    // into the block — it would mint lines the caller never wrote.
    if let Some((name, _)) = substituted_multiline(text, vars) {
        return Err(SlotNewline {
            key: key.to_owned(),
            placeholder: (*name).to_owned(),
        });
    }
    Ok(fill_all(text, vars))
}

/// Where a frontmatter line's VALUE starts: past `key:` and its one space, or
/// past a sequence item's `- `. `None` when the line is neither shape.
fn value_region(text: &str) -> Option<usize> {
    let indent = text.len() - text.trim_start().len();
    if let Some(rest) = text.trim_start().strip_prefix('-') {
        return Some(indent + 1 + (rest.len() - rest.trim_start().len()));
    }
    let colon = text.find(':')?;
    let rest = &text[colon + 1..];
    Some(colon + 1 + (rest.len() - rest.trim_start().len()))
}

/// The placeholder inside an author-quoted lone value (`"{{actor}}"`), whose
/// quotes spell "this whole value is the variable" — the encoder re-mints that
/// quoting canonically, so stripping them here reads the author's intent rather
/// than dropping it.
fn quoted_lone_placeholder(region: &str) -> Option<&str> {
    let r = region.trim();
    let inner = r
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| r.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))?;
    (inner.starts_with("{{") && inner.ends_with("}}") && inner.len() > 4).then_some(inner)
}

/// Replace every birth placeholder in `text`.
fn fill_all(text: &str, vars: &[(&str, &str)]) -> String {
    vars.iter().fold(text.to_string(), |acc, (name, value)| {
        acc.replace(name, value)
    })
}

/// The first placeholder that both occurs in `text` and carries a newline.
fn substituted_multiline<'a>(
    text: &str,
    vars: &'a [(&'a str, &'a str)],
) -> Option<&'a (&'a str, &'a str)> {
    vars.iter()
        .find(|(name, value)| text.contains(name) && value.contains(['\n', '\r']))
}

/// The [`SlotNewline`] for whichever placeholder in `core` carried the newline
/// — the composed value is the caller's, so the fact names which value it was.
/// (Unreachable fallback: the encoder only fails on a newline, and a newline
/// in a single-line `core` can only arrive by substitution.)
fn slot_newline(key: &str, core: &str, vars: &[(&str, &str)]) -> SlotNewline {
    let placeholder = substituted_multiline(core, vars).map_or("{{?}}", |(name, _)| *name);
    SlotNewline {
        key: key.to_owned(),
        placeholder: placeholder.to_owned(),
    }
}

/// `mrd new`'s wording of a [`SlotNewline`]: the uniform § A.6.3a sentence —
/// one owner, `policy::defs` — plus this door's provenance clause naming the
/// birth placeholder that carried the newline.
fn birth_newline_refusal(e: &SlotNewline) -> RefusalReason {
    let SlotNewline { key, placeholder } = e;
    let key = if key.is_empty() { placeholder } else { key };
    RefusalReason::bad_request(format!(
        "{} — the birth door filled {placeholder} there, and `mrd new` stamps a \
         caller's value exactly as given (§3.4), so it refuses rather than rewrite it",
        policy::defs::multi_line_value_refusal(key)
    ))
}

/// Replace the four birth placeholders in `text`.
fn fill_vars(text: &str, id: &str, kind: &str, actor: Option<&str>, now: Option<&str>) -> String {
    fill_all(text, &birth_vars(id, kind, actor, now))
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

    /// Every born file's path — the birth sweep surface.
    #[must_use]
    pub fn births(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter_map(|f| match f {
                FileOutcome::Born { path } => Some(path.as_str()),
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
    },
    /// The path already exists — the `if_absent` CAS refused the birth;
    /// nothing was clobbered.
    Occupied {
        /// The scaffold file path.
        path: String,
        /// The `cas_mismatch` reason.
        reason: RefusalReason,
    },
}

/// Materialize a preset's declared scaffold (`mrd unfold <preset>`, d3 §6).
/// Every `# Unfold` file is born through the U2.6 guarded create; a path that
/// already exists refuses via the `if_absent` CAS and is left byte-untouched.
/// The root record is born pinning the preset (an `inputs` block sequence
/// rendered through the U2.11 safe grain).
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
            render_root_record(&def, opts.now.as_deref())
        } else {
            render_stub(&def, path, opts.now.as_deref())
        };
        let outcome = match birth(root, path, &body, opts)? {
            BirthResult::Born => FileOutcome::Born { path: path.clone() },
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
// reconcile-toward-scaffold (ruling #3 — the asymmetric reconcile law)
// ---------------------------------------------------------------------------

/// The asymmetric reconcile plan (ruling #3), a pure function of the declared
/// scaffold, the declared-ephemeral allowlist, and the live tree:
///
/// - Materialize (additive): ALL missing declared paths — set-difference.
/// - Remove (subtractive): ONLY declared-ephemeral files + empty-undeclared
///   directories — an allowlist, never set-difference. (The empty-dir half is
///   computed by the apply; this plan carries the file half.)
/// - Undeclared content files render as [`findings`](ReconcilePlan::findings),
///   never prune actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ReconcilePlan {
    /// Missing declared scaffold paths → materialize (guarded create).
    pub materialize: Vec<String>,
    /// Declared-ephemeral files present in the tree → prune (guarded remove).
    /// Only paths the def marks disposable.
    pub prune: Vec<String>,
    /// Undeclared content files present in the tree → check-plane findings,
    /// never prune actions.
    pub findings: Vec<String>,
}

/// Compute the asymmetric reconcile plan (pure, no I/O). A live file is:
/// converged if declared; pruned if it matches the ephemeral allowlist; a
/// finding otherwise. A declared path absent from the tree is materialized.
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
    /// Undeclared content files rendered as findings — reported, never pruned.
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
/// <preset> [--prune]`; ruling #3). Materializes all missing declared paths
/// through the U2.6 guarded create; with `prune`, removes only
/// declared-ephemeral files (guarded remove) and empty-undeclared directories
/// (rmdir). Undeclared content files are rendered as findings, never pruned.
///
/// The reconcile scope is the set of directories the declared scaffold
/// occupies — reconcile never scans or prunes outside it.
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

    // Additive: materialize every missing declared path (guarded create, same
    // door as unfold).
    let mut materialized = Vec::with_capacity(plan.materialize.len());
    for path in &plan.materialize {
        let body = if *path == def.root_record {
            render_root_record(&def, opts.now.as_deref())
        } else {
            render_stub(&def, path, opts.now.as_deref())
        };
        let outcome = match birth(root, path, &body, opts)? {
            BirthResult::Born => FileOutcome::Born { path: path.clone() },
            BirthResult::Occupied(reason) => FileOutcome::Occupied {
                path: path.clone(),
                reason,
            },
        };
        materialized.push(outcome);
    }

    // Subtractive (allowlist, only under `--prune`): declared-ephemeral files
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
        if_file_rev: Some(if_file_rev),
        actor: opts.actor.clone(),
        now: opts.now.clone(),
        if_root: None,
        dry: opts.dry,
    };
    match wire_serve::write::remove(root, None, &args, &[]) {
        Ok(_) => Ok(PruneOutcome::Removed {
            path: path.to_owned(),
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

/// Remove empty-undeclared directories in the scaffold scope (the dir half of
/// the prune allowlist, §5.3). A directory is prunable iff it lives strictly
/// beneath a directory the scaffold itself creates, is not a prefix of a
/// declared path, holds no finding, and is empty. Deepest-first, so a nest of
/// empty dirs collapses in one pass. Raw `rmdir` — a directory carries no
/// governed rev and no bytes, the one guarded-door exception (§3.3); dry runs
/// report without removing.
///
/// The candidate set must be walked live, not derived from the declared paths:
/// a declared path only ever names its own ancestors, which are exactly the
/// directories that must be KEPT.
fn prune_empty_dirs(
    root: &fs::WorkspaceRoot,
    declared: &[String],
    findings: &[String],
    dry: bool,
) -> Vec<String> {
    let scaffold_dirs = scope_dirs(declared);
    let mut dirs = live_subdirs(root, &scaffold_dirs);
    // Deepest-first: more path separators = deeper. A parent that held only
    // empty children is itself empty by the time it is reached.
    dirs.sort_by(|a, b| {
        b.matches('/')
            .count()
            .cmp(&a.matches('/').count())
            .then(b.cmp(a))
    });
    let mut removed = Vec::new();
    for dir in dirs {
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

/// Every live directory strictly beneath a scaffold directory, excluding the
/// scaffold directories themselves (§5.3). A scaffold that declares only
/// top-level files offers no candidate: the workspace root is never walked.
fn live_subdirs(
    root: &fs::WorkspaceRoot,
    scaffold_dirs: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut queue: Vec<String> = scaffold_dirs.iter().cloned().collect();
    let mut found = Vec::new();
    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(root.0.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // A dotfile directory (`.git`) is engine/system, never the shape's.
            if name.starts_with('.') {
                continue;
            }
            let rel = format!("{dir}/{name}");
            // A scaffold directory is already in the queue and must be KEPT: a
            // declared path lives under it.
            if !scaffold_dirs.contains(&rel) {
                found.push(rel.clone());
                queue.push(rel);
            }
        }
    }
    found
}

/// Every workspace-relative ancestor directory of a path (`a/b/c.md` → `a`,
/// `a/b`), the set a directory must avoid to be "undeclared".
fn ancestors_of(path: &str) -> Vec<&str> {
    path.char_indices()
        .filter(|(_, ch)| *ch == '/')
        .map(|(i, _)| &path[..i])
        .collect()
}

/// The directories the declared scaffold CREATES — every ancestor of a declared
/// path. These are kept unconditionally; the empty-dir prune walks beneath them.
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
            // Skip engine/system files — a dotfile (`.git`, `.DS_Store`) is
            // never "undeclared content" to reconcile.
            if name.starts_with('.') {
                continue;
            }
            let rel = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            live.push(rel);
        }
    }
    live.sort();
    live.dedup();
    live
}

/// Render the session root record's birth bytes: `inputs` pins the DEF
/// (`defpath@rev`) so the declared shape is re-derivable forever (d3 §1), then
/// every declared floor pin, in declared order, so the law the session was born
/// under is readable from the session itself (d3 §6, Law 6.2). The pin sequence
/// is a multi-line block sequence rendered through the U2.11 safe grain and
/// written atomically as the birth bytes — never a single-line properties patch.
/// `preset:` names the DEF, never the record itself (Law 3.5).
fn render_root_record(def: &PresetDef, now: Option<&str>) -> String {
    let created = now.unwrap_or("");
    let mut pins = Vec::with_capacity(1 + def.inputs.len());
    pins.push(format!("{}@{}", def.path, def.rev));
    pins.extend(def.inputs.iter().cloned());
    let inputs = render_block_sequence("inputs", &pins);
    format!(
        "---\ntype: {kind}\npreset: {preset}\ncreated: {created}\n{inputs}\n---\n\n# {kind} — born from {preset}\n",
        kind = def.defines,
        preset = def.path,
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

/// One guarded birth's result: [`Born`](BirthResult::Born), or
/// [`Occupied`](BirthResult::Occupied) when the `if_absent` CAS refused.
enum BirthResult {
    Born,
    Occupied(RefusalReason),
}

/// Birth one file through the U2.6 guarded create ([`wire_serve::write::create`]):
/// CAS `if_absent`, the gate seam over the bare commit (`&[]`).
/// The `if_absent` CAS is the single no-clobber guard — an occupied path returns
/// `cas_mismatch` with `expected` = [`wire::ABSENT_REV`], mapped to
/// [`BirthResult::Occupied`], never a clobber. Every OTHER `cas_mismatch`
/// variety is a write fault and surfaces as one.
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
        fields: BTreeMap::default(),
        // A preset ships whole documents — its frontmatter is body bytes.
        props: BTreeMap::default(),
    };
    match wire_serve::write::create(root, None, &args, &[]) {
        Ok(_) => Ok(BirthResult::Born),
        // The occupancy finding is keyed on the refusal's `expected`, not on
        // this call site: `cas_mismatch` also spells the drift/remove-CAS and
        // the splice verdict, and reading one of THOSE as "already there"
        // would report a birth that never happened.
        Err(e) if e.is_path_occupied() => {
            Ok(BirthResult::Occupied(RefusalReason::cas_mismatch(path)))
        }
        Err(e) => Err(PresetError::Write(format!("{e:?}"))),
    }
}

/// Whether a session preset's `inputs` pin the convention floor — non-empty and
/// every pin under the def's OWN floor prefix ([`PresetDef::floor_prefix`]:
/// `floor:`, else [`DEFAULT_FLOOR_PREFIX`]). A read-only check over the parsed
/// def (d2 §5.5, "inputs pins the pack floor"); the prefix is the def's
/// declaration, so a floor filed anywhere is as valid (run-plane.md § 6,
/// Law 6.3).
#[must_use]
pub fn pins_floor(def: &PresetDef) -> bool {
    !def.inputs.is_empty()
        && def
            .inputs
            .iter()
            .all(|pin| pin.starts_with(&def.floor_prefix))
}
