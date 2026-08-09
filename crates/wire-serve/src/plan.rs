//! Lower `splice.plan_edits` to native wire edits (Go `buildSpliceEdit` /
//! `buildPropertyEdits` emulation). Byte-faithful to the deleted host arms so
//! downstream validate/CAS/armed/reparse behave identically by construction.

use wire::{Edit, EditShape, ErrorBody, HpathSeg, NodeRev, PutAt, SecRef};

use crate::bad_request;

/// One resolved heading in the host-face index (`tocIndex` `tocNode` lift).
struct HeadingFacts {
    /// The published RAW address — carries `n` only where ambiguous, exactly as
    /// the read face publishes it. Rides verbatim into the native edit target.
    raw_hpath: Vec<HpathSeg>,
    /// Heading level (create child depth).
    level: u32,
    /// Full node span (subtree-inclusive).
    span: (usize, usize),
    /// Heading-excluded content span, when present.
    content_span: Option<(usize, usize)>,
}

/// Host-face put index: the read face's own address table + fm keys in order.
///
/// The index is the same [`wire_map::facts::read_facts`] table the read plane
/// resolves against, so one address grammar and one occurrence law span both
/// planes. (A sanitized-join key map made `# A/B` and `# A B` one key,
/// last-wins, so a write could land on the wrong section silently.)
struct PlanIndex {
    headings: Vec<HeadingFacts>,
    fm_keys: Vec<String>,
}

/// Why an address resolved to no single section.
enum Miss {
    NotFound,
    /// The address matched more than one section — it abstained on `n` where
    /// the document is ambiguous.
    Ambiguous(usize),
}

impl PlanIndex {
    fn new(doc: &model::Document) -> Self {
        let rows = wire_map::project_toc(doc);
        let fm_keys = rows
            .iter()
            .filter(|r| r.kind == "frontmatter")
            .flat_map(|r| r.keys.clone().unwrap_or_default())
            .collect();
        let headings = wire_map::facts::read_facts(&rows, doc.raw.as_bytes())
            .into_iter()
            .filter(|f| f.anchor.is_none() && !f.hpath.is_empty())
            .map(|f| HeadingFacts {
                raw_hpath: f.hpath,
                level: f.depth,
                span: (span_usize(f.span.0), span_usize(f.span.1)),
                content_span: f
                    .content_span
                    .map(|cs| (span_usize(cs.0), span_usize(cs.1))),
            })
            .collect();
        PlanIndex { headings, fm_keys }
    }

    /// Resolve an address to exactly one section, or say why not.
    ///
    /// The occurrence law is `model::resolve_hpath_node`'s, not the read face's
    /// first-wins: a selector segment with `n: None` demands uniqueness, and an
    /// ambiguous address refuses — the write plane never silently picks.
    fn get(&self, addr: &[HpathSeg]) -> Result<&HeadingFacts, Miss> {
        if addr.is_empty() {
            return Err(Miss::NotFound);
        }
        let mut hits = self.headings.iter().filter(|f| seg_chain_matches(addr, f));
        match (hits.next(), hits.count()) {
            (None, _) => Err(Miss::NotFound),
            (Some(only), 0) => Ok(only),
            (Some(_), rest) => Err(Miss::Ambiguous(rest + 1)),
        }
    }
}

/// Per-segment address equality against a published address: same length, raw
/// text byte-equal, and an occurrence that either abstains (`n: None`) or names
/// the section's own. A published `n: None` means "unique among its siblings",
/// i.e. occurrence 1 — so `n: Some(1)` against a unique heading matches, as it
/// does natively.
fn seg_chain_matches(addr: &[HpathSeg], f: &HeadingFacts) -> bool {
    addr.len() == f.raw_hpath.len()
        && addr.iter().zip(&f.raw_hpath).all(|(sel, pub_seg)| {
            sel.h == pub_seg.h && sel.n.is_none_or(|k| k == pub_seg.n.unwrap_or(1))
        })
}

/// The section-miss refusal, shared by every addressing arm.
///
/// The `^` arm steers the message only, never the resolution: an anchor-shaped
/// address is sent to `anchors[]` rather than to a section listing.
fn section_miss(addr: &[HpathSeg], miss: &Miss) -> Box<ErrorBody> {
    let shown = crate::display_hpath(addr);
    if let Miss::Ambiguous(n) = miss {
        return bad_request(format!(
            "address {} matches {n} sections. {} Fix: pass the occurrence — the read face \
             publishes it as `n` on the ambiguous segment.",
            policy::defs::go_quote(&shown),
            crate::NO_PARTIAL_WRITE_CLAUSE,
        ));
    }
    let anchor_shaped = matches!(addr, [only] if only.h.starts_with('^'));
    bad_request(format!(
        "no section addressed by {}. {} {}",
        policy::defs::go_quote(&shown),
        crate::NO_PARTIAL_WRITE_CLAUSE,
        crate::section_recovery(if anchor_shaped { "^" } else { "" }, None)
    ))
}

/// Wire `u64` spans → `usize` checked (never lossy `as`; saturated miss hits Go bounds guards).
fn span_usize(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Lower one plan-level batch to native edits: properties first as one group,
/// then body ops in request order. Returned batch feeds the native splice path.
///
/// # Errors
/// First failing plan edit: `bad_request` teaching (host arms minus `put: ` prefix).
pub fn lower(
    doc: &model::Document,
    plan_edits: &[wire::PlanEdit],
) -> Result<Vec<Edit>, Box<ErrorBody>> {
    let idx = PlanIndex::new(doc);
    let raw = doc.raw.as_bytes();

    // Properties first (host order); last value per key wins; keys sorted.
    let mut props: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for e in plan_edits {
        if let wire::PlanEdit::SetProperty { key, value, .. } = e {
            props.insert(key, value);
        }
    }
    let mut edits = if props.is_empty() {
        Vec::new()
    } else {
        lower_property_group(doc, &idx, &props)?
    };

    for e in plan_edits {
        match e {
            wire::PlanEdit::SetProperty { .. } => {}
            wire::PlanEdit::Append { hpath, body, rev } => {
                edits.push(lower_append(&idx, raw, hpath, body, rev.as_deref())?);
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
                rev,
            } => {
                edits.push(lower_create(
                    &idx,
                    parent_hpath,
                    title,
                    body,
                    rev.as_deref(),
                )?);
            }
        }
    }
    Ok(edits)
}

/// Append arm: block targets refuse; ensureTrailingNL + leading `\n` when the
/// pre-batch byte before insert is not one. Lowered to `Put{end}` on raw hpath.
fn lower_append(
    idx: &PlanIndex,
    raw: &[u8],
    hpath: &[HpathSeg],
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    if matches!(hpath, [only] if only.h.starts_with('^')) {
        return Err(bad_request(format!(
            "append to a block anchor {} is not supported — append targets a section (the containing heading path)",
            policy::defs::go_quote(&crate::display_hpath(hpath))
        )));
    }
    let node = idx.get(hpath).map_err(|m| section_miss(hpath, &m))?;
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
        if_node_rev: rev
            .filter(|r| !r.is_empty())
            .map(|r| NodeRev(r.to_string())),
    })
}

/// Replace arm: unique-anchor Match, or all:true RMW over heading-excluded
/// content written back as `Put{content}`.
fn lower_match(
    idx: &PlanIndex,
    raw: &[u8],
    hpath: &[HpathSeg],
    old: &str,
    new: &str,
    all: bool,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    let node = idx.get(hpath).map_err(|m| section_miss(hpath, &m))?;
    // Peel `@fp` from `old` (needle in stored bytes) before search; `new` is payload.
    let old = &*syntax::strip_fp(old);
    let if_node_rev = rev
        .filter(|r| !r.is_empty())
        .map(|r| NodeRev(r.to_string()));
    if all {
        // Go stripHeading: content-span offset; defensive full-span fallbacks.
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
                policy::defs::go_quote(&crate::display_hpath(hpath))
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

/// `replace_section`: rev required; ensureTrailingNL (empty stays empty);
/// `Put{content}` + `if_node_rev`.
fn lower_replace_section(
    idx: &PlanIndex,
    hpath: &[HpathSeg],
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    let node = idx.get(hpath).map_err(|m| section_miss(hpath, &m))?;
    let rev = rev.unwrap_or("");
    if rev.is_empty() {
        return Err(bad_request(format!(
            "replace_section on {} requires a fresh rev (a whole-section rewrite is destructive) — read the section and pass its rev",
            policy::defs::go_quote(&crate::display_hpath(hpath))
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

/// Create: parent-append as `Put{end}` on parent; top-level / parent-miss refuse.
/// `rev` is the PARENT's node-grain token, threaded to the lowered append's
/// `if_node_rev` — one rev derivation, no second comparison rule (§ A.3).
fn lower_create(
    idx: &PlanIndex,
    parent_hpath: &[HpathSeg],
    title: &str,
    body: &str,
    rev: Option<&str>,
) -> Result<Edit, Box<ErrorBody>> {
    let full = if parent_hpath.is_empty() {
        title.to_string()
    } else {
        format!("{}/{title}", crate::display_hpath(parent_hpath))
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
    // An ambiguous parent says so by name: `cannot_place` would claim the
    // parent is absent when the document holds several of it.
    let parent = idx.get(parent_hpath).map_err(|m| match m {
        Miss::NotFound => cannot_place(),
        m @ Miss::Ambiguous(_) => section_miss(parent_hpath, &m),
    })?;
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
        if_node_rev: rev
            .filter(|r| !r.is_empty())
            .map(|r| NodeRev(r.to_string())),
    })
}

/// Property group: existing → `Put{all}`; absent after last key (carrier fold
/// if last is being set). Keys/values only through `yaml_safe_key`/
/// `yaml_preserve_or_encode` — an existing key's stored line feeds the
/// § A.6.3c no-op preservation, so a write-back of the served value recomposes
/// the stored spelling byte-identically.
fn lower_property_group(
    doc: &model::Document,
    idx: &PlanIndex,
    props: &std::collections::BTreeMap<&str, &str>,
) -> Result<Vec<Edit>, Box<ErrorBody>> {
    // Key owner first (same refusal surface as rebuild::plan_set_property).
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
        // An existing key's stored line, for § A.6.3c preservation. A resolve
        // miss (e.g. a duplicate key) just falls back to the fresh encode —
        // preservation is byte quiet, never a new refusal surface.
        let stored_line = if fm_key_set.contains(k.as_str()) {
            model::resolve(doc, &model::Ref::FmKey(k.as_str().to_string()))
                .ok()
                .map(|t| doc.raw[t.span].to_string())
        } else {
            None
        };
        // A newline in a value forges frontmatter keys — refuse, never sanitize.
        let safe = policy::defs::yaml_preserve_or_encode(stored_line.as_deref(), v).map_err(|_| {
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
        // Inserts after the last key; fold into Put{all} if that key is itself being set.
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
    use wire::{EditShape, HpathSeg, PlanEdit, PutAt, SecRef};

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

    /// Append newline discipline (trailing ensured; leading when pre-batch byte not `\n`).
    #[test]
    fn append_newline_discipline() {
        let e = lower1(
            "# Memo\n\nline\n",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "Memo".into(),
                    n: None,
                }],
                body: "added".into(),
                rev: None,
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::End);
        assert_eq!(text, "added\n", "trailing NL ensured, no leading NL");

        let e = lower1(
            "# Memo\n\nline",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "Memo".into(),
                    n: None,
                }],
                body: "added\n".into(),
                rev: None,
            },
        )
        .expect("lowers");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\nadded\n", "leading NL against a bare final line");
    }

    /// Block replace refuses; no `put: ` prefix.
    #[test]
    fn replace_on_block_refuses_no_section_addressed() {
        let err = lower1(
            "# Tasks\n\n- [ ] one ^task1\n",
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "^task1".into(),
                    n: None,
                }],
                old: "one".into(),
                new: "two".into(),
                all: false,
                rev: None,
            },
        )
        .expect_err("block replace target refuses");
        // `toc` carries no `^` anchors, so this miss must not send the reader
        // to the section listing.
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"^task1\". No edit was applied; the batch is \
                 refused whole. Fix: the section map does not list `^` anchors — find \
                 the id inline in the section's content, or via CLI `--json` in its \
                 `anchors[]`."
            )
        );
    }

    /// Append-to-block refusal arm string (arm-faithful for direct callers).
    #[test]
    fn append_to_block_refuses_arm_string() {
        let err = lower1(
            "# Tasks\n\n- item ^t1\n",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "^t1".into(),
                    n: None,
                }],
                body: "x".into(),
                rev: None,
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

    /// Append section-miss includes toc remedy tail.
    #[test]
    fn append_miss_names_a_runnable_recovery() {
        let err = lower1(
            "# A\n\nx\n",
            PlanEdit::Append {
                hpath: vec![HpathSeg {
                    h: "Ghost".into(),
                    n: None,
                }],
                body: "x".into(),
                rev: None,
            },
        )
        .expect_err("miss refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"Ghost\". No edit was applied; the batch is \
                 refused whole. Fix: list the document's section paths with a toc read \
                 (MCP read: mode:\"toc\"; CLI: a read with no --section), then feed its \
                 row back in one of the two addressing forms: the row's raw heading \
                 segments as an hpath array (one entry per heading, no joining), or its \
                 dewey ordinal (CLI: `--section 1.2`). The joined selector string splits \
                 on `/`, so a heading whose raw text carries one is reachable only by \
                 those two forms."
            )
        );
    }

    /// Top-level create refuses.
    #[test]
    fn create_top_level_refuses() {
        let err = lower1(
            "# A\n\nx\n",
            PlanEdit::Create {
                parent_hpath: vec![],
                title: "Brand".into(),
                body: "b".into(),
                rev: None,
            },
        )
        .expect_err("top-level create refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(r#"cannot place new section "Brand" — its parent is not in the document"#)
        );
    }

    /// Create parent-append shape: `Put{end}` on parent, child depth = parent+1.
    #[test]
    fn create_parent_append_shape() {
        let e = lower1(
            "# A\n\nx\n\n## B\n\ny\n",
            PlanEdit::Create {
                parent_hpath: vec![
                    HpathSeg {
                        h: "A".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "B".into(),
                        n: None,
                    },
                ],
                title: "New Kid".into(),
                body: "hello".into(),
                rev: None,
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

    /// match all:true RMW over heading-excluded content; not-found arm host-verbatim.
    #[test]
    fn match_all_rmw_and_not_found() {
        let raw = "# Todo\n\n- item a\n- item b\n";
        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Todo".into(),
                    n: None,
                }],
                old: "item".into(),
                new: "task".into(),
                all: true,
                rev: Some("deadbeefdeadbeef".into()),
            },
        )
        .expect("lowers");
        let (at, text) = put_text(&e);
        assert_eq!(*at, PutAt::Content);
        // Content span starts after heading (blank separator rides content) — Go stripHeading.
        assert_eq!(text, "\n- task a\n- task b\n");
        assert_eq!(
            e.if_node_rev.as_ref().map(|r| r.0.as_str()),
            Some("deadbeefdeadbeef")
        );

        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Todo".into(),
                    n: None,
                }],
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

    /// `replace_section`: rev required; trailing-NL; empty body empty; `if_node_rev`.
    #[test]
    fn replace_section_rev_and_payload() {
        let raw = "# Notes\n\nold\n";
        let err = lower1(
            raw,
            PlanEdit::ReplaceSection {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
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
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
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

    /// Property dance: Put{all} existing; absent after last; carrier fold when last set.
    #[test]
    fn property_group_dance() {
        let raw = "---\nstatus: open\nowner: d\n---\n# A\n\nx\n";

        let edits = super::lower(
            &doc(raw),
            &[PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed".into(),
                rev: None,
            }],
        )
        .expect("lowers");
        assert_eq!(edits.len(), 1);
        let (at, text) = put_text(&edits[0]);
        assert_eq!((*at, text), (PutAt::All, "status: closed"));
        assert!(matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "status"));

        let edits = super::lower(
            &doc(raw),
            &[PlanEdit::SetProperty {
                key: "zeta".into(),
                value: "1".into(),
                rev: None,
            }],
        )
        .expect("lowers");
        assert_eq!(edits.len(), 1);
        let (at, text) = put_text(&edits[0]);
        assert_eq!((*at, text), (PutAt::End, "\nzeta: 1"));
        assert!(matches!(&edits[0].target, SecRef::FmKey { fm_key } if fm_key == "owner"));

        let edits = super::lower(
            &doc(raw),
            &[
                PlanEdit::SetProperty {
                    key: "owner".into(),
                    value: "e".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "zeta".into(),
                    value: "1".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed".into(),
                    rev: None,
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

    /// No frontmatter to anchor a new key refuses.
    #[test]
    fn property_no_frontmatter_refuses() {
        let err = super::lower(
            &doc("# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "status".into(),
                value: "open".into(),
                rev: None,
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

    /// Values with `: ` quote through shared `yaml_safe_value` predicate — in
    /// the § A.6.3 double-quoted spelling, since the predicate is the one owner.
    #[test]
    fn property_value_quotes_through_shared_predicate() {
        let edits = super::lower(
            &doc("---\nnote: x\n---\n# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "note".into(),
                value: "a: b".into(),
                rev: None,
            }],
        )
        .expect("lowers");
        let (_, text) = put_text(&edits[0]);
        assert_eq!(text, "note: \"a: b\"");
    }

    /// Empty value keeps the trailing space (never the model-upsert `k:`) and
    /// lands the § A.6.3 empty STRING: this plane is typed `string`, so a bare
    /// `k: ` would emit a null the caller has no way to mean.
    #[test]
    fn property_empty_value_keeps_trailing_space() {
        let edits = super::lower(
            &doc("---\nnote: x\n---\n# A\n\nx\n"),
            &[PlanEdit::SetProperty {
                key: "note".into(),
                value: String::new(),
                rev: None,
            }],
        )
        .expect("lowers");
        let (_, text) = put_text(&edits[0]);
        assert_eq!(text, "note: \"\"");
    }

    /// Duplicate headings refuse without an occurrence rather than silently
    /// picking one.
    #[test]
    fn duplicate_headings_refuse_without_an_occurrence() {
        let raw = "# Notes\n\nfirst\n\n# Notes\n\nsecond\n";
        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect_err("an ambiguous address refuses");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "address \"Notes\" matches 2 sections. No edit was applied; the batch is \
                 refused whole. Fix: pass the occurrence — the read face publishes it as \
                 `n` on the ambiguous segment."
            )
        );

        // The occurrence the read face publishes reaches exactly one of them.
        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: Some(2),
                }],
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect("`n: 2` names the second");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\n2nd\n");
    }

    /// `sanitize_heading` is non-injective (`# A/B` and `# A B` both sanitize
    /// to `A-B`), so a sanitize-keyed index made one of them unaddressable.
    /// Each heading must have an address that reaches it and nothing else.
    #[test]
    fn each_colliding_heading_keeps_its_own_address() {
        let raw = "# A/B\n\nfirst\n\n# A B\n\nsecond\n";
        // Pinned premise: under a sanitize-keyed grammar these two headings
        // were one key.
        assert_eq!(wire_map::gotext::sanitize_heading("A/B"), "A-B");
        assert_eq!(wire_map::gotext::sanitize_heading("A B"), "A-B");

        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "A/B".into(),
                    n: None,
                }],
                old: "first".into(),
                new: "1st".into(),
                all: true,
                rev: None,
            },
        )
        .expect("the `A/B` section is addressable");
        let (_, text) = put_text(&e);
        // Trailing blank line rides `A/B`'s content span (it precedes `# A B`).
        assert_eq!(text, "\n1st\n\n", "reaches `A/B`, not `A B`");

        let e = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "A B".into(),
                    n: None,
                }],
                old: "second".into(),
                new: "2nd".into(),
                all: true,
                rev: None,
            },
        )
        .expect("the `A B` section is addressable");
        let (_, text) = put_text(&e);
        assert_eq!(text, "\n2nd\n", "reaches `A B`, not `A/B`");
    }

    /// A write must land on the section the caller addressed — never on a
    /// different one while reporting success. The sanitized-join grammar made
    /// this reachable and silent: an edit addressed to `A/B` lowered to a
    /// target on `A B` and returned `Ok`.
    #[test]
    fn a_write_addressed_to_one_section_never_lands_on_another() {
        // The anchor text is present in both sections, so a wrong-section write
        // succeeds instead of refusing — the silent path, not the loud one.
        let raw = "# A/B\n\nnote here\n\n# A B\n\nnote here\n";

        for (addressed, other) in [("A/B", "A B"), ("A B", "A/B")] {
            let e = lower1(
                raw,
                PlanEdit::Match {
                    hpath: vec![HpathSeg {
                        h: addressed.into(),
                        n: None,
                    }],
                    old: "note".into(),
                    new: "NOTE".into(),
                    all: true,
                    rev: None,
                },
            )
            .unwrap_or_else(|e| panic!("{addressed} is addressable: {:?}", e.message));

            let SecRef::Hpath { hpath } = &e.target else {
                panic!("hpath target")
            };
            let landed = hpath.iter().map(|s| s.h.as_str()).collect::<Vec<_>>();
            assert_eq!(
                landed,
                vec![addressed],
                "addressed {addressed:?} but the edit targets {landed:?} — a write \
                 aimed at one section landed on {other:?} and reported success"
            );
        }
    }

    /// The plan face speaks raw segments: the raw title is the address, and
    /// the sanitized spelling names no heading.
    #[test]
    fn raw_title_addresses_and_the_sanitized_spelling_misses() {
        let raw = "# My Section\n\nx\n";
        lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "My Section".into(),
                    n: None,
                }],
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect("the raw title IS the address");

        let err = lower1(
            raw,
            PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "My-Section".into(),
                    n: None,
                }],
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            },
        )
        .expect_err("the sanitized spelling names no heading");
        assert_eq!(
            err.message.as_deref(),
            Some(
                "no section addressed by \"My-Section\". No edit was applied; the batch \
                 is refused whole. Fix: list the document's section paths with a toc \
                 read (MCP read: mode:\"toc\"; CLI: a read with no --section), then feed \
                 its row back in one of the two addressing forms: the row's raw heading \
                 segments as an hpath array (one entry per heading, no joining), or its \
                 dewey ordinal (CLI: `--section 1.2`). The joined selector string splits \
                 on `/`, so a heading whose raw text carries one is reachable only by \
                 those two forms."
            )
        );
    }
}
