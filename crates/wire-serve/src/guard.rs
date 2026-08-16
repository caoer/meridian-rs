//! Fingerprint-or-force at every wire door's splice intake, per-edit.
//!
//! Content-mutating writes on every wire door require fingerprint match or
//! `force`; guard fields stay schema-optional; `force` is any client's
//! refuse→rewrite path, not a separate trust plane.
//!
//! # Frame legality vs semantic refusal
//! A guardless splice is a legal wire frame forever (decision 007): guard
//! fields stay optional and nothing here rejects a frame. This unit adds a
//! semantic refusal after decode. `u10_guard.rs` asserts the split.
//!
//! # The mount point
//! Native `edits` reach [`crate::write::splice`] without ever being lowered, so
//! a guard mounted at lowering is bypassed by a field rename. This guard mounts
//! at the intake post-lowering, the one point both faces have already reached.
//!
//! Each face is judged on its own rows: the native face on the lowered batch,
//! the plan face on its plan rows, where its guard tokens live (the file-grain
//! doc-root token `set_property.rev` has no slot in the lowered `Edit`, and a
//! section create's absence guard is a fact lowering has turned into a
//! parent-append). Both passes run here, so neither face reaches disk around
//! the other.
//!
//! # Scope
//! - Every edit mutating existing content demands its fingerprint, or `force`.
//!   The law is content-change-scoped, not replace-shaped: append is guarded
//!   like any other content change.
//! - Births are guarded by absence, never by fingerprint. A plan `create`
//!   demands that its section be absent; the whole-file birth op keeps its own
//!   `if_absent` CAS at the disk edge and is not touched here.
//! - `set_properties` demands a file-grain token: frontmatter semantics are
//!   file-scoped, so the doc-root token — not a key-line rev — is the honest
//!   grain.
//! - `force` is the one bypass, and it bypasses the fingerprint plane whole: a
//!   missing token and a stale token both land, and every bypassed plane is
//!   named back to the caller.
//!
//! # Every wire door enforces; there are no trust planes
//! The resident daemon — the one wire door (§3.3) — enforces; no door is
//! exempted for who is behind it. [`Origin`] is door bookkeeping. The
//! in-process path ([`Origin::InProcess`] — `mrd`, the run plane, tests) is not
//! a wire door, so the rule does not reach it: scope, not trust.

use std::path::PathBuf;

use wire::{
    Edit, EditShape, ErrorBody, ErrorCode, NodeRev, Path, PlanEdit, PutAt, SecRef, Severity, Span,
    Verdict,
};

/// Which door a splice arrived through — bookkeeping, stated by the caller and
/// never sniffed. It carries no trust class.
///
/// Deliberately no `Default`: a door added later must state which side of the
/// wire it is on, or a default would silently enrol it in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A decoded wire frame — the resident daemon's socket (the one wire
    /// door, §3.3). Every wire door enforces.
    Wire,
    /// An in-process call: `mrd`, the run plane, the test harness. Not a wire
    /// door, so the rule does not reach it.
    InProcess,
}

/// One §5.4 premise: an optional workspace-relative scope (`None` = the root
/// premise — the v2 world guard as a list entry) and its claimed value.
/// Constructed by in-process callers now (tests, the script door's touch
/// set) and by the cap-gated wire decode when the family's cap lands;
/// checked against the resident tree in `write.rs`, counted by the §5.5
/// Coverage Law here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Premise {
    /// Workspace-relative path node the premise names; `None` is the root.
    pub scope: Option<PathBuf>,
    pub value: PremiseValue,
}

/// A premise's claimed value: a spelled `Root`-family token, or lawful
/// absence (§5.6 — the reserved non-hex `absent`, a value, not an error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PremiseValue {
    /// A spelled token (`b3…:<64hex>`) — equality-compared, opaque.
    Token(String),
    /// The reserved `absent` value: the premise holds iff no node exists at
    /// the scope.
    Absent,
}

impl Premise {
    /// §5.5 coverage: is this premise's scope ancestor-or-self of `file`?
    /// The root premise covers everything; comparison is component-wise,
    /// never a string prefix (`a/b` must not cover `a/bc.md`).
    #[must_use]
    pub fn covers(&self, file: &Path) -> bool {
        match &self.scope {
            None => true,
            Some(scope) => std::path::Path::new(&file.0).starts_with(scope),
        }
    }
}

/// Resolve the §4.7 mint pair to a filesystem path. `None` is the world
/// mint (both spellings absent). Pair-both is a decode fault, not this
/// function's.
///
/// # Errors
/// `bad_request` on an undecodable `scope_bytes`.
pub fn mint_scope_path(
    scope: Option<&Path>,
    scope_bytes: Option<&str>,
) -> Result<Option<std::path::PathBuf>, Box<ErrorBody>> {
    match (scope, scope_bytes) {
        (None, None) => Ok(None),
        (Some(p), None) => Ok(Some(std::path::PathBuf::from(&p.0))),
        (None, Some(b64)) => Ok(Some(path_from_scope_bytes(b64)?)),
        (Some(_), Some(_)) => Err(crate::bad_request(wire::mint_pair_teaching())),
    }
}

/// Lower the wire sugar + list into the door's premise list.
///
/// `if_root` + no `scope` stays the v2 world guard (returned as the first
/// of the pair so `SpliceArgs.if_root` / coverage's `root_premise` stay
/// byte-identical). `if_root` + `scope` desugars to one scoped list entry
/// and consumes the world slot. List entries append. Pair faults that
/// escaped decode refuse here too (one function, every door).
///
/// # Errors
/// `bad_request` on a broken pair or an undecodable `scope_bytes`.
pub fn lower_premises(
    if_root: Option<wire::Root>,
    sugar_scope: Option<Path>,
    guards: &[wire::GuardEntry],
) -> Result<(Option<wire::Root>, Vec<Premise>), Box<ErrorBody>> {
    if sugar_scope.is_some() && if_root.is_none() {
        return Err(crate::bad_request(wire::broken_premise_pair_teaching(
            "scope without if_fingerprint",
        )));
    }
    let mut premises = Vec::with_capacity(guards.len() + usize::from(sugar_scope.is_some()));
    for entry in guards {
        premises.push(entry_to_premise(entry)?);
    }
    match (if_root, sugar_scope) {
        (Some(token), Some(scope)) => {
            premises.push(Premise {
                scope: Some(std::path::PathBuf::from(&scope.0)),
                value: premise_value(&token.0),
            });
            Ok((None, premises))
        }
        (if_root, None) => Ok((if_root, premises)),
        (None, Some(_)) => unreachable!("pair checked above"),
    }
}

fn entry_to_premise(entry: &wire::GuardEntry) -> Result<Premise, Box<ErrorBody>> {
    if entry.scope.is_some() && entry.scope_bytes.is_some() {
        return Err(crate::bad_request(wire::broken_premise_pair_teaching(
            "both scope and scope_bytes in one premise",
        )));
    }
    let scope = match (&entry.scope, &entry.scope_bytes) {
        (Some(p), None) => Some(std::path::PathBuf::from(&p.0)),
        (None, Some(b64)) => Some(path_from_scope_bytes(b64)?),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("pair checked above"),
    };
    Ok(Premise {
        scope,
        value: premise_value(&entry.fingerprint),
    })
}

fn premise_value(token: &str) -> PremiseValue {
    if token == "absent" {
        PremiseValue::Absent
    } else {
        PremiseValue::Token(token.to_owned())
    }
}

/// Decode a §5.4 `scope_bytes` (base64url over raw path bytes) into a
/// `PathBuf`. The bytes are the path as the OS stores it — not a UTF-8
/// `Path` noun.
fn path_from_scope_bytes(b64: &str) -> Result<std::path::PathBuf, Box<ErrorBody>> {
    let bytes = decode_base64url(b64).ok_or_else(|| {
        crate::bad_request(format!(
            "`scope_bytes` is not base64url: `{b64}` — mint the pair again \
             (fingerprint{{scope_bytes}}) and send the echoed spelling"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::path::PathBuf::from(std::ffi::OsString::from_vec(
            bytes,
        )))
    }
    #[cfg(not(unix))]
    {
        let s = String::from_utf8(bytes).map_err(|_| {
            crate::bad_request(
                "`scope_bytes` is not UTF-8 and this host cannot address raw \
                 path bytes",
            )
        })?;
        Ok(std::path::PathBuf::from(s))
    }
}

/// Base64url (RFC 4648 §5), padding optional. None on any illegal input —
/// the door refuses rather than guessing.
fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let stripped: Vec<u8> = input
        .as_bytes()
        .iter()
        .copied()
        .filter(|&c| c != b'=')
        .collect();
    if stripped.iter().any(|c| val(*c).is_none()) {
        return None;
    }
    let mut out = Vec::with_capacity(stripped.len() * 3 / 4);
    for chunk in stripped.chunks(4) {
        let a = val(chunk[0])?;
        let b = val(*chunk.get(1)?)?;
        out.push((a << 2) | (b >> 4));
        if chunk.len() >= 3 {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk.len() == 4 {
                let d = val(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    Some(out)
}

/// One plane a forced write bypassed — rendered back to the caller so a `force`
/// is never silent.
#[derive(Debug, Clone)]
pub struct Bypass {
    /// The plane's name, as the caller reads it.
    pub plane: &'static str,
    /// What was written past it, named exactly.
    pub subject: String,
}

/// The grain a guard is demanded at — the word the teaching message uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grain {
    /// A section, block, or frontmatter key: `if_node_rev`.
    Node,
    /// The whole document: the doc-root token.
    File,
}

/// Which slot the missing token belongs in. The two faces spell the same guard
/// differently, so a refusal must name the slot the caller actually has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Every slot is a rev slot — the shared postfix is the point, and each prefix
// names the face whose field the refusal must spell.
#[allow(clippy::enum_variant_names)]
enum Slot {
    /// A native edit: `if_node_rev`.
    NativeNodeRev,
    /// A plan `match` / `replace_section` / `append` row: `rev`.
    PlanRowRev,
    /// A plan `create` row at an occurrence-addressed parent: `rev`, the
    /// PARENT section's token.
    PlanCreateRev,
    /// A plan `set_property` row: `rev`, the doc-root token.
    PlanFileRev,
}

/// Why one demand went unmet, and therefore which refusal it mints.
#[derive(Debug)]
enum Unmet {
    /// No token at all — `guard_required` with its teaching message.
    NoGuard { grain: Grain, slot: Slot },
    /// A file-grain token that does not match the document — `cas_mismatch`.
    StaleFileRev { expected: String, actual: String },
    /// A birth whose subject already exists — `cas_mismatch`, the same class the
    /// whole-file birth raises on an occupied path.
    AlreadyBorn,
}

/// One unmet demand, with the subject it is about.
#[derive(Debug)]
struct Demand {
    subject: String,
    unmet: Unmet,
}

/// The guard. Run at the wire-origin splice intake, post-lowering, over both
/// faces. On a wire-origin write it demands a fingerprint for every edit that
/// changes existing content, absence for every birth, and a doc-root token for
/// `set_properties` — or an explicit `force`.
///
/// *(Amended per §5.5/A.1, 2026-08-15 law.)* The demand's satisfying set is
/// the §5.4 premise vocabulary, judged by the Coverage Law: a batch premise
/// whose scope is ancestor-or-self of this file (`premises`, or the sugar's
/// root premise `root_premise`) covers every missing-token demand in it.
/// `guard_required` keeps its exact meaning — a write carrying NO premise at
/// all; a write carrying premises that fail coverage refuses
/// `scope_does_not_cover` naming the uncovered target set. Validity demands
/// (a STALE supplied token, an occupied birth) are never waived by coverage.
///
/// Under `force` the demands are still computed, so the response can name every
/// bypassed plane, and `edits` has its node-grain tokens stripped: a forced
/// write with a stale rev lands and says so, instead of refusing at a CAS rung
/// `force` never reached.
///
/// Returns the bypassed planes — empty for an ordinary write, and empty for
/// [`Origin::InProcess`], which is exempt.
///
/// # Errors
/// `guard_required` when a demanded guard is absent and the write carries no
/// premise at all; `scope_does_not_cover` when supplied premises leave a
/// caller-authored target uncovered; `cas_mismatch` when a file-grain token
/// is stale or a birth's subject already exists. Nothing has been written
/// when any returns.
#[allow(clippy::too_many_arguments)]
pub fn guard_batch(
    origin: Origin,
    force: bool,
    doc: &model::Document,
    path: &Path,
    plan_edits: &[PlanEdit],
    edits: &mut [Edit],
    premises: &[Premise],
    root_premise: bool,
) -> Result<Vec<Bypass>, Box<ErrorBody>> {
    let demands = coverage_gate(
        origin,
        force,
        doc,
        path,
        plan_edits,
        edits,
        premises,
        root_premise,
    )?;
    validity_gate(force, path, demands, edits)
}

/// The demand set [`coverage_gate`] computed, carried to [`validity_gate`] —
/// the two phases share one computation so the §5.1 order (coverage →
/// premises → per-row validity) costs no second pass.
pub struct BatchDemands {
    unmet: Vec<Demand>,
    exempt: bool,
}

/// Phase 1 — §5.5 coverage at ADMISSION, before any premise value is
/// resolved and before any per-row token is compared. Refuses
/// `guard_required` (A.1: no premise at all) or `scope_does_not_cover`
/// (premises present, a target uncovered). Under `force` it only computes —
/// requiredness is exactly what `force` bypasses — so the caller can still
/// name every bypassed plane.
///
/// # Errors
/// `guard_required` | `scope_does_not_cover`; nothing written when either
/// returns.
#[allow(clippy::too_many_arguments)]
pub fn coverage_gate(
    origin: Origin,
    force: bool,
    doc: &model::Document,
    path: &Path,
    plan_edits: &[PlanEdit],
    edits: &[Edit],
    premises: &[Premise],
    root_premise: bool,
) -> Result<BatchDemands, Box<ErrorBody>> {
    if origin == Origin::InProcess {
        return Ok(BatchDemands {
            unmet: Vec::new(),
            exempt: true,
        });
    }

    let mut unmet = Vec::new();
    if plan_edits.is_empty() {
        native_demands(doc, edits, &mut unmet);
    } else {
        plan_demands(doc, plan_edits, &mut unmet);
    }

    // §5.5 coverage: a batch premise at this file or an ancestor covers the
    // missing-token demands (requiredness). Stale-token and occupied-birth
    // demands are validity, not requiredness — never waived.
    let covered = root_premise || premises.iter().any(|p| p.covers(path));
    if covered {
        unmet.retain(|d| !matches!(d.unmet, Unmet::NoGuard { .. }));
    }

    if !force
        && let Some(first) = unmet
            .iter()
            .find(|d| matches!(d.unmet, Unmet::NoGuard { .. }))
    {
        // A.1's split: NO premise at all → `guard_required` with its
        // teaching (today's refusal, byte-identical); premises present
        // but not covering → `scope_does_not_cover` naming the set.
        let any_premise = root_premise
            || !premises.is_empty()
            || edits.iter().any(|e| e.if_node_rev.is_some())
            || plan_rows_carry_a_rev(plan_edits);
        if any_premise {
            return Err(coverage_refusal(path, &unmet));
        }
        return Err(refusal(path, first));
    }
    Ok(BatchDemands {
        unmet,
        exempt: false,
    })
}

/// Phase 2 — the per-row VALIDITY rung, after every supplied premise was
/// checked (§5.1 order): a stale supplied token or an occupied birth refuses
/// `cas_mismatch`; under `force` the node-grain tokens are stripped (that is
/// what makes `force` reach CAS) and every bypassed plane is named.
///
/// # Errors
/// `cas_mismatch`; nothing written when it returns.
pub fn validity_gate(
    force: bool,
    path: &Path,
    demands: BatchDemands,
    edits: &mut [Edit],
) -> Result<Vec<Bypass>, Box<ErrorBody>> {
    if demands.exempt {
        return Ok(Vec::new());
    }
    let unmet = demands.unmet;
    if !force {
        if let Some(first) = unmet.first() {
            // Coverage answered the NoGuard class in phase 1; what is left
            // to refuse here is validity. (A NoGuard can still sit in the
            // list on the force path, for naming.)
            debug_assert!(
                !matches!(first.unmet, Unmet::NoGuard { .. }),
                "phase 1 refuses every NoGuard on the unforced path"
            );
            return Err(refusal(path, first));
        }
        return Ok(Vec::new());
    }

    // Forced: dropping the node-grain tokens is what makes `force` reach CAS —
    // the sealed batch no longer compares a rev the caller wrote past.
    for edit in edits.iter_mut() {
        edit.if_node_rev = None;
    }
    Ok(unmet
        .iter()
        .map(|d| Bypass {
            plane: match d.unmet {
                Unmet::NoGuard {
                    grain: Grain::File, ..
                }
                | Unmet::StaleFileRev { .. } => "content fingerprint (file grain)",
                Unmet::NoGuard {
                    grain: Grain::Node, ..
                } => "content fingerprint (node grain)",
                Unmet::AlreadyBorn => "birth absence",
            },
            subject: d.subject.clone(),
        })
        .collect())
}

/// Does any plan row carry its own rev token? One of A.1's "carries a
/// premise" facts — a row's rev is an exact-section (or doc-root) premise.
fn plan_rows_carry_a_rev(plan_edits: &[PlanEdit]) -> bool {
    plan_edits.iter().any(|row| match row {
        PlanEdit::SetProperty { rev, .. } => rev.is_some(),
        PlanEdit::Create { rev, .. }
        | PlanEdit::Match { rev, .. }
        | PlanEdit::ReplaceSection { rev, .. }
        | PlanEdit::Append { rev, .. } => rev.as_deref().is_some_and(|r| !r.is_empty()),
    })
}

/// The §5.5 coverage refusal: `scope_does_not_cover` naming the UNCOVERED
/// caller-authored target set (every missing-token demand), with the §8.2
/// register text. One refusal for the whole set — the single-error shape
/// names the set, not its first member.
fn coverage_refusal(path: &Path, unmet: &[Demand]) -> Box<ErrorBody> {
    let uncovered: Vec<String> = unmet
        .iter()
        .filter(|d| matches!(d.unmet, Unmet::NoGuard { .. }))
        .map(|d| d.subject.clone())
        .collect();
    let joined = uncovered.join(", ");
    let mut e = ErrorBody::new(ErrorCode::ScopeDoesNotCover);
    e.path = Some(path.clone());
    e.message = Some(wire::scope_does_not_cover_teaching(&joined, &path.0));
    e.uncovered = Some(uncovered);
    Box::new(e)
}

/// The native face: every edit is judged on the lowered/native `Edit` itself.
///
/// Only well-formed mutations of existing content are this rung's to answer. An
/// edit that is not one — a dangling or ambiguous target, a malformed `upsert`,
/// the birth of an absent frontmatter key — is skipped so the rung that
/// describes the caller's situation answers instead (`ref_not_found` /
/// `ambiguous_ref` / `bad_request`).
///
/// Pinned by `a_target_that_does_not_resolve_is_not_this_rungs_to_answer` and
/// the `selector_ambiguity` / `splice_e2e` suites.
fn native_demands(doc: &model::Document, edits: &[Edit], out: &mut Vec<Demand>) {
    for edit in edits {
        if edit.if_node_rev.is_some() {
            continue;
        }
        if !is_guardable_mutation(doc, edit) {
            continue;
        }
        out.push(Demand {
            subject: subject_of(&edit.target),
            unmet: Unmet::NoGuard {
                grain: Grain::Node,
                slot: Slot::NativeNodeRev,
            },
        });
    }
}

/// True when this edit is a well-formed mutation of existing content — the only
/// thing this rung governs. Everything else belongs to a rung that says
/// something more useful, so it is skipped rather than pre-empted.
fn is_guardable_mutation(doc: &model::Document, edit: &Edit) -> bool {
    // A ref the grammar cannot even translate (a block id outside the §2.4
    // charset) is the decode-adjacent rung's to refuse.
    let Ok(target) = crate::read::to_model_ref(&edit.target) else {
        return false;
    };

    // `put at:upsert` is the one create-or-replace shape and has its own domain
    // rules (an `fm_key` target, a single-line value): violations are
    // `bad_request` from the edit builder; an upsert of an absent key is a
    // birth, guarded by absence. Neither is a fingerprint question.
    if let EditShape::Put {
        at: PutAt::Upsert,
        text,
    } = &edit.edit
    {
        let model::Ref::FmKey(key) = &target else {
            return false;
        };
        if text.contains(['\n', '\r']) {
            return false;
        }
        return model::resolve(doc, &model::Ref::FmKey(key.clone())).is_ok();
    }

    // Everything else: it mutates existing content exactly when its target
    // resolves. A miss or an ambiguity is the resolution rung's to answer.
    model::resolve(doc, &target).is_ok()
}

/// The PLAN face, judged on its own rows: the file-grain token lives on
/// `set_property` and the birth-absence fact lives on `create`, and neither
/// survives lowering into a slot this guard could read.
fn plan_demands(doc: &model::Document, plan_edits: &[PlanEdit], out: &mut Vec<Demand>) {
    let file_rev = doc.root.node_rev.0.as_str();
    for row in plan_edits {
        match row {
            // Frontmatter is file-scoped, so the guard is the doc-root token —
            // a key-line rev would guard a grain the semantics do not live at.
            PlanEdit::SetProperty { key, rev, .. } => match rev.as_deref() {
                None => out.push(Demand {
                    subject: format!("frontmatter key \"{key}\""),
                    unmet: Unmet::NoGuard {
                        grain: Grain::File,
                        slot: Slot::PlanFileRev,
                    },
                }),
                Some(got) if got != file_rev => out.push(Demand {
                    subject: format!("frontmatter key \"{key}\""),
                    unmet: Unmet::StaleFileRev {
                        expected: got.to_owned(),
                        actual: file_rev.to_owned(),
                    },
                }),
                Some(_) => {}
            },
            // A birth: guarded by absence — and, at an occurrence-addressed
            // parent, by the caller's parent rev. `{h, n}` binds by position
            // among identical texts, and the absence check below resolves the
            // engine's OWN current answer, so it cannot see a sibling insert
            // re-binding `n` between the caller's read and this create. The
            // caller's parent rev is the only fact tying the birth to the
            // tree the caller read (Law A-1 at the create door). Occurrence
            // floor only: rev-free creates at unique parents stay legal.
            PlanEdit::Create {
                parent_hpath,
                title,
                rev,
                ..
            } => {
                if parent_hpath.iter().any(|s| s.n.is_some())
                    && rev.as_deref().is_none_or(str::is_empty)
                {
                    out.push(Demand {
                        subject: format!("section \"{}\"", crate::display_hpath(parent_hpath)),
                        unmet: Unmet::NoGuard {
                            grain: Grain::Node,
                            slot: Slot::PlanCreateRev,
                        },
                    });
                }
                let full = format!("{}/{title}", crate::display_hpath(parent_hpath));
                if section_exists(doc, parent_hpath, title) {
                    out.push(Demand {
                        subject: format!("section \"{full}\""),
                        unmet: Unmet::AlreadyBorn,
                    });
                }
            }
            // Content changes, all three: `append` does not escape by not being
            // replace-shaped.
            PlanEdit::Match { hpath, rev, .. }
            | PlanEdit::ReplaceSection { hpath, rev, .. }
            | PlanEdit::Append { hpath, rev, .. } => {
                if rev.as_deref().is_none_or(str::is_empty) {
                    // A `^id` row is a block, and the demand must say so — the
                    // read face splits the two planes deliberately, and a
                    // refusal calling a block a section sends the caller to
                    // the wrong listing.
                    let noun = if matches!(hpath.as_slice(),
                        [only] if only.h.starts_with('^') || only.h.starts_with("#^"))
                    {
                        "block"
                    } else {
                        "section"
                    };
                    out.push(Demand {
                        subject: format!("{noun} \"{}\"", crate::display_hpath(hpath)),
                        unmet: Unmet::NoGuard {
                            grain: Grain::Node,
                            slot: Slot::PlanRowRev,
                        },
                    });
                }
            }
        }
    }
}

/// Does `parent_hpath` + `title` already address a section? The birth's absence
/// guard — an ambiguous resolve counts as existing.
///
/// The parent arrives as segments and the child chain is a concat — never a
/// split on `/`, which would mis-parse a parent heading containing `/`.
fn section_exists(doc: &model::Document, parent_hpath: &[wire::HpathSeg], title: &str) -> bool {
    if parent_hpath.is_empty() {
        return false;
    }
    let segs: Vec<model::HpathSeg> = parent_hpath
        .iter()
        .map(|s| model::HpathSeg {
            h: s.h.clone(),
            n: s.n,
        })
        .chain(std::iter::once(model::HpathSeg {
            h: title.to_owned(),
            n: None,
        }))
        .collect();
    !matches!(
        model::resolve(doc, &model::Ref::Hpath(segs)),
        Err(model::ResolveError::NotFound)
    )
}

/// How an edit's target is named back to the caller.
fn subject_of(target: &SecRef) -> String {
    match target {
        SecRef::Hpath { hpath } => format!(
            "section \"{}\"",
            hpath
                .iter()
                .map(|s| s.h.as_str())
                .collect::<Vec<_>>()
                .join("/")
        ),
        SecRef::Anchor { anchor } => format!("block \"^{anchor}\""),
        SecRef::FmKey { fm_key } => format!("frontmatter key \"{fm_key}\""),
    }
}

/// Mint the refusal for the first unmet demand — the batch is refused whole, so
/// one demand is the whole answer. The message carries subject, cause, grain,
/// the fact that nothing landed, and a runnable command that mints the missing
/// token.
///
/// Rung zero of the mismatch-recovery ladder: a plain [`ErrorBody`] whose
/// `expected`/`actual`/`message` slots are what a ladder rung enriches.
fn refusal(path: &Path, demand: &Demand) -> Box<ErrorBody> {
    let file = path.0.as_str();
    let subject = &demand.subject;
    match &demand.unmet {
        Unmet::NoGuard { grain, slot } => {
            let cause = match slot {
                // The create demand has its own why: the occurrence class.
                Slot::PlanCreateRev => {
                    "a create under an OCCURRENCE-addressed parent (an `n`-bearing segment) \
                     carries the parent's fingerprint, or an explicit `force` — `n` binds by \
                     position among identical texts, and the child-absence check cannot see a \
                     sibling re-binding it (Law A-1)"
                }
                _ => match grain {
                    Grain::Node => {
                        "a wire write that changes existing content carries its fingerprint at \
                         NODE grain, or an explicit `force`"
                    }
                    Grain::File => {
                        "frontmatter semantics are file-scoped, so a wire write carries the \
                         doc-root token at FILE grain, or an explicit `force`"
                    }
                },
            };
            // The fix names the LAW first and a runnable command second, because
            // the callers of this door do not share one surface. A raw wire
            // client sends the token itself and `mrd read` is how it mints one.
            // A SCRIPT never sees a rev at all — since the CAS relaxation
            // (ruling 2026-08-13, `run-plane.md` § entry-rev threading) its
            // tokens thread from the entry state with no read ritual, so a
            // script row meeting this refusal means the entry state could not
            // name its target (or the CLI lane's mint trip failed): the
            // reachable remedy there is re-run against the current world, and
            // the message says so instead of teaching a dissolved ritual. One
            // message, both spellings: a face re-phrasing this text for its own
            // callers would fork the refusal across two repos.
            let fix = match slot {
                Slot::NativeNodeRev => format!(
                    "Fix: the token is the read you already did — send that node's `sec_rev` as \
                     `if_node_rev` on the edit (`mrd read {file} --json` mints one; a script \
                     threads its tokens from its entry state itself — from one, re-run)."
                ),
                Slot::PlanRowRev => format!(
                    "Fix: the token is the read you already did — send that section's `sec_rev` \
                     as `rev` on the plan edit (`mrd read {file} --json` mints one; a script \
                     threads its tokens from its entry state itself — from one, re-run)."
                ),
                Slot::PlanCreateRev => format!(
                    "Fix: run `mrd read {file} --json` and send the PARENT section's `sec_rev` as \
                     `rev` on the `create` — the toc read that named the `n`-bearing parent \
                     carries it."
                ),
                Slot::PlanFileRev => format!(
                    "Fix: the token is the read you already did — send its `file_rev` as `rev` on \
                     each `set_property` (`mrd read {file} --json` mints one; a script threads \
                     its tokens from its entry state itself — from one, re-run)."
                ),
            };
            let mut e = ErrorBody::new(ErrorCode::GuardRequired);
            e.path = Some(path.clone());
            e.message = Some(format!(
                "{subject} in {file} changes existing content with no fingerprint — {cause}. {} \
                 {fix}",
                crate::NO_PARTIAL_WRITE_CLAUSE
            ));
            Box::new(e)
        }
        Unmet::StaleFileRev { expected, actual } => {
            let mut e = ErrorBody::new(ErrorCode::CasMismatch);
            e.path = Some(path.clone());
            e.expected = Some(NodeRev(expected.clone()));
            e.actual = Some(NodeRev(actual.clone()));
            e.message = Some(format!(
                "{subject} in {file} carries a doc-root token that is not this document's — the \
                 file moved under your plan. {} Fix: run `mrd read {file} --json` for its current \
                 `file_rev`, re-decide, and send the fresh token.",
                crate::NO_PARTIAL_WRITE_CLAUSE
            ));
            Box::new(e)
        }
        Unmet::AlreadyBorn => {
            let mut e = ErrorBody::new(ErrorCode::CasMismatch);
            e.path = Some(path.clone());
            e.message = Some(format!(
                "{subject} in {file} already exists, and a birth is guarded by ABSENCE — a \
                 fingerprint cannot stand in for it. {} Fix: run `mrd read {file}` to see the \
                 section that is already there, then edit it with its `sec_rev` instead of \
                 creating it.",
                crate::NO_PARTIAL_WRITE_CLAUSE
            ));
            Box::new(e)
        }
    }
}

/// Render the bypassed planes as §11.1 verdicts — a forced write names what it
/// wrote past, on the rendered surface (not the journal).
#[must_use]
pub fn bypass_verdicts(bypasses: &[Bypass], doc: &model::Document, path: &Path) -> Vec<Verdict> {
    bypasses
        .iter()
        .map(|b| Verdict {
            rule: "fingerprint-or-force".to_owned(),
            severity: Severity::Warn,
            path: path.clone(),
            hpath: None,
            span: Span(0, 0),
            node_rev: NodeRev(doc.root.node_rev.0.clone()),
            message: format!(
                "forced write: bypassed the {} plane for {}",
                b.plane, b.subject
            ),
        })
        .collect()
}
