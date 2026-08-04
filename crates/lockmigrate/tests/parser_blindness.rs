//! **THE TRIPWIRE AND THE RULE** — the parser-blindness card.
//!
//! # The finding
//! Twelve lock blocks in the field archive sit inside outer ` ```` ` fences, and
//! the parser emits no node for them. That protection HOLDS TODAY AND IS NOBODY'S
//! DECISION: it is a property of the parser's reach that happens to coincide
//! with what we want. If the reach changes for an unrelated good reason, the
//! blocks become visible and **no gate moves, because no gate is watching.**
//!
//! # What this file adds — two things, and they answer different questions
//! - **THE TRIPWIRE** (`the_parser_does_not_admit_an_enclosed_lock_fence`):
//!   fires when the parser's reach CHANGES. It does not protect anything; it
//!   makes a silent change loud.
//! - **THE RULE** (`an_enclosed_lock_fence_is_not_engine_placed`): keeps the
//!   OUTCOME the same in the world where the reach has changed. Protection by
//!   decision rather than by blindness.
//!
//! Neither replaces the other. A tripwire with no rule means you find out and
//! then have to decide under pressure; a rule with no tripwire means the world
//! changed and nobody noticed.
//!
//! # What this file deliberately does NOT do
//! It does not name the twelve, or their files, or their count. A hand-listed
//! set is the trap the card names — the predicate is a derived query over bytes,
//! so it covers blocks nobody has written yet.

/// A v1 lock block, the shape the field archive actually carries.
fn lock_block() -> String {
    "```meridian-lock\nversion: 1\nobjects:\n  \"t.md\": \"deadbeef\"\npins:\n  - ref: \"t.md#A\"\n    fingerprint: \"fp1.span2.b3.aa\"\n```"
        .to_string()
}

/// The archive shape, measured: a lock block nested inside a ` ````text ` sample.
/// Both live trace files carry exactly this — six blocks under two such fences.
fn enclosed_page() -> String {
    format!(
        "# Trace\n\nZT said:\n\n````text\nas for the pin lock:\n\n{}\n\nand the ref should be:\n````\n\n...that is the discussion.\n",
        lock_block()
    )
}

// ── THE TRIPWIRE ───────────────────────────────────────────────────────────

/// **THE PARSER DOES NOT ADMIT AN ENCLOSED LOCK FENCE — and if that ever
/// changes, this test goes RED instead of the change going unnoticed.**
///
/// This is the whole point of the card. The property was true, load-bearing, and
/// unwatched. It is now watched.
///
/// # This test can go red, and here is the proof rather than the claim
/// Un-nesting the fixture — the one-line change that simulates the parser
/// gaining reach — flips `block_spans` from 0 to 1 and fails the assertion. That
/// is asserted directly below as the CONTROL arm, so this file cannot pass by
/// measuring nothing.
#[test]
fn the_parser_does_not_admit_an_enclosed_lock_fence() {
    let raw = enclosed_page();
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let spans = lock::block_spans(&doc);

    assert_eq!(
        spans.len(),
        0,
        "THE PARSER'S REACH CHANGED. A `meridian-lock` fence nested inside a \
         longer fence is now visible to `lock::block_spans`.\n\n\
         This is not necessarily a bug — it may be a deliberate improvement. But \
         it silently changes which archive pages a lock sweep can reach, and the \
         field archive carries such blocks inside verbatim session traces and \
         ratified decision records.\n\n\
         The RULE that keeps the outcome correct in this world is \
         `an_enclosed_lock_fence_is_not_engine_placed` in this file, and \
         `lockmigrate::classify` rule 0. Confirm it still holds, then re-pin this \
         count deliberately.\n\n\
         found {} span(s)",
        spans.len()
    );

    // CONTROL, the acceptance half: the SAME block, NOT enclosed, IS visible.
    // Without this, a `block_spans` that returned empty for everything — or a
    // fixture that stopped containing a lock block at all — would pass.
    let unnested = format!("# Trace\n\n{}\n\ntrailing prose.\n", lock_block());
    let doc2 = model::build(unnested.clone(), syntax::parse(&unnested));
    assert_eq!(
        lock::block_spans(&doc2).len(),
        1,
        "the control must be visible, or this test measures nothing"
    );
}

// ── THE RULE ───────────────────────────────────────────────────────────────

/// **An enclosed lock fence is NOT-ENGINE-PLACED, by rule** — so the archive
/// stays protected in the world where the parser HAS gained reach.
///
/// Asserted through the predicate rather than through a sweep, because a sweep
/// cannot reach this state today: the parser emits no span, so the sweep sees
/// `NoLock` and the rule never runs. **The rule is deliberately unreachable
/// code today.** That is what makes it a decision instead of a description.
#[test]
fn an_enclosed_lock_fence_is_not_engine_placed() {
    let raw = enclosed_page();
    // The span the parser WOULD hand us if its reach changed: the byte offset of
    // the nested opener, found in the raw text.
    let nested_at = raw.find("```meridian-lock").expect("the fixture nests one");

    assert!(
        lockmigrate::enclosed_by_code_fence_for_test(&raw, nested_at),
        "a fence inside a ````text sample must read as ENCLOSED"
    );

    // ── The discriminator: a TOP-LEVEL block is NOT enclosed. Without this the
    // predicate could return true for everything and the rule would exclude the
    // whole corpus, which is the failure that looks like success.
    let top = format!("# Page\n\nbody\n\n{}\n", lock_block());
    let top_at = top.find("```meridian-lock").expect("present");
    assert!(
        !lockmigrate::enclosed_by_code_fence_for_test(&top, top_at),
        "a real page lock must NOT read as enclosed"
    );

    // ── THE HOLE THIS CLOSES, stated as a fixture. A page whose ONLY lock fence
    // is enclosed AND sits at the very end passes placement (terminal) and
    // arity (single) and would be MIGRATED — a code sample rewritten as if it
    // were a lock. Rule 0 is what refuses it.
    let hole = format!("# Doc\n\n````text\nexample:\n\n{}\n````\n", lock_block());
    let hole_at = hole.find("```meridian-lock").expect("present");
    assert!(
        lockmigrate::enclosed_by_code_fence_for_test(&hole, hole_at),
        "the terminal-single-nested page is the case rule 0 exists for"
    );

    // ── And a fence closed BEFORE the block does not enclose it: a page that
    // merely CONTAINS a code sample earlier on is an ordinary page.
    let after = format!(
        "# Page\n\n````text\nunrelated sample\n````\n\nbody\n\n{}\n",
        lock_block()
    );
    let after_at = after.find("```meridian-lock").expect("present");
    assert!(
        !lockmigrate::enclosed_by_code_fence_for_test(&after, after_at),
        "a CLOSED earlier fence must not swallow a later real lock"
    );
}

/// **THE CONTAINER RULE, and the accident it replaced.**
///
/// A blockquoted lock fence (`> ```meridian-lock`) IS visible to the parser —
/// the live archive carries one. It is excluded because the engine mints a lock
/// by appending at EOF, unindented and unquoted, so a real page lock's fence
/// always OPENS a line.
///
/// # Why this test exists rather than the outcome being taken on trust
/// The blockquoted shape was already coming out excluded before this rule
/// existed — **for the wrong reason.** Its span starts mid-line (after the
/// `> ` marker), and `enclosed_by_code_fence` walked up to `span_start` rather
/// than to the start of its LINE, so the block's OWN opener was counted as its
/// enclosure. Right answer, accidental mechanism: the exact defect class this
/// whole card exists to remove, reproduced inside the fix for it.
#[test]
fn a_fence_that_does_not_open_its_line_is_not_engine_placed() {
    let block = lock_block();
    let quoted: String = block
        .lines()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let raw = format!("# Doc\n\nZT wrote:\n\n{quoted}\n");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let spans = lock::block_spans(&doc);
    assert_eq!(spans.len(), 1, "a blockquoted fence IS parser-visible");
    let start = spans[0].start;

    assert!(
        !lockmigrate::fence_starts_the_line_for_test(&raw, start),
        "a blockquoted fence does not open its line"
    );
    // AND the enclosure predicate must NOT be what excludes it — that was the
    // accident. If this flips to true, the off-by-one is back.
    assert!(
        !lockmigrate::enclosed_by_code_fence_for_test(&raw, start),
        "a blockquoted fence is NOT enclosed; its own opener must not be \
         mistaken for its enclosure"
    );

    // Discriminator: a real page lock DOES open its line.
    let top = format!("# Page\n\nbody\n\n{block}\n");
    let tdoc = model::build(top.clone(), syntax::parse(&top));
    let tstart = lock::block_spans(&tdoc)[0].start;
    assert!(
        lockmigrate::fence_starts_the_line_for_test(&top, tstart),
        "a real page lock must open its line, or this rule excludes everything"
    );
}

/// The two protective layers are INDEPENDENT, and the card's premise that the
/// archive is protected *only* by parser blindness is too strong — measured.
///
/// The live traces carry six blocks each with prose after the last, so even in
/// the reach-changed world the PLACEMENT rule already excludes them. Rule 0
/// covers the case placement does not: a single enclosed block in terminal
/// position.
#[test]
fn placement_is_a_second_independent_layer() {
    // Six visible blocks + trailing prose — the trace shape, with visibility.
    let block = lock_block();
    let mut page = String::from("# Trace\n\n");
    for _ in 0..6 {
        page.push_str(&block);
        page.push_str("\n\nand then:\n\n");
    }
    page.push_str("...that is the whole discussion.\n");

    let doc = model::build(page.clone(), syntax::parse(&page));
    assert_eq!(
        lock::block_spans(&doc).len(),
        6,
        "the fixture is the visible-world trace shape"
    );
    // Nothing here is enclosed, so rule 0 does NOT fire — and the page is still
    // excluded, by placement. That is the independence claim.
    let first = page.find("```meridian-lock").expect("present");
    assert!(
        !lockmigrate::enclosed_by_code_fence_for_test(&page, first),
        "this fixture isolates PLACEMENT by carrying no enclosure"
    );
}
