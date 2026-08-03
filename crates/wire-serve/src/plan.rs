//! Lower `splice.plan_edits` to native wire edits (Go `buildSpliceEdit` /
//! `buildPropertyEdits` emulation). Byte-faithful to the deleted host arms so
//! downstream validate/CAS/armed/reparse behave identically by construction.

use wire::{Edit, EditShape, ErrorBody, HpathSeg, NodeRev, PutAt, SecRef};

use crate::bad_request;

/// One resolved heading in the host-face index (`tocIndex` `tocNode` lift).
struct HeadingFacts {
    /// Raw hpath segments for the native edit target.
    raw_hpath: Vec<HpathSeg>,
    /// Heading level (create child depth).
    level: u32,
    /// Full node span (subtree-inclusive).
    span: (usize, usize),
    /// Heading-excluded content span, when present.
    content_span: Option<(usize, usize)>,
}

/// Host-face put index: sanitized-chain map (Go last-wins) + fm keys in order.
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
                    // Go map overwrite: LAST duplicate wins.
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
            wire::PlanEdit::SetProperty { .. } => {}
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

/// Append arm: block targets refuse; ensureTrailingNL + leading `\n` when the
/// pre-batch byte before insert is not one. Lowered to `Put{end}` on raw hpath.
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

/// Replace arm: unique-anchor Match, or all:true RMW over heading-excluded
/// content written back as `Put{content}`.
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
    // S10: peel `@fp` from `old` (needle in stored bytes) before search; `new` is payload.
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

/// `replace_section`: rev required; ensureTrailingNL (empty stays empty);
/// `Put{content}` + `if_node_rev`.
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

/// Create: parent-append as `Put{end}` on parent; top-level / parent-miss refuse.
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

/// Property group: existing → `Put{all}`; absent after last key (carrier fold
/// if last is being set). Keys/values only through `yaml_safe_key`/`yaml_safe_value`.
fn lower_property_group(
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
        // D11: newline in value forges frontmatter keys — refuse, never sanitize.
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
        // Inserts after LAST key; fold into Put{all} if that key is itself being set.
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

    /// Append newline discipline (trailing ensured; leading when pre-batch byte not `\n`).
    #[test]
    fn append_newline_discipline() {
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

    /// Block replace refuses (goldens p-replace-on-block); no `put: ` prefix.
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

    /// Append-to-block refusal arm string (arm-faithful for direct callers).
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

    /// Append section-miss includes toc remedy tail.
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

    /// Top-level create refuses (goldens p-create-top).
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

    /// Create parent-append shape: `Put{end}` on parent, child depth = parent+1.
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

    /// match all:true RMW over heading-excluded content; not-found arm host-verbatim.
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
        // Content span starts after heading (blank separator rides content) — Go stripHeading.
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

    /// `replace_section`: rev required; trailing-NL; empty body empty; `if_node_rev`.
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

    /// Property dance: Put{all} existing; absent after last; carrier fold when last set.
    #[test]
    fn property_group_dance() {
        let raw = "---\nstatus: open\nowner: d\n---\n# A\n\nx\n";

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

    /// No frontmatter to anchor a new key refuses.
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

    /// Values with `: ` quote through shared `yaml_safe_value` predicate.
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

    /// Empty value keeps trailing space (`k: `, not model-upsert `k:`).
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

    /// Duplicate sanitized chains: last wins (Go map overwrite).
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

    /// Raw-title addresses miss (map keys sanitized only).
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
