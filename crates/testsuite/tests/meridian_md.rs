//! The `MERIDIAN.md` in-file schema pack, replayed (stage 3, U6 over U2's pack).
//!
//! `crates/testsuite/data/meridian-md/cases.json` pairs **every** case with its
//! required outcome, and this module drives all 37 through `config::resolve` —
//! the same entry point the CLI calls, not a parse-only shortcut, so the four
//! resolution states of schema §2.2 are exercised as states rather than as
//! functions.
//!
//! # Why this is a MODULE and not `tests/meridian_md.rs`
//! `crates/testsuite/Cargo.toml` sets `autotests = false`. A stray
//! `tests/meridian_md.rs` is never compiled and never runs — a green board that
//! measured nothing. The registration in `tests/main.rs`, the
//! `testsuite::meridian_md_dir()` accessor, and [`case_count_is_pinned`]'s
//! cardinality assert are the three things that make this replay non-vacuous;
//! the pack's README names all three.
//!
//! # The sandbox
//! Every case runs in its own `tempfile::tempdir()`. `$SANDBOX` in the pack
//! resolves to it and `$HOME` to `$SANDBOX/home`; nothing outside is read or
//! written, and no case touches the real `HOME` or the real `MERIDIAN_CONFIG`
//! (the chain is driven through `config::Env`, which is data — see that type's
//! own note on why it is injected rather than read globally).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Fixtures on disk, per the pack README: 7 acceptances + 23 refusals.
const FIXTURES_ON_DISK: usize = 30;
/// Cases in the manifest: the 30 fixtures plus 7 that cannot have a file.
const CASE_COUNT: usize = 37;
/// The acceptance / refusal split the pack states.
const ACCEPTANCES: usize = 11;
const REFUSALS: usize = 26;
/// Acceptance fixtures under `corpus/`; the remaining 23 on disk are refusals.
const ACCEPTANCE_FIXTURES: usize = 7;
/// Cases carrying `expect_not` — the ones where the wrong behaviour still looks
/// healthy, so the refusal alone is not evidence: `env-path-missing` (a silent
/// fallback LOADS the default file), `no-frontmatter` and `unsupported-version`
/// (each carries a valid-under-v1 mount block).
const EXPECT_NOT_CASES: usize = 3;

fn pack() -> PathBuf {
    testsuite::meridian_md_dir()
}

fn manifest() -> Value {
    let path = pack().join("cases.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("cases.json is JSON")
}

fn cases(manifest: &Value) -> &Vec<Value> {
    manifest["cases"].as_array().expect("cases is an array")
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// The cardinality gate. Directory-scan discovery plus a pinned count is what
/// stops a silently-dropped fixture from turning this replay green over less
/// than it was written to cover — the same discipline `gt_parse.rs` pins its
/// ten GT files with.
#[test]
fn case_count_is_pinned() {
    let manifest = manifest();
    let cases = cases(&manifest);
    assert_eq!(
        cases.len(),
        CASE_COUNT,
        "the pack is 37 cases; a case was added or dropped without this gate moving"
    );

    let accepts = cases
        .iter()
        .filter(|c| text(&c["expect"]["outcome"]) == "accept")
        .count();
    let refuses = cases
        .iter()
        .filter(|c| text(&c["expect"]["outcome"]) == "refuse")
        .count();
    assert_eq!(accepts, ACCEPTANCES, "acceptances");
    assert_eq!(refuses, REFUSALS, "refusals");
    assert_eq!(
        accepts + refuses,
        CASE_COUNT,
        "every case states an outcome; `outcome` is not optional"
    );

    // Every case's id is unique — two cases sharing an id would silently make
    // one of them unreachable through any id-keyed report.
    let mut ids: Vec<&str> = cases.iter().map(|c| text(&c["id"])).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "case ids are unique");
}

/// The pairing gate: 0 orphans (a fixture with no case is untestable) and 0
/// dangling references (a case naming a file that is not there).
#[test]
fn every_fixture_is_paired_and_every_reference_resolves() {
    let manifest = manifest();
    let cases = cases(&manifest);

    let mut referenced: Vec<PathBuf> = Vec::new();
    for case in cases {
        for key in ["fixture", "env"] {
            let named: Vec<&str> = match key {
                "fixture" => case["fixture"].as_str().into_iter().collect(),
                _ => ["default_file", "override_file"]
                    .iter()
                    .filter_map(|k| case["env"][*k].as_str())
                    .collect(),
            };
            for rel in named {
                let path = pack().join(rel);
                assert!(
                    path.is_file(),
                    "case `{}` names {rel}, which is not a file in the pack",
                    text(&case["id"])
                );
                if !referenced.contains(&path) {
                    referenced.push(path);
                }
            }
        }
    }

    let mut on_disk: Vec<PathBuf> = Vec::new();
    for sub in ["corpus", "refusals"] {
        let dir = pack().join(sub);
        for entry in std::fs::read_dir(&dir).expect("pack subdirectory") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_some_and(|e| e == "md") {
                on_disk.push(path);
            }
        }
    }
    assert_eq!(
        on_disk.len(),
        FIXTURES_ON_DISK,
        "the pack ships 30 fixture files"
    );
    for path in &on_disk {
        assert!(
            referenced.contains(path),
            "orphan fixture: {} is on disk and no case names it, so it is untestable",
            path.display()
        );
    }
}

/// The replay: all 37 cases, each asserting its REQUIRED OUTCOME — the parsed
/// mounts and tools for an acceptance, the reason word, the file line and what
/// the message must name for a refusal, and `expect_not` where the wrong
/// behaviour still looks healthy.
#[test]
fn every_case_holds() {
    let manifest = manifest();
    let mut checked = 0usize;
    for case in cases(&manifest) {
        run_case(case);
        checked += 1;
    }
    assert_eq!(
        checked, CASE_COUNT,
        "every case ran; a `continue` that skipped one would otherwise pass silently"
    );
}

/// One case, end to end: scaffold the sandbox its `env` describes, resolve the
/// chain, and hold the result against `expect` / `expect_not`.
fn run_case(case: &Value) {
    let id = text(&case["id"]);
    let sandbox = tempfile::tempdir().unwrap_or_else(|e| panic!("{id}: tempdir: {e}"));
    let root = sandbox.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{id}: mkdir home: {e}"));
    // Sandbox scaffolding the pack's `$SANDBOX` placeholder implies: a real
    // directory for `env-path-directory` to aim at. `$SANDBOX/nowhere/` is
    // never created — that is what `env-path-missing` measures.
    std::fs::create_dir_all(root.join("a-directory"))
        .unwrap_or_else(|e| panic!("{id}: mkdir a-directory: {e}"));

    let subst = |s: &str| {
        s.replace("$SANDBOX", &root.display().to_string())
            .replace("$HOME", &home.display().to_string())
    };

    // A case WITH a fixture puts that fixture at the default path — the file
    // under test IS the fixture. `default_file` / `override_file` exist for the
    // seven cases that cannot have a fixture, where the point is which of two
    // files the chain picks.
    let env = &case["env"];
    if let Some(rel) = case["fixture"].as_str() {
        place(&pack().join(rel), &home.join(config::CONFIG_FILENAME), id);
    }
    if let Some(rel) = env["default_file"].as_str() {
        place(&pack().join(rel), &home.join(config::CONFIG_FILENAME), id);
    }

    let meridian_config = match &env["MERIDIAN_CONFIG"] {
        Value::Null => None, // absent or explicitly null: unset, either way
        v => Some(subst(text(v))),
    };
    if let (Some(rel), Some(target)) = (env["override_file"].as_str(), meridian_config.as_deref()) {
        place(&pack().join(rel), Path::new(target), id);
    }

    // `HOME` present with an explicit null is unset; absent means the sandbox's.
    let home_value = if env.get("HOME").is_some_and(Value::is_null) {
        None
    } else {
        Some(home.display().to_string())
    };

    let resolved = config::resolve(&config::Env {
        meridian_config,
        home: home_value,
    });

    let expect = &case["expect"];
    match text(&expect["outcome"]) {
        "accept" => check_accept(id, expect, &resolved),
        "refuse" => check_refuse(id, case, &subst, &resolved),
        other => panic!("{id}: unknown outcome `{other}`"),
    }
}

fn place(from: &Path, to: &Path, id: &str) {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{id}: mkdir: {e}"));
    }
    std::fs::copy(from, to)
        .unwrap_or_else(|e| panic!("{id}: copy {} -> {}: {e}", from.display(), to.display()));
}

fn check_accept(
    id: &str,
    expect: &Value,
    resolved: &Result<config::Resolution, config::ConfigError>,
) {
    let resolution = match resolved {
        Ok(r) => r,
        Err(e) => panic!("{id}: must ACCEPT, refused with `{}`: {e}", e.reason.word()),
    };

    match text(&expect["state"]) {
        "A" => assert!(
            matches!(resolution, config::Resolution::Absent { .. }),
            "{id}: state A is the absent file, not a loaded empty one"
        ),
        "D" => {
            assert!(
                matches!(resolution, config::Resolution::Loaded(_)),
                "{id}: state D parsed a file"
            );
            assert!(
                resolution.mounts().is_empty(),
                "{id}: state D declares zero mounts"
            );
            assert!(
                resolution.file_rev().is_some(),
                "{id}: a parsed file carries its rev — the ONE permitted difference from state A"
            );
        }
        "loaded" => {
            assert!(
                matches!(resolution, config::Resolution::Loaded(_)),
                "{id}: a file was expected to load"
            );
        }
        other => panic!("{id}: unknown accept state `{other}`"),
    }

    if let Some(expected) = expect["mounts"].as_array() {
        assert_eq!(
            resolution.mounts().len(),
            expected.len(),
            "{id}: mount count — document order is the contract, so a missing or extra block is a failure here"
        );
        for (i, want) in expected.iter().enumerate() {
            let got = &resolution.mounts()[i];
            match want {
                // The short form: the canonical name only.
                Value::String(name) => assert_eq!(&got.name, name, "{id}: mount {i} name"),
                // The long form: every field, asserted.
                Value::Object(_) => {
                    assert_eq!(got.name, text(&want["name"]), "{id}: mount {i} name");
                    assert_eq!(got.path, text(&want["path"]), "{id}: mount {i} path");
                    assert_eq!(
                        got.kind.as_str(),
                        text(&want["kind"]),
                        "{id}: mount {i} kind"
                    );
                    assert_eq!(
                        got.vault.as_deref(),
                        want["vault"].as_str(),
                        "{id}: mount {i} vault"
                    );
                    assert_eq!(
                        got.pin.as_deref(),
                        want["pin"].as_str(),
                        "{id}: mount {i} pin — carried VERBATIM, never normalized or re-minted"
                    );
                }
                other => {
                    panic!("{id}: mount {i} expectation is neither a name nor a record: {other}")
                }
            }
        }
    }
    if let Some(count) = expect["mount_count"].as_u64() {
        assert_eq!(
            resolution.mounts().len(),
            usize::try_from(count).expect("a fixture count fits"),
            "{id}: the mount COUNT is asserted, not just membership — a decoy that also loads passes membership"
        );
    }
    if let Some(expected) = expect["tools"].as_array() {
        assert_eq!(resolution.tools().len(), expected.len(), "{id}: tool count");
        for (i, want) in expected.iter().enumerate() {
            let got = &resolution.tools()[i];
            assert_eq!(got.name, text(&want["name"]), "{id}: tool {i} name");
            assert_eq!(got.kind, text(&want["kind"]), "{id}: tool {i} kind");
            assert_eq!(
                got.config.as_deref(),
                want["config"].as_str(),
                "{id}: tool {i} payload — carried BYTE-VERBATIM, indentation included, never interpreted"
            );
        }
    }
}

fn check_refuse(
    id: &str,
    case: &Value,
    subst: &dyn Fn(&str) -> String,
    resolved: &Result<config::Resolution, config::ConfigError>,
) {
    let expect = &case["expect"];
    let err = match resolved {
        Err(e) => e,
        Ok(r) => panic!(
            "{id}: must REFUSE, accepted with {} mount(s)",
            r.mounts().len()
        ),
    };

    assert_eq!(
        err.reason.word(),
        text(&expect["reason"]),
        "{id}: reason word"
    );
    assert_eq!(
        err.line,
        expect["line"]
            .as_u64()
            .map(|n| usize::try_from(n).expect("a fixture line number fits")),
        "{id}: the refusal's 1-based FILE line — a reason word alone does not satisfy \"and where\""
    );

    // Each state's own discriminator, so B and C cannot be satisfied by each
    // other's reason word.
    match expect["state"].as_str() {
        Some("B") => assert!(
            !matches!(
                err.reason,
                config::Reason::ConfigPathUnusable | config::Reason::HomeUnresolvable
            ),
            "{id}: state B is a MALFORMED file, not an unresolvable path"
        ),
        Some("C") => assert_eq!(
            err.reason,
            config::Reason::ConfigPathUnusable,
            "{id}: state C is the stated-path-unusable class"
        ),
        _ => {}
    }

    // The refusal TEACHES: it names what is broken, and §8.3's three mandatory
    // clauses are present in every one.
    let message = err.to_string();
    for name in expect["names"].as_array().into_iter().flatten() {
        let wanted = subst(text(name));
        assert!(
            message.contains(&wanted),
            "{id}: the refusal must name `{wanted}`, got: {message}"
        );
    }
    assert!(
        message.contains(config::NO_PARTIAL_LOAD_CLAUSE),
        "{id}: the refusal must state that nothing loaded: {message}"
    );
    assert!(
        message.contains(" Fix: "),
        "{id}: the refusal must teach the fix: {message}"
    );
    if let Some(line) = err.line {
        assert!(
            message.contains(&format!(" line {line}:")),
            "{id}: the line is in the message a human reads, not only in the struct: {message}"
        );
    }

    // `expect_not` — where the wrong behaviour still looks healthy. A build
    // that silently falls back, or half-loads, publishes these names.
    for name in case["expect_not"]["mounts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        assert!(
            resolved.is_err(),
            "{id}: a refusal publishes no mount table at all"
        );
        assert!(
            !message.contains(&format!("mount `{}` loaded", text(name))),
            "{id}: nothing about `{}` may read as loaded",
            text(name)
        );
    }
}

/// The `expect_not` cases carry a perfectly valid mount block, so the wrong
/// behaviour LOOKS healthy. This asserts the absence directly: for every such
/// case, the refusal's own type carries no mount table — there is no `Ok` value
/// to read one out of, and `config::Config` has private fields with `parse` as
/// its only constructor, so a partially-populated one cannot be constructed at
/// all.
#[test]
fn no_refusal_can_publish_a_partial_mount_table() {
    let manifest = manifest();
    let mut with_expect_not = 0usize;
    let mut reparsed = 0usize;

    for case in cases(&manifest) {
        let id = text(&case["id"]);
        if let Some(names) = case["expect_not"]["mounts"].as_array() {
            assert!(
                !names.is_empty(),
                "{id}: an empty expect_not asserts nothing"
            );
            with_expect_not += 1;
        }
        // EVERY refusal fixture, not only the `expect_not` ones. The three
        // `expect_not` cases all fault before any block is read, so a build that
        // half-loads at BLOCK level satisfies them and is caught only here —
        // measured: mutating `parse_mount`'s `?` into a `continue` left the
        // expect_not-only form of this test green.
        if text(&case["expect"]["outcome"]) != "refuse" {
            continue;
        }
        let Some(rel) = case["fixture"].as_str() else {
            continue; // the env cases have no file; `every_case_holds` drives them
        };
        let path = pack().join(rel);
        let raw = std::fs::read_to_string(&path).expect("fixture");
        let result = config::parse(&raw, &path);
        assert!(
            result.is_err(),
            "{id}: must refuse, and a refusal yields NO Config — so no caller can observe a \
             partial mount table, whatever the fixture declared before the fault"
        );
        assert!(result.ok().is_none(), "{id}: publishes nothing");
        reparsed += 1;
    }

    assert_eq!(
        with_expect_not, EXPECT_NOT_CASES,
        "the pack's expect_not cases are the ones where the WRONG behaviour still looks healthy; \
         dropping one would remove a guard without reddening anything else"
    );
    assert_eq!(
        reparsed,
        FIXTURES_ON_DISK - ACCEPTANCE_FIXTURES,
        "every refusal FIXTURE is re-parsed here, not a subset"
    );
}

/// **U36 INVERTED this test, and the inversion is the proof.**
///
/// As U6 wrote it, this test asserted the DEFECT: the render face elided every
/// `meridian-*` block (predicate `lock::is_meridian_lang`), so a config's
/// human-authored mount blocks vanished and the rendered section was
/// **byte-identical** to rendering the file with those blocks physically
/// deleted. That byte-identity WAS the missing marker (S3-R14(a)) — a config
/// file rendering as a config file that declares nothing, with no signal.
///
/// U36 made elision **per-language**, derived from *"does the ENGINE EMIT this
/// block's bytes?"* (`lock::is_engine_emitted`). `meridian-mount` has no
/// canonical writer — the user authors it and `config` only parses it — so it is
/// no longer elided, and the defect this test pinned is gone. What the test now
/// asserts is the inversion, point for point:
///
/// 1. the rendered section carries **all three** declared roots;
/// 2. it is **NOT** byte-identical to rendering the file with those blocks
///    deleted — the two differ, which is what "the reader can tell" means. No
///    marker bytes were invented to achieve it (M1's *"no invented placeholder
///    bytes"* stands): the authored bytes themselves are the difference;
/// 3. S10's `NOTICE:` mechanism still does not fire — nothing needed it;
/// 4. the raw face still carries the blocks VERBATIM and the config plane still
///    parses all three — the acceptance half, unchanged, without which (1) is
///    satisfied by a build that renders the file uninterpreted.
///
/// The original defect was reproduced first-hand on the installed binary before
/// it was written down: `mrd read MERIDIAN.md --section '…/Roots'` printed the
/// prose, dropped all three blocks, and exited 0. The pre-U36 assertions are
/// preserved verbatim in the U36 card's redden evidence.
#[test]
fn the_render_face_shows_config_blocks_and_differs_from_blocks_deleted() {
    use render::{Header, RenderJob, Renderer, SectionRow, ToonRenderer};
    use wire_map::facts::read_facts;

    let raw = std::fs::read_to_string(pack().join("corpus/multi-root.md")).expect("fixture");
    // The same file with every engine block removed: what a config that
    // declares nothing looks like.
    let stripped = strip_engine_blocks(&raw);
    assert!(
        !stripped.contains("meridian-mount"),
        "the control really has no engine block"
    );

    let roots_content = |source: &str| -> String {
        let doc = model::build(source.to_string(), syntax::parse(source));
        let facts = read_facts(&wire_map::project_toc(&doc), doc.raw.as_bytes());
        let fact = facts
            .iter()
            .find(|f| f.hpath.last().is_some_and(|s| s.h == "Roots"))
            .expect("the Roots section resolves");
        let rows = [SectionRow {
            sel: &wire::ReadSel::parse("Roots"),
            fact,
        }];
        let rendered = ToonRenderer::with_meridian_elision()
            .render(
                &doc,
                &RenderJob::Sections {
                    header: Header {
                        display_path: "~/MERIDIAN.md",
                        file_rev: &doc.root.node_rev.0,
                        words_total: 0,
                        decorations: &render::NO_DECORATIONS,
                    },
                    rows: &rows,
                    notice: None,
                },
            )
            .expect("renders");
        rendered.sections[0].content.clone()
    };

    let rendered = roots_content(&raw);
    for root in ["field-notes", "sessions", "archive"] {
        assert!(
            rendered.contains(root),
            "the render face must show the declared root `{root}` the USER authored: {rendered}"
        );
    }
    assert!(
        rendered.contains("The wiki: domains, decisions, effects"),
        "the prose beside each block survives: {rendered}"
    );
    assert!(
        !rendered.contains("NOTICE"),
        "no marker bytes were invented — the authored bytes ARE the signal: {rendered}"
    );
    assert_ne!(
        rendered,
        roots_content(&stripped),
        "a config that declares three roots must NOT render identically to one that \
         declares nothing — this is the defect U6 pinned, now inverted"
    );

    // The acceptance half (S3-R8(c)): the bytes ARE there, and the config plane
    // publishes them. A build that rendered nothing at all would satisfy every
    // assertion above and ship a broken engine.
    assert!(
        raw.contains("```meridian-mount"),
        "the raw face is verbatim"
    );
    let config = config::parse(&raw, Path::new("~/MERIDIAN.md")).expect("the config parses");
    assert_eq!(
        config
            .mounts()
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["field-notes", "sessions", "archive"],
        "all three roots still parse — the render face now shows exactly the content \
         that loads, and the two faces agree"
    );
}

/// Remove every fenced `meridian-*` block, block by block — the control for the
/// byte-identity assertion above.
fn strip_engine_blocks(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut inside = false;
    for line in raw.lines() {
        if inside {
            if line.trim_end() == "```" {
                inside = false;
            }
            continue;
        }
        if let Some(info) = line.strip_prefix("```")
            && lock::is_meridian_lang(info)
        {
            inside = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The pack's own `kills` discipline, enforced: a case that cannot say what
/// wrong implementation it rules out is probably vacuous, and every case here
/// says it.
#[test]
fn every_case_states_its_law_and_what_it_kills() {
    let manifest = manifest();
    for case in cases(&manifest) {
        let id = text(&case["id"]);
        assert!(
            !text(&case["law"]).is_empty(),
            "{id}: names the law it pins"
        );
        assert!(
            !text(&case["kills"]).is_empty(),
            "{id}: states what wrong implementation it rules out"
        );
    }
}
