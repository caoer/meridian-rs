//! U2 — the `put()` builtin: pure arming of wire `splice.plan_edits[]` items,
//! the single-CONTENT-path law, and the armed-edit ceiling.
//!
//! Authority: `docs/run-plane.md` § The script entry ("One CONTENT path per
//! commit (v1 law)", "Recorded-read purity") and
//! `decisions/2026-08-07-script-put-builtin-edit-grammar.md` § RULING (B′) —
//! `put()` speaks the wire's second dialect, `splice.plan_edits[]`, so no third
//! edit grammar is minted. The bare-`append=` target detail is settled at
//! `decisions/2026-08-07-script-bare-append-target.md`.

use std::collections::BTreeMap;

use effects::{
    ArmedEdit, EvalError, ReadFault, ScriptCtx, ScriptHost, ScriptLimits, SecFacts, TocEntry,
    TocFacts, eval_script,
};
use wire::{HpathSeg, PlanEdit, ReadSel};

/// A host that serves two pages and counts its calls — enough to prove `put()`
/// consulted nothing.
struct ArmHost {
    calls: usize,
}

impl ArmHost {
    fn new() -> Self {
        Self { calls: 0 }
    }
}

impl ScriptHost for ArmHost {
    fn toc(&mut self, path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        self.calls += 1;
        let mut fm = BTreeMap::new();
        fm.insert("owner".to_owned(), String::new());
        fm.insert("status".to_owned(), "todo".to_owned());
        Ok(TocFacts {
            rev: format!("rev-{path}"),
            fm,
            toc: vec![TocEntry {
                section: "Notes".to_owned(),
                anchor: None,
                rev: "sec-notes".to_owned(),
                hpath: vec![HpathSeg {
                    h: "Notes".to_owned(),
                    n: None,
                }],
            }],
            words: 41,
        })
    }

    fn cat(
        &mut self,
        path: &str,
        section: &ReadSel,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        self.calls += 1;
        Ok(SecFacts {
            text: format!("{path}#{}\n", section.display()),
            rev: "sec-notes".to_owned(),
        })
    }

    fn actor(&self) -> &'static str {
        "8ab41c02"
    }
}

/// A host that panics on every seam: `put()` reaching it is a test failure that
/// cannot be missed (acceptance 4).
struct PanicHost;

impl ScriptHost for PanicHost {
    fn toc(&mut self, _path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        panic!("put() must perform zero I/O — the host was consulted");
    }

    fn cat(
        &mut self,
        _path: &str,
        _section: &ReadSel,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        panic!("put() must perform zero I/O — the host was consulted");
    }

    fn actor(&self) -> &'static str {
        "8ab41c02"
    }
}

fn run(script: &str) -> effects::ScriptEval {
    let ctx = ScriptCtx {
        id: "s1".to_owned(),
        args: BTreeMap::new(),
        files: vec!["tasks/0011.md".to_owned(), "tasks/0012.md".to_owned()],
        effects: Vec::new(),
    };
    eval_script(script, &ctx, ScriptLimits::default(), &mut ArmHost::new())
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.to_owned(),
        n: None,
    }
}

// ---------------------------------------------------------------------------
// Purity — acceptance 4
// ---------------------------------------------------------------------------

#[test]
fn put_performs_no_host_call() {
    let ctx = ScriptCtx {
        id: "s1".to_owned(),
        args: BTreeMap::new(),
        files: vec!["tasks/0011.md".to_owned()],
        effects: Vec::new(),
    };
    let eval = eval_script(
        r#"
put("tasks/0011.md", props={"owner": "8ab41c02"})
put("tasks/0011.md", section="Notes", append="- a line\n")
"#,
        &ctx,
        ScriptLimits::default(),
        &mut PanicHost,
    );
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(eval.armed.len(), 2);
    assert_eq!(eval.telemetry.reads_used, 0);
}

// ---------------------------------------------------------------------------
// The armed shapes are wire plan edits, carried verbatim — RULING (B′)
// ---------------------------------------------------------------------------

#[test]
fn props_arm_one_set_property_per_key_sorted() {
    let eval = run(r#"put("tasks/0011.md", props={"status": "doing", "owner": "8ab41c02"})"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    let edits: Vec<&PlanEdit> = eval.armed.iter().map(|a| &a.edit).collect();
    assert_eq!(
        edits,
        vec![
            &PlanEdit::SetProperty {
                key: "owner".to_owned(),
                value: "8ab41c02".to_owned(),
                rev: None,
            },
            &PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "doing".to_owned(),
                rev: None,
            },
        ],
        "one item per key, keys sorted — the MCP put face's own order \
         (putregistry.go lowerPropPlans)"
    );
}

#[test]
fn append_arms_a_section_addressed_plan_append() {
    let eval = run(r#"put("tasks/0011.md", section="Notes", append="- a line\n")"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(
        eval.armed[0].edit,
        PlanEdit::Append {
            hpath: vec![seg("Notes")],
            body: "- a line\n".to_owned(),
            rev: None,
        }
    );
}

#[test]
fn a_nested_section_address_is_segments_not_a_joined_string() {
    let eval = run(r#"put("tasks/0011.md", section="Notes/Fresh", append="x\n")"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(
        eval.armed[0].edit,
        PlanEdit::Append {
            hpath: vec![seg("Notes"), seg("Fresh")],
            body: "x\n".to_owned(),
            rev: None,
        },
        "§2.1 addresses are segments — a joined sanitized string is host debt"
    );
}

/// **One address grammar, one parser.** `section=` is parsed by
/// `ReadSel::parse` on both faces, so the `^anchor` a toc row PUBLISHES — and
/// `read(path, section="^id")` accepts — is a real address here too, and it is
/// refused as one. A raw `split('/')` coerced it into a heading literally named
/// `^r-000118`: an address no document has, armed silently, refused by the wire
/// as `NotFound` with the caller told nothing about why.
#[test]
fn a_block_anchor_section_refuses_as_an_address_never_as_a_heading_named_caret() {
    let eval = run(r#"put("tasks/0011.md", section="^r-000118", append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("^r-000118") && reason.contains("BLOCK"),
        "the refusal names the address and what KIND it is: {reason}"
    );
    assert!(
        reason.contains("read("),
        "and it names the face the address DOES work on: {reason}"
    );
    assert!(eval.armed.is_empty(), "a refusal arms nothing");
}

/// The other spelling `ReadSel::parse` decides and an `append` has no target
/// for: a dewey ordinal addresses a row of a table the caller is holding, and
/// does not survive an edit. It refuses in its own words rather than arming a
/// heading literally named `1.2`.
#[test]
fn a_dewey_section_refuses_as_a_positional_address() {
    let eval = run(r#"put("tasks/0011.md", section="1.2", append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("1.2") && reason.contains("positional"),
        "the refusal names the address and why it cannot be written to: {reason}"
    );
    assert!(eval.armed.is_empty(), "a refusal arms nothing");
}

#[test]
fn a_bare_append_refuses_naming_the_missing_section() {
    // decisions/2026-08-07-script-bare-append-target.md: an empty hpath refuses
    // NotFound in both dialects, so a document-grain append has no wire target.
    let eval = run(r#"put("tasks/0011.md", append="- a line\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("section"),
        "the refusal must name the missing address: {reason}"
    );
    assert!(eval.armed.is_empty(), "a refusal arms nothing");
}

#[test]
fn a_put_with_no_edit_kwarg_refuses() {
    let eval = run(r#"put("tasks/0011.md")"#);
    assert!(
        matches!(eval.outcome, Err(EvalError::Runtime { .. })),
        "a put that arms nothing is a caller error, not a silent no-op: {:?}",
        eval.outcome
    );
}

// ---------------------------------------------------------------------------
// The armed set spans files — the §4.4 set form's input (run-plane.md
// § One COMMIT per attempt; the arm-time multi_file_write_set law is retired)
// ---------------------------------------------------------------------------

/// Replay applies recorded § A.7 expansions to the ORIGINAL `files[]`: the
/// pattern member substitutes its recorded match list, so the replayed
/// binding is byte-identical to the live attempt's — the purity law with
/// patterns included.
#[test]
fn replay_substitutes_recorded_expansions() {
    let live_ctx = ScriptCtx {
        id: "script".to_owned(),
        args: BTreeMap::new(),
        // The EXPANDED list, as the host bound it live.
        files: vec!["tasks/a.md".to_owned(), "tasks/b.md".to_owned()],
        effects: Vec::new(),
    };
    // No reads in the script, so an empty recording serves as the live host.
    let empty = effects::ScriptRecording::default();
    let mut host = effects::RecordedHost::new(&empty);
    let mut eval = eval_script(
        "n = len(files)\nfirst = files[0]\n",
        &live_ctx,
        ScriptLimits::default(),
        &mut host,
    );
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    // The host stamps the entry fact after eval, as both lanes do.
    eval.recording.expansions = vec![effects::ExpansionRecord {
        pattern: "tasks/*.md".to_owned(),
        matched: vec!["tasks/a.md".to_owned(), "tasks/b.md".to_owned()],
    }];

    // Replay with the ORIGINAL member list — the pattern, unexpanded.
    let replay_ctx = ScriptCtx {
        id: "script".to_owned(),
        args: BTreeMap::new(),
        files: vec!["tasks/*.md".to_owned()],
        effects: Vec::new(),
    };
    let replayed = effects::replay_script(
        "n = len(files)\nfirst = files[0]\n",
        &replay_ctx,
        ScriptLimits::default(),
        &eval.recording,
    );
    let live = eval.outcome.expect("live ok");
    let back = replayed.outcome.expect("replay ok");
    assert_eq!(
        live.bindings, back.bindings,
        "replayed bindings are byte-identical (n=2, first=tasks/a.md)"
    );
}

#[test]
fn a_second_content_path_arms_a_set() {
    let eval = run(r#"
put("tasks/0011.md", props={"owner": ""})
put("tasks/0012.md", props={"owner": ""})
"#);
    assert!(
        eval.outcome.is_ok(),
        "two content paths arm one set — the commit is the §4.4 set form, \
         not an arm-time refusal: {:?}",
        eval.outcome
    );
    assert_eq!(eval.armed.len(), 2, "both arms stand");
    assert_eq!(
        eval.content_paths(),
        vec!["tasks/0011.md".to_string(), "tasks/0012.md".to_string()],
        "content_paths lists the members in first-arm order"
    );
}

#[test]
fn many_edits_to_one_path_do_not_refuse() {
    let eval = run(r#"
put("tasks/0011.md", props={"owner": "8ab41c02", "status": "doing"})
put("tasks/0011.md", section="Notes", append="- claimed\n")
"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(eval.armed.len(), 3, "2 props + 1 append = 3 plan items");
    assert_eq!(eval.content_paths(), vec!["tasks/0011.md"]);
}

#[test]
fn the_receipt_companion_is_not_an_armed_content_path() {
    // §6.4: a receipt rides the splice request's own `receipt:{path,anchor}`
    // field, appended in the SAME batch commit (§6.1 — one fingerprint advance
    // covering both files). It is never a `put()` target, so the law over armed
    // paths cannot forbid the two-file receipt commit.
    let eval = run(r#"put("tasks/0011.md", props={"owner": "8ab41c02"})"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(eval.content_paths(), vec!["tasks/0011.md"]);
    assert!(
        eval.armed.iter().all(|a| !a.path.starts_with("receipts/")),
        "the script mints no receipt — the engine does, outside the armed list"
    );
}

// ---------------------------------------------------------------------------
// The armed ceiling — acceptance 3
// ---------------------------------------------------------------------------

#[test]
fn the_sixty_fifth_armed_edit_refuses_without_truncating() {
    let eval = run(r#"
for i in range(65):
    put("tasks/0011.md", section="Notes", append="- line " + str(i) + "\n")
"#);
    let Err(EvalError::ArmedBudget { limit, .. }) = &eval.outcome else {
        panic!(
            "expected an armed-edit budget refusal, got {:?}",
            eval.outcome
        );
    };
    assert_eq!(*limit, 64);
    let text = eval.outcome.as_ref().unwrap_err().to_string();
    assert!(
        text.contains("→") && text.contains("slice the targets"),
        "the refusal carries its fitted recovery — chunk at the ceiling: {text}"
    );
    assert_eq!(
        eval.armed.len(),
        64,
        "the ceiling refuses the 65th — it never truncates the 64 that landed"
    );
}

#[test]
fn exactly_the_ceiling_is_legal() {
    let eval = run(r#"
for i in range(64):
    put("tasks/0011.md", section="Notes", append="- line " + str(i) + "\n")
"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(eval.armed.len(), 64);
}

// ---------------------------------------------------------------------------
// Arm order, line, depth — acceptance 5
// ---------------------------------------------------------------------------

#[test]
fn arm_order_is_source_execution_order() {
    let eval = run(r#"
for n in ["b", "a", "c"]:
    put("tasks/0011.md", section="Notes", append=n)
"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    let bodies: Vec<String> = eval
        .armed
        .iter()
        .map(|a| match &a.edit {
            PlanEdit::Append { body, .. } => body.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(bodies, vec!["b", "a", "c"], "execution order, never sorted");
}

#[test]
fn every_armed_edit_records_its_source_line_and_nesting_depth() {
    let eval = run(r#"
def claim(path):
    put(path, props={"owner": "8ab41c02"})

put("tasks/0011.md", section="Notes", append="- top level\n")
claim("tasks/0011.md")
"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(eval.armed[0].line, 5);
    assert_eq!(eval.armed[0].depth, 0, "a top-level arm is depth 0");
    assert_eq!(eval.armed[1].line, 3);
    assert_eq!(
        eval.armed[1].depth, 1,
        "an arm inside a def is depth 1 — recorded, never suppressed"
    );
}

#[test]
fn a_read_and_an_arm_interleave_in_one_script() {
    let eval = run(r#"
card = read("tasks/0011.md")
if card["fm"]["owner"] == "":
    put("tasks/0011.md", props={"owner": me(), "status": "doing"})
"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(eval.telemetry.reads_used, 1);
    assert_eq!(eval.armed.len(), 2, "golden scenario 1: 2 edits, one file");
    assert_eq!(
        eval.armed[0].edit,
        PlanEdit::SetProperty {
            key: "owner".to_owned(),
            value: "8ab41c02".to_owned(),
            rev: None,
        },
        "me() threads the host's actor into the armed value"
    );
}

// ---------------------------------------------------------------------------
// The §2.1 segment form on `section=` — D-1's machine escape (dogfood r7 F1)
// ---------------------------------------------------------------------------

/// The joined coat splits on `/` (D-1, coat never widened), so a heading whose
/// raw text carries one is reachable only by the machine form. `section=` takes
/// the wire's own hpath array — one `{h, n?}` entry per heading, `/` as heading
/// TEXT — which is the exact form the engine's section-miss refusal teaches.
#[test]
fn a_segment_list_arms_one_segment_per_entry_with_slash_as_heading_text() {
    let eval =
        run(r#"put("tasks/0011.md", section=[{"h": "r7-c"}, {"h": "A/B split"}], append="x\n")"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(
        eval.armed[0].edit,
        PlanEdit::Append {
            hpath: vec![seg("r7-c"), seg("A/B split")],
            body: "x\n".to_owned(),
            rev: None,
        },
        "one array entry per heading, no joining, no splitting"
    );
}

/// The occurrence index rides the structured form only (`ReadSel::parse` doc):
/// a segment's `n` pins the k-th same-text sibling, exactly as the wire spells
/// it.
#[test]
fn a_segment_carries_its_occurrence() {
    let eval = run(
        r#"put("tasks/0011.md", section=[{"h": "Log"}, {"h": "Entry", "n": 2}], append="x\n")"#,
    );
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(
        eval.armed[0].edit,
        PlanEdit::Append {
            hpath: vec![
                seg("Log"),
                HpathSeg {
                    h: "Entry".to_owned(),
                    n: Some(2),
                },
            ],
            body: "x\n".to_owned(),
            rev: None,
        }
    );
}

/// A bare string inside the list is the retired v1 spelling, refused with the
/// wire's own single-sourced text naming the offending value (v2 §2.1) — the
/// same refusal the wire door answers, so one law speaks one sentence.
#[test]
fn a_bare_string_inside_a_segment_list_refuses_with_the_wire_refusal() {
    let eval = run(r#"put("tasks/0011.md", section=["A/B split"], append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains(wire::HPATH_SEG_V1_REFUSAL) && reason.contains("A/B split"),
        "the wire's own refusal, offending value named: {reason}"
    );
    assert!(eval.armed.is_empty(), "a refusal arms nothing");
}

/// An empty list addresses nothing — refused loud, never treated as an absent
/// kwarg: the caller passed a list, so the answer speaks the list's grammar.
#[test]
fn an_empty_segment_list_refuses_as_addressing_nothing() {
    let eval = run(r#"put("tasks/0011.md", section=[], append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("section=[]") && reason.contains("{\"h\": …}"),
        "the refusal names the empty list and the segment shape: {reason}"
    );
    assert!(eval.armed.is_empty());
}

/// Refuse what can never exist (D-1's line): occurrences are 1-based, so
/// `n: 0` is outside the minting grammar — `bad_request`-class, never a miss.
#[test]
fn a_zero_occurrence_refuses_as_outside_the_grammar() {
    let eval = run(r#"put("tasks/0011.md", section=[{"h": "A", "n": 0}], append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("1-based"),
        "the refusal names the 1-based law: {reason}"
    );
    assert!(eval.armed.is_empty());
}

/// A key the segment shape does not carry refuses loud — a typo'd `"N"` must
/// not silently drop the occurrence it meant to pin.
#[test]
fn an_unknown_segment_key_refuses_loud() {
    let eval = run(r#"put("tasks/0011.md", section=[{"h": "A", "N": 2}], append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("`N`") && reason.contains("{h, n?}"),
        "the unknown key and the lawful shape are both named: {reason}"
    );
    assert!(eval.armed.is_empty());
}

/// An empty heading text addresses nothing any document carries — refused at
/// the boundary, never armed to miss at the wire.
#[test]
fn an_empty_heading_text_in_a_segment_refuses() {
    let eval = run(r#"put("tasks/0011.md", section=[{"h": ""}], append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("empty heading text"),
        "the refusal names the empty text: {reason}"
    );
    assert!(eval.armed.is_empty());
}

/// A `section=` that is neither a string nor a list refuses naming BOTH
/// accepted forms — the refusal may not teach less than the plane accepts.
#[test]
fn a_non_string_non_list_section_refuses_naming_both_forms() {
    let eval = run(r#"put("tasks/0011.md", section=42, append="x\n")"#);
    let Err(EvalError::Runtime { reason, .. }) = &eval.outcome else {
        panic!("expected a runtime refusal, got {:?}", eval.outcome);
    };
    assert!(
        reason.contains("joined heading path") && reason.contains("{h, n?}"),
        "both accepted forms named: {reason}"
    );
    assert!(eval.armed.is_empty());
}

/// The string coat is UNTOUCHED (C2 stays reserved): the joined spelling still
/// splits on `/`, and the two-segment reading of "A/B" is still what arms —
/// the characterization that keeps D-1's boundary observable from this plane.
#[test]
fn the_string_coat_still_splits_on_slash() {
    let eval = run(r#"put("tasks/0011.md", section="A/B", append="x\n")"#);
    assert!(eval.outcome.is_ok(), "{:?}", eval.outcome);
    assert_eq!(
        eval.armed[0].edit,
        PlanEdit::Append {
            hpath: vec![seg("A"), seg("B")],
            body: "x\n".to_owned(),
            rev: None,
        },
        "the coat is not widened — the machine form is the escape, not an \
         escape grammar inside the string"
    );
}
