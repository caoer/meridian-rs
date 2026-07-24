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
mod corpus_tier;
mod daemon;
mod engine;
mod expect;
mod gc;
mod history_cmd;
mod init;
mod new_cmd;
mod preset_cmd;
mod realise_cmd;
mod reconcile_cmd;
mod resolve;
mod run_cmd;
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

const USAGE: &str = "\
mrd — the meridian workspace CLI

usage:
  mrd init [PATH]          create a .meridian.toml marker, register the drawer,
                           reconcile shadowed tier-4 drawers
  mrd unregister [PATH]    drop the daemon entry (if a daemon answers) and the
                           workspace's drawer
  mrd resolve [PATH]       report how a path resolves (read-only, writes nothing)
  mrd links [PATH]         the corpus edge map (whole corpus, or one file),
                           answered by the daemon (auto-spawned) or in-process
  mrd walk <PAGE> [--down] [--depth N]
                           the context-assembly listing over the ^inputs pin
                           graph: up (default) = what PAGE draws from, --down =
                           who pins PAGE + blast radius (--depth 1 = direct
                           dependents). Read-only; every answer cites the revs
                           it read. Exits: 0 clean / 1 a red edge / 2 bad
                           invocation or in-snapshot cycle
  mrd check [--core]       the pure READ validity verb (what lies?): layer-0 core
                           recomputes the receipt journal's chain continuity and
                           the foreign_edit trace (last-receipt-vs-live) over the
                           resolved workspace. Writes nothing. Exits: 0 green / 1 a
                           chain break or foreign_edit finding / 2 bad invocation
  mrd cache ls             list registered drawers
  mrd cache clean [--all]  reap stale / orphaned / retired drawers (--all: every
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
  mrd daemon               run the registry daemon in the foreground
  mrd test <PATH>          run scenario file(s) (a *.md file, or a dir of them):
                           mount base/ into a real tmpdir, route ^put through the
                           production write path, assert ^expect starlark over
                           t.result / t.doc(path) / t.journal. Exits: 0 clean /
                           1 an ^expect failed / 2 malformed or pairing hard error
  mrd test --corpus <SPEC> tier-2 corpus runner: drive a convention's check_change
                           over SYNTHETIC changes derived from the 18-02 corpus and
                           report fire-where-expected, dead rules, and fuel + heap
                           p50/p99 budgets. Exits: 0 clean / 1 a fire mismatch or
                           dead rule / 2 malformed spec or unreadable corpus
  mrd test --history WORKSPACE --convention SLUG
                           the history tier: JOIN the receipt journal's rows
                           against git (the commit that appended each ^r-NNNNNN
                           row gives the write's before/after bytes), rebuild the
                           docs, and run conventions/SLUG's check_change over each
                           reconstructed change. A would-refuse item absent from
                           conventions/SLUG/GOLDEN.md fails the run; a declared
                           item passes with its reason rendered; unreconstructable
                           rows are counted grey, never guessed. Exits: 0 clean /
                           1 an undeclared would-refuse item / 2 tool failure
  mrd run <PAGE> [TASK] [-- ARGS]
                           run a task block addressed by the page's frontmatter
                           (task.<name> bindings; PAGE is workspace-relative).
                           TASK omitted: one declared task runs, several list
                           and exit 2. Exits: 0 clean / 1 run refused or failed
                           / 2 bad invocation
  mrd new <KIND> <ID> [--dry] [--actor A] [--now T]
                           file birth (U5.3): resolve the def (presets/<KIND>.md
                           or a page path), fill its ^template, validate the
                           filled record against its ^properties, and birth the
                           first rev through the guarded create (inline birth
                           receipt). An invalid def refuses def_invalid naming the
                           rule; an occupied target refuses cas_mismatch. Exits: 0
                           born (or dry) / 1 refused / 2 bad invocation
  mrd unfold <PRESET> [--dry] [--actor A] [--now T]
                           materialize a preset's declared scaffold (U5.3): every
                           # Unfold file is born through the guarded create, so
                           each carries a birth receipt; an existing path refuses
                           via the if_absent CAS, byte-untouched. Exits: 0 all
                           born (or dry) / 1 a path already existed / 2 bad
                           invocation
  mrd reconcile <PRESET> [--prune] [--dry] [--actor A] [--now T]
                           reconcile the tree toward a preset's declared scaffold
                           (U3.5b; ZT ruling #3): materialize ALL missing declared
                           paths (guarded create). --prune removes ONLY declared
                           -ephemeral files (guarded remove) + empty-undeclared
                           dirs; undeclared content renders as findings, NEVER a
                           prune. Exits: 0 converged (or dry) / 1 a finding / 2 bad
                           invocation
  mrd realise <PAGE> [--dry] [--truth index|file]
                           the reconciliation loop (U3.5b): observe -> check ->
                           apply (only on drift, once) -> re-check over the page's
                           declared claim (realise.field/realise.expected +
                           realise.apply). Apply rides mrd run. Reports one terminal
                           state: converged / drifted-fixed / non-convergent /
                           pending-agent. --truth index|file resolves a file<->index
                           convention divergence (realise-as-deploy). Exits: 0
                           converged/drifted-fixed (or dry) / 1 non-convergent or
                           pending-agent / 2 bad invocation

options:
  --json                   emit JSON instead of a human table
  --env KEY=VALUE          (run) supply one declared env entry (repeatable)
  --dry                    (run) starlark: evaluate hermetically and print the
                           full effect set, apply nothing; bash: show the block
                           + resolved caps, refuse to exec
  --list                   (run) list the page's tasks with contracts and caps
  --history                (test) the history tier over WORKSPACE (a git repo)
  --convention SLUG        (test --history) the conventions/SLUG folder to run
  -h, --help               print this help
";

/// A command failure: the process exit code plus a diagnostic for stderr.
#[derive(Debug)]
pub(crate) struct Fail {
    pub(crate) code: u8,
    pub(crate) message: String,
}

impl Fail {
    /// A tool failure (exit 2).
    pub(crate) fn tool(message: String) -> Self {
        Fail {
            code: EXIT_FAIL,
            message,
        }
    }

    /// A findings failure (exit 1): the verb ran, but reported failures.
    pub(crate) fn findings(message: String) -> Self {
        Fail {
            code: EXIT_FINDINGS,
            message,
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

/// Parse `args` (argv without the program name) and run the selected verb.
#[must_use]
pub fn run(args: &[String]) -> ExitCode {
    match dispatch(args) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(fail) => {
            eprintln!("mrd: {}", fail.message);
            ExitCode::from(fail.code)
        }
    }
}

fn dispatch(args: &[String]) -> Result<(), Fail> {
    let Some(verb) = args.first() else {
        eprint!("{USAGE}");
        return Err(Fail::tool("no subcommand given".to_owned()));
    };
    match verb.as_str() {
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        "init" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            init::run(p.positional.as_deref(), p.format())
        }
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
        "walk" => walk_cmd::dispatch(&args[1..]),
        "check" => check_cmd::dispatch(&args[1..]),
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
        "daemon" => {
            reject_extra(&args[1..])?;
            daemon::run()
        }
        other => {
            eprint!("{USAGE}");
            Err(Fail::tool(format!("unknown subcommand: {other}")))
        }
    }
}

fn dispatch_cache(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        eprint!("{USAGE}");
        return Err(Fail::tool(
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
        other => {
            eprint!("{USAGE}");
            Err(Fail::tool(format!("unknown cache subcommand: {other}")))
        }
    }
}

fn dispatch_view(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        eprint!("{USAGE}");
        return Err(Fail::tool("view needs a subcommand (status)".to_owned()));
    };
    match sub.as_str() {
        "status" => view_status::run(&args[1..]),
        other => {
            eprint!("{USAGE}");
            Err(Fail::tool(format!("unknown view subcommand: {other}")))
        }
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
