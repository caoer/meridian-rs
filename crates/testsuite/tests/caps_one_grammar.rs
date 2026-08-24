//! ONE `caps:` grammar across BOTH planes — the policy plane's HOOK loader
//! (`crates/policy/src/hook.rs`) and the run plane's capability resolver
//! (`crates/run/src/caps.rs`).
//!
//! The two planes read the SAME frontmatter key with DIFFERENT vocabularies —
//! the policy plane names descriptor kinds (`proto.send`), the run plane names
//! verbs optionally scoped by a path glob (`md.edit:tasks/*.md`). What they may
//! never differ on is the SPELLING layer: which YAML shapes of `caps:` count as
//! a declaration, and what tokens each shape yields. They did differ, in
//! opposite directions, and each direction was measured in production:
//!
//! * flow sequence (`caps: [md.create, md.edit]`) — accepted by the policy
//!   plane's `serde_yaml` sequence, FAULTED `invalid capability '[md.create'`
//!   at the run plane, whose `CapSet::parse` trimmed quotes off the whole
//!   string and never brackets (card
//!   `caps-flow-sequence-faults-at-run-plane`, pair-6 gate run).
//! * plain scalar (`caps: md.create`) — accepted by the run plane, REFUSED by
//!   the policy plane's `Option<Vec<String>>` as `invalid type: string`
//!   (sibling card `born-card-caps-never-exercised-by-the-birth-we-score`,
//!   13:40 run on engine `f3b586ae`).
//! * block sequence — accepted by the policy plane, read as the EMPTY STRING at
//!   the run plane, because `YamlMap` keeps only the key LINE's remainder. That
//!   one is worse than a fault: an explicit read-only grant silently replaces
//!   the caps the page declares. Same defect class as card
//!   `fm-block-list-sql-empty`.
//!
//! So this gate is written spelling-first: one table of shapes, both planes,
//! and the assertion is that the shapes agree with each other AND with the
//! shared tokenizer (`model::parse_caps_list`). It deliberately proves the
//! parse BY TEST rather than by observing a live pair behave — the current
//! daemon/engine pair is CLEAN by an unisolated change (engine switch
//! `f3b586ae` → `dffb229ca` and a fleet-page re-land moved together between the
//! faulting run and the clean one, and no instrument on hand separates them,
//! per `dafb738f`). A green observation on today's pair is evidence that
//! today's pair happens to agree, never that the contract is fixed.

use model::Document;
use policy::{CheckLimits, PageRef, ScopeLayer, register_page};
use run::caps::{Cap, CapSet, CapsError};

/// Render the `caps:` frontmatter lines for a list of cap names, in one YAML
/// spelling.
type Render = fn(&[&str]) -> String;

/// One spelling under test: its name, for the assertion messages, and its
/// renderer.
type Spelling = (&'static str, Render);

/// The four spellings every `caps:`-shaped key must read identically.
///
/// `caps: []` and a bare `caps:` are separate cases below — they are about
/// DECLARED-EMPTY vs NOT-DECLARED, not about how a non-empty list is spelled.
const SPELLINGS: [Spelling; 4] = [
    ("plain scalar", |names| {
        format!("caps: {}\n", names.join(", "))
    }),
    ("flow sequence", |names| {
        format!("caps: [{}]\n", names.join(", "))
    }),
    ("flow sequence, quoted items", |names| {
        let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
        format!("caps: [{}]\n", quoted.join(", "))
    }),
    ("block sequence", |names| {
        let mut out = String::from("caps:\n");
        for n in names {
            out.push_str("  - ");
            out.push_str(n);
            out.push('\n');
        }
        out
    }),
];

// ---------------------------------------------------------------- policy plane

/// A `rules/hook` page carrying `caps_block` verbatim. `budget:`/`how:` are
/// always present so a refusal can only be about the caps.
///
/// `predicate` is the predicate BODY: a page declaring caps must reach for the
/// constructors they grant, and a page declaring none must reach for nothing —
/// the load-time capability ceiling (`check_ceiling`) refuses otherwise, and
/// that refusal would be about the predicate, not about the spelling this gate
/// is measuring.
fn hook_page(caps_block: &str, predicate: &str) -> String {
    format!(
        "---\n\
         tags: [type/rule, rules/hook]\n\
         id: caps.spelling-probe\n\
         severity: info\n\
         paths: [\"tasks/*.md\"]\n\
         {caps_block}\
         budget: {{ steps: 10000, mem: 4194304 }}\n\
         how:\n  \
         route:    {{ info: channel-review }}\n  \
         batching: 30s\n\
         ---\n\
         \n\
         # caps spelling probe\n\
         \n\
         ```starlark\n\
         def on_change(event):\n    \
         {predicate}\n\
         ```\n"
    )
}

/// The predicate a `proto.send`-granting page carries.
const SENDS: &str = "send(to = [\"reviewer\"], message = \"probe\")";
/// The predicate a page granting NOTHING carries.
const EMITS_NOTHING: &str = "pass";

/// Load a hook page through the PUBLIC seam (`register_page` → `load_rule`),
/// answering with the declared caps as wire names or the refusal's own text.
fn policy_caps(caps_block: &str) -> Result<Vec<String>, String> {
    policy_caps_with(caps_block, SENDS)
}

fn policy_caps_with(caps_block: &str, predicate: &str) -> Result<Vec<String>, String> {
    let md = hook_page(caps_block, predicate);
    let registration = register_page(PageRef {
        layer: ScopeLayer::Workspace,
        page: "rules/caps-spelling-probe.md",
        bytes: &md,
    })
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "the probe page did not register as a rule page".to_string())?;

    let rule =
        policy::load_rule(&registration, &md, CheckLimits::default()).map_err(|e| e.to_string())?;
    let hook = rule
        .hook()
        .ok_or_else(|| "the probe page loaded without a hook leg".to_string())?;
    Ok(hook.caps().iter().map(|k| k.as_str().to_string()).collect())
}

// ------------------------------------------------------------------- run plane

/// A declaring page whose page-level `caps:` the run plane reads.
fn run_page(caps_block: &str) -> String {
    format!(
        "---\n\
         type: hooks\n\
         {caps_block}\
         ---\n\
         \n\
         # run-plane caps spelling probe\n"
    )
}

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The run plane's page-level caps, as canonical cap strings, or the refusal.
fn run_caps(caps_block: &str) -> Result<Option<Vec<String>>, CapsError> {
    let raw = run_page(caps_block);
    let document = doc(&raw);
    Ok(run::caps::page_caps(&document)?
        .map(|set| set.0.iter().map(Cap::as_string).collect::<Vec<_>>()))
}

// ------------------------------------------------------------------- the gates

/// **The both-planes spelling regression.** Every spelling of a non-empty list
/// yields the SAME caps on the plane that owns the vocabulary, and the two
/// planes accept the same set of spellings.
#[test]
fn every_spelling_reads_identically_on_both_planes() {
    // Policy-plane vocabulary: slice 1 admits `proto.send`.
    let policy_expected = vec!["proto.send".to_string()];
    // Run-plane vocabulary: the three verbs.
    let run_expected = vec!["md.create".to_string(), "md.edit".to_string()];

    for (name, render) in SPELLINGS {
        let policy_got = policy_caps(&render(&["proto.send"]))
            .unwrap_or_else(|e| panic!("policy plane refused the {name} spelling: {e}"));
        assert_eq!(
            policy_got, policy_expected,
            "policy plane read the {name} spelling as {policy_got:?}"
        );

        let run_got = run_caps(&render(&["md.create", "md.edit"]))
            .unwrap_or_else(|e| panic!("run plane refused the {name} spelling: {e}"))
            .unwrap_or_else(|| panic!("run plane read the {name} spelling as UNDECLARED"));
        assert_eq!(
            run_got, run_expected,
            "run plane read the {name} spelling as {run_got:?}"
        );
    }
}

/// Every spelling splits into the same TOKENS through the one shared splitter —
/// the function both planes call, so agreement above cannot be a coincidence of
/// two parsers that happen to agree today.
#[test]
fn the_shared_splitter_reads_every_spelling_identically() {
    let names = ["md.create", "md.edit:tasks/*.md"];
    let expected: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    for (spelling, render) in SPELLINGS {
        // `render` produces `caps: <value>` lines; the splitter takes the VALUE,
        // which is what `model::fm_value` serves for every one of these shapes.
        let block = format!("---\n{}---\n", render(&names));
        let value = model::fm_value(&block, "caps")
            .unwrap_or_else(|| panic!("`caps:` unreadable in the {spelling} spelling"));
        assert_eq!(
            model::parse_caps_list(&value),
            expected,
            "the {spelling} spelling tokenized differently"
        );
    }
}

/// A cap name that is not in the plane's vocabulary must arrive at the
/// vocabulary check INTACT — the refusal naming `md.create`, never
/// `[md.create` or `"md.create"`. This is what proves the spelling layer ran
/// before the vocabulary layer on every shape.
#[test]
fn a_rejected_cap_is_named_without_its_spelling() {
    for (spelling, render) in SPELLINGS {
        let err = policy_caps(&render(&["proto.send", "md.create"]))
            .expect_err("`md.create` is outside slice 1 — the load must refuse");
        assert!(
            err.contains("`md.create`"),
            "the {spelling} spelling refused with {err:?}, which does not name `md.create` intact"
        );

        let err = run_caps(&render(&["md.create", "proto.send"]))
            .expect_err("`proto.send` is not a run-plane verb — the parse must refuse");
        let text = err.to_string();
        assert!(
            text.contains("'proto.send'"),
            "the {spelling} spelling refused with {text:?}, which does not name `proto.send` intact"
        );
    }
}

/// `caps: []` is a DECLARED empty grant on both planes — explicit read-only,
/// distinct from not declaring the key at all.
#[test]
fn a_declared_empty_list_is_explicit_read_only_on_both_planes() {
    assert_eq!(
        policy_caps_with("caps: []\n", EMITS_NOTHING).expect("`caps: []` loads"),
        Vec::<String>::new()
    );
    assert_eq!(
        run_caps("caps: []\n").expect("`caps: []` parses"),
        Some(Vec::<String>::new()),
        "the run plane must read `caps: []` as a DECLARED empty grant"
    );
}

/// An absent `caps:` is not a declaration on either plane: the policy plane
/// refuses loudly, the run plane answers `None` (deny-by-default). A bare
/// `caps:` with nothing after it is the same statement — the engine never
/// invents `[]` for it (`model::fm_value`).
#[test]
fn an_undeclared_caps_key_is_not_an_empty_grant_on_either_plane() {
    let err = policy_caps("").expect_err("a page with no `caps:` must refuse");
    assert!(
        err.contains("caps"),
        "the refusal does not name the missing key: {err:?}"
    );
    assert_eq!(
        run_caps("").expect("no `caps:` parses"),
        None,
        "no `caps:` is deny-by-default, never an explicit empty grant"
    );

    let err = policy_caps("caps:\n").expect_err("a bare `caps:` declares nothing");
    assert!(
        err.contains("caps"),
        "the refusal does not name the bare key: {err:?}"
    );
    assert_eq!(
        run_caps("caps:\n").expect("a bare `caps:` parses"),
        None,
        "a bare `caps:` is not a declaration — the engine never invents `[]`"
    );
}

/// The run plane's scoped form survives every spelling: a glob-scoped verb is
/// one token, brackets and quotes stripped around it and never inside it.
#[test]
fn a_glob_scoped_cap_survives_every_spelling() {
    let expected = vec!["md.edit:agents/*/CARD.md".to_string()];
    for (spelling, render) in SPELLINGS {
        let got = run_caps(&render(&["md.edit:agents/*/CARD.md"]))
            .unwrap_or_else(|e| panic!("run plane refused the scoped cap in {spelling}: {e}"))
            .unwrap_or_else(|| panic!("run plane read the scoped cap in {spelling} as UNDECLARED"));
        assert_eq!(got, expected, "the {spelling} spelling mangled the scope");
    }
}

/// `CapSet::parse` — the public string door — accepts every spelling too. It is
/// the entry the convention table (`run.caps.<pattern>`) and `--list` reach
/// through, so a shape the page plane accepts and this door refuses would be
/// the same split by another name.
#[test]
fn the_public_capset_door_accepts_every_spelling() {
    let expected = CapSet::parse("md.create md.edit").expect("baseline parses");
    for raw in [
        "md.create, md.edit",
        "md.create md.edit",
        "[md.create, md.edit]",
        "[\"md.create\", \"md.edit\"]",
        "\"md.create, md.edit\"",
    ] {
        let got = CapSet::parse(raw).unwrap_or_else(|e| panic!("`{raw}` refused: {e}"));
        assert_eq!(got, expected, "`{raw}` parsed to a different set");
    }
}
