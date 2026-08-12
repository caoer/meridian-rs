//! `mrd skill hook` — the emitted document is the contract, so the document
//! is what these arms measure. The verb's whole product is a stream of bytes
//! on stdout: the stream is the artifact, the document and the engine must
//! agree, and the refusal rules that used to be installer code must be
//! legible as prose.
//!
//! Two devices keep these arms falsifiable: the engine's constants
//! (`mrd::hook`) are the expectation, never a literal; and the body is
//! executed, not read (`sh -n`, plus the real-git arms in
//! `hook_plane_fence.rs` and `u15_hook_fence_e2e.rs`, which place this
//! document's body and drive real commits through it).
//!
//! The retired fence ran `mrd check --staged`, whose worst-of exit lets a
//! worktree finding over bytes no commit would record refuse the commit;
//! `--commit-gate` gates the one interval a commit records, on the pin plane
//! alone (S4-R19).

use std::process::{Command, Output};

use mrd::hook::{FENCE_VERSION, FENCED_HOOKS, HOOK_MARKER, parse_fence_version};

/// The binary every drive goes through — the shipped CLI, never a library call.
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
    String::from_utf8(out.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// The document, as an agent receives it.
fn document() -> String {
    let out = mrd(&["skill", "hook"]);
    assert_eq!(code(&out), 0, "emitting is a success: {}", stderr(&out));
    stdout(&out)
}

/// The fence body, extracted the way the document says to: the one fenced
/// block, and it is the file. The extraction is deliberately naive — an agent
/// following the document does exactly this — so a second fenced block
/// breaks it.
fn fence_body(doc: &str) -> String {
    let mut lines = doc.lines();
    let mut body = String::new();
    let opener = lines
        .by_ref()
        .find(|l| l.starts_with("```"))
        .expect("the document carries a fenced block");
    assert_eq!(
        opener, "```sh",
        "the block the reader must place is tagged as shell"
    );
    for line in lines {
        if line.starts_with("```") {
            return body;
        }
        body.push_str(line);
        body.push('\n');
    }
    panic!("the fenced block is never closed");
}

// ── the stream is the artifact ───────────────────────────────────────────────

/// Stdout carries the document and nothing else, and stderr stays empty: the
/// arm asserts the stream equals the document rather than merely containing
/// it.
#[test]
fn the_document_is_the_whole_of_stdout_and_stderr_stays_empty() {
    let out = mrd(&["skill", "hook"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "", "stderr is not part of the product");

    let doc = stdout(&out);
    assert!(
        doc.starts_with("# MERIDIAN-HOOK.md"),
        "the stream opens ON the document, with no preamble: {:?}",
        doc.lines().next()
    );
    assert!(
        doc.ends_with("declares.\n") && !doc.ends_with("\n\n"),
        "the stream ends ON the document: no trailing summary and no added newline"
    );
    assert!(
        !doc.contains("mrd:"),
        "the engine's diagnostic prefix never reaches the artifact"
    );
}

/// The emitter touches no disk and needs no world: it runs in a bare
/// directory and leaves it holding nothing.
#[test]
fn the_emitter_writes_nothing_and_needs_neither_repository_nor_workspace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();

    let out = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(["skill", "hook"])
        .current_dir(dir)
        // No HOME and no config: the verb must not resolve a workspace, dial a
        // daemon, or read a bootstrap chain to answer.
        .env_remove("HOME")
        .env_remove("MERIDIAN_CONFIG")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("run mrd");

    assert_eq!(
        code(&out),
        0,
        "a bare directory is enough to emit: {}",
        stderr(&out)
    );
    assert!(stdout(&out).contains(HOOK_MARKER), "the document arrived");

    let left: Vec<_> = std::fs::read_dir(dir)
        .expect("read the directory back")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert!(
        left.is_empty(),
        "the emitter left something behind: {left:?}"
    );
}

// ── the document and the engine are one fact ─────────────────────────────────

/// The generation the document declares is the one `mrd::hook` judges placed
/// fences by — the arm a body change without a bump fails on.
#[test]
fn the_documents_generation_is_the_one_this_engine_judges_placed_fences_by() {
    let doc = document();
    assert_eq!(
        parse_fence_version(&doc),
        Some(FENCE_VERSION),
        "the document's `# {HOOK_MARKER} <n>` line and the engine's FENCE_VERSION are one fact"
    );
    // And the number is in the BODY, not merely in the prose around it — the reader places the
    // body, and a generation declared only in the surrounding document would land on disk as an
    // unversioned fence.
    let body = fence_body(&doc);
    assert_eq!(parse_fence_version(&body), Some(FENCE_VERSION));
    assert!(
        body.lines().nth(1).is_some_and(|l| l.contains(HOOK_MARKER)),
        "the marker rides line 2, where the engine's reader and every refusal say it is"
    );
}

/// The document names every door the engine reads, and claims exactly the
/// coverage the engine reports on.
#[test]
fn the_document_names_every_door_this_engine_reads() {
    let doc = document();
    for door in FENCED_HOOKS {
        assert!(
            doc.contains(door),
            "the document does not tell the reader to place {door}, which the engine \
             nevertheless counts when it reports coverage"
        );
    }
    assert!(
        doc.contains("git rev-parse --git-common-dir"),
        "the placement location is a claim about N worktrees sharing ONE hooks dir; \
         a document that does not say how to find it leaves the reader to guess"
    );
}

// ── the body ─────────────────────────────────────────────────────────────────

/// The extraction the document prescribes must be unambiguous: "the one
/// fenced block, and it is the file" is only an instruction if there is
/// exactly one.
#[test]
fn exactly_one_fenced_block_so_the_extraction_is_unambiguous() {
    let doc = document();
    let fences = doc.lines().filter(|l| l.starts_with("```")).count();
    assert_eq!(
        fences, 2,
        "one block means exactly two fence lines; every other command in this document \
         is inline code for precisely this reason"
    );
    let body = fence_body(&doc);
    assert!(
        body.starts_with("#!/bin/sh\n"),
        "git execs the hook: the shebang is what makes chmod+x mean anything"
    );
    assert!(
        body.contains(HOOK_MARKER),
        "a body without the marker is one the engine can never recognise as its own, \
         at any generation"
    );
}

/// The scoped question: the retired fence ran `mrd check --staged`, whose
/// worst-of exit lets a WORKTREE finding — over bytes no commit would record —
/// refuse the commit; `--commit-gate` implies `--staged` and gates the one
/// interval a commit records, on the pin plane alone, with `pins-hold` as the
/// passing word (S4-R19).
#[test]
fn the_body_asks_the_commit_question_and_not_the_whole_tree_one() {
    let body = fence_body(&document());
    assert!(
        body.contains("mrd check --commit-gate"),
        "the fence asks the commit-interval question:\n{body}"
    );
    assert!(
        !body.contains("mrd check --staged"),
        "the superseded invocation is still in the body — its worst-of exit lets \
         worktree bytes no commit would record refuse the commit"
    );
    // The exit-2 leg must name the flag it actually passes — a skew message
    // naming a different flag sends the operator to test the wrong thing.
    assert!(
        body.contains("does not carry --commit-gate"),
        "the skew teaching names the flag whose absence causes the 2 it is explaining"
    );
}

/// The extracted block is a shell program, parsed by a shell: a grep-only
/// suite would pass over an unbalanced quote, shipping a fence that refuses
/// every commit for a reason nobody can read.
#[test]
fn the_body_is_a_shell_program_that_parses() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("pre-commit");
    std::fs::write(&path, fence_body(&document())).expect("write the body");

    let out = Command::new("/bin/sh")
        .arg("-n")
        .arg(&path)
        .output()
        .expect("run sh -n");
    assert!(
        out.status.success(),
        "the body does not parse as sh: {}",
        stderr(&out)
    );
}

/// The force grammar is one law stated twice: once as the `case` the shell
/// runs, once as the table a reader acts on. The shell spells its case-fold
/// as a character-class glob, so that is what is searched for — `true` alone
/// would also match the prose.
#[test]
fn the_force_grammar_is_one_law_in_both_of_its_spellings() {
    let doc = document();
    let body = fence_body(&doc);
    for word in ["1", "true", "yes", "on", "0", "false", "no", "off"] {
        let glob: String = if word.chars().all(|c| c.is_ascii_digit()) {
            word.to_owned()
        } else {
            word.chars().fold(String::new(), |mut acc, c| {
                use std::fmt::Write as _;
                let _ = write!(acc, "[{}{c}]", c.to_ascii_uppercase());
                acc
            })
        };
        assert!(
            body.contains(&glob),
            "{word:?} is in the document's grammar table and not in the fence's `case` \
             (looked for {glob:?}) — two spellings of one law that disagree is the defect"
        );
        assert!(
            doc.contains(&format!("`{word}`")),
            "{word:?} is in the fence's `case` and not in the table the reader acts on"
        );
    }
    assert!(
        !body.contains(r#"[ -n "${MRD_HOOK_FORCE:-}" ]"#),
        "the non-emptiness test is what this grammar replaces: it opened the gate on \
         every spelling of `do not force`"
    );
    assert!(
        body.contains("set -uf"),
        "-f keeps a force value of `*` from being pathname-expanded into a file list \
         by the word-split that trims it"
    );
}

/// The specificity pair, over the bytes: the bypass leg writes to stderr, and the not-a-force
/// leg falls through carrying no `printf` of its own. A notice that fired on both would be no
/// notice at all.
#[test]
fn the_bypass_is_announced_and_only_the_bypass_is_announced() {
    let body = fence_body(&document());
    let force_leg = body
        .split("case \"$mrd_force\" in")
        .nth(1)
        .expect("the force case");
    let bypass = force_leg.split(";;").next().expect("the bypass leg");
    assert!(
        bypass.contains("BYPASSED") && bypass.contains(">&2"),
        "a forced commit that printed nothing was indistinguishable from an honest one"
    );
    let not_a_force = force_leg
        .split(";;")
        .nth(1)
        .expect("the fence-normally leg");
    assert!(
        !not_a_force.contains("printf"),
        "the fence-normally leg must stay silent, or the notice says nothing"
    );
    let unparseable = force_leg.split(";;").nth(2).expect("the third leg");
    assert!(
        unparseable.contains("exit 1"),
        "an unrecognised value refuses: the fence fails closed and does not guess"
    );
}

/// The ratified bound (silver §5): a fence that grew a selector, a rev or a
/// colour word would be a second gate — refusal's legal home is engine-side.
#[test]
fn the_body_holds_no_markdown_semantics() {
    let body = fence_body(&document());
    for forbidden in [
        "meridian-lock",
        "^inputs",
        "fingerprint",
        "blake3",
        "grey(",
        "red(",
        "frontmatter",
    ] {
        assert!(
            !body.contains(forbidden),
            "the fence is an adapter over the engine; {forbidden:?} is engine semantics"
        );
    }
}

// ── the rules that used to be code ───────────────────────────────────────────

/// Every refusal the retired installer enforced is legible here. Those were
/// guards in code and are now instructions to a reader; a missing instruction
/// is a rule that stopped existing without anyone deciding to remove it.
#[test]
fn every_refusal_the_retired_installer_enforced_is_legible_in_the_document() {
    let doc = document();
    for (rule, probe) in [
        ("submodule", "submodule"),
        ("core.hooksPath redirect", "core.hooksPath"),
        ("a foreign hook", "did not write"),
        ("a fence from a newer engine", "HIGHER generation"),
        ("an undeclarable generation", "no readable generation"),
        ("workspace below top-level", "not the worktree top-level"),
        ("not a git repository", "not a git repository"),
    ] {
        assert!(
            doc.contains(probe),
            "the document does not tell the reader to refuse on {rule}, which the retired \
             installer refused in code (looked for {probe:?})"
        );
    }
    // The inversion is the one rule whose absence is silently catastrophic:
    // every other state's remedy is "place the fence", and this one's is not.
    assert!(
        doc.contains("do not re-place"),
        "the ahead-skew's remedy inverts, and a document that does not say so sends the \
         reader to downgrade a fence"
    );
}

/// The document tells the reader how to check what they placed, and names the face that
/// answers. A contract with no verification step ends at "I wrote three files", which is not
/// the same claim as "this checkout is fenced".
#[test]
fn the_document_teaches_how_to_verify_what_was_placed() {
    let doc = document();
    assert!(
        doc.contains("mrd check"),
        "the verification face is `mrd check`'s fence line"
    );
    assert!(
        doc.contains("fence doors:"),
        "the per-door line is what makes a disagreement locatable rather than merely reported"
    );
    assert!(
        doc.contains("gates_the_exit") && doc.contains("never moves"),
        "a reader who takes the fence line for a verdict will read a fresh clone as broken"
    );
}

/// The document is the successor surface and teaches no retired verb — the same
/// law `crates/mrd/tests/retired_verbs.rs` holds USAGE to.
#[test]
fn the_document_teaches_no_retired_verb() {
    let doc = document();
    for retired in ["mrd hook install", "mrd hook uninstall", "mrd hook status"] {
        assert!(
            !doc.contains(retired),
            "the document sends its reader to a verb that no longer parses: {retired}"
        );
    }
}

// ── the invocation ───────────────────────────────────────────────────────────

/// The bad-invocation leg, over the process boundary: exit 2 and a teaching, on every shape of
/// wrong. **No exit-1 leg exists** — an emitter has no findings, and a document that printed is
/// the whole of what this verb can succeed at.
#[test]
fn every_bad_invocation_is_a_loud_exit_2_that_teaches() {
    for (args, probe) in [
        (vec!["skill"], "skill needs a name"),
        (vec!["skill", "nope"], "unknown skill: nope"),
        (vec!["skill", "hook", "extra"], "unexpected argument: extra"),
        (vec!["skill", "--nope", "hook"], "unknown flag: --nope"),
        // Every other verb in this CLI takes `--json`, so a caller WILL try it.
        // "unknown flag" would read as an oversight rather than a contract.
        (vec!["skill", "hook", "--json"], "markdown"),
    ] {
        let out = mrd(&args);
        assert_eq!(code(&out), 2, "{args:?} is a bad invocation");
        assert!(
            stderr(&out).contains(probe),
            "{args:?} refuses without teaching (looked for {probe:?}): {}",
            stderr(&out)
        );
        assert!(
            out.stdout.is_empty(),
            "{args:?} printed to stdout while refusing — a caller piping this to a file \
             would write a partial artifact"
        );
    }
}
