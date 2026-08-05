//! U10 — **fingerprint-or-force** at every wire door's splice intake, per-edit.
//!
//! # The ruling this implements (ZT, 2026-08-03), verbatim
//! > Content-mutating writes on every wire door require fingerprint match or
//! > force; guard fields stay schema-optional; force is any client's
//! > refuse→rewrite path; MCP is the main agent client that implements that
//! > path, not a separate trust plane.
//!
//! # Frame legality vs semantic refusal — the distinction the whole unit rests on
//! Decision 007 says a guardless splice is a legal wire frame forever. That
//! survives **in full at the schema half**: guard fields stay OPTIONAL, a
//! guardless frame DECODES, and nothing here rejects a frame. What this unit
//! adds is a SEMANTIC refusal after decode — the frame is well-formed and the
//! WRITE is refused. A path that rejected the frame instead would violate the
//! ruling, not merely this module's intent; `u10_guard.rs` asserts the split.
//!
//! # Why here and not at plan lowering
//! Plan lowering is reachable only through the plan face: native `edits` reach
//! [`crate::write::splice`] without ever being lowered, so a guard mounted at
//! lowering is bypassed by a field rename. This guard mounts at the intake
//! **post-lowering**, the one point both faces have already reached.
//!
//! # The two faces are judged on their own rows, at the one mount
//! The lowered batch is what the NATIVE face is judged on. The PLAN face is
//! judged on its plan rows, because that is where its guard tokens live: the
//! file-grain doc-root token (`set_property.rev`, P3) has no slot in the lowered
//! `Edit`, and a section create's absence guard is a fact about a section the
//! lowered form has already turned into a parent-append. Both passes run at this
//! one mount, so neither face can reach disk around the other.
//!
//! # Scope (deliberate, each rule is a ruling)
//! - Every edit mutating **existing content** demands its fingerprint, or `force`.
//!   The law is content-change-scoped, **not replace-shaped** (security finding
//!   S3): append is a content change and is guarded like any other.
//! - **Births are guarded by absence**, never by fingerprint — a thing that does
//!   not exist has no fingerprint to match. A plan `create` demands that its
//!   section be absent; the whole-file birth op (`create`) keeps its own
//!   `if_absent` CAS at the disk edge and is not touched here.
//! - **`set_properties` demands a FILE-grain token.** Frontmatter semantics are
//!   file-scoped, so the doc-root token — not a key-line rev — is the honest
//!   grain (P3).
//! - **`force` is the ONE bypass** — any client's refuse→rewrite path — and it
//!   bypasses the FINGERPRINT PLANE whole: a missing token and a stale token both
//!   land, and every plane the write bypassed is named back to the caller (P15).
//!   Before this unit `force` and CAS were disconnected; [`guard_batch`] joins them.
//!
//! # EVERY wire door enforces; there are no trust planes here
//! Both the resident daemon and the per-workspace sidecar are wire doors and both
//! enforce. MCP is the main agent client that implements the refuse→rewrite path,
//! **not a separate trust plane** — so no door is guarded or exempted for WHO is
//! behind it. [`Origin`] is door bookkeeping, nothing more.
//!
//! The in-process path ([`Origin::InProcess`] — `mrd`, the run plane, tests) is
//! not a wire door, so the ruling does not reach it and its behaviour is
//! unchanged. That is scope, not trust: a reader who finds `mrd put` writing
//! without a fingerprint is looking at a path the ruling does not govern.
//!
//! The **run-plane effects shim is a DIFFERENT DOOR** and carries no fingerprint
//! unless ZT re-rules — requirements **decision 18**, which supersedes the
//! earlier Q3 reference as the governing authority. A NON-GOAL of this unit: do
//! not guard it here.

use wire::{
    Edit, EditShape, ErrorBody, ErrorCode, NodeRev, Path, PlanEdit, PutAt, SecRef, Severity, Span,
    Verdict,
};

/// WHICH DOOR a splice arrived through — bookkeeping, stated by the caller and
/// never sniffed. It carries no trust class: every wire door enforces
/// identically, whoever is behind it.
///
/// There is deliberately NO `Default`: a door added later must STATE which side
/// of the wire it is on, because a default would silently enrol it in whichever
/// side the default named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A decoded wire frame — the resident daemon's socket and the sidecar's
    /// stdio host alike. EVERY wire door enforces.
    Wire,
    /// An in-process call: `mrd`, the run plane, the test harness. Not a wire
    /// door, so the ruling does not reach it; behaviour is unchanged here.
    InProcess,
}

/// One plane a forced write bypassed — rendered back to the caller so a `force`
/// is never silent (P15).
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

/// WHICH slot the missing token belongs in. The two faces spell the same guard
/// differently, so a refusal that names one slot for both teaches the wrong fix
/// to half its readers — this is what keeps the fix clause honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Every slot IS a rev slot — the shared postfix is the point, and each prefix
// names the face whose field the refusal must spell.
#[allow(clippy::enum_variant_names)]
enum Slot {
    /// A native edit: `if_node_rev`.
    NativeNodeRev,
    /// A plan `match` / `replace_section` / `append` row: `rev`.
    PlanRowRev,
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

/// **The guard.** Run at the wire-origin splice intake, post-lowering, over both
/// faces. On a wire-origin write it demands a fingerprint for every edit that
/// changes existing content, absence for every birth, and a doc-root token for
/// `set_properties` — or an explicit `force`.
///
/// When `force` is set the demands are still COMPUTED, so the response can name
/// every plane the write bypassed; `edits` then has its node-grain tokens
/// stripped, which is what joins `force` to CAS (they were two disconnected
/// mechanisms before this unit): a forced write with a stale rev lands and says
/// so, instead of refusing at a CAS rung `force` never reached.
///
/// Returns the bypassed planes — empty for an ordinary write, and empty for
/// [`Origin::InProcess`], which is exempt.
///
/// # Errors
/// `guard_required` when a demanded guard is absent, or `cas_mismatch` when a
/// file-grain token is stale or a birth's subject already exists. Nothing has
/// been written when either returns.
pub fn guard_batch(
    origin: Origin,
    force: bool,
    doc: &model::Document,
    path: &Path,
    plan_edits: &[PlanEdit],
    edits: &mut [Edit],
) -> Result<Vec<Bypass>, Box<ErrorBody>> {
    if origin == Origin::InProcess {
        return Ok(Vec::new());
    }

    let mut unmet = Vec::new();
    if plan_edits.is_empty() {
        native_demands(doc, edits, &mut unmet);
    } else {
        plan_demands(doc, plan_edits, &mut unmet);
    }

    if !force {
        if let Some(first) = unmet.first() {
            return Err(refusal(path, first));
        }
        return Ok(Vec::new());
    }

    // Forced: the fingerprint plane is bypassed whole. Dropping the node-grain
    // tokens here is what makes `force` reach CAS — the sealed batch below no
    // longer compares a rev the caller asked us to write past.
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

/// The NATIVE face: every edit is judged on the lowered/native `Edit` itself.
///
/// # This rung speaks ONLY about well-formed mutations of existing content
/// The law is scoped to edits that mutate EXISTING content. An edit that is not
/// one — a dangling or ambiguous target, a malformed `upsert`, the birth of an
/// absent frontmatter key — is skipped here so the rung that actually describes
/// the caller's situation gets to answer.
///
/// This is a CORRECTNESS requirement, not politeness, and it was a real defect
/// before it was a rule. A guard that speaks first answers "you need a
/// fingerprint" for a node that does not exist, or for a request that is simply
/// malformed — sending the caller to fetch a rev that cannot help them, and
/// burying `ref_not_found` / `ambiguous_ref` / `bad_request` behind a refusal
/// about the wrong problem. Ordering is the whole content of this rule: the
/// guard is a rung, and it is not the FIRST rung.
///
/// Pinned by `a_target_that_does_not_resolve_is_not_this_rungs_to_answer` and by
/// the `selector_ambiguity` / `splice_e2e` suites end-to-end.
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

/// Is this edit a WELL-FORMED mutation of EXISTING content — the only thing this
/// rung governs? Everything else belongs to a rung that says something more
/// useful, so it is skipped rather than pre-empted.
fn is_guardable_mutation(doc: &model::Document, edit: &Edit) -> bool {
    // A ref the grammar cannot even translate (a block id outside the §2.4
    // charset) is the decode-adjacent rung's to refuse.
    let Ok(target) = crate::read::to_model_ref(&edit.target) else {
        return false;
    };

    // `put at:upsert` is the ONE create-or-replace shape and it has its own
    // domain rules — an `fm_key` target, a single-line value. A violation is
    // `bad_request` from the edit builder; an upsert of an ABSENT key is a birth,
    // guarded by absence. Neither is a fingerprint question.
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
            // Frontmatter is file-scoped, so the guard is the doc-root token
            // (P3) — a key-line rev would guard a grain the semantics do not
            // live at.
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
            // A birth: guarded by absence. Lowering has already turned it into
            // a parent-append, which is why the absence is checked HERE.
            PlanEdit::Create {
                parent_hpath,
                title,
                ..
            } => {
                let full = format!("{parent_hpath}/{title}");
                if section_exists(doc, parent_hpath, title) {
                    out.push(Demand {
                        subject: format!("section \"{full}\""),
                        unmet: Unmet::AlreadyBorn,
                    });
                }
            }
            // Content changes, all three of them. `append` is a content change
            // like any other (S3): it does not escape by not being replace-shaped.
            PlanEdit::Match { hpath, rev, .. }
            | PlanEdit::ReplaceSection { hpath, rev, .. }
            | PlanEdit::Append { hpath, rev, .. } => {
                if rev.as_deref().is_none_or(str::is_empty) {
                    out.push(Demand {
                        subject: format!("section \"{hpath}\""),
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

/// Does `parent_hpath/title` already address a section? The birth's absence
/// guard — an ambiguous resolve counts as EXISTING (something is there).
fn section_exists(doc: &model::Document, parent_hpath: &str, title: &str) -> bool {
    if parent_hpath.is_empty() {
        return false;
    }
    let segs: Vec<model::HpathSeg> = parent_hpath
        .split('/')
        .chain(std::iter::once(title))
        .map(|h| model::HpathSeg {
            h: h.to_owned(),
            n: None,
        })
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

/// Mint the refusal for the FIRST unmet demand. The batch is refused whole, so
/// one demand is the whole answer — and the message TEACHES: it names the
/// subject, the cause, the grain, the fact that nothing landed, and a runnable
/// command that mints the token the caller is missing. A caller that reads it
/// can proceed without a second guess and without a wasted read.
///
/// # This refusal is rung ZERO of the ladder, not a separate mechanism
/// The ruled `force` path is one continuous act — write → refuse (+ladder) →
/// rewrite — so the R1.2 mismatch-recovery ladder ATTACHES to this refusal
/// rather than living beside it. Nothing here forecloses that: the refusal is a
/// plain [`ErrorBody`] whose `expected`/`actual`/`message` slots are exactly
/// what a ladder rung enriches. A later unit adds rungs; it does not replace
/// this.
fn refusal(path: &Path, demand: &Demand) -> Box<ErrorBody> {
    let file = path.0.as_str();
    let subject = &demand.subject;
    match &demand.unmet {
        Unmet::NoGuard { grain, slot } => {
            let cause = match grain {
                Grain::Node => {
                    "a wire write that changes existing content carries its fingerprint at NODE \
                     grain, or an explicit `force`"
                }
                Grain::File => {
                    "frontmatter semantics are file-scoped, so a wire write carries the doc-root \
                     token at FILE grain, or an explicit `force`"
                }
            };
            // The two faces spell the same guard differently — the fix names the
            // slot the CALLER actually has, never the other face's.
            let fix = match slot {
                Slot::NativeNodeRev => format!(
                    "Fix: run `mrd read {file} --json` and send that node's `sec_rev` as \
                     `if_node_rev` on the edit — the read you already did carries it."
                ),
                Slot::PlanRowRev => format!(
                    "Fix: run `mrd read {file} --json` and send that section's `sec_rev` as `rev` \
                     on the plan edit."
                ),
                Slot::PlanFileRev => format!(
                    "Fix: run `mrd read {file} --json` and send its `file_rev` as `rev` on each \
                     `set_property`."
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

/// Render the bypassed planes as §11.1 verdicts — the P15 obligation that a
/// forced write NAMES what it wrote past. The rendered surface carries this; the
/// journal is dead by ruling and is not where a reader looks for it.
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
