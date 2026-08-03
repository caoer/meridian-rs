//! The `mrd` CLI: wire the workspace / cache / registry foundation into the
//! settled verb surface.
//!
//! # Charter
//! **Owns:** hand-rolled subcommand parsing (no clap — dependency discipline),
//! the per-invocation resolution flow ([`resolve`]), the client side of the
//! resident-daemon inversion ([`engine`]: auto-spawn the daemon on first use,
//! degrade to an in-process ephemeral engine — decision 0002 §3), and the verbs
//! `init`, `unregister`, `resolve`, `links`, `cache ls`, `cache clean`, and
//! `daemon`. Output is the house grammar: a human table by default, JSON under
//! `--json`, and the exit codes 0 (clean) / 1 (findings) / 2 (tool failure).
//! Errors and logs use the settled vocabulary — workspace / fingerprint /
//! cache key — never bare "root".
//!
//! **Never does:** name a workspace (that is `workspace`), own the drawer
//! payload lifecycle (`cache`), or hold the registry map + resident engine
//! (`registry`). It wires those crates and dials the daemon socket; it
//! re-implements none of them. Out of scope this iteration: the `DuckDB` view
//! organ, the daemon-side engine internals, serve-mode changes, and any v2
//! `root` removal.

use std::path::PathBuf;
use std::process::ExitCode;

mod cache_cmd;
mod check_cmd;
mod config_cmd;
mod corpus_tier;
mod daemon;
mod engine;
mod gc;
mod history_cmd;
// The commit-fence READ plane — what is standing in this checkout's hook doors,
// which `mrd check`'s fence line reports. It is PUBLIC because the design tests
// for `mrd skill hook` hold the emitted document to this module's constants (the
// generation, the door set, the marker), and that is a claim about two artifacts
// agreeing, so a test has to be able to name both. The fence itself is placed by
// the reader of `skill_cmd`'s document; nothing here writes.
pub mod hook;
mod init;
mod journal_cmd;
mod new_cmd;
mod pin_cmd;
mod preset_cmd;
mod put_cmd;
mod read_cmd;
mod realise_cmd;
mod reconcile_cmd;
mod resolve;
mod rules_cmd;
/// The rules walk (registration rework): the disk edge that enumerates the scope
/// ladder's roots and offers every page in their hash domain to tag-indexed
/// registration. `policy` stays I/O-free; this is the caller that feeds it.
pub mod rules_walk;
mod run_cmd;
mod skill_cmd;
mod sql;
mod status_cmd;
mod test_cmd;
mod unfold_cmd;
mod unregister;
mod view_status;
mod walk_cmd;

/// Exit code: a clean success.
const EXIT_OK: u8 = 0;
/// Exit code: findings — a verb ran cleanly but reported failures (e.g. `mrd
/// test` scenarios whose `^expect` did not hold).
const EXIT_FINDINGS: u8 = 1;
/// Exit code: a tool failure — bad usage, a refused deny ceiling, an I/O error,
/// or a structural fault (a malformed scenario, a pairing hard error).
const EXIT_FAIL: u8 = 2;

/// The title line and the gutter legend — everything above the listing. It is
/// held apart from [`LISTING`] so the legend, which is prose and not a verb
/// block, can never be lexed as one.
const HEADER: &str = "\
mrd — the meridian workspace CLI
  `!` in the gutter marks a verb that CHANGES something — files, the drawer, or
  the journal. Every unmarked verb is a read.
";

/// The verb listing and the options block: the one human-authored source for
/// `mrd --help` AND every `<verb> --help`, which is lexed back out of this text
/// rather than restated in a table that would drift from it the first time
/// either side is edited.
///
/// The shape the lexer reads: a block opens on a two-byte gutter followed by
/// `mrd `; the gutter is `! ` when the verb writes and `  ` when it does not, so
/// the write mark is part of the text and cannot fall out of step with a side
/// table; descriptions begin at column 27 and continuation lines carry their own
/// 27 spaces; an option belongs to the verb named first inside the `(...)` tag
/// its description opens with.
const LISTING: &str = "\
usage:
! mrd init [PATH] [--name NAME]
                           declare the root — write PATH's own MERIDIAN.md
                           self-declaration (type: meridian-root, named after
                           the directory unless --name says otherwise), register
                           the drawer, reconcile shadowed descendant drawers. A
                           declaration does NOT anchor the resolution ladder, so
                           the report also names the tier and root this path
                           resolves to. An existing valid declaration is left
                           byte-for-byte; a MERIDIAN.md that is present but does
                           not read as a root declaration refuses (exit 2)
! mrd unregister [PATH]    drop the daemon entry (if a daemon answers) and the
                           workspace's drawer
  mrd resolve [PATH]       report how a path resolves — the tier that answered
                           and the root it named (read-only, writes nothing)
  mrd links [PATH]         the corpus edge map (whole corpus, or one file),
                           answered by the daemon (auto-spawned) or in-process
  mrd read <PATH>[#FRAG] [--mode toc|sections] [--section SEL]
                           the composed read: addressing + content + render at
                           ONE engine snapshot, answered by the daemon (auto
                           -spawned) or in-process. Default mode toc = the
                           section map (dewey ordinal, depth, title, hpath,
                           words, sec_rev) + the rendered text; --section
                           (repeatable: a heading path, dewey ordinal, or
                           ^anchor) selects sections and implies mode sections.
                           Human output is the rendered text verbatim. Exits:
                           0 served / 1 the engine refused (its message,
                           verbatim) / 2 bad invocation
! mrd put <PATH> [--dry] [--force] [--actor A] [--now T]
          [--if-fingerprint FP] [--receipt PATH#ANCHOR]
                           the batch write: edits JSON on STDIN (the wire §4.4
                           grammar, [{target, edit, if_node_rev?}]), routed
                           through the production splice choke-point (CAS +
                           armed gate + write flock — never bypassed). --dry:
                           everything except disk. --force: escape an armed
                           binding-break/block refusal (the skip is journaled
                           and rendered, never silent). --if-fingerprint: the
                           world-grain guard. Exits: 0 committed (or dry) /
                           1 refused (the engine's message, verbatim) / 2 bad
                           invocation
! mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry]
                           the attestation verb: record in PAGE's meridian-lock
                           block that it draws from TARGET#SELECTOR at that
                           section's content fingerprint, and give the target a
                           stable slug ^block-id to be addressed by. PAGE is the
                           drawing end (A pins B); SELECTOR is a sanitized
                           heading path or a ^id, in the same grammar mrd read
                           takes. The lock write rides the production splice
                           choke-point, so the page's content and its lock land
                           in ONE flocked commit. --vibe additionally writes the
                           target's blob into git's object store, so the pin is
                           retrievable before any commit references it. Exits:
                           0 pinned (or dry) / 1 refused (the engine's message,
                           verbatim) / 2 bad invocation
  mrd walk <PAGE> [--down] [--depth N]
                           the context-assembly listing over the ^inputs pin
                           graph: up (default) = what PAGE draws from, --down =
                           who pins PAGE + blast radius (--depth 1 = direct
                           dependents). Read-only; every answer cites the revs
                           it read. Exits: 0 clean / 1 a red edge / 2 bad
                           invocation or in-snapshot cycle
  mrd rules [PATH] [--workspace | --user]
                           the effective-rules print verb: what governs at PATH
                           (default cwd) after id-based override resolution.
                           Per rule id, one block — the winning page with its
                           rev and scope, then every page it SHADOWS beneath it,
                           winner first in ladder order, never collapsed (`git
                           config --show-origin`). The scope ladder is user
                           space (rules under the MERIDIAN.md anchor's scope) ->
                           workspace root -> folder/session tree, and resolution
                           is NARROWED to PATH's own chain: a same-id page on a
                           sibling chain is no conflict. `armed=` is a SEPARATE
                           column, read from the attested armed set (never
                           recomputed), so `registered here` and `armed here`
                           stay distinct — `armed=-` unarmed, `armed=<mode>`
                           armed on the page that governs,
                           `armed=<mode>@<page>` armed on a different page (the
                           freeze: arming pins resolution, later discovery never
                           moves it), `(drifted)`/`(missing)` when the pinned
                           page no longer stands. An id in collision at one
                           scope on one chain renders REFUSED naming every tied
                           page — never an arbitrary winner, never omitted.
                           --workspace prints the workspace-root layer alone;
                           --user the user layer alone (empty, and saying so,
                           when no MERIDIAN.md anchors a user scope). Read-only:
                           arms nothing, mints no receipt, spends no cap.
                           Exits: 0 clean / 1 a finding (a collision, a refused
                           rule page, a red armed row, an unreadable armed set)
                           / 2 bad invocation or a PATH outside the workspace
  mrd config               the MERIDIAN.md config plane: resolve the bootstrap
                           chain (MERIDIAN_CONFIG, then $HOME/MERIDIAN.md) and
                           print what it found — the resolved path, the state,
                           the origin: which rung supplied that path, which the
                           path cannot say when both rungs name one file,
                           the config's own rev and fingerprint, the BOUND mount
                           table (canonical name / vault name / path, plus each
                           root's state), and the declared tools in document
                           order. This is the verb that PUBLISHES the mount
                           table: the render face elides meridian-* blocks, so
                           `mrd read` on the same file shows its prose and none
                           of its entries. Read-only.
                           Exits: 0 resolved and every root bound / 1 the config
                           refused, or any root refuses — grey(...) and red(...)
                           alike, each with its own reason word / 2 bad
                           invocation
  mrd check [--core]       the pure READ validity verb (what lies?): layer-0 core
                           recomputes the receipt journal's chain continuity and
                           the foreign_edit trace (last-receipt-vs-live) over the
                           resolved workspace. Writes nothing. When the journal
                           cannot date the live tree — no rows, or a last receipt
                           the tree no longer matches — both detectors refuse
                           grey(cannot-assess) instead of claiming a green they
                           never read or an out-of-writer edit they cannot
                           identify. Exits: 0 green / 1 a chain break, or
                           grey(cannot-assess) — the exit says do-not-proceed, the
                           reason word says why / 2 bad invocation
  mrd skill hook           EMIT the commit-fence contract to stdout, and nothing
                           else: the markdown IS the contract — what to place, at
                           which three doors (pre-commit, pre-merge-commit,
                           pre-applypatch, per $GIT_COMMON_DIR), the fence body
                           verbatim (it runs `mrd check --commit-gate` and
                           rejects on its exit), the MRD_HOOK_FORCE grammar, the
                           generation line, when to REFUSE to place it (a
                           submodule, core.hooksPath set, a foreign hook, a fence
                           from a newer engine, a workspace that is not the
                           worktree top-level, a non-repository), and how to
                           verify. The READER of the document does the placing:
                           this verb writes no file, reads no git dir, resolves
                           no workspace. `mrd check` reports what a checkout is
                           actually fenced by, on its fence: line. There is no
                           --json face — the document is markdown. Exits: 0 the
                           document was written to stdout / 2 bad invocation
  mrd cache ls             list registered drawers
! mrd cache clean [--all]  reap stale / orphaned / retired drawers (--all: every
                           drawer)
  mrd sql <query>          run SQL client-side over the daemon-published DuckDB
                           view, with the honest-tense freshness frame
  mrd view status          per-workspace view freshness + refresh telemetry (OD7)
  mrd status [--cwd PATH]  the bare, pure-local drift + freshness summary: the
                           armed INDEX line (armed / drifted / forced-since
                           -realise), the composed three-axis line (pin color ·
                           anchor state · convention severity), the anchor
                           -qualified tip axis, and one violation row per forced
                           write. O(armed) — reads ONE index file, fetch-less,
                           never evaluates a predicate. Exits: 0 clean / 1 a
                           finding (drift / forced / faulted INDEX) / 2 bad
                           invocation
! mrd daemon               run the registry daemon in the foreground
  mrd test --corpus <SPEC> tier-2 corpus runner: drive CHECK/HOOK rule pages over
                           SYNTHETIC changes and report fire-where-expected, zero
                           dead rules, fuel + heap p50/p99, and FIX/HOOK quiescence
                           by reachable trigger graph + counterfactual fuel. Exits:
                           0 clean / 1 a mismatch, dead rule, budget finding, or
                           failed quiescence / 2 malformed spec or unreadable corpus
  mrd test --history WORKSPACE --rule PAGE [--spec PAGE]
                           the history tier: JOIN the receipt journal's rows
                           against git (the commit that appended each ^r-NNNNNN
                           row gives the write's before/after bytes), rebuild the
                           docs, and compare PAGE's CHECK refusals with the golden
                           list declared in --spec's ```golden fence (a HOOK-only
                           page refuses zero changes). --spec names a spec page
                           whose `rule:` reference must resolve to PAGE; omitting
                           it declares nothing. The report always names the exact
                           journal span; a would-refuse item absent from that fence
                           fails, a declared item passes with its reason rendered,
                           and unreconstructable rows are counted grey, never
                           guessed. Exits: 0 clean /
                           1 an undeclared would-refuse item / 2 tool failure
! mrd run <PAGE> [TASK] [-- ARGS]
                           run a task block addressed by the page's frontmatter
                           (task.<name> bindings; PAGE is workspace-relative).
                           TASK omitted: one declared task runs, several list
                           and exit 2. Exits: 0 clean / 1 run refused or failed
                           / 2 bad invocation
! mrd new <KIND> <ID> [--dry] [--actor A] [--now T]
                           file birth: resolve the def (presets/<KIND>.md or a
                           page path), fill its ^template, validate the filled
                           record against its ^properties, and birth the first
                           rev through the guarded create (inline birth
                           receipt). An invalid def refuses def_invalid naming the
                           rule; an occupied target refuses cas_mismatch. Exits: 0
                           born (or dry) / 1 refused / 2 bad invocation
! mrd unfold <PRESET> [--dry] [--actor A] [--now T]
                           materialize a preset's declared scaffold: every
                           # Unfold file is born through the guarded create, so
                           each carries a birth receipt; an existing path refuses
                           via the if_absent CAS, byte-untouched. Exits: 0 all
                           born (or dry) / 1 a path already existed / 2 bad
                           invocation
! mrd reconcile <PRESET> [--prune] [--dry] [--actor A] [--now T]
                           reconcile the tree toward a preset's declared scaffold:
                           materialize ALL missing declared paths (guarded
                           create). --prune removes ONLY declared-ephemeral files
                           (guarded remove) + empty-undeclared dirs beneath the
                           scaffold; undeclared content renders as findings,
                           NEVER a prune. Exits: 0 converged (or dry) / 1 a
                           finding / 2 bad invocation
! mrd journal genesis --ruling <REF> [--archive PATH] [--dry] [--json]
                           the GOVERNED reset of the receipt journal (G2): move
                           every row to a dated archive page, truncate, and open
                           the new chain with a `op=genesis` row naming that
                           archive. The write door refuses the reserved journal,
                           so the only alternative was a hand rewrite — which
                           teaches that the attested record is editable when it
                           is inconvenient. --ruling is REQUIRED: the engine
                           never invents a justification it was not given. The
                           row attaches to the ARCHIVE (in the hash domain, so
                           its creation really does move the root); truncating
                           the journal moves none, by design. Rows are
                           superseded, never destroyed. NOTE the chain reads
                           grey(no-baseline) afterwards, not green. Exits: 0
                           done (or dry) / 1 the plane refused (nothing to
                           archive, archive exists) / 2 bad invocation
! mrd realise <PAGE> [--dry]
                           the reconciliation loop: observe -> check ->
                           apply (only on drift, once) -> re-check over the page's
                           declared claim (realise.field/realise.expected +
                           realise.apply). Apply rides mrd run. Reports one terminal
                           state: converged / drifted-fixed / non-convergent /
                           pending-agent. Exits: 0 converged/drifted-fixed (or dry)
                           / 1 non-convergent or pending-agent / 2 bad invocation

options:
  --json                   emit JSON instead of a human table
  --env KEY=VALUE          (run) supply one declared env entry (repeatable)
  --dry                    (run) starlark: evaluate hermetically and print the
                           full effect set, apply nothing; bash: show the block
                           + resolved caps, refuse to exec
  --list                   (run) list the page's tasks with contracts and caps
  --history                (test) the history tier over WORKSPACE (a git repo)
  --rule PAGE              (test --history) the workspace-relative rule PAGE to run
  --spec PAGE              (test --history) the workspace-relative SPEC page whose
                           ```golden fence declares the exceptions; its `rule:`
                           reference must resolve to --rule's PAGE. Omitted:
                           nothing is declared
  -h, --help               print this help
";

/// A command failure: the process exit code plus a diagnostic for stderr.
#[derive(Debug)]
pub(crate) struct Fail {
    pub(crate) code: u8,
    pub(crate) message: String,
    /// Print the whole verb surface beneath the diagnostic. Set only by
    /// [`Fail::usage`] — a refusal that named no verb the CLI knows, where the
    /// listing is the answer rather than noise.
    pub(crate) usage: bool,
}

impl Fail {
    /// A tool failure (exit 2).
    pub(crate) fn tool(message: String) -> Self {
        Fail {
            code: EXIT_FAIL,
            message,
            usage: false,
        }
    }

    /// A findings failure (exit 1): the verb ran, but reported failures.
    pub(crate) fn findings(message: String) -> Self {
        Fail {
            code: EXIT_FINDINGS,
            message,
            usage: false,
        }
    }

    /// A tool failure (exit 2) whose answer is the verb surface itself: no verb
    /// was named, or the name given is not one. [`run`] prints the diagnostic
    /// FIRST and the listing under it, so the reason a caller is looking for is
    /// never the line that scrolled off the top.
    pub(crate) fn usage(message: String) -> Self {
        Fail {
            code: EXIT_FAIL,
            message,
            usage: true,
        }
    }

    /// A failure carrying an explicit exit code: a verb whose own ladder names
    /// the leg (`EXIT_FINDING`, `EXIT_RUN`), and the signal legs of `mrd run`,
    /// whose code is `128 + signal` and belongs to no ladder at all.
    pub(crate) fn with_code(code: u8, message: String) -> Self {
        Fail {
            code,
            message,
            usage: false,
        }
    }
}

/// Output shape: a human table by default, JSON under `--json`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Format {
    Human,
    Json,
}

/// The current working directory, as a tool failure when it cannot be read.
pub(crate) fn current_dir() -> Result<PathBuf, Fail> {
    std::env::current_dir()
        .map_err(|e| Fail::tool(format!("cannot read the current directory: {e}")))
}

/// The whole help text: the header and its legend, a blank line, then the
/// listing — the shape `mrd --help` has always printed.
fn usage() -> String {
    format!("{HEADER}\n{LISTING}")
}

/// Parse `args` (argv without the program name) and run the selected verb.
#[must_use]
pub fn run(args: &[String]) -> ExitCode {
    match dispatch(args) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(fail) => {
            // The diagnostic leads. Deciding the order HERE, once, is what stops
            // a refusal from landing under 239 lines of help it shares stderr
            // with — no call site can put the two back the wrong way round.
            eprintln!("mrd: {}", fail.message);
            if fail.usage {
                eprint!("{}", usage());
            }
            ExitCode::from(fail.code)
        }
    }
}

fn dispatch(args: &[String]) -> Result<(), Fail> {
    let Some(verb) = args.first() else {
        return Err(Fail::usage("no subcommand given".to_owned()));
    };
    if let Some(page) = help::for_invocation(args) {
        print!("{page}");
        return Ok(());
    }
    match verb.as_str() {
        "help" | "-h" | "--help" => {
            print!("{}", usage());
            Ok(())
        }
        "init" => init::dispatch(&args[1..]),
        "unregister" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            unregister::run(p.positional.as_deref(), p.format())
        }
        "resolve" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            resolve::run_command(p.positional.as_deref(), p.format())
        }
        "links" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            engine::run_command(p.positional.as_deref(), p.format())
        }
        "read" => read_cmd::dispatch(&args[1..]),
        "put" => put_cmd::dispatch(&args[1..]),
        "pin" => pin_cmd::dispatch(&args[1..]),
        "walk" => walk_cmd::dispatch(&args[1..]),
        "rules" => rules_cmd::dispatch(&args[1..]),
        "check" => check_cmd::dispatch(&args[1..]),
        "skill" => skill_cmd::dispatch(&args[1..]),
        "config" => {
            let p = Parsed::parse(&args[1..], NO_PATH, NO_ALL)?;
            config_cmd::run(p.format())
        }
        "cache" => dispatch_cache(&args[1..]),
        "sql" => sql::run(&args[1..]),
        "status" => status_cmd::run(&args[1..]),
        "view" => dispatch_view(&args[1..]),
        "test" => test_cmd::dispatch(&args[1..]),
        "run" => run_cmd::dispatch(&args[1..]),
        "new" => new_cmd::run(&args[1..]),
        "unfold" => unfold_cmd::run(&args[1..]),
        "reconcile" => reconcile_cmd::run(&args[1..]),
        "realise" => realise_cmd::run(&args[1..]),
        "journal" => journal_cmd::run(&args[1..]),
        "daemon" => {
            reject_extra(&args[1..])?;
            daemon::run()
        }
        other => Err(Fail::usage(format!("unknown subcommand: {other}"))),
    }
}

fn dispatch_cache(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        return Err(Fail::usage(
            "cache needs a subcommand (ls | clean)".to_owned(),
        ));
    };
    match sub.as_str() {
        "ls" => {
            let p = Parsed::parse(&args[1..], NO_PATH, NO_ALL)?;
            cache_cmd::ls(p.format())
        }
        "clean" => {
            let p = Parsed::parse(&args[1..], NO_PATH, ALLOW_ALL)?;
            cache_cmd::clean(p.all, p.format())
        }
        other => Err(Fail::usage(format!("unknown cache subcommand: {other}"))),
    }
}

fn dispatch_view(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        return Err(Fail::usage("view needs a subcommand (status)".to_owned()));
    };
    match sub.as_str() {
        "status" => view_status::run(&args[1..]),
        other => Err(Fail::usage(format!("unknown view subcommand: {other}"))),
    }
}

/// Refuse any argument to a verb that takes none.
fn reject_extra(args: &[String]) -> Result<(), Fail> {
    match args.first() {
        None => Ok(()),
        Some(a) => Err(Fail::tool(format!("unexpected argument: {a}"))),
    }
}

// Named booleans for the shared parser, so a verb states exactly what it
// accepts and an unknown/duplicate argument is a loud exit-2, never ignored.
const ALLOW_PATH: bool = true;
const NO_PATH: bool = false;
const ALLOW_ALL: bool = true;
const NO_ALL: bool = false;

/// The parsed tail of a verb: an optional positional, `--json`, `--all`.
struct Parsed {
    positional: Option<String>,
    json: bool,
    all: bool,
}

impl Parsed {
    fn parse(tail: &[String], allow_path: bool, allow_all: bool) -> Result<Self, Fail> {
        let mut parsed = Parsed {
            positional: None,
            json: false,
            all: false,
        };
        for arg in tail {
            match arg.as_str() {
                "--json" => parsed.json = true,
                "--all" if allow_all => parsed.all = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if allow_path && parsed.positional.is_none() => {
                    parsed.positional = Some(value.to_owned());
                }
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        Ok(parsed)
    }

    fn format(&self) -> Format {
        if self.json {
            Format::Json
        } else {
            Format::Human
        }
    }
}

/// `<verb> --help`: the per-verb face of [`LISTING`], LEXED back out of the same
/// text `mrd --help` prints.
///
/// Nothing here is a second copy of the help. A block is found by its addressing
/// words and reprinted verbatim, so a description is editable in exactly one
/// place, and the write mark a caller sees is the byte that actually stands in
/// the listing's gutter. A table beside the prose would answer the same
/// questions until the day someone edited one side of it.
mod help {
    use super::{HEADER, LISTING};

    /// Where a description begins — on a block's own line, and on every
    /// continuation line beneath it.
    const DESC_COL: usize = 27;

    /// One verb block: the words that address it, and its lines as they stand.
    struct Block {
        words: Vec<String>,
        lines: Vec<String>,
    }

    /// One options entry: the flags it defines, the verb named by the `(...)`
    /// tag its description opens with, and its lines. An untagged entry names no
    /// owner, and reaches a page only through a synopsis that offers its flag.
    struct Opt {
        flags: Vec<String>,
        owner: Option<String>,
        lines: Vec<String>,
    }

    /// Does this line carry a gutter — the two bytes that open a block or
    /// continue one? `! ` marks a verb that writes and `  ` one that does not;
    /// both are gutters, and the mark rides along with the line rather than
    /// living in a table that could disagree with it. A line with no gutter
    /// (a section label, a blank) closes whatever block was open.
    fn has_gutter(line: &str) -> bool {
        matches!(line.get(..2), Some("! " | "  "))
    }

    /// The part of a line before its description begins.
    ///
    /// The description column is only there when it is really there: a synopsis
    /// long enough to overflow it (`mrd pin <PAGE> <TARGET>#<SELECTOR> …`)
    /// carries no description on its own line, and keeps its whole tail.
    fn head_of(line: &str) -> &str {
        if line.as_bytes().get(DESC_COL - 1) == Some(&b' ') {
            // Byte 26 is a space, so byte 27 is a char boundary — the fallback
            // is for a caller that changes DESC_COL, not for this listing.
            line.get(..DESC_COL).unwrap_or(line)
        } else {
            line
        }
    }

    /// Strip the notation a listing wraps its flags in: `[--dry]` is `--dry`.
    fn bare(token: &str) -> &str {
        token.trim_matches(|c: char| matches!(c, '[' | ']' | ',' | '|' | '(' | ')'))
    }

    /// The addressing words of a synopsis: `! mrd cache clean [--all]` gives
    /// `["cache", "clean"]`. They stop at the first token that is not a bare
    /// lowercase name, which is where the operands and flags begin.
    fn words_of(line: &str) -> Vec<String> {
        head_of(line)
            .get(2..)
            .and_then(|synopsis| synopsis.strip_prefix("mrd "))
            .unwrap_or_default()
            .split_whitespace()
            .take_while(|word| word.chars().all(|c| c.is_ascii_lowercase()))
            .map(str::to_owned)
            .collect()
    }

    /// The flags an options entry defines — `-h, --help` defines two.
    fn flags_of(line: &str) -> Vec<String> {
        head_of(line)
            .split_whitespace()
            .map(bare)
            .filter(|token| token.starts_with('-') && token.len() > 1)
            .map(str::to_owned)
            .collect()
    }

    /// A block's synopsis: its operands and flags, with the description left
    /// out. A synopsis can wrap (`mrd put`), and the line it wraps onto is
    /// exactly the one that does NOT begin at the description column.
    fn synopsis_of(block: &Block) -> String {
        let mut synopsis = String::new();
        for (n, line) in block.lines.iter().enumerate() {
            let is_description = line.bytes().take(DESC_COL).all(|b| b == b' ');
            if n == 0 || !is_description {
                synopsis.push_str(head_of(line));
                synopsis.push(' ');
            }
        }
        synopsis
    }

    /// Does a verb's own synopsis offer this flag?
    fn offers(synopsis: &str, flag: &str) -> bool {
        synopsis
            .split_whitespace()
            .map(bare)
            .any(|token| token == flag)
    }

    /// The verb an option belongs to: the first word inside the `(...)` its
    /// description opens with, so `--rule PAGE  (test --history) ...` belongs to
    /// `test`.
    fn owner_of(line: &str) -> Option<String> {
        let owner: String = line
            .get(DESC_COL..)?
            .trim_start()
            .strip_prefix('(')?
            .chars()
            .take_while(char::is_ascii_lowercase)
            .collect();
        (!owner.is_empty()).then_some(owner)
    }

    /// Lex the listing's verb blocks. A block opens on a gutter followed by
    /// `mrd ` and runs to the first line with no gutter at all.
    fn blocks() -> Vec<Block> {
        let mut blocks: Vec<Block> = Vec::new();
        let mut open: Option<usize> = None;
        for line in LISTING.lines() {
            if !has_gutter(line) {
                open = None;
            } else if line[2..].starts_with("mrd ") {
                open = Some(blocks.len());
                blocks.push(Block {
                    words: words_of(line),
                    lines: vec![line.to_owned()],
                });
            } else if let Some(block) = open {
                blocks[block].lines.push(line.to_owned());
            }
        }
        blocks
    }

    /// Lex the options block the same way: an entry opens on a gutter followed
    /// by a `-`.
    fn options() -> Vec<Opt> {
        let mut opts: Vec<Opt> = Vec::new();
        let mut open: Option<usize> = None;
        for line in LISTING.lines() {
            if !has_gutter(line) {
                open = None;
            } else if line[2..].starts_with('-') {
                open = Some(opts.len());
                opts.push(Opt {
                    flags: flags_of(line),
                    owner: owner_of(line),
                    lines: vec![line.to_owned()],
                });
            } else if let Some(opt) = open {
                opts[opt].lines.push(line.to_owned());
            }
        }
        opts
    }

    /// Is this invocation asking for help? Only up to a `--` separator: in `mrd
    /// run PAGE TASK -- --help` the flag is the TASK's argument, and answering
    /// it here would print this CLI's help instead of running the task.
    fn asks(args: &[String]) -> bool {
        args.iter()
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg.as_str() == "-h" || arg.as_str() == "--help")
    }

    /// The verb words an invocation names: its leading bare-lowercase arguments.
    /// The first operand or flag ends them, so `mrd read notes.md --help` asks
    /// about `read`.
    fn query_of(args: &[String]) -> Vec<String> {
        args.iter()
            .take_while(|arg| !arg.is_empty() && arg.chars().all(|c| c.is_ascii_lowercase()))
            .cloned()
            .collect()
    }

    /// One word list is a prefix of the other. So `mrd cache --help` answers
    /// with both cache subcommands, `mrd view status --help` with just that one,
    /// and `mrd read notes.md --help` with `read`.
    fn addresses(query: &[String], words: &[String]) -> bool {
        !query.is_empty() && !words.is_empty() && query.iter().zip(words).all(|(q, w)| q == w)
    }

    /// The page: the header and its legend, the matched block(s) under `usage:`,
    /// then the options those verbs take.
    ///
    /// An option reaches a page two ways: the `(...)` tag names the verb, OR the
    /// verb's own synopsis offers the flag. The second is what keeps the page
    /// honest — `[--dry]` stands in seven synopses whose entry is tagged
    /// `(run)`, and a page that showed the flag above while hiding its meaning
    /// below would be a worse answer than no page.
    ///
    /// An untagged global no synopsis offers stays absent. `--json` under `mrd
    /// skill hook --help` would promise a face that verb's own description says
    /// does not exist, and a help page that lies costs more than a short one.
    fn render(matched: &[Block]) -> String {
        let owners: Vec<&str> = matched
            .iter()
            .filter_map(|block| block.words.first())
            .map(String::as_str)
            .collect();
        let synopses: Vec<String> = matched.iter().map(synopsis_of).collect();
        let owned: Vec<Opt> = options()
            .into_iter()
            .filter(|opt| {
                opt.owner.as_deref().is_some_and(|o| owners.contains(&o))
                    || opt
                        .flags
                        .iter()
                        .any(|flag| synopses.iter().any(|synopsis| offers(synopsis, flag)))
            })
            .collect();

        let mut page = String::from(HEADER);
        page.push_str("\nusage:\n");
        for line in matched.iter().flat_map(|block| &block.lines) {
            page.push_str(line);
            page.push('\n');
        }
        if !owned.is_empty() {
            page.push_str("\noptions:\n");
            for line in owned.iter().flat_map(|opt| &opt.lines) {
                page.push_str(line);
                page.push('\n');
            }
        }
        page.push_str("\nsee `mrd --help` for every verb.\n");
        page
    }

    /// The help page for the verb an invocation addresses — or `None` when it
    /// asks for no help, or names nothing this CLI knows. That refusal is
    /// dispatch's to make, not this module's: falling through is what keeps
    /// `mrd nope --help` an `unknown subcommand`.
    pub(super) fn for_invocation(args: &[String]) -> Option<String> {
        if !asks(args) {
            return None;
        }
        let query = query_of(args);
        let matched: Vec<Block> = blocks()
            .into_iter()
            .filter(|block| addresses(&query, &block.words))
            .collect();
        if matched.is_empty() {
            return None;
        }
        Some(render(&matched))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The lexer's premise: every block it finds is addressable. A block
        /// whose words came out empty could never be reached by `--help`, and
        /// would fail silently — the page would simply not exist.
        #[test]
        fn every_block_is_addressable() {
            for block in blocks() {
                assert!(
                    !block.words.is_empty(),
                    "a block no --help can ever address:\n{}",
                    block.lines.join("\n")
                );
            }
        }

        /// The two-word verbs resolve to both of their words — the case that
        /// tells a real lexer apart from `split_whitespace().next()`.
        #[test]
        fn multi_word_verbs_keep_both_words() {
            let found: Vec<Vec<String>> = blocks()
                .into_iter()
                .map(|block| block.words)
                .filter(|words| words.len() > 1)
                .collect();
            assert_eq!(
                found,
                vec![
                    vec!["skill", "hook"],
                    vec!["cache", "ls"],
                    vec!["cache", "clean"],
                    vec!["view", "status"],
                    vec!["journal", "genesis"],
                ],
                "the two-word verbs of the listing"
            );
        }

        /// The operands stop the words. `mrd test --corpus <SPEC>` and `mrd test
        /// --history WORKSPACE` are two blocks that share one verb name, which
        /// is why `mrd test --help` must print both.
        #[test]
        fn operands_and_flags_end_the_words() {
            let test_blocks: Vec<Vec<String>> = blocks()
                .into_iter()
                .map(|block| block.words)
                .filter(|words| words.first().is_some_and(|word| word == "test"))
                .collect();
            assert_eq!(test_blocks, vec![vec!["test"], vec!["test"]]);
        }

        /// A synopsis that overflows the description column is not truncated
        /// into nonsense — the guard is that byte 26 is a space, not that the
        /// line is long enough to slice.
        #[test]
        fn an_overflowing_synopsis_still_lexes() {
            let overflowing = "  mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry]";
            assert_ne!(
                overflowing.as_bytes()[DESC_COL - 1],
                b' ',
                "the case this test is for: no description column on this line"
            );
            assert_eq!(words_of(overflowing), vec!["pin"]);
        }

        /// The write mark is the gutter, so the classification is countable
        /// straight off the text. 12 verbs write; the rest read.
        #[test]
        fn twelve_verbs_are_marked_as_writers() {
            let marked: Vec<&str> = LISTING
                .lines()
                .filter(|line| line.starts_with("! "))
                .collect();
            assert_eq!(
                marked.len(),
                12,
                "marked as writers:\n{}",
                marked.join("\n")
            );
            assert_eq!(blocks().len(), 26, "verb blocks in the listing");
        }

        /// Every option that names an owner names a verb that exists, so a
        /// per-verb page can never silently drop one.
        #[test]
        fn every_tagged_option_names_a_real_verb() {
            let verbs: Vec<String> = blocks()
                .into_iter()
                .filter_map(|block| block.words.first().cloned())
                .collect();
            for opt in options() {
                if let Some(owner) = opt.owner {
                    assert!(
                        verbs.contains(&owner),
                        "option tagged ({owner}), a verb the listing does not have:\n{}",
                        opt.lines.join("\n")
                    );
                }
            }
        }

        /// The query stops where the verb does — an operand is not a verb word.
        #[test]
        fn a_query_is_the_leading_verb_words_only() {
            assert_eq!(
                query_of(&["read", "notes.md", "--help"].map(String::from)),
                ["read"]
            );
            assert_eq!(query_of(&["cache", "--help"].map(String::from)), ["cache"]);
            assert_eq!(
                query_of(&["view", "status", "--help"].map(String::from)),
                ["view", "status"]
            );
            assert!(query_of(&["--help"].map(String::from)).is_empty());
        }

        /// A `--` separator ends the CLI's own arguments: `mrd run PAGE TASK --
        /// --help` is the TASK's flag, and answering it here would print help
        /// instead of running what the caller asked for.
        #[test]
        fn a_separator_hides_the_flag_from_this_cli() {
            assert!(asks(&["run", "page.md", "--help"].map(String::from)));
            assert!(!asks(
                &["run", "page.md", "task", "--", "--help"].map(String::from)
            ));
            assert!(
                for_invocation(&["run", "p.md", "t", "--", "--help"].map(String::from)).is_none()
            );
        }
    }
}
