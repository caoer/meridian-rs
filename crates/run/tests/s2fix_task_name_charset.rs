//! Task-name charset guard — refuses decorated claim-link spellings and other
//! non-identifier names at address time. Names that the session tree and
//! grammar admit stay green.

mod support;

use run::address::{self, AddressError};
use support::doc;

/// The task name IS a decorated claim link — the name refuses, so no run
/// starts and no receipt line exists to carry the token.
#[test]
fn a_task_named_with_a_decorated_claim_link_refuses() {
    let page = "---\ntask.[[guide#^goal@green.b3af12cd|G]]: \"[[#^t-1]]\"\n---\n\n```bash\necho hi\n```\n^t-1\n";
    let err = address::resolve_task(&doc(page), None).unwrap_err();
    let AddressError::InvalidTaskName { name } = &err else {
        panic!("expected InvalidTaskName, got {err:?}");
    };
    assert_eq!(name, "[[guide#^goal@green.b3af12cd|G]]");
}

/// The refusal names what is wrong, why the name is not decoration, and what
/// to do instead.
#[test]
fn the_refusal_teaches_the_charset_and_the_reason() {
    let page = "---\ntask.[[guide#^goal@green.b3af12cd|G]]: \"[[#^t-1]]\"\n---\n";
    let err = address::resolve_task(&doc(page), None).unwrap_err();
    let msg = err.to_string();
    for owed in ["[A-Za-z0-9-]", "receipt", "run:", "rename"] {
        assert!(msg.contains(owed), "refusal must name `{owed}`: {msg}");
    }
}

/// The refusal is at `bindings`, not at `resolve_task`'s name match — so a
/// hostile name cannot hide behind `--list` either. One broken declaration must
/// refuse loudly rather than silently vanish (the crate's own law), and listing
/// it would print the forged bytes.
#[test]
fn a_hostile_name_refuses_even_when_another_task_is_the_one_addressed() {
    let page = "---\ntask.ok: \"[[#^t-1]]\"\ntask.[[a#^b@green.deadbeef|x]]: \"[[#^t-1]]\"\n---\n\n```bash\necho hi\n```\n^t-1\n";
    let d = doc(page);
    assert!(
        matches!(
            address::declared(&d),
            Err(AddressError::InvalidTaskName { .. })
        ),
        "the whole binding set refuses, so --list cannot print the forged name"
    );
    assert!(
        matches!(
            address::resolve_task(&d, Some("ok")),
            Err(AddressError::InvalidTaskName { .. })
        ),
        "addressing a sibling task does not route around the boundary"
    );
}

/// Why the guard is a CHARSET and not an `@fp` strip: each of these renders as
/// structure inside the receipt line — a second `key=value` token, a row, a
/// block anchor, a wikilink — and none is an `@fp` token a strip would catch.
#[test]
fn every_markdown_forging_name_shape_refuses_not_just_the_fp_token() {
    for hostile in [
        "fix root_after=b3", // shadows the chain detector's own input
        "fix ^r-000001",     // a forged block anchor
        "[[a]]",             // a wikilink, undecorated
        "fix.drift",         // ambiguous with the reserved-suffix grammar
        "fix_drift",         // ruling 011 deleted the `_` superset
        "fix drift",         // a token boundary
        "fix\\nrow",         // the YAML escape spelling, kept as two literal chars
    ] {
        let page = format!("---\n\"task.{hostile}\": \"[[#^t-1]]\"\n---\n");
        assert!(
            matches!(
                address::declared(&doc(&page)),
                Err(AddressError::InvalidTaskName { .. })
            ),
            "name {hostile:?} must refuse at the boundary"
        );
    }
}

/// Where the boundary is NOT the guard: a literal line ending cannot survive
/// into a frontmatter key — YAML's own grammar breaks the mapping before
/// `task.` is read, so no binding forms and there is nothing to refuse.
#[test]
fn a_literal_line_ending_never_becomes_a_task_key_at_all() {
    let page = "---\n\"task.fix\nrow\": \"[[#^t-1]]\"\n---\n";
    let all = address::declared(&doc(page)).expect("no binding forms, so none refuses");
    assert!(
        all.is_empty(),
        "a newline-bearing key does not parse as a binding: {all:?}"
    );
}

/// Every task name in the corpora is `[a-z0-9-]+` — the guard admits all of
/// them, so nothing existing breaks.
#[test]
fn every_task_name_in_the_corpora_still_resolves() {
    for name in [
        "alias",
        "bad-block",
        "bare",
        "check-links",
        "check-sh",
        "dangling",
        "dup",
        "fix",
        "fix-cheat",
        "fix-drift",
        "fix-note",
        "fix-sh",
        "fix-shim",
        "fix-uncapped",
        "fix-widen",
        "fix-x",
        "flip",
        "nudge",
        "only",
        "plain",
        "py",
        "solo",
    ] {
        let page = format!("---\ntask.{name}: \"[[#^t-1]]\"\n---\n\n```bash\necho hi\n```\n^t-1\n");
        let t = address::resolve_task(&doc(&page), Some(name))
            .unwrap_or_else(|e| panic!("corpus name {name:?} must still resolve: {e}"));
        assert_eq!(t.binding.name, name);
    }
}

/// The reserved sub-key grammar is unchanged: `.caps`/`.args`/`.env` are
/// declarations, skipped before the name guard — so a legal task with
/// declarations does not trip on its own sub-keys.
#[test]
fn reserved_sub_keys_are_skipped_before_the_name_guard() {
    let page = "---\ntask.fix-drift: \"[[#^t-1]]\"\ntask.fix-drift.caps: md.set_field:status\ntask.fix-drift.args: page\ntask.fix-drift.env: HOME_WIKI\n---\n\n```bash\necho hi\n```\n^t-1\n";
    let all = address::declared(&doc(page)).expect("declarations are not bindings");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "fix-drift");
}
