//! The §4.1/§4.2 read faces served from a borrowed document — what the
//! in-process host answers `read()` with.
//!
//! One conversion, mirrored 1:1 from the CLI lane's wire host
//! (`mrd::script::wire_host`): the toc face is the `toc` op's rows plus the
//! composed read's `words_total` and § A.6-decoded `props`; the cat face is
//! the `cat` op's span bytes and rev. Here both come from ONE borrowed
//! [`model::Document`], so the CLI lane's revision bracket is unnecessary by
//! construction — one document is one revision.

use std::collections::BTreeMap;

use wire::{ReadSel, ResponseBody, SecFacts, SecRef, TocEntry, TocFacts, TocNode};

/// The toc face for one document: rows, § A.6-decoded frontmatter, the
/// delivered word count, and the file rev.
///
/// # Errors
/// A rendering of the serve arms' own refusal when one arm cannot answer.
pub fn toc_facts(
    doc: &model::Document,
    path: &wire::Path,
    ambient: &wire::Root,
) -> Result<TocFacts, String> {
    let ResponseBody::Toc {
        file_rev, nodes, ..
    } = crate::read::toc(doc, path, ambient)
    else {
        return Err("the toc arm answered a non-toc body".to_owned());
    };
    let composed = crate::read::composed_read(
        doc,
        path,
        ambient,
        &crate::read::ReadParams::default(),
        None,
        &crate::read::Decorations::default(),
    )
    .map_err(|e| refusal_text(&e))?;
    let ResponseBody::Read {
        words_total, props, ..
    } = composed
    else {
        return Err("the composed-read arm answered a non-read body".to_owned());
    };
    let words = usize::try_from(words_total)
        .map_err(|_| "the composed read answered a words_total this host cannot hold".to_owned())?;
    let mut fm = BTreeMap::new();
    for prop in props {
        // Values arrive decoded by the one § A.6 codec (`read_props`) — this
        // host does not decode them a second time.
        fm.insert(prop.key, prop.value);
    }
    Ok(TocFacts {
        rev: file_rev.0,
        fm,
        toc: nodes.iter().filter_map(toc_entry).collect(),
        words,
    })
}

/// The cat face for one `section` of the document.
///
/// # Errors
/// The dewey refusal (a positional ordinal is not a document address), or a
/// rendering of the cat arm's own refusal.
pub fn sec_facts(doc: &model::Document, section: &str) -> Result<SecFacts, String> {
    let sec = sec_ref(section).ok_or_else(|| {
        "a dewey ordinal addresses a row of a table you are holding, not a document — \
         pass the heading path or a ^anchor"
            .to_owned()
    })?;
    let body = crate::read::cat(doc, Some(sec)).map_err(|e| refusal_text(&e))?;
    let ResponseBody::Cat {
        node_rev, content, ..
    } = body
    else {
        return Err("the cat arm answered a non-cat body".to_owned());
    };
    Ok(SecFacts {
        text: content,
        rev: node_rev.0,
    })
}

/// The `sec` selector for a `section=` string, through [`ReadSel::parse`] —
/// the one human-string→selector door, so this face and every other face
/// parse an address the same way. `None` for the dewey arm.
fn sec_ref(section: &str) -> Option<SecRef> {
    match ReadSel::parse(section) {
        ReadSel::Hpath { hpath } => Some(SecRef::Hpath { hpath }),
        ReadSel::Anchor { anchor } => Some(SecRef::Anchor { anchor }),
        ReadSel::Dewey { .. } => None,
    }
}

/// One toc row → one script-face entry, or nothing — the CLI lane's
/// conversion, over the typed row: a heading row publishes its hpath joined
/// by `/` (what [`ReadSel::parse`] splits again), an anchor-bearing block row
/// publishes its `^id`, and the frontmatter row reaches the face as `fm`.
fn toc_entry(node: &TocNode) -> Option<TocEntry> {
    let rev = node.node_rev.0.clone();
    let anchor = node.anchor.as_ref().map(|id| format!("^{id}"));
    let section = match &node.hpath {
        Some(hpath) => hpath
            .iter()
            .map(|seg| seg.h.as_str())
            .collect::<Vec<_>>()
            .join("/"),
        None => anchor.clone()?,
    };
    Some(TocEntry {
        section,
        anchor,
        rev,
    })
}

/// Render an engine refusal the way the CLI lane's fault reasons read:
/// `code: message`, with the recovery-less fallback the frame's own shape.
pub(crate) fn refusal_text(error: &wire::ErrorBody) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", error.code));
    match &error.message {
        Some(message) => format!("{code}: {message}"),
        None => code,
    }
}
