//! The `unresolved` plane on the composed read (wire-contract § A.3, the
//! 07-05 miss-facts card): one structured row per failed section selector,
//! the machine tense of the partial-read `notice` — a consumer acts on each
//! failure individually instead of parsing one human sentence. One law per
//! test, the `props`/`anchors` plane precedent.

use wire::{Path as WPath, ReadSel, ResponseBody, UnresolvedReason};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

/// Two list-item anchors (`^goal`, `^gate`), one task-hosted `^t1` outside
/// the face's anchor plane, and two same-titled `Twin` sections for the
/// ambiguity arm.
const DOC: &str = "# Tasks\n\n- ship the gate ^goal\n- prove the gate ^gate\n- [ ] boxed ^t1\n\n# Twin\n\none\n\n# Twin\n\ntwo\n";

fn read_doc(raw: &str, sections: Option<Vec<ReadSel>>) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    let d = model::build(raw.to_string(), syntax::parse(raw));
    composed_read(
        &d,
        &WPath("card.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            sections,
            display_path: Some("card.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
}

fn unresolved_of(body: ResponseBody) -> (Vec<wire::ReadUnresolved>, Option<String>) {
    let ResponseBody::Read {
        unresolved, notice, ..
    } = body
    else {
        panic!("composed read answers a Read body");
    };
    (unresolved, notice)
}

/// Emission law: always emitted — empty means "every selector resolved",
/// and a toc read trivially so. Never "ask again with a flag".
#[test]
fn all_resolved_and_toc_mode_serve_an_empty_plane() {
    let (rows, notice) = unresolved_of(
        read_doc(DOC, Some(vec![ReadSel::parse("Tasks")])).expect("Tasks serves"),
    );
    assert!(rows.is_empty(), "every selector resolved ⇒ empty plane");
    assert!(notice.is_none(), "no failures ⇒ no notice");
    let (rows, _) = unresolved_of(read_doc(DOC, None).expect("toc mode serves"));
    assert!(rows.is_empty(), "toc mode carries the plane, trivially empty");
}

/// A heading miss is one `no_match` row — selector echoed in its request
/// grammar, every other fact slot empty.
#[test]
fn heading_miss_is_a_bare_no_match_row() {
    let (rows, notice) = unresolved_of(
        read_doc(DOC, Some(vec![ReadSel::parse("Tasks"), ReadSel::parse("Ghost")]))
            .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.sel, ReadSel::parse("Ghost"), "the failed selector, echoed");
    assert_eq!(row.reason, UnresolvedReason::NoMatch);
    assert!(row.candidates.is_empty() && row.nearest.is_empty());
    assert_eq!((row.count, row.host.as_deref()), (None, None));
    assert!(
        notice.expect("partial read keeps its prose notice").contains("Ghost"),
        "the prose tense stays beside the structured one"
    );
}

/// An ambiguous heading row carries each candidate's machine address as the
/// §2.1 `n`-carrying segment array — actual arrays, never encoded strings.
#[test]
fn ambiguous_heading_row_carries_machine_addresses() {
    let (rows, _) = unresolved_of(
        read_doc(DOC, Some(vec![ReadSel::parse("Tasks"), ReadSel::parse("Twin")]))
            .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.reason, UnresolvedReason::Ambiguous);
    assert_eq!(row.candidates.len(), 2, "both Twin occurrences named");
    for (i, cand) in row.candidates.iter().enumerate() {
        let seg = cand.last().expect("a candidate address is non-empty");
        assert_eq!(seg.h, "Twin");
        assert_eq!(seg.n, Some(i as u32 + 1), "the n that pins the occurrence");
    }
    assert!(row.nearest.is_empty() && row.count.is_none() && row.host.is_none());
}

/// A duplicated block id counts its carriers and names no candidate — no
/// per-candidate machine address exists (the door-symmetry law).
#[test]
fn duplicate_anchor_row_counts_carriers_with_no_candidates() {
    let raw = "# H\n\n- one ^dup\n- two ^dup\n\n# K\n\nbody\n";
    let (rows, _) = unresolved_of(
        read_doc(raw, Some(vec![ReadSel::parse("K"), ReadSel::parse("^dup")]))
            .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.reason, UnresolvedReason::DuplicateAnchor);
    assert_eq!(row.count, Some(2));
    assert!(
        row.candidates.is_empty(),
        "a duplicated id has no per-candidate machine address"
    );
}

/// An id that exists on a host outside the face's anchor plane is
/// `unaddressable_host` with the TRUE host kind — distinct from `no_match`,
/// and never carrying nearest candidates (a limit must not imply absence).
#[test]
fn unaddressable_host_row_names_the_true_kind() {
    let (rows, _) = unresolved_of(
        read_doc(DOC, Some(vec![ReadSel::parse("Tasks"), ReadSel::parse("^t1")]))
            .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.reason, UnresolvedReason::UnaddressableHost);
    assert_eq!(row.host.as_deref(), Some("task"));
    assert!(row.nearest.is_empty(), "a limit refusal implies no absence");
}

/// The season-1b addendum law: the nearest-candidate pool spans every `^id`
/// on the page, non-addressable hosts included — a typo one character short
/// of a paragraph-hosted id gets its candidate WITH the host kind, so the
/// render teaches the host-kind gate instead of refusing bare.
#[test]
fn nearest_pool_includes_non_addressable_ids() {
    let raw = "# H\n\nprose ^dogfood-anchor\n\n# K\n\nbody\n";
    let (rows, notice) = unresolved_of(
        read_doc(
            raw,
            Some(vec![ReadSel::parse("K"), ReadSel::parse("^dogfood-anchr")]),
        )
        .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.reason, UnresolvedReason::NoMatch);
    assert_eq!(row.nearest.len(), 1, "the paragraph-hosted id IS the candidate");
    assert_eq!(row.nearest[0].anchor, "dogfood-anchor");
    assert_eq!(row.nearest[0].kind, "paragraph");
    let notice = notice.expect("notice");
    assert!(
        notice.contains("^dogfood-anchor (paragraph-hosted"),
        "the prose tense teaches the host-kind gate on the candidate: {notice}"
    );
}

/// The negative half: a page with no `^id` of any host kind says so plainly
/// and serves an empty `nearest` — no fabricated candidates.
#[test]
fn anchorless_page_serves_empty_nearest_and_says_so() {
    let raw = "# H\n\nprose only\n\n# K\n\nbody\n";
    let (rows, notice) = unresolved_of(
        read_doc(raw, Some(vec![ReadSel::parse("K"), ReadSel::parse("^gone")]))
            .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason, UnresolvedReason::NoMatch);
    assert!(rows[0].nearest.is_empty());
    assert!(
        notice.expect("notice").contains("this page carries no block anchors"),
        "the bare-page claim is now unconditionally true"
    );
}

/// Rows ride in request order and pair 1:1 with the notice's entries — one
/// resolution pass, three tenses, no disagreement possible.
#[test]
fn rows_follow_request_order_beside_the_notice() {
    let (rows, notice) = unresolved_of(
        read_doc(
            DOC,
            Some(vec![
                ReadSel::parse("Ghost"),
                ReadSel::parse("Tasks"),
                ReadSel::parse("Twin"),
            ]),
        )
        .expect("partial read serves"),
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].sel, ReadSel::parse("Ghost"));
    assert_eq!(rows[1].sel, ReadSel::parse("Twin"));
    let notice = notice.expect("notice");
    let (g, t) = (
        notice.find("Ghost").expect("Ghost in notice"),
        notice.find("Twin").expect("Twin in notice"),
    );
    assert!(g < t, "prose and rows share one order: {notice}");
}

/// The wire law pair: serialization is unconditional (`"unresolved":[]` on a
/// clean read), and decoding stays tolerant of older recorded frames that
/// never carried the key.
#[test]
fn serialize_unconditionally_decode_tolerantly() {
    let body = read_doc(DOC, Some(vec![ReadSel::parse("Tasks")])).expect("serves");
    let json = serde_json::to_string(&body).expect("serialize");
    assert!(
        json.contains("\"unresolved\":[]"),
        "empty is a served fact, not an omission: {json}"
    );
    let old_frame = json.replace(",\"unresolved\":[]", "");
    assert!(!old_frame.contains("unresolved"), "the older frame lacks the key");
    let decoded: ResponseBody = serde_json::from_str(&old_frame).expect("tolerant decode");
    let ResponseBody::Read { unresolved, .. } = decoded else {
        panic!("still a Read body");
    };
    assert!(unresolved.is_empty(), "absent key decodes as the empty plane");
}
