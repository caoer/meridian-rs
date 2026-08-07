//! Per-verb help, over the process boundary. Every verb answers `--help` with
//! exit 0 on stdout, naming its own synopsis; a caller can tell a read from a
//! write; a refusal leads with its reason; and the surface is written for its
//! reader.

use std::process::{Command, Output};

fn mrd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .output()
        .expect("run mrd")
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// The full listing, as `mrd --help` prints it.
fn listing() -> String {
    let out = mrd(&["--help"]);
    assert_eq!(code(&out), 0, "mrd --help is a success");
    stdout(&out)
}

/// The description column, less the two-byte gutter these synopses have already
/// had stripped.
const SYNOPSIS_WIDTH: usize = 25;

/// Every verb line of the listing, read as `(writes, "mrd put <PATH> …")` — the
/// gutter becomes the flag, and the rest of the line is carried whole.
fn verb_lines(listing: &str) -> Vec<(bool, String)> {
    listing
        .lines()
        .filter_map(|line| {
            let writes = match line.get(..2)? {
                "! " => true,
                "  " => false,
                _ => return None,
            };
            line.get(2..)?
                .starts_with("mrd ")
                .then(|| (writes, line[2..].to_owned()))
        })
        .collect()
}

/// The synopsis half of a verb line: the head when the description column is
/// there, the whole line when the synopsis overflows it. Same rule the CLI
/// lexes by — without it, `mrd daemon   run the registry daemon…` reads as
/// eight verb words, and `mrd cache clean --all …` loses its flags.
fn head_of(synopsis: &str) -> &str {
    if synopsis.as_bytes().get(SYNOPSIS_WIDTH - 1) == Some(&b' ') {
        synopsis.get(..SYNOPSIS_WIDTH).unwrap_or(synopsis)
    } else {
        synopsis
    }
}

/// The words that address a verb: `mrd cache clean [--all]  reap …` -> `cache
/// clean` — read left to right until a token stops looking like a verb name.
fn address_of(synopsis: &str) -> Vec<&str> {
    let head = head_of(synopsis);
    head.strip_prefix("mrd ")
        .unwrap_or(head)
        .split_whitespace()
        .take_while(|word| word.chars().all(|c| c.is_ascii_lowercase()))
        .collect()
}

// ── 1. every verb answers ─────────────────────────────────────────────────────

/// Every verb in the listing answers `--help` with exit 0 on stdout, and the
/// page it prints names that verbs own synopsis. The verb list is read out of
/// `mrd --help` rather than typed here, so this cannot pass by being kept in
/// step with the CLI by hand.
#[test]
fn every_verb_in_the_listing_answers_its_own_help() {
    let listing = listing();
    let verbs = verb_lines(&listing);
    assert_eq!(verbs.len(), 27, "verbs in the listing:\n{listing}");

    for (_, synopsis) in &verbs {
        let address = address_of(synopsis);
        assert!(!address.is_empty(), "a verb with no name: {synopsis}");

        // Only ever `mrd <verb> --help`. The bare verb is never invoked here:
        // help is answered BEFORE dispatch, which is what lets this loop cover
        // `mrd daemon` (a foreground process) and `mrd init` (a write) without
        // running either.
        let mut args = address.clone();
        args.push("--help");
        let helped = mrd(&args);

        assert_eq!(
            code(&helped),
            0,
            "`mrd {} --help` must succeed, not refuse: {}",
            address.join(" "),
            stderr(&helped)
        );
        assert!(
            helped.stderr.is_empty(),
            "help is not a diagnostic: `mrd {} --help` wrote to stderr: {}",
            address.join(" "),
            stderr(&helped)
        );
        assert!(
            stdout(&helped).contains(synopsis),
            "`mrd {} --help` does not print its own synopsis ({synopsis}):\n{}",
            address.join(" "),
            stdout(&helped)
        );
    }
}

/// `-h` is the same door as `--help`. The options block advertises both, so a
/// caller who reads the listing and types the short one must not be refused.
#[test]
fn the_short_flag_opens_the_same_page() {
    let long = mrd(&["walk", "--help"]);
    let short = mrd(&["walk", "-h"]);
    assert_eq!(code(&short), 0, "{}", stderr(&short));
    assert_eq!(stdout(&short), stdout(&long));
}

/// A verb spelled with more than one word answers on its own, and its PARENT answers with every
/// child. `mrd cache --help` is what a caller types when they have forgotten whether it is
/// `clean` or `clear`.
#[test]
fn a_parent_verb_answers_with_all_of_its_children() {
    let page = stdout(&mrd(&["cache", "--help"]));
    assert!(page.contains("mrd cache ls"), "{page}");
    assert!(page.contains("mrd cache clean"), "{page}");

    let child = stdout(&mrd(&["cache", "clean", "--help"]));
    assert!(child.contains("mrd cache clean"), "{child}");
    assert!(
        !child.contains("mrd cache ls"),
        "the child page is the child's alone:\n{child}"
    );
}

/// `mrd test` is two tiers under one name, and `--help` owes a caller both —
/// the corpus tier and the history tier are different verbs to invoke.
#[test]
fn a_verb_with_two_tiers_prints_both() {
    let page = stdout(&mrd(&["test", "--help"]));
    assert!(page.contains("mrd test --corpus <SPEC>"), "{page}");
    assert!(page.contains("mrd test --history WORKSPACE"), "{page}");
}

/// An operand between the verb and the flag does not hide the verb: `mrd read notes.md --help`
/// is what a caller types when the invocation they just ran refused and they want the grammar.
#[test]
fn an_operand_before_the_flag_still_resolves_the_verb() {
    let out = mrd(&["read", "some-page.md", "--help"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("mrd read <PATH>"), "{}", stdout(&out));
}

/// The page carries only the options its own verb owns. `--json` is real for most verbs but is
/// NOT offered under `mrd skill hook`, whose description says in its own words that no such
/// face exists — a help page that advertised it would be teaching a lie.
#[test]
fn a_page_carries_its_own_options_and_no_false_promises() {
    let run = stdout(&mrd(&["run", "--help"]));
    assert!(run.contains("--env KEY=VALUE"), "run owns --env:\n{run}");
    assert!(run.contains("--list"), "run owns --list:\n{run}");
    assert!(
        !run.contains("--rule PAGE"),
        "--rule belongs to test, not run:\n{run}"
    );

    // The claim is what the page OFFERS, not the letters on it: this verbs own
    // description says "There is no --json face", so only the options block can
    // promise the flag. The probed span must not cross the listing's line wrap.
    let hook = stdout(&mrd(&["skill", "hook", "--help"]));
    assert!(
        hook.contains("--json face — the document is markdown"),
        "the description is reprinted verbatim:\n{hook}"
    );
    assert!(
        !hook.contains("options:"),
        "this verb owns no options and its synopsis offers none, so it must \
         advertise nothing:\n{hook}"
    );

    let test = stdout(&mrd(&["test", "--help"]));
    assert!(test.contains("--history"), "{test}");
    assert!(test.contains("--rule PAGE"), "{test}");
    assert!(test.contains("--spec PAGE"), "{test}");
}

/// If a verb's synopsis offers a flag, and the options block has an entry for
/// that flag, that entry appears on the verb's page.
#[test]
fn a_flag_offered_in_a_synopsis_is_explained_beneath_it() {
    let listing = listing();
    // The flags the options block actually defines; a flag it never documents
    // (`--force`, `--vibe`) is explained in prose and is not this test's claim.
    let documented: Vec<&str> = listing
        .lines()
        .skip_while(|line| !line.starts_with("options:"))
        .filter(|line| line.starts_with("  -"))
        .flat_map(|line| line.split_whitespace().take(2))
        .map(|token| token.trim_matches(','))
        .filter(|token| token.starts_with('-') && token.len() > 1)
        .collect();
    assert!(
        documented.contains(&"--dry") && documented.contains(&"--json"),
        "the options block still documents the flags this test is about: {documented:?}"
    );

    for (_, synopsis) in verb_lines(&listing) {
        let address = address_of(&synopsis);
        let mut args = address.clone();
        args.push("--help");
        let page = stdout(&mrd(&args));

        // Only the synopsis half of the line offers flags — a description that
        // MENTIONS `--json` is not the same as a verb that takes it.
        for offered in head_of(&synopsis)
            .split_whitespace()
            .map(|token| token.trim_matches(|c: char| matches!(c, '[' | ']' | '|')))
            .filter(|token| documented.contains(token))
        {
            assert!(
                page.contains("options:"),
                "`mrd {} --help` offers {offered} and has no options block:\n{page}",
                address.join(" ")
            );
            let options = page.split("options:").nth(1).unwrap_or_default();
            assert!(
                options.contains(offered),
                "`mrd {} --help` offers {offered} in its synopsis and never explains it:\n{page}",
                address.join(" ")
            );
        }
    }
}

/// The `--dry` case by name, so the regression is legible without deriving it.
#[test]
fn dry_is_explained_under_every_verb_that_takes_it() {
    for verb in [
        vec!["put"],
        vec!["pin"],
        vec!["new"],
        vec!["unfold"],
        vec!["reconcile"],
        vec!["realise"],
        vec!["run"],
    ] {
        let mut args = verb.clone();
        args.push("--help");
        let page = stdout(&mrd(&args));
        let options = page.split("options:").nth(1).unwrap_or_default();
        assert!(
            options.contains("--dry"),
            "`mrd {} --help` takes --dry and does not explain it:\n{page}",
            verb.join(" ")
        );
    }
}

/// Help is a page, not a fragment: it opens with the title, explains the gutter
/// mark it uses, and closes by naming where the rest of the verbs are.
#[test]
fn a_page_is_self_contained() {
    let page = stdout(&mrd(&["status", "--help"]));
    assert!(
        page.starts_with("mrd — the meridian workspace CLI\n"),
        "{page}"
    );
    assert!(page.contains("in the gutter marks a verb"), "{page}");
    assert!(page.contains("usage:"), "{page}");
    assert!(
        page.trim_end()
            .ends_with("see `mrd --help` for every verb."),
        "{page}"
    );
}

/// A `--` separator hands the rest of the line to the task, so `mrd run PAGE TASK -- --help`
/// must reach the run plane rather than printing this CLIs help. Whatever the run refuses, it
/// is not a help page on stdout.
#[test]
fn a_separator_passes_the_flag_through() {
    let out = mrd(&["run", "no-such-page.md", "task", "--", "--help"]);
    assert!(
        !stdout(&out).contains("see `mrd --help` for every verb."),
        "the CLI answered a flag that belonged to the task:\n{}",
        stdout(&out)
    );
}

// ── 2. a caller can tell a read from a write ──────────────────────────────────

/// The write mark is part of the listing, so it travels into the per-verb page
/// with the line: a caller who asks about one verb learns whether it writes
/// without reading the whole listing.
#[test]
fn the_write_mark_travels_into_the_verb_page() {
    let writer = stdout(&mrd(&["put", "--help"]));
    assert!(
        writer.contains("! mrd put <PATH>"),
        "a write verb is marked on its own page:\n{writer}"
    );

    let reader = stdout(&mrd(&["walk", "--help"]));
    assert!(
        reader.contains("  mrd walk <PAGE>"),
        "a read verb is unmarked on its own page:\n{reader}"
    );
}

/// The classification itself, pinned: fourteen verbs change files or the
/// drawer, the other thirteen are reads. `retire mark` writes because it sweeps
/// `~~` markers across the vault's markdown, and `--dry` exempts it no more
/// than it exempts `pin`, `realise`, `reconcile`, `new`, `unfold` or `script`;
/// `mrd test` is unmarked because it writes only into temporary directories.
///
/// The count is in the test NAME on purpose — a classification whose total can
/// drift silently is one nobody reviews.
#[test]
fn the_write_classification_is_fourteen_of_twenty_seven() {
    let listing = listing();
    let (writers, readers): (Vec<_>, Vec<_>) = verb_lines(&listing)
        .into_iter()
        .partition(|(writes, _)| *writes);

    let named = |verbs: &[(bool, String)]| -> Vec<String> {
        verbs
            .iter()
            .map(|(_, synopsis)| address_of(synopsis).join(" "))
            .collect()
    };

    assert_eq!(
        named(&writers),
        [
            "init",
            "unregister",
            "put",
            "pin",
            "repair",
            "retire",
            "cache clean",
            "daemon",
            "run",
            "script",
            "new",
            "unfold",
            "reconcile",
            "realise",
        ],
        "the verbs marked as writers"
    );
    assert_eq!(
        readers.len(),
        13,
        "the rest are reads: {:?}",
        named(&readers)
    );
    assert_eq!(writers.len() + readers.len(), 27, "the whole surface");
}

/// `mrd test` writes only into temporary directories, and `mrd sql` queries an
/// ephemeral in-memory projection — both stand unmarked.
#[test]
fn the_tempdir_and_read_only_verbs_are_not_marked() {
    let listing = listing();
    for unmarked in ["  mrd test --corpus", "  mrd sql <query>", "  mrd check"] {
        assert!(
            listing.contains(unmarked),
            "{unmarked} must stand unmarked:\n{listing}"
        );
    }
}

// ── 3. a refusal leads with its reason ────────────────────────────────────────

/// The diagnostic is the FIRST line of stderr, not the last under a screenful
/// of help.
#[test]
fn an_unknown_subcommand_puts_its_error_first() {
    let out = mrd(&["nope"]);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert_eq!(
        err.lines().next(),
        Some("mrd: unknown subcommand: nope"),
        "the reason must lead:\n{err}"
    );
    assert!(
        err.contains("mrd init [PATH]"),
        "and the listing still follows it:\n{err}"
    );
}

/// Every refusal that answers with the whole surface leads the same way — a
/// missing subcommand, and a bad subcommand of a two-word verb.
#[test]
fn every_usage_refusal_leads_with_its_reason() {
    for (args, reason) in [
        (vec![], "mrd: no subcommand given"),
        (vec!["cache"], "mrd: cache needs a subcommand (ls | clean)"),
        (vec!["cache", "nope"], "mrd: unknown cache subcommand: nope"),
    ] {
        let out = mrd(&args);
        assert_eq!(code(&out), 2, "{args:?}");
        let err = stderr(&out);
        assert_eq!(err.lines().next(), Some(reason), "{args:?}:\n{err}");
        assert!(
            err.lines().count() > 100,
            "{args:?} still teaches the surface beneath the reason:\n{err}"
        );
    }
}

/// A verb this CLI does not have gets a refusal, not a help page — `--help` is
/// not a way to make an unknown name succeed.
#[test]
fn an_unknown_verb_is_refused_even_with_the_flag() {
    let out = mrd(&["nope", "--help"]);
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty(), "{}", stdout(&out));
    assert_eq!(
        stderr(&out).lines().next(),
        Some("mrd: unknown subcommand: nope")
    );
}

/// `mrd --help`, `mrd -h` and `mrd help` remain the whole surface on stdout,
/// exit 0 — the per-verb pages are an addition to that door, not a replacement.
#[test]
fn the_bare_help_verb_still_prints_everything() {
    let full = listing();
    assert!(full.contains("mrd init [PATH]"), "{full}");
    assert!(full.contains("! mrd realise <PAGE>"), "{full}");
    assert!(full.contains("options:"), "{full}");
    for door in [vec!["-h"], vec!["help"]] {
        let out = mrd(&door);
        assert_eq!(code(&out), 0, "{door:?}");
        assert_eq!(stdout(&out), full, "{door:?} is the same door");
    }
}

// ── 4. the surface is written for its reader ──────────────────────────────────

/// No internal unit tag (`U5.3`) reaches help output — a docket id reads to an
/// outside caller as a version, a flag, or an error code. The claim is derived
/// rather than listed: every page the CLI can print is scanned for the shape, so
/// a tag added to a verb written next year fails here without this file being
/// edited.
#[test]
fn no_internal_unit_tag_reaches_a_help_page() {
    /// The dockets tag shape: `U`, a digit, a dot, then an alphanumeric — `U5.3`,
    /// `U3.5b`, `U2.11`. Scanned by hand; a regex crate is a dependency this
    /// workspace does not take for one predicate.
    fn unit_tags(text: &str) -> Vec<String> {
        let bytes: Vec<char> = text.chars().collect();
        let mut found = Vec::new();
        for (i, ch) in bytes.iter().enumerate() {
            if *ch != 'U' {
                continue;
            }
            // A `U` that continues a word (`REDUCE`) is not a tag opener.
            if i > 0 && bytes[i - 1].is_alphanumeric() {
                continue;
            }
            let tag: String = bytes[i..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == '.')
                .collect();
            let mut parts = tag[1..].split('.');
            let (Some(major), Some(minor)) = (parts.next(), parts.next()) else {
                continue;
            };
            if !major.is_empty()
                && major.chars().all(|c| c.is_ascii_digit())
                && !minor.is_empty()
                && minor.starts_with(|c: char| c.is_ascii_digit())
            {
                found.push(tag);
            }
        }
        found
    }

    // Positive control: a broken predicate would report a clean surface forever.
    assert_eq!(
        unit_tags("file birth (U5.3): resolve the def"),
        vec!["U5.3".to_owned()],
        "the scanner must detect the tag it exists to detect"
    );
    assert_eq!(
        unit_tags("the reconciliation loop (U3.5b; ZT ruling #3)"),
        vec!["U3.5b".to_owned()]
    );
    assert!(
        unit_tags("a REDUCE.2 word and a bare U and version 2.1").is_empty(),
        "the scanner does not fire on ordinary prose"
    );

    let full = listing();
    assert!(
        unit_tags(&full).is_empty(),
        "internal unit tags in `mrd --help`: {:?}",
        unit_tags(&full)
    );

    // Every per-verb page too.
    for (_, synopsis) in verb_lines(&full) {
        let address = address_of(&synopsis);
        let mut args = address.clone();
        args.push("--help");
        let page = stdout(&mrd(&args));
        assert!(
            unit_tags(&page).is_empty(),
            "internal unit tags in `mrd {} --help`: {:?}",
            address.join(" "),
            unit_tags(&page)
        );
    }
}
