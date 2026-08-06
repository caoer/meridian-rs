//! `@fp` strip on the RUN plane, at DOCUMENT grain (R32 (3)).
//! Strips free-text doors that must not enter the procedure hash / receipt
//! identity. Same discipline as other planes; this module is the run-plane
//! owner of that strip.

use std::ops::Range;

use model::{
    CandidateDocument, Document, EditKind, MerkleRoot, PutAt, SpliceRequest, Target, ValidatedBatch,
};

use crate::executor::ExecError;

/// The document this batch is about to commit — the sealed edits dry-applied to
/// the pre-image and reparsed. The SAME bytes `fs::apply_batch` writes, so every
/// judgment downstream (this strip, the lock artifact guard, the armed gate, the
/// post-apply revs) reads one candidate with one spelling.
pub(crate) fn candidate(page: &str, doc: &Document, sealed: &ValidatedBatch) -> CandidateDocument {
    model::candidate_of_batch(page, &doc.raw, sealed)
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

/// Which planned edit produced the sealed region — by the target span the model
/// itself resolved, never by text similarity. `validate_batch` refuses batches
/// with non-disjoint target spans, so a non-empty region has at most one
/// container; a contested region is `None` (refuse, never guess).
///
/// Boundary rule: sections are contiguous, so the EMPTY region of a
/// `put{at:"end"}` sits exactly on the byte where one section ends and its
/// sibling begins — both contain it, and the owner is the one that ENDS there.
/// Only `put{at:"end"}` produces an empty region (a `match` needle is non-empty
/// by validation), so this decides every case it applies to.
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
    after_doc: &mut CandidateDocument,
) -> Result<(), ExecError> {
    let mut per_edit: Vec<Vec<Range<usize>>> = vec![Vec::new(); batch.edits.len()];
    let mut introduced = 0usize;
    for origin in classify(doc, after_doc.document(), sealed, before_facts, page)? {
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
            // `md.set_field` composes the frontmatter line, so payload offsets
            // are not the sealed line's — and frontmatter is no claim-link
            // position. A token attributed here means the grammar moved; refuse.
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
    *after_doc = candidate(page, doc, sealed);

    // THE ASSERTION (R32 (1)'s wording): this write INTRODUCES no token. Live on
    // the real apply, not test-only — a door that reaches these bytes without
    // passing the strip refuses here instead of landing silently.
    if classify(doc, after_doc.document(), sealed, before_facts, page)?
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

    /// The composed branch, driven directly: the run plane's own two verbs
    /// cannot reach it (`md.set_field` writes frontmatter, `md.append_section`
    /// always lands content on its own line, and a wikilink never spans a line
    /// break). A backstop for a future insertion shape, proved with the `match`
    /// edit the run plane does not plan.
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
        let after = candidate("page.md", &doc, &sealed);
        assert_eq!(
            syntax::fp_removals(after.raw()).len(),
            1,
            "the candidate DOES carry a claim token: {}",
            after.raw()
        );
        let before_facts: Vec<Target> = vec![model::resolve(&doc, &plan_ref()).unwrap()];
        let refused = classify(&doc, after.document(), &sealed, &before_facts, "page.md");
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
        let after = candidate("page.md", &doc, &sealed);
        let before_facts: Vec<Target> = vec![model::resolve(&doc, &sub).unwrap()];
        let origins = classify(&doc, after.document(), &sealed, &before_facts, "page.md")
            .expect("an unrelated edit on a damaged page classifies");
        assert!(
            matches!(origins.as_slice(), [FpOrigin::Retained]),
            "the token is retained, so the strip leaves it exactly as found"
        );
    }
}
