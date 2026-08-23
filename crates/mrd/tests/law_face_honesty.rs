//! THE FACE-HONESTY LAW — every face states the bound of its own answer.
//!
//! `docs/laws.md` § Amendment — the face-honesty law is the ruling; this file is
//! what makes it hold. A reader who proposes softening either must answer both.
//!
//! The defect this gate exists to prevent, measured 2026-08-10 by worker
//! `8cb84386` against the published binary at `27cf2bca`: `mrd links` printed 6
//! lines naming 2 files while the corpus held 112, and said nothing about the
//! 110 it withheld. **A person stops there and concludes the corpus holds 2
//! files.** The information existed; only `--json` revealed it. The renderer's
//! `continue` past every edgeless file WAS the withholding.
//!
//! All four clauses are gated here, and clause 3's second half is gated as hard
//! as its first: a refusal points at the verb the caller evidently wanted ONLY
//! when one clearly exists, because **a wrong pointer is worse than none.** Each
//! pointer test therefore carries a control that must stay silent.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod common;

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut c = common::mrd_command(&self.home, &self.cache_home);
        c.args(args)
            .current_dir(cwd)
            .env_remove("MERIDIAN_WORKSPACE");
        c
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    /// `script` takes its source on STDIN, so the budget clause needs this.
    fn run_stdin(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, stdin.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, body).expect("write");
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Files that carry an outgoing link, so the human face lists them.
const LINKED: usize = 2;
/// CONTENT files with no edges — the population the old face dropped in silence.
const CONTENT_EDGELESS: usize = 5;
/// `mrd init` writes exactly one declaration into the corpus it declares.
/// Stated here independently of the implementation constant on purpose.
const ENGINE_OWNED: usize = 1;
/// Every file in the fixture.
const TOTAL: usize = LINKED + CONTENT_EDGELESS + ENGINE_OWNED;
/// What the human face withholds. The declaration `mrd init` wrote carries no
/// edges either, so IT IS WITHHELD TOO — pinned here deliberately: the withheld
/// count is a fact about the whole enumeration, never about content alone.
const WITHHELD: usize = CONTENT_EDGELESS + ENGINE_OWNED;

/// A corpus shaped like the dogfood one in miniature: a few linked files, many
/// edgeless ones, and the engine's own declaration sitting among them.
fn corpus(sb: &Sandbox) -> PathBuf {
    let ws = sb.tmp.path().join("corpus");
    std::fs::create_dir_all(&ws).expect("mkdir");
    write(&ws, "target.md", "# Target\n\nlanding page.\n");
    for i in 0..LINKED {
        write(
            &ws,
            &format!("linked{i}.md"),
            "# Linked\n\npoints at [[target]].\n",
        );
    }
    // `target.md` is itself edgeless, so it counts in the withheld population.
    for i in 0..(CONTENT_EDGELESS - 1) {
        write(
            &ws,
            &format!("lonely{i}.md"),
            "# Lonely\n\nno links here.\n",
        );
    }
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", stderr(&init));
    ws
}

/// CLAUSE 1 — a subset answer is marked: the count withheld, the criterion, and
/// the pointer to the face that carries the rows.
///
/// This is the assertion that fails against the behaviour shipped at
/// `27cf2bca`, where the renderer skipped edgeless files and printed nothing.
#[test]
fn the_links_enumeration_states_what_it_withheld() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let out = sb.run(&ws, &["links"]);
    let said = stdout(&out);

    // Positive control FIRST: if the face listed nothing at all, every bound
    // below would pass vacuously — an absent line satisfies any claim about
    // what a line must contain.
    assert!(
        said.contains("linked0.md"),
        "the face printed no linked file, so this gate would pass vacuously — \
         the fixture is wrong, not the marking: {said}"
    );

    // The COUNT of the withheld population, and the size it was drawn from.
    assert!(
        said.contains(&format!("shown {LINKED} of {TOTAL}")),
        "a filtering face must state how many it showed OUT OF how many — the \
         bound is the whole finding, and a bare count of what was shown is the \
         defect this gate exists to prevent: {said}"
    );
    assert!(
        said.contains(&format!("{WITHHELD} with no outgoing links not listed")),
        "the withheld COUNT and its CRITERION must both appear, or the reader \
         cannot tell a filtered corpus from a small one: {said}"
    );

    // The POINTER: capping the prose must move the information to the machine
    // channel, never lose it.
    assert!(
        said.contains("`mrd links --json`"),
        "a marked subset must point at the face that enumerates, or the mark \
         announces a gap without a way to close it: {said}"
    );

    // Enumeration stays MACHINE-side: the human face must not have grown the
    // full listing, which is the walk-payload failure from the other direction.
    assert!(
        !said.contains("lonely0.md"),
        "the human face must MARK the withheld population, not enumerate it — \
         flooding this face is the failure this law also forbids: {said}"
    );
}

/// CLAUSE 4 — engine-owned files are counted AND labeled, never silently
/// either way. `mrd init` writes its declaration into the corpus, so anyone who
/// enumerates and counts otherwise gets the engine in their denominator.
#[test]
fn the_population_line_separates_content_from_engine_owned() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let said = stdout(&sb.run(&ws, &["links"]));
    let content = TOTAL - ENGINE_OWNED;

    assert!(
        said.contains(&format!(
            "{TOTAL} files: {content} content + {ENGINE_OWNED} engine-owned"
        )),
        "the count that includes the engine's own bookkeeping must say so in \
         the same breath. EXCLUDING it hides a filter and COUNTING it unlabeled \
         lets the engine pollute the content denominator; both readings stay \
         available or neither is honest: {said}"
    );
    assert!(
        said.contains("MERIDIAN.md"),
        "the label must NAME the engine-owned file, so the reader can check the \
         split rather than trust it: {said}"
    );
}

/// The mark reports a filtering that HAPPENED. A named path is served whole, so
/// the face filtered nothing and must claim no withheld population — a bound
/// that fires when nothing was withheld teaches readers to ignore bounds.
#[test]
fn a_named_path_claims_no_withheld_population() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let said = stdout(&sb.run(&ws, &["links", "linked0.md"]));

    assert!(
        said.contains("target.md"),
        "control: the named form must still answer: {said}"
    );
    assert!(
        !said.contains("not listed"),
        "a named path was served whole — claiming a withheld population here \
         reports a filtering that did not happen: {said}"
    );
}

/// CLAUSE 3 — a refusal carries its recovery when one clearly exists.
/// `mrd read .` is a caller asking to see the corpus at a door that serves one
/// page; `links --json` is the door that answers.
#[test]
fn the_read_refusal_points_at_the_verb_that_enumerates() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let out = sb.run(&ws, &["read", "."]);
    let said = stderr(&out);

    assert!(
        said.contains("is not a workspace-relative path"),
        "control: the refusal itself must still fire, unchanged — this law adds \
         a recovery, it does not soften a correct refusal: {said}"
    );
    assert!(
        said.contains("`mrd links --json`"),
        "a refusal that knows which verb the caller wanted must say so, or the \
         caller learns only that they are wrong: {said}"
    );
}

/// CLAUSE 3's SECOND HALF, gated as hard as the first: silence when no pointer
/// is clearly right. A mistyped filename wants the respelling it already gets —
/// pointing it at `links --json` would be a wrong pointer, which the law rules
/// WORSE than none.
#[test]
fn a_read_refusal_with_no_obvious_recovery_stays_silent() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let out = sb.run(&ws, &["read", "../outside.md"]);
    let said = stderr(&out);

    assert!(
        said.contains("is not a workspace-relative path"),
        "control: this must still be a path refusal, or the test proves nothing \
         about pointer restraint: {said}"
    );
    assert!(
        !said.contains("links --json"),
        "this caller wanted ONE page and mis-spelled it; enumeration is not the \
         verb they wanted. A wrong pointer is worse than none: {said}"
    );
}

/// CLAUSE 3 at the walk door, with the same pair. Since the door-family path
/// law (`wire-contract` §12.1 line 878), `walk .` refuses at the §1 admission
/// in the ONE family voice the read door refuses `.` in — six doors no longer
/// speak six refusals for one mistake — and the pointer this clause owes rides
/// that voice unchanged.
#[test]
fn the_walk_refusal_points_at_the_verb_that_enumerates() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let said = stderr(&sb.run(&ws, &["walk", ".", "--down"]));

    assert!(
        said.contains("is not a workspace-relative path"),
        "control: the refusal must still fire, in the family's §1 voice: {said}"
    );
    assert!(
        said.contains("`mrd links --json`"),
        "`walk .` is the corpus question asked at a single-page door: {said}"
    );
}

#[test]
fn a_walk_refusal_for_a_real_missing_page_stays_silent() {
    let sb = sandbox();
    let ws = corpus(&sb);

    let said = stderr(&sb.run(&ws, &["walk", "gone.md", "--down"]));

    assert!(
        said.contains("walk root not in the corpus"),
        "control: the refusal must still fire: {said}"
    );
    assert!(
        !said.contains("links --json"),
        "a genuinely missing page is not a request to enumerate — no pointer is \
         right here, so none is owed: {said}"
    );
}

/// The read budget, stated independently of the implementation constant. A test
/// that imports the value it checks passes for any value.
const EXPECTED_BUDGET: usize = 64;

/// CLAUSE 2 — a limit that can refuse is discoverable BEFORE it refuses.
///
/// The defect was never a missing help page: `mrd script --help` exists and
/// documents its flags. It was a missing SENTENCE on a page that existed.
#[test]
fn the_script_help_states_the_budget_before_it_can_refuse() {
    let sb = sandbox();
    let ws = corpus(&sb);

    // Whitespace-collapsed, because a claim about a SENTENCE must not become a
    // claim about where the description column wrapped it. Measured 2026-08-23:
    // `REFUSES, never truncates` broke across a line the moment the paragraph
    // above it changed length (card
    // `script-door-commit-premise-world-grain-vs-touch-set`), reddening a clause
    // the help still stated in full.
    let said = stdout(&sb.run(&ws, &["script", "--help"]))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        said.contains("mrd script"),
        "control: this must be the script help page: {said}"
    );
    assert!(
        said.contains(&EXPECTED_BUDGET.to_string()),
        "the budget's NUMBER must be discoverable without tripping it: {said}"
    );
    // The unit is the entire finding: measured, three files exhausted a budget
    // a reader would have called "64 files".
    assert!(
        said.contains("read() CALLS") && said.contains("NOT 64 files"),
        "the budget must carry its UNIT. `64 reads` without `calls, not files` \
         is the qualifier that was dropped in transit and it is the whole \
         finding: {said}"
    );
    assert!(
        said.contains("REFUSES, never truncates"),
        "the help must say what happens at the edge, so a caller knows partial \
         rows are never an answer: {said}"
    );
    // The guarantee names its own domain and the protocol that survives it.
    //
    // ⭐ THE RULE IS THE SAME CLAUSE; ITS WORDS CHANGED WITH THE LAW. This read
    // `EQUAL entry fingerprints` — the composition rule of the WORLD-GRAIN
    // premise: equal entry fingerprints across runs meant one snapshot, unequal
    // meant re-run. Card `script-door-commit-premise-world-grain-vs-touch-set`
    // deleted that premise, so a help text still teaching it would send a reader
    // to build retry loops around corpus churn that no longer refuses them —
    // which is the failure this clause exists to prevent, not an instance of
    // obeying it. The composition rule the help must now state is the touch
    // set's, and it is stated: each run answers for its own.
    assert!(
        said.contains("each run's commit answers for its OWN touch set"),
        "a guarantee bounded by the budget must state its composition rule, or \
         it stops holding silently above a boundary the caller cannot learn: \
         {said}"
    );
    assert!(
        !said.contains("EQUAL entry fingerprints"),
        "and it must not state the RETIRED rule beside the live one: a reader \
         cannot obey both, and the retired one is premised on a guard the engine \
         no longer applies: {said}"
    );
}

/// The help number must equal the ENFORCED number. Two surfaces agreeing is the
/// point here, so this deliberately measures the limit by tripping it rather
/// than importing `ScriptLimits`.
///
/// It also pins the STEP 1 measurement permanently: SECTION reads count against
/// the same budget, so a `--files` list far under 64 still refuses.
#[test]
fn section_reads_count_against_the_budget_the_help_advertises() {
    let sb = sandbox();
    let ws = sb.tmp.path().join("sections");
    std::fs::create_dir_all(&ws).expect("mkdir");
    let page: String = std::iter::once("# F\n".to_owned())
        .chain((1..=(EXPECTED_BUDGET + 6)).map(|i| format!("\n## S{i}\n\nbody {i}\n")))
        .collect();
    write(&ws, "f.md", &page);
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", stderr(&init));

    let script = |n: usize| {
        (1..=n)
            .map(|i| format!("x{i} = read(\"f.md\", section=\"F/S{i}\")"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // POSITIVE CONTROL, and it differs from the test in exactly ONE variable:
    // the number of read() calls. Same file, same list, same addressing.
    let under = sb.run_stdin(
        &ws,
        &["script", "--files", "f.md"],
        &script(EXPECTED_BUDGET - 4),
    );
    assert!(
        under.status.success(),
        "a run UNDER the budget must succeed, or the refusal below proves \
         nothing about the budget: {}",
        stderr(&under)
    );

    let over = sb.run_stdin(
        &ws,
        &["script", "--files", "f.md"],
        &script(EXPECTED_BUDGET + 6),
    );
    let said = stderr(&over);
    assert!(
        !over.status.success(),
        "the budget must refuse above its ceiling: {said}"
    );
    assert!(
        said.contains(&format!(
            "read budget of {EXPECTED_BUDGET} reads per attempt"
        )),
        "the ENFORCED number must be the number the help advertises, or the \
         help is a claim about a different engine: {said}"
    );
    assert!(
        said.contains("refused, never truncated"),
        "the exemplary refusal message is the standard the other faces are \
         measured against and does not change: {said}"
    );
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
