//! Purity guard for the `rulepack-api@2` change surface (U1.1) — the door-fact
//! mirror of `crates/rules/tests/purity.rs`.
//!
//! The rules-crate guard proves the effect sandbox exposes no I/O NAME. This
//! guard proves the door's *fact vocabulary* names no git/clock/random/io
//! SOURCE: a `check_change(change)` predicate reads only facts derived from the
//! before/after markdown states and one hop of pinned evidence. "yes. no git"
//! (ZT ruling, door fact surface).
//!
//! It is a load-bearing NEGATIVE: `every_allowed_fact_key_is_pure` passes on the
//! real vocabulary, and `the_guard_fails_on_an_introduced_impure_field` proves
//! the assertion actually BITES — introduce a git/clock/random/io-sourced field
//! and it fails loud, naming the key and its class.

use policy::{CHANGE_FACT_VOCAB, assert_vocab_pure, impure_source, vocab, vocab_keys};

/// The surface is exactly the closed 14-key vocabulary, and `vocab()` (the
/// `policy_vocab` op body) surfaces those same keys.
#[test]
fn vocab_is_the_closed_14_key_change_surface() {
    assert_eq!(CHANGE_FACT_VOCAB.len(), 14, "the change surface is 14 keys");
    let declared: Vec<&str> = vocab().into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        declared,
        vocab_keys(),
        "policy::vocab surfaces the vocabulary keys verbatim"
    );
}

/// The named purity assertion: EVERY allowed fact key is a pure state/evidence
/// fact — none is git-, clock-, random-, or io-sourced. This is the positive
/// half; the negative below proves it is not vacuous.
#[test]
fn every_allowed_fact_key_is_pure() {
    assert_vocab_pure(&vocab_keys())
        .expect("the rulepack-api@2 change surface is free of git/clock/random/io facts");
}

/// Load-bearing negative: introduce a git/clock/random/io-sourced field into the
/// vocabulary and the guard FAILS — naming the offending key and its impurity
/// class. Run once per class so the guard is proven to bite on all four.
#[test]
fn the_guard_fails_on_an_introduced_impure_field() {
    for (field, class) in [
        ("git_commit_sha", "git"),
        ("modified_time", "clock"),
        ("random_seed", "random"),
        ("fetched_from_disk", "io"),
    ] {
        // Sanity: the classifier attributes the field to its source.
        assert_eq!(
            impure_source(field),
            Some(class),
            "`{field}` must classify as {class}-sourced"
        );

        // The guard, run over the real vocabulary WITH the impure field spliced
        // in, refuses loud.
        let mut poisoned = vocab_keys();
        poisoned.push(field);
        let err = assert_vocab_pure(&poisoned)
            .expect_err("the guard must FAIL once an impure field enters the vocabulary");
        assert!(err.contains(field), "the refusal names the field: {err}");
        assert!(
            err.contains(&format!("({class}-sourced)")),
            "the refusal names the impurity class: {err}"
        );
    }
}

/// A pure key that merely resembles an impure one is NOT rejected — the guard is
/// exact, not a keyword panic. (`nodes`, `frontmatter`, `path`, `sections_changed`
/// live next to `now`/`date`/`search`/`git` in string space but source cleanly.)
#[test]
fn the_guard_does_not_over_reject_the_real_surface() {
    for key in [
        "path",
        "nodes",
        "frontmatter",
        "sections_changed",
        "fields_changed",
    ] {
        assert_eq!(
            impure_source(key),
            None,
            "`{key}` is a pure state fact, never flagged"
        );
    }
}
