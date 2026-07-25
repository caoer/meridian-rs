//! The `@fp` strip on the RUN plane, at DOCUMENT grain (advisor R32 (3)).
//!
//! `@green.b3af12cd` after a block ref is a render-face decoration the engine
//! mints on READ (S10) — never storable content. The wire choke-point strips it
//! from every put; the run plane lands bytes through `fs::apply_batch`, bypassing
//! that choke-point entirely, so an `md.append_section` body carrying a token
//! used to land it on disk. **A guarded invariant with an unguarded door is not
//! an invariant** (R32): without this, "no write introduces an fp token in a
//! claim-link position" is false on the run surface.
//!
//! # The grain is the CANDIDATE, never the payload
//! Tokens are identified in the document this batch is about to commit, through
//! the ONE dialect parse ([`syntax::fp_removals`]), and removed from the payload
//! that carries them. A payload-field walk is the defect class fix2a deleted: it
//! cannot see a token the batch composes out of bytes it does not supply, and it
//! strips what the document law calls a code sample (R22).
//!
//! # Three origins, three outcomes
//! - **Introduced** — the token sits inside bytes THIS batch supplies: removed
//!   from that payload, the batch re-validated, the candidate rebuilt, and the
//!   result asserted to introduce none.
//! - **Retained** — the token was already on disk at that offset: left exactly as
//!   found (R32 (1)). Removing bytes this write does not own would move the
//!   fingerprint of a node it never addressed, reddening unrelated pins.
//! - **Composed** — a claim the batch creates out of bytes it does not supply
//!   (an edit that un-masks retained text). Nothing to strip and nobody to
//!   attribute it to: REFUSE, loud, having applied nothing.

use std::ops::Range;

use model::{Document, EditKind, MerkleRoot, PutAt, SpliceRequest, Target, ValidatedBatch};

use crate::executor::ExecError;

/// The document this batch is about to commit — the sealed edits dry-applied to
/// the pre-image and reparsed. The SAME bytes `fs::apply_batch` writes, so every
/// judgment downstream (this strip, the lock artifact guard, the armed gate, the
/// post-apply revs) reads one candidate with one spelling.
pub(crate) fn candidate(doc: &Document, sealed: &ValidatedBatch) -> Document {
    let mut raw = doc.raw.clone();
    for edit in sealed.edits.iter().rev() {
        raw.replace_range(edit.span.clone(), &edit.text);
    }
    let nodes = syntax::parse(&raw);
    model::build(raw, nodes)
}

/// One `@fp` token run in the candidate, classified by WHO put it there.
#[derive(Debug)]
enum FpOrigin {
    /// Bytes THIS batch supplies: request edit `edit`, at `local` inside its
    /// payload. Removable — this is the strip.
    Introduced { edit: usize, local: Range<usize> },
    /// A token already on disk, retained verbatim by this batch. NOT this
    /// write's to remove.
    Retained,
}

/// Classify every `@fp` token run in `after` — the ONE identification, shared by
/// the strip and by the assertion that follows it.
fn classify(
    doc: &Document,
    after: &Document,
    sealed: &ValidatedBatch,
    before_facts: &[Target],
    page: &str,
) -> Result<Vec<FpOrigin>, ExecError> {
    let removals = syntax::fp_removals(&after.raw);
    if removals.is_empty() {
        return Ok(Vec::new());
    }
    // The after image, walked ONCE. The sealed spans index the pre-image and are
    // sorted and disjoint, so a single forward scan places every inserted text
    // AND every surviving run in after coordinates — no shift arithmetic.
    let mut inserted: Vec<(Range<usize>, Range<usize>)> = Vec::with_capacity(sealed.edits.len());
    let mut retained: Vec<(Range<usize>, usize)> = Vec::with_capacity(sealed.edits.len() + 1);
    let mut after_pos = 0usize;
    let mut pre_pos = 0usize;
    for e in &sealed.edits {
        let gap = e.span.start.saturating_sub(pre_pos);
        if gap > 0 {
            retained.push((after_pos..after_pos + gap, pre_pos));
            after_pos += gap;
        }
        inserted.push((after_pos..after_pos + e.text.len(), e.span.clone()));
        after_pos += e.text.len();
        pre_pos = e.span.end;
    }
    let tail = doc.raw.len().saturating_sub(pre_pos);
    if tail > 0 {
        retained.push((after_pos..after_pos + tail, pre_pos));
    }
    let pre_existing = syntax::fp_removals(&doc.raw);

    let mut out = Vec::with_capacity(removals.len());
    for r in removals {
        if let Some((after_range, region)) = inserted
            .iter()
            .find(|(a, _)| a.start <= r.start && r.end <= a.end)
        {
            let edit =
                attribute_region(region, before_facts).ok_or_else(|| ExecError::FpClaim {
                    page: page.to_owned(),
                    cause: "an @fp decoration token cannot be attributed to any effect in this \
                        batch — the engine will not remove a claim token it cannot place"
                        .to_owned(),
                })?;
            out.push(FpOrigin::Introduced {
                edit,
                local: r.start - after_range.start..r.end - after_range.start,
            });
            continue;
        }
        let was_already_there = retained
            .iter()
            .find(|(a, _)| a.start <= r.start && r.end <= a.end)
            .is_some_and(|(after_range, pre_start)| {
                let start = pre_start + (r.start - after_range.start);
                pre_existing.contains(&(start..start + (r.end - r.start)))
            });
        if was_already_there {
            out.push(FpOrigin::Retained);
        } else {
            return Err(ExecError::FpClaim {
                page: page.to_owned(),
                cause: "this apply would COMPOSE an @fp claim token out of bytes it does not \
                        supply — no payload carries it, so there is nothing to strip"
                    .to_owned(),
            });
        }
    }
    Ok(out)
}

/// Which planned edit produced the sealed region — by the TARGET SPAN the model
/// itself resolved, never by text similarity. `validate_batch` refuses a batch
/// whose target spans are not pairwise disjoint (containment counts), so a
/// non-empty region has at most one container; a region contested past the
/// boundary rule below is `None` (refuse, never guess).
///
/// # The boundary rule, which containment alone cannot decide
/// Sections are contiguous: a section's span ENDS on the byte where its next
/// sibling's span BEGINS. An `md.append_section` plans `put{at:"end"}`, whose
/// replaced region is EMPTY and sits exactly on that shared byte (§4.4), so both
/// siblings contain it. The one the model planned it from is the one that ENDS
/// there — the other merely begins there. Empty regions are the only ones that
/// can land on a shared byte, and `put{at:"end"}` is the only shape that produces
/// one (a `match` needle is non-empty by validation), so this decides every case
/// it applies to and touches no other.
fn attribute_region(region: &Range<usize>, before_facts: &[Target]) -> Option<usize> {
    let containers: Vec<usize> = before_facts
        .iter()
        .enumerate()
        .filter(|(_, t)| t.span.start <= region.start && region.end <= t.span.end)
        .map(|(i, _)| i)
        .collect();
    if let [only] = containers.as_slice() {
        return Some(*only);
    }
    if !region.is_empty() {
        return None;
    }
    let mut hit = None;
    for &i in &containers {
        if before_facts[i].span.end == region.end {
            if hit.is_some() {
                return None;
            }
            hit = Some(i);
        }
    }
    hit
}

/// `text` with `ranges` (payload-local, non-overlapping, ascending) removed.
fn remove_ranges(text: &str, ranges: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for r in ranges {
        if r.start < cursor || r.end > text.len() {
            continue;
        }
        out.push_str(&text[cursor..r.start]);
        cursor = r.end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// **The `@fp` strip, at document grain.** Remove every token this batch
/// INTRODUCES from the payload that carries it, re-seal so the commit lands
/// exactly the judged bytes, and assert the candidate introduces none — the loud
/// refusal that catches the next missed door.
///
/// The REQUEST batch is rewritten (not just the sealed copy) because the
/// executor re-validates the request once more to fold in the receipt: a strip
/// applied only to the sealed batch would judge bytes the commit does not write.
///
/// # Errors
/// [`ExecError::FpClaim`] — a composed or unattributable token, a token
/// attributed to a shape the strip cannot place, a re-validation refusal, or a
/// token still standing after the strip. Nothing was applied in any case.
pub(crate) fn strip_candidate(
    doc: &Document,
    before_facts: &[Target],
    page: &str,
    live_root: &MerkleRoot,
    batch: &mut SpliceRequest,
    sealed: &mut ValidatedBatch,
    after_doc: &mut Document,
) -> Result<(), ExecError> {
    let mut per_edit: Vec<Vec<Range<usize>>> = vec![Vec::new(); batch.edits.len()];
    let mut introduced = 0usize;
    for origin in classify(doc, after_doc, sealed, before_facts, page)? {
        if let FpOrigin::Introduced { edit, local } = origin {
            introduced += 1;
            per_edit
                .get_mut(edit)
                .map(|v| v.push(local))
                .ok_or_else(|| ExecError::FpClaim {
                    page: page.to_owned(),
                    cause: "an @fp token attributes to a span no request effect owns".to_owned(),
                })?;
        }
    }
    if introduced == 0 {
        return Ok(());
    }

    for (i, ranges) in per_edit.iter().enumerate() {
        if ranges.is_empty() {
            continue;
        }
        let payload = match &mut batch.edits[i].edit {
            // `md.set_field`'s composed `{key}: {value}` frontmatter line: its
            // payload offsets are not the sealed line's, and frontmatter carries
            // no claim-link position in the one grammar — so a token attributed
            // here means the grammar moved under this code. Refuse rather than
            // splice blind.
            EditKind::Put {
                at: PutAt::Upsert, ..
            } => {
                return Err(ExecError::FpClaim {
                    page: page.to_owned(),
                    cause: "an @fp token attributes to a frontmatter property line — frontmatter \
                            is not a claim-link position (S10/R22); the strip cannot place it"
                        .to_owned(),
                });
            }
            EditKind::Put { text, .. } => text,
            // The run plane plans `put` only (`executor::plan_edit`), so a token
            // attributed to a `match` means the planner grew a shape this strip
            // has never seen. Refuse rather than edit a needle.
            EditKind::Match { .. } => {
                return Err(ExecError::FpClaim {
                    page: page.to_owned(),
                    cause: "an @fp token attributes to a match edit, which the run plane does \
                            not plan"
                        .to_owned(),
                });
            }
        };
        *payload = remove_ranges(payload, ranges);
    }

    *sealed = match model::validate_batch(doc, Some(live_root), batch, None) {
        model::SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(ExecError::FpClaim {
                page: page.to_owned(),
                cause: format!(
                    "the batch no longer validates after its @fp decoration tokens were \
                     stripped ({refused:?})"
                ),
            });
        }
    };
    *after_doc = candidate(doc, sealed);

    // THE ASSERTION (R32 (1)'s wording): this write INTRODUCES no token. Live on
    // the real apply, not test-only — a door that reaches these bytes without
    // passing the strip refuses here instead of landing silently.
    if classify(doc, after_doc, sealed, before_facts, page)?
        .iter()
        .any(|o| matches!(o, FpOrigin::Introduced { .. }))
    {
        return Err(ExecError::FpClaim {
            page: page.to_owned(),
            cause: "an @fp claim token survived the document-grain strip".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FpOrigin, candidate, classify};
    use model::{Edit, EditKind, HpathSeg, Ref, SpliceRequest, SpliceVerdict, Target};

    fn doc_of(raw: &str) -> model::Document {
        let raw = raw.to_owned();
        let nodes = syntax::parse(&raw);
        model::build(raw, nodes)
    }

    fn plan_ref() -> Ref {
        Ref::Hpath(vec![HpathSeg {
            h: "Plan".to_owned(),
            n: None,
        }])
    }

    /// **The composed branch, driven directly — and why it is driven directly.**
    /// The classifier refuses a claim the batch creates out of bytes it does not
    /// supply. The run plane's own two verbs cannot reach it: `md.set_field`
    /// writes frontmatter (no claim-link position) and `md.append_section` plans
    /// `put{at:"end"}`, which always lands its content on its own line — the
    /// planner pushes a leading newline when the target does not end on one, and
    /// always a trailing one — and a wikilink never spans a line break. So the
    /// branch is a BACKSTOP for the day a third insertion shape appears, and it
    /// is proved here at the law's own grain with the `match` edit the run plane
    /// does not plan, rather than through a fixture that pretends to be a run.
    #[test]
    fn a_token_composed_out_of_retained_bytes_refuses() {
        let doc = doc_of("# Plan\n\nsee [[guide#^goal@green.b3af12cd\n");
        assert!(
            syntax::fp_removals(&doc.raw).is_empty(),
            "the pre-image carries no token: the link never closes"
        );
        let batch = SpliceRequest {
            if_root: None,
            edits: vec![Edit {
                target: plan_ref(),
                edit: EditKind::Match {
                    old: "b3af12cd".to_owned(),
                    new: "b3af12cd|G]]".to_owned(),
                },
                if_node_rev: None,
            }],
            engine: None,
        };
        let SpliceVerdict::Validated(sealed) = model::validate_batch(&doc, None, &batch, None)
        else {
            panic!("the batch validates — only the claim it composes is the question");
        };
        let after = candidate(&doc, &sealed);
        assert_eq!(
            syntax::fp_removals(&after.raw).len(),
            1,
            "the candidate DOES carry a claim token: {}",
            after.raw
        );
        let before_facts: Vec<Target> = vec![model::resolve(&doc, &plan_ref()).unwrap()];
        let refused = classify(&doc, &after, &sealed, &before_facts, "page.md");
        let cause = refused.expect_err("a composed claim refuses").to_string();
        assert!(
            cause.contains("COMPOSE"),
            "the refusal names what it refused: {cause}"
        );
    }

    /// The retained half of the same law, at the same grain: a token already on
    /// disk in bytes this batch does not touch is classified `Retained`, not
    /// removed and not refused (R32 (1)).
    #[test]
    fn a_pre_existing_token_classifies_as_retained() {
        let doc = doc_of("# Plan\n\nsee [[guide#^goal@green.b3af12cd|G]]\n\n## Sub\n\nsub body\n");
        let sub = Ref::Hpath(vec![
            HpathSeg {
                h: "Plan".to_owned(),
                n: None,
            },
            HpathSeg {
                h: "Sub".to_owned(),
                n: None,
            },
        ]);
        let batch = SpliceRequest {
            if_root: None,
            edits: vec![Edit {
                target: sub.clone(),
                edit: EditKind::Put {
                    at: model::PutAt::End,
                    text: "one more line.\n".to_owned(),
                },
                if_node_rev: None,
            }],
            engine: None,
        };
        let SpliceVerdict::Validated(sealed) = model::validate_batch(&doc, None, &batch, None)
        else {
            panic!("the batch validates");
        };
        let after = candidate(&doc, &sealed);
        let before_facts: Vec<Target> = vec![model::resolve(&doc, &sub).unwrap()];
        let origins = classify(&doc, &after, &sealed, &before_facts, "page.md")
            .expect("an unrelated edit on a damaged page classifies");
        assert!(
            matches!(origins.as_slice(), [FpOrigin::Retained]),
            "the token is retained, so the strip leaves it exactly as found"
        );
    }
}
