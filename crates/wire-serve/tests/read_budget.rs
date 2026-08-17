//! The read plane's own budget refusals (session `12-04-f2-mrd-integration`,
//! card `read-budget-refusal-missing`; dogfood r9 § F1).
//!
//! F1 measured the read door serving without any bound of its own: one section
//! of 223,137 words was served whole and the MCP HOST clipped it — the caller
//! got a host truncation, no engine banner, no `→` line, and the answer lost.
//! The same call with 65 identical selectors served 65 byte-identical copies.
//!
//! Two bounds close it, and neither is invented here:
//!
//! - **Words, not bytes.** The face already speaks words (`words_total`, a toc
//!   row's `words:N`), so the ceiling is discoverable BEFORE it refuses —
//!   face-honesty clause 2 — for free. A byte bound would be invisible until
//!   tripped.
//! - **64 distinct selectors**, the § A.8 fan-out ceiling every face list
//!   carries, applied after identical selectors dedupe.
//!
//! The recovery the refusal points at must stay SERVABLE (clause 3): the toc
//! read is the way in, so it is never word-bounded.

use std::fmt::Write as _;

use wire::{ErrorCode, Path as WPath, ReadSel, ResponseBody};
use wire_serve::read::{
    NO_DECORATIONS, READ_MAX_SELECTORS, READ_MAX_WORDS, ReadParams, composed_read,
};

/// The ceiling as a `usize`, for the fixture builders. Stated once so the
/// tests below read as arithmetic on the constant, not on a cast.
fn max_words() -> usize {
    usize::try_from(READ_MAX_WORDS).expect("the ceiling fits a usize on any host that runs this")
}

/// A document whose one section carries `words` words — receipt A's shape at
/// test scale.
fn doc_of(words: usize) -> String {
    let body = vec!["word"; words].join(" ");
    format!("---\ntype: note\n---\n\n# Big\n\n{body}\n")
}

/// Small sections, so a selector-count gate is reachable without a large
/// document.
fn many_sections(n: usize) -> String {
    let mut raw = String::from("---\ntype: note\n---\n\n");
    for i in 0..n {
        let _ = write!(raw, "# S{i}\n\nbody {i}\n\n");
    }
    raw
}

fn read(raw: &str, params: &ReadParams) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    composed_read(
        &doc,
        &WPath("page.md".into()),
        &wire::Root("r".into()),
        params,
        &NO_DECORATIONS,
    )
}

fn sections(sels: Vec<ReadSel>) -> ReadParams {
    ReadParams {
        sections: Some(sels),
        display_path: Some("page.md".into()),
        ..ReadParams::default()
    }
}

fn dewey(n: &str) -> ReadSel {
    ReadSel::Dewey { n: n.into() }
}

/// Receipt A, at the door: a section past the ceiling refuses with its own
/// measured number instead of letting the host clip the answer away.
#[test]
fn an_oversized_section_refuses_with_its_measured_words() {
    let raw = doc_of(max_words() + 500);
    let err =
        read(&raw, &sections(vec![dewey("1")])).expect_err("past the ceiling, the read refuses");
    let msg = err.message.clone().unwrap_or_default();
    assert_eq!(
        err.code,
        ErrorCode::BadRequest,
        "a fix-class refusal: {msg}"
    );
    assert!(
        msg.contains(&READ_MAX_WORDS.to_string()),
        "the refusal names the ceiling: {msg}"
    );
    let biggest = msg
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|t| t.parse::<u64>().ok())
        .max()
        .unwrap_or_default();
    assert!(
        biggest > READ_MAX_WORDS,
        "the refusal names the words this call would have served, measured: {msg}"
    );
    assert!(
        msg.contains("→ narrow"),
        "the refusal carries its fitted recovery: {msg}"
    );
    assert!(
        msg.contains("toc") && msg.contains("sections"),
        "the recovery names the verbs that answer the smaller question: {msg}"
    );
}

/// The ceiling is over the WHOLE call, not per section: two halves that each
/// fit and together do not still refuse.
#[test]
fn the_ceiling_bounds_the_call_not_the_section() {
    let half = max_words() / 2 + 200;
    let body = vec!["word"; half].join(" ");
    let raw = format!("---\ntype: note\n---\n\n# A\n\n{body}\n\n# B\n\n{body}\n");
    read(&raw, &sections(vec![dewey("1")])).expect("one half fits");
    read(&raw, &sections(vec![dewey("1"), dewey("2")]))
        .expect_err("both halves together are past the ceiling");
}

/// Receipt B: 65 identical selectors are one question asked 65 times. The
/// section is served once, and the dedup is MARKED (face-honesty clause 1) —
/// never silently dropped.
#[test]
fn repeated_identical_selectors_are_served_once_and_marked() {
    let raw = many_sections(3);
    let body = read(&raw, &sections(vec![dewey("1"); 65])).expect("repeats are not a refusal");
    let ResponseBody::Read {
        sections,
        notice,
        truncated,
        ..
    } = body
    else {
        panic!("composed read answers a Read body");
    };
    let served = sections.expect("a section read serves sections");
    assert_eq!(
        truncated, None,
        "a collapsed repeat is not a truncation — every distinct selector was served"
    );
    assert_eq!(
        served.len(),
        1,
        "one selector, one row — 65 copies is waste"
    );
    let notice = notice.unwrap_or_default();
    assert!(
        notice.contains("64"),
        "the notice counts the repeats it collapsed: {notice}"
    );
}

/// Dedup is by selector, and distinct selectors that resolve to the same node
/// are NOT collapsed: the caller asked two different questions and each row
/// carries its own `sel` back.
#[test]
fn distinct_spellings_of_one_node_both_serve() {
    let raw = many_sections(3);
    let both = vec![
        dewey("1"),
        ReadSel::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "S0".into(),
                n: None,
            }],
        },
    ];
    let ResponseBody::Read { sections, .. } = read(&raw, &sections(both)).expect("both serve")
    else {
        panic!("composed read answers a Read body");
    };
    assert_eq!(
        sections.expect("sections").len(),
        2,
        "two spellings are two questions"
    );
}

/// The § A.8 ceiling every face list carries, at this face: past 64 DISTINCT
/// selectors the call refuses instead of fanning out.
#[test]
fn distinct_selectors_past_the_face_ceiling_refuse() {
    let raw = many_sections(READ_MAX_SELECTORS + 2);
    let sels: Vec<ReadSel> = (1..=READ_MAX_SELECTORS + 1)
        .map(|i| dewey(&i.to_string()))
        .collect();
    let err = read(&raw, &sections(sels)).expect_err("past the list ceiling, the read refuses");
    let msg = err.message.clone().unwrap_or_default();
    assert_eq!(
        err.code,
        ErrorCode::BadRequest,
        "a fix-class refusal: {msg}"
    );
    assert!(
        msg.contains(&READ_MAX_SELECTORS.to_string())
            && msg.contains(&(READ_MAX_SELECTORS + 1).to_string()),
        "the refusal names both the ask and the ceiling: {msg}"
    );
    assert!(
        msg.contains("→ "),
        "the refusal carries its fitted recovery: {msg}"
    );
}

/// Exactly at the ceiling still serves — an off-by-one here would refuse a
/// call the contract admits.
#[test]
fn the_ceiling_itself_serves() {
    let raw = many_sections(READ_MAX_SELECTORS + 1);
    let sels: Vec<ReadSel> = (1..=READ_MAX_SELECTORS)
        .map(|i| dewey(&i.to_string()))
        .collect();
    let ResponseBody::Read { sections, .. } =
        read(&raw, &sections(sels)).expect("64 selectors serve")
    else {
        panic!("composed read answers a Read body");
    };
    assert_eq!(sections.expect("sections").len(), READ_MAX_SELECTORS);
}

/// The recovery must be servable (face-honesty clause 3): a toc read of the
/// very document the section refusal names still answers, at any size. Bound
/// the toc and the refusal would point at a door that also refuses.
#[test]
fn the_toc_read_is_never_word_bounded() {
    let raw = doc_of(max_words() * 2);
    let body = read(
        &raw,
        &ReadParams {
            display_path: Some("page.md".into()),
            ..ReadParams::default()
        },
    )
    .expect("the toc — the recovery this plane points at — always serves");
    let ResponseBody::Read { toc, .. } = body else {
        panic!("composed read answers a Read body");
    };
    assert!(toc.is_some(), "the toc mode answers a shape table");
}

/// An ordinary read is untouched: no notice, no `truncated`, the content
/// served verbatim.
#[test]
fn an_ordinary_section_read_is_unchanged() {
    let raw = many_sections(3);
    let ResponseBody::Read {
        sections,
        notice,
        truncated,
        ..
    } = read(&raw, &sections(vec![dewey("2")])).expect("an ordinary read serves")
    else {
        panic!("composed read answers a Read body");
    };
    let served = sections.expect("sections");
    assert_eq!(served.len(), 1);
    assert!(
        served[0].content.contains("body 1"),
        "the bytes are verbatim"
    );
    assert_eq!(notice, None, "nothing to mark on an ordinary read");
    assert_eq!(truncated, None, "nothing was withheld");
}
