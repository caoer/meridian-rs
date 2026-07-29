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
mod expect;
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

const USAGE: &str = "\
mrd — the meridian workspace CLI

usage:
  mrd init [PATH] [--name NAME]
                           declare the root — write PATH's own MERIDIAN.md
                           self-declaration (type: meridian-root, named after
                           the directory unless --name says otherwise), register
                           the drawer, reconcile shadowed descendant drawers. A
                           declaration does NOT anchor the resolution ladder, so
                           the report also names the tier and root this path
                           resolves to. An existing valid declaration is left
                           byte-for-byte; a MERIDIAN.md that is present but does
                           not read as a root declaration refuses (exit 2)
  mrd unregister [PATH]    drop the daemon entry (if a daemon answers) and the
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
  mrd put <PATH> [--dry] [--force] [--actor A] [--now T]
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
  mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry]
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
  mrd journal genesis --ruling <REF> [--archive PATH] [--dry] [--json]
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
