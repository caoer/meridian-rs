//! M1 U8b: lower `splice.plan_edits` (the put-plan vocabulary) to NATIVE wire
//! edits at the splice intake — the Go daemon's `buildSpliceEdit` +
//! `buildPropertyEdits` emulation (`putsidecar.go` / `puttoc.go`), moved behind
//! the wire and deleted host-side.
//!
//! # The one law: byte-faithful to the deleted Go arms
//! Every lowered native edit is byte-identical to the edit the host used to
//! build from its `toc`+`cat` view, so everything DOWNSTREAM (model validate,
//! CAS, armed facts, reparse gate, `would_corrupt`) behaves identically by
//! construction. Fidelity points, each deliberate:
//!
//! - **Address resolution = the host `tocIndex` MAP**, not the read face and
//!   not the `check_write` rebuild: exact sanitized-hpath-chain lookup, LAST
//!   occurrence wins on duplicates (Go map overwrite), NO raw-title fallback.
//!   (Duplicate/ambiguous targets never reach here through the put face —
//!   `check_write`'s rebuild refuses `E_AMBIGUOUS` first, host-sequenced.)
//! - **Append discipline** reads the PRE-batch bytes only (`fileBytes[at-1]`),
//!   with NO in-batch pending-newline tracking — the host emulation had none
//!   (`policy::defs::rebuild`'s `pending_nl` is the `check_write` candidate's
//!   lib-faithful twin; two appends to one section refuse as overlap anyway).
//! - **`create` = PARENT-APPEND** (`"\n" + "#"*level + " " + title + "\n\n" +
//!   body + "\n"`, `Put{end}` on the parent) — emulation-faithful. The
//!   `check_write` candidate places creates at EOF (`rebuild::plan_create_section`,
//!   lib-faithful): the NAMED U8b residual (G-class, C rider 3) — invisible
//!   while no def governs created sections; stage-2 unifies if one ever does.
//! - **`set_property` = the property-group dance**, NOT native `at:upsert`:
//!   model's upsert inserts an absent key at FIRST-key position
//!   (`plan_fm_upsert`), the Go dance inserts after the LAST key — divergent
//!   bytes. BOTH halves of the composed line pass the ONE shared owner —
//!   keys through `policy::defs::yaml_safe_key`, values through
//!   `policy::defs::yaml_safe_value` — so the `check_write` candidate and the
//!   written bytes cannot drift, and neither door can forge frontmatter.
//! - **Refusals**: `bad_request` + `message` = the exact Go-face teaching
//!   MINUS the `put: ` verb prefix (the host renders the prefix; op-owner
//!   ruling 2026-07-24 — engine owns the sentence, host owns the verb).
//!   `%q` spellings mint through `policy::defs::go_quote`.

use wire::{Edit, EditShape, ErrorBody, HpathSeg, NodeRev, PutAt, SecRef};

use crate::bad_request;

/// One resolved heading in the host-face index (the `tocIndex` `tocNode` lift).
struct HeadingFacts {
    /// RAW hpath segments — the native edit target spelling the host used
    /// (`Target: {"hpath": node.rawHpath}`).
    raw_hpath: Vec<HpathSeg>,
    /// Heading level (for `create`'s child-heading depth).
    level: u32,
    /// Full node span (subtree-inclusive) — `node.span`.
    span: (usize, usize),
    /// Heading-excluded content span, when the section has content.
    content_span: Option<(usize, usize)>,
}

/// The host-face put index (`newTocIndex` over the SAME projection the wire
/// `toc` serves): sanitized-chain map with Go map-overwrite semantics, plus
/// the frontmatter key list in document order.
struct PlanIndex {
    headings: std::collections::HashMap<String, HeadingFacts>,
    fm_keys: Vec<String>,
}

impl PlanIndex {
    fn new(doc: &model::Document) -> Self {
        let mut headings = std::collections::HashMap::new();
        let mut fm_keys = Vec::new();
        for row in wire_map::project_toc(doc) {
            match row.kind.as_str() {
                "heading" => {
                    let segs = row.hpath.unwrap_or_default();
                    let key = segs
                        .iter()
                        .map(|s| wire_map::gotext::sanitize_heading(&s.h))
                        .collect::<Vec<_>>()
                        .join("/");
                    // Go map overwrite: the LAST duplicate wins.
                    headings.insert(
                        key,
                        HeadingFacts {
                            raw_hpath: segs,
                            level: row.level.unwrap_or(0),
                            span: (span_usize(row.span.0), span_usize(row.span.1)),
                            content_span: row
                                .content_span
                                .map(|cs| (span_usize(cs.0), span_usize(cs.1))),
                        },
                    );
                }
                "frontmatter" => {
                    fm_keys.extend(row.keys.unwrap_or_default());
                }
                _ => {}
            }
        }
        PlanIndex { headings, fm_keys }
    }
}

/// Wire spans are `u64` byte offsets minted from THIS document's own indices —
/// checked, never a lossy `as` (a saturated miss falls into the same defensive
/// bounds guards the Go host carried).
fn span_usize(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Lower one plan-level batch to native edits — properties FIRST as one group,
/// then body ops in request order (the host's edit order, verbatim). The
/// returned batch feeds the unchanged native splice path; armed facts align
/// 1:1 with the LOWERED edits.
///
/// # Errors
/// A target-class `bad_request` teaching (the deleted host arms' refusals,
/// minus the `put: ` prefix), at the FIRST failing plan edit.
pub fn lower(
    doc: &model::Document,
    plan_edits: &[wire::PlanEdit],
) -> Result<Vec<Edit>, Box<ErrorBody>> {
    let idx = PlanIndex::new(doc);
    let raw = doc.raw.as_bytes();

    // Properties as ONE group (host order: the props group precedes every
    // body edit), Go map semantics: last value per key wins, keys sorted.
    let mut props: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for e in plan_edits {
        if let wire::PlanEdit::SetProperty { key, value } = e {
            props.insert(key, value);
        }
    }
    let mut edits = if props.is_empty() {
        Vec::new()
    } else {
        lower_property_group(&idx, &props)?
    };

    for e in plan_edits {
        match e {
            wire::PlanEdit::SetProperty { .. } => {} // grouped above
            wire::PlanEdit::Append { hpath, body } => {
                edits.push(lower_append(&idx, raw, hpath, body)?);
            }
            wire::PlanEdit::Match {
                hpath,
                old,
                new,
                all,
                rev,
            } => {
                edits.push(lower_match(
                    &idx,
                    raw,
                    hpath,
                    old,
                    new,
                    *all,
                    rev.as_deref(),
                )?);
            }
            wire::PlanEdit::ReplaceSection { hpath, body, rev } => {
                edits.push(lower_replace_section(&idx, hpath, body, rev.as_deref())?);
            }
            wire::PlanEdit::Create {
                parent_hpath,
                title,
                body,
            } => {
                edits.push(lower_create(&idx, parent_hpath, title, body)?);
            }
        }
    }
    Ok(edits)
}

/// `buildSpliceEdit` append arm (`putsidecar.go:202-226`): block targets
/// refuse; the payload gets `ensureTrailingNL` + a leading `\n` when the
/// pre-batch byte before the insertion point is not one. Lowered to
/// `Put{end}` on the RAW hpath. (Dedupe is the HOST's — a deduped plan is
/// never sent.)
fn lower_append(
    idx: &PlanIndex,
    raw: &[u8],
    hpath: &str,
    body: &str,
) -> Result<Edit, Box<ErrorBody>> {
    if hpath.starts_with('^') {
        return Err(bad_request(format!(
            "append to a block anchor {} is not supported — append targets a section (the containing heading path)",
            policy::defs::go_quote(hpath)
        )));
    }
    let Some(node) = idx.headings.get(hpath) else {
        return Err(bad_request(format!(
            "no section addressed by {}. {} {}",
            policy::defs::go_quote(hpath),
            crate::NO_PARTIAL_WRITE_CLAUSE,
            crate::section_recovery(hpath, None)
        )));
    };
    let at = node.span.1;
    let mut payload = policy::defs::ensure_trailing_nl(body);
    if at > 0 && at <= raw.len() && raw[at - 1] != b'\n' {
        payload.insert(0, b'\n');
    }
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: node.raw_hpath.clone(),
        },
        edit: EditShape::Put {
            at: PutAt::End,
            text: String::from_utf8_lossy(&payload).into_owned(),
        },
        if_node_rev: None,
    })
}

/// `buildSpliceEdit` replace arm (`putsidecar.go:244-275`): unique-anchor
/// `Match{old,new}`, or the `all:true` read-modify-write — every occurrence
/// replaced over the heading-EXCLUDED content, written back `Put{content}`.
/// (`foreign_changes` is journal-derived and stays with the HOST.)
fn lower_match(
    idx: &PlanIndex,
    raw: &[u8],
    hpath: &str,
    old: &str,
    new: &str,
    all: bool,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    let Some(node) = idx.headings.get(hpath) else {
        return Err(bad_request(format!(
            "no section addressed by {}. {} {}",
            policy::defs::go_quote(hpath),
            crate::NO_PARTIAL_WRITE_CLAUSE,
            crate::section_recovery(hpath, None)
        )));
    };
    // S10's ADDRESS half (advisor R25): `old` is a NEEDLE searched in STORED
    // bytes, which never carry an `@fp` token, so an agent's needle copied from
    // the decorated render face must be peeled before the search — the same rule
    // `read::to_model_ref` applies to a `SecRef::Anchor`. It is peeled HERE
    // because the search happens here; `new` is a PAYLOAD and rides verbatim into
    // the candidate, where the document-grain strip owns it.
    let old = &*syntax::strip_fp(old);
    let if_node_rev = rev
        .filter(|r| !r.is_empty())
        .map(|r| NodeRev(r.to_string()));
    if all {
        // Go stripHeading: content = span bytes from the content-span offset
        // (defensive fallbacks verbatim: no content span, or a malformed
        // offset, serve the FULL span bytes).
        let (s, e) = node.span;
        let full = String::from_utf8_lossy(&raw[s..e]);
        let content = match node.content_span {
            Some((cs, _)) if cs >= s && cs <= e => String::from_utf8_lossy(&raw[cs..e]),
            _ => full,
        };
        if !content.contains(old) {
            return Err(bad_request(format!(
                "replace anchor {} not found in {}",
                policy::defs::go_quote(old),
                policy::defs::go_quote(hpath)
            )));
        }
        let new_content = content.replace(old, new);
        return Ok(Edit {
            target: SecRef::Hpath {
                hpath: node.raw_hpath.clone(),
            },
            edit: EditShape::Put {
                at: PutAt::Content,
                text: new_content,
            },
            if_node_rev,
        });
    }
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: node.raw_hpath.clone(),
        },
        edit: EditShape::Match {
            old: old.to_string(),
            new: new.to_string(),
        },
        if_node_rev,
    })
}

/// `buildSpliceEdit` `replace_section` arm (`putsidecar.go:228-242`): requires a
/// rev (destructive), payload `ensureTrailingNL` (empty stays empty), lowered
/// to `Put{content}` + `if_node_rev`. The empty-rev refusal is DEAD through
/// the put face (`check_write`'s ECAS fires first, host-sequenced) — minted
/// arm-faithful for direct wire callers.
fn lower_replace_section(
    idx: &PlanIndex,
    hpath: &str,
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    let Some(node) = idx.headings.get(hpath) else {
        return Err(bad_request(format!(
            "no section addressed by {}. {} {}",
            policy::defs::go_quote(hpath),
            crate::NO_PARTIAL_WRITE_CLAUSE,
            crate::section_recovery(hpath, None)
        )));
    };
    let rev = rev.unwrap_or("");
    if rev.is_empty() {
        return Err(bad_request(format!(
            "replace_section on {} requires a fresh rev (a whole-section rewrite is destructive) — read the section and pass its rev",
            policy::defs::go_quote(hpath)
        )));
    }
    let text = if body.is_empty() {
        String::new()
    } else {
        String::from_utf8_lossy(&policy::defs::ensure_trailing_nl(body)).into_owned()
    };
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: node.raw_hpath.clone(),
        },
        edit: EditShape::Put {
            at: PutAt::Content,
            text,
        },
        if_node_rev: Some(NodeRev(rev.to_string())),
    })
}

/// `createTarget` + the create arm (`puttoc.go:125-139`, `putsidecar.go:277-286`):
/// PARENT-APPEND emulation — `"\n" + "#"*(parent.level+1) + " " + title +
/// "\n\n" + body + "\n"` as `Put{end}` on the parent. Top-level (`parent_hpath`
/// empty) and parent-miss refuse with the ONE host teaching, naming the full
/// target as the caller spelled it.
fn lower_create(
    idx: &PlanIndex,
    parent_hpath: &str,
    title: &str,
    body: &str,
) -> Result<Edit, Box<ErrorBody>> {
    let full = if parent_hpath.is_empty() {
        title.to_string()
    } else {
        format!("{parent_hpath}/{title}")
    };
    let cannot_place = || {
        bad_request(format!(
            "cannot place new section {} — its parent is not in the document",
            policy::defs::go_quote(&full)
        ))
    };
    if parent_hpath.is_empty() {
        return Err(cannot_place());
    }
    let Some(parent) = idx.headings.get(parent_hpath) else {
        return Err(cannot_place());
    };
    let level = (parent.level + 1) as usize;
    let heading = format!("\n{} {title}\n\n{body}\n", "#".repeat(level));
    Ok(Edit {
        target: SecRef::Hpath {
            hpath: parent.raw_hpath.clone(),
        },
        edit: EditShape::Put {
            at: PutAt::End,
            text: heading,
        },
        if_node_rev: None,
    })
}

/// `buildPropertyEdits` (`puttoc.go:147-209`), byte-faithful: each existing
/// key being set is a whole-line `Put{all}` on its `fm_key`; absent keys land
/// as `"\n{k}: {v}"` lines AFTER the last existing key — folded into that
/// key's `Put{all}` when it is itself being set (the carrier), else a
/// `Put{end}` after it. No frontmatter to anchor on refuses the host teaching.
/// BOTH halves of the composed `{key}: {value}` line pass their ONE shared
/// fallible owner before any byte is built: the key through `yaml_safe_key`
/// (charset — the pre-flight's own refusal, `rebuild::plan_set_property`), the
/// value through the conditional-quote predicate, which REFUSES a multi-line
/// value (D11).
fn lower_property_group(
    idx: &PlanIndex,
    props: &std::collections::BTreeMap<&str, &str>,
) -> Result<Vec<Edit>, Box<ErrorBody>> {
    // The KEY owner first, in the pre-flight's ordering: an unvalidated key is
    // composed into the frontmatter line exactly as an unvalidated value is,
    // so this door must refuse what `rebuild::plan_set_property` refuses.
    let mut keyed: std::collections::BTreeMap<policy::defs::SafeKey<'_>, &str> =
        std::collections::BTreeMap::new();
    for (k, v) in props {
        let key = policy::defs::yaml_safe_key(k).map_err(|_| {
            bad_request(format!(
                "invalid frontmatter key {} — a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')",
                policy::defs::go_quote(k)
            ))
        })?;
        keyed.insert(key, v);
    }

    let fm_key_set: std::collections::HashSet<&str> =
        idx.fm_keys.iter().map(String::as_str).collect();
    if idx.fm_keys.is_empty() {
        for k in keyed.keys() {
            if !fm_key_set.contains(k.as_str()) {
                return Err(bad_request(
                    "cannot set a new property — the file has no frontmatter to anchor it (add a '---' block first)",
                ));
            }
        }
    }
    let mut quoted: std::collections::BTreeMap<policy::defs::SafeKey<'_>, String> =
        std::collections::BTreeMap::new();
    for (k, v) in &keyed {
        // D11: the composed line is `{key}: {value}`, so a newline in the value
        // forges frontmatter keys — and a single-quoted YAML scalar cannot
        // escape one. Refuse, never sanitize; the shared predicate owns the law.
        let safe = policy::defs::yaml_safe_value(v).map_err(|_| {
            bad_request(format!(
                "property value for {} contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section",
                policy::defs::go_quote(k.as_str())
            ))
        })?;
        quoted.insert(*k, safe);
    }
    let line = |k: policy::defs::SafeKey<'_>| format!("{k}: {}", quoted[&k]);

    let mut existing = Vec::new();
    let mut absent = Vec::new();
    for k in quoted.keys() {
        if fm_key_set.contains(k.as_str()) {
            existing.push(*k);
        } else {
            absent.push(*k);
        }
    }

    let fm_put = |key: &str, at: PutAt, text: String| Edit {
        target: SecRef::FmKey {
            fm_key: key.to_string(),
        },
        edit: EditShape::Put { at, text },
        if_node_rev: None,
    };

    let mut edits = Vec::new();
    let mut carrier: Option<&str> = None;
    if !absent.is_empty() {
        let mut absent_lines = String::new();
        for k in &absent {
            absent_lines.push('\n');
            absent_lines.push_str(&line(*k));
        }
        // pkg/body inserts a new key immediately before the closing '---' —
        // the END of frontmatter, after the LAST key. If the last key is
        // itself being set, FOLD the inserts into its Put{all} (one disjoint
        // edit); otherwise Put{end} after it.
        let last = idx
            .fm_keys
            .last()
            .expect("fm_keys non-empty when keys are absent")
            .as_str();
        if let Some(carrier_key) = quoted.keys().copied().find(|k| k.as_str() == last) {
            edits.push(fm_put(
                last,
                PutAt::All,
                format!("{}{absent_lines}", line(carrier_key)),
            ));
            carrier = Some(last);
        } else {
            edits.push(fm_put(last, PutAt::End, absent_lines));
        }
    }
    for k in existing {
        if carrier == Some(k.as_str()) {
            continue;
        }
        edits.push(fm_put(k.as_str(), PutAt::All, line(k)));
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use wire::{EditShape, PlanEdit, PutAt, SecRef};

    fn doc(raw: &str) -> model::Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    fn lower1(raw: &str, e: PlanEdit) -> Result<wire::Edit, Box<wire::ErrorBody>> {
        super::lower(&doc(raw), &[e]).map(|mut v| v.remove(0))
    }

    fn put_text(e: &wire::Edit) -> (&PutAt, &str) {
        match &e.edit {
            EditShape::Put { at, text } => (at, text),
            EditShape::Match { .. } => panic!("expected put"),
        }
    }

    /// Append discipline (`putsidecar.go:219-223`): trailing NL ensured; a
    /// leading NL exactly when the pre-batch byte before the insertion point
    /// is not one.
    #[test]
    fn append_newline_discipline() {
        // File ends WITH a newline → no leading \n synthesized.
        let e = lower1(
            "# Memo\n\nline\n",
            PlanEdit::Append {
                hpath: "Memo".into(),
                body: "added".into(),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::End);
        assert_eq!(text, "added\n", "trailing NL ensured, no leading NL");

        // Terminator-less final line → leading \n synthesized.
        let e = lower1(
            "# Memo\n\nline",
            PlanEdit::Append {
                hpath: "Memo".into(),
                body: "added\n".into(),
            },
        )
        .expect("lowers");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\nadded\n", "leading NL against a bare final line");
    }

    /// The U8b MUST-CARRY pin (goldens/rebuild.json p-replace-on-block): a
    /// `^block` is NOT a replace section target — the exact host bytes, minus
    /// the host's `put: ` verb prefix.
    #[test]
    fn replace_on_block_refuses_no_section_addressed() {
        let err = lower1(
            "# Tasks\n\n- [ ] one ^task1\n",
            PlanEdit::Match {
                hpath: "^task1".into(),
                old: "one".into(),
                new: "two".into(),
                all: false,
                rev: None,
            },
        )
        .expect_err("block replace target refuses");
        // The anchor arm of the recovery: `toc` does not carry `^` anchors, so
        // this miss must NOT send the reader to the section listing (issue-05 /
        // gaps § 3.1).
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"^task1\". No edit was applied; the batch is \
                 refused whole. Fix: read the page with --json and use its `anchors[]` — \
                 the section map does not list `^` anchors."
            )
        );
    }

    /// The append-to-block arm (dead through the put face — `check_write`'s
    /// `E_FAIL_LOUD` fires first; arm-faithful for direct callers).
    #[test]
    fn append_to_block_refuses_arm_string() {
        let err = lower1(
            "# Tasks\n\n- item ^t1\n",
            PlanEdit::Append {
                hpath: "^t1".into(),
                body: "x".into(),
            },
        )
        .expect_err("block append refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                r#"append to a block anchor "^t1" is not supported — append targets a section (the containing heading path)"#
            )
        );
    }

    /// The append section-miss arm string (its own remedy tail, distinct from
    /// the replace arms').
    #[test]
    fn append_miss_names_a_runnable_recovery() {
        let err = lower1(
            "# A\n\nx\n",
            PlanEdit::Append {
                hpath: "Ghost".into(),
                body: "x".into(),
            },
        )
        .expect_err("miss refuses");
        // Names an ACT the caller can perform, never the internal mode name they
        // never selected (issue-05), and discloses that nothing landed.
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"Ghost\". No edit was applied; the batch is \
                 refused whole. Fix: read the page with no selector to list its section \
                 paths."
            )
        );
    }

    /// The U8b MUST-CARRY pin (goldens/basic.json p-create-top): top-level
    /// create refuses with the one host teaching.
    #[test]
    fn create_top_level_refuses() {
        let err = lower1(
            "# A\n\nx\n",
            PlanEdit::Create {
                parent_hpath: String::new(),
                title: "Brand".into(),
                body: "b".into(),
            },
        )
        .expect_err("top-level create refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(r#"cannot place new section "Brand" — its parent is not in the document"#)
        );
    }

    /// `createTarget` heading text, emulation-verbatim: leading `\n`, child
    /// depth = parent + 1, blank line, body, trailing `\n` — `Put{end}` on the
    /// PARENT (the named EOF-vs-parent residual lives in `check_write`'s
    /// candidate, not here).
    #[test]
    fn create_parent_append_shape() {
        let e = lower1(
            "# A\n\nx\n\n## B\n\ny\n",
            PlanEdit::Create {
                parent_hpath: "A/B".into(),
                title: "New Kid".into(),
                body: "hello".into(),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::End);
        assert_eq!(text, "\n### New Kid\n\nhello\n");
        let SecRef::Hpath { hpath } = &e.target else {
            panic!("hpath target")
        };
        assert_eq!(
            hpath.iter().map(|s| s.h.as_str()).collect::<Vec<_>>(),
            vec!["A", "B"],
            "targets the RAW parent chain"
        );
    }

    /// match all:true = the read-modify-write moved engine-side: every
    /// occurrence over the heading-EXCLUDED content, written back
    /// `Put{content}`; the not-found arm string is host-verbatim.
    #[test]
    fn match_all_rmw_and_not_found() {
        let raw = "# Todo\n\n- item a\n- item b\n";
        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: "Todo".into(),
                old: "item".into(),
                new: "task".into(),
                all: true,
                rev: Some("deadbeefdeadbeef".into()),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::Content);
        // The content span starts at the byte AFTER the heading line, so the
        // blank separator line rides the content — exactly the bytes the Go
        // host's stripHeading served and round-tripped through Put{content}.
        assert_eq!(text, "\n- task a\n- task b\n");
        assert_eq!(
            e.if_node_rev.as_ref().map(|r| r.0.as_str()),
            Some("deadbeefdeadbeef")
        );

        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: "Todo".into(),
                old: "ghost".into(),
                new: "x".into(),
                all: true,
                rev: None,
            },
        )
        .expect_err("not found refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(r#"replace anchor "ghost" not found in "Todo""#)
        );
    }

    /// `replace_section`: rev required (arm-faithful dead branch), payload
    /// trailing-NL ensured, empty body stays empty, `if_node_rev` threaded.
    #[test]
    fn replace_section_rev_and_payload() {
        let raw = "# Notes\n\nold\n";
        let err = lower1(
            raw,
            PlanEdit::ReplaceSection {
                hpath: "Notes".into(),
                body: "new".into(),
                rev: None,
            },
        )
        .expect_err("rev-less refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                r#"replace_section on "Notes" requires a fresh rev (a whole-section rewrite is destructive) — read the section and pass its rev"#
            )
        );

        let e = lower1(
            raw,
            PlanEdit::ReplaceSection {
                hpath: "Notes".into(),
                body: "new".into(),
                rev: Some("cafebabecafebabe".into()),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::Content);
        assert_eq!(text, "new\n");
        assert_eq!(
            e.if_node_rev.as_ref().map(|r| r.0.as_str()),
            Some("cafebabecafebabe")
        );
    }

    /// The property dance, byte-faithful (`buildPropertyEdits`): existing key
    /// → whole-line `Put{all}`; absent keys → `"\nk: v"` lines after the LAST
    /// key; carrier fold when the last key is itself being set.
    #[test]
    fn property_group_dance() {
        let raw = "---\nstatus: open\nowner: d\n---\n# A\n\nx\n";

        // Existing key alone → one Put{all} with the composed line.
        let edits = super::lower(
            &doc(raw),
            &[PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed".into(),
            }],
        )
        .expect("lowers");
        assert_eq!(edits.len(), 1);
        let (at, text) = put_text(&edits[0]);
        assert_eq!((*at, text), (PutAt::All, "status: closed"));
        assert!(matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "status"));

        // Absent key, last key NOT being set → Put{end} after the last key.
        let edits = super::lower(
            &doc(raw),
            &[PlanEdit::SetProperty {
                key: "zeta".into(),
                value: "1".into(),
            }],
        )
        .expect("lowers");
        assert_eq!(edits.len(), 1);
        let (at, text) = put_text(&edits[0]);
        assert_eq!((*at, text), (PutAt::End, "\nzeta: 1"));
        assert!(matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "owner"));

        // Absent key + last key being set → the carrier fold: ONE Put{all}
        // carrying its own line plus the absent lines, then the other
        // existing key's Put{all}.
        let edits = super::lower(
            &doc(raw),
            &[
                PlanEdit::SetProperty {
                    key: "owner".into(),
                    value: "e".into(),
                },
                PlanEdit::SetProperty {
                    key: "zeta".into(),
                    value: "1".into(),
                },
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed".into(),
                },
            ],
        )
        .expect("lowers");
        assert_eq!(edits.len(), 2);
        let (at, text) = put_text(&edits[0]);
        assert_eq!((*at, text), (PutAt::All, "owner: e\nzeta: 1"));
        assert!(matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "owner"));
        let (at, text) = put_text(&edits[1]);
        assert_eq!((*at, text), (PutAt::All, "status: closed"));
    }

    /// No frontmatter to anchor a NEW key → the host teaching (dead through
    /// the put face: `check_write`'s `E_NO_MATCH` fires first).
    #[test]
    fn property_no_frontmatter_refuses() {
        let err = super::lower(
            &doc("# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "status".into(),
                value: "open".into(),
            }],
        )
        .expect_err("no fm refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "cannot set a new property — the file has no frontmatter to anchor it (add a '---' block first)"
            )
        );
    }

    /// The conditional quote rides the ONE shared predicate: a `": "`-bearing
    /// value lands single-quoted, exactly as the `check_write` candidate
    /// composes it (single-owner, no drift).
    #[test]
    fn property_value_quotes_through_shared_predicate() {
        let edits = super::lower(
            &doc("---\nnote: x\n---\n# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "note".into(),
                value: "a: b".into(),
            }],
        )
        .expect("lowers");
        let (_, text) = put_text(&edits[0]);
        assert_eq!(text, "note: 'a: b'");
    }

    /// Empty value composes the host's `"k: "` line (trailing space) — the
    /// dance's `line(k)` verbatim, never model-upsert's `"k:"`.
    #[test]
    fn property_empty_value_keeps_trailing_space() {
        let edits = super::lower(
            &doc("---\nnote: x\n---\n# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "note".into(),
                value: String::new(),
            }],
        )
        .expect("lowers");
        let (_, text) = put_text(&edits[0]);
        assert_eq!(text, "note: ");
    }

    /// Duplicate sanitized chains: Go map overwrite — the LAST occurrence
    /// wins (unreachable through the put face: `check_write` refuses
    /// `E_AMBIGUOUS` first; pinned for direct wire callers).
    #[test]
    fn duplicate_headings_last_wins() {
        let e = lower1(
            "# Notes\n\nfirst\n\n# Notes\n\nsecond\n",
            PlanEdit::Match {
                hpath: "Notes".into(),
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect("resolves the LAST duplicate");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\n2nd\n");
    }

    /// Raw-title addresses MISS (the host map keys sanitized chains only —
    /// no title fallback, unlike `check_write`'s rebuild resolve).
    #[test]
    fn raw_title_address_misses() {
        let err = lower1(
            "# My Section\n\nx\n",
            PlanEdit::Match {
                hpath: "My Section".into(),
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect_err("raw title misses the sanitized map");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"My Section\". No edit was applied; the batch \
                 is refused whole. Fix: read the page with no selector to list its \
                 section paths."
            )
        );
        // The sanitized spelling hits.
        lower1(
            "# My Section\n\nx\n",
            PlanEdit::Match {
                hpath: "My-Section".into(),
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect("sanitized address resolves");
    }
}
