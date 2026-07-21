//! The `mrd` CLI: wire the workspace / cache / registry foundation into the
//! settled verb surface.
//!
//! # Charter
//! **Owns:** hand-rolled subcommand parsing (no clap — dependency discipline),
//! the per-invocation resolution flow ([`resolve`]), and the verbs `init`,
//! `unregister`, `resolve`, `cache ls`, `cache clean`, and `daemon`. Output is
//! the house grammar: a human table by default, JSON under `--json`, and the
//! exit codes 0 (clean) / 1 (findings) / 2 (tool failure). Errors and logs use
//! the settled vocabulary — workspace / fingerprint / cache key — never bare
//! "root".
//!
//! **Never does:** name a workspace (that is `workspace`), own the drawer
//! payload lifecycle (`cache`), or hold the registry map (`registry`). It wires
//! those crates; it re-implements none of them. Out of scope this iteration:
//! the `DuckDB` view organ, a resident-daemon inversion, serve-mode changes, and
//! any v2 `root` removal.

use std::path::PathBuf;
use std::process::ExitCode;

mod cache_cmd;
mod daemon;
mod gc;
mod init;
mod resolve;
mod unregister;

/// Exit code: a clean success.
const EXIT_OK: u8 = 0;
/// Exit code: a tool failure — bad usage, a refused deny ceiling, or an I/O
/// error. (Exit 1 is reserved for future "findings"-style verbs.)
const EXIT_FAIL: u8 = 2;

const USAGE: &str = "\
mrd — the meridian workspace CLI

usage:
  mrd init [PATH]          create a .meridian.toml marker, register the drawer,
                           reconcile shadowed tier-4 drawers
  mrd unregister [PATH]    drop the daemon entry (if a daemon answers) and the
                           workspace's drawer
  mrd resolve [PATH]       report how a path resolves (read-only, writes nothing)
  mrd cache ls             list registered drawers
  mrd cache clean [--all]  reap stale / orphaned / retired drawers (--all: every
                           drawer)
  mrd daemon               run the registry daemon in the foreground

options:
  --json                   emit JSON instead of a human table
  -h, --help               print this help
";

/// A command failure: the process exit code plus a diagnostic for stderr.
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
}

/// Output shape: a human table by default, JSON under `--json`.
#[derive(Clone, Copy)]
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
        "cache" => dispatch_cache(&args[1..]),
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
