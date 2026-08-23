//! The timing instrument — per-phase wall clock on a release binary, off by
//! default.
//!
//! **Charter.** One environment switch (`MRD_TIMING`), resolved ONCE per
//! process into a sink, and one span type that emits a single line per
//! completed phase. It writes to stderr or a file, never to stdout; it changes
//! no output and no exit code; and when the switch is off it reads no clock,
//! allocates nothing and writes nothing.
//!
//! **What it is not.** Not a log framework and not a tracer: no levels, no
//! subscriber, no spans in flight, no second time unit. `us` — microseconds,
//! integer — is the one time noun, the same one the wire frame's
//! `meta.duration_us` already uses.
//!
//! The line, `\n`-terminated and written in ONE `write_all` so concurrent
//! threads interleave whole lines, never halves:
//!
//! ```text
//! mrd-timing cmd=run phase=snapshot.read us=402118
//! ```
//!
//! `cmd=` names the PROCESS's entry verb (set once by [`label`]), never one
//! request — a daemon serving many ops reports `cmd=daemon` and distinguishes
//! its work in the phase name. `phase=` is a dotted name whose dot is nesting.
//! Lines print in COMPLETION order, so an inner phase prints before the phase
//! containing it.
//!
//! Adding a phase is one line at the site that owns it:
//!
//! ```no_run
//! let _t = timing::phase("snapshot.read");
//! ```
//!
//! Surface and lanes: `docs/status.md` § The timing mode. The `mrd run` phase
//! list: `docs/run-plane.md` § Timing phases.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// The environment switch. Its VALUE is the sink: an off word, an on word, or
/// a file path (`docs/status.md` § The timing mode).
pub const SWITCH: &str = "MRD_TIMING";

/// The fixed first token of every emitted line, so `grep '^mrd-timing '`
/// separates this mode from everything else sharing the stream.
pub const PREFIX: &str = "mrd-timing";

/// The `cmd=` a process that never called [`label`] reports.
const DEFAULT_LABEL: &str = "mrd";

/// Values that mean OFF. Spelled out rather than "anything but the on words"
/// because the fallback arm CREATES A FILE: without these, `MRD_TIMING=off`
/// would write a file named `off` into the caller's working directory.
const OFF_WORDS: [&str; 4] = ["0", "off", "false", "no"];

/// Values that mean the stderr sink.
const ON_WORDS: [&str; 4] = ["1", "on", "true", "yes"];

/// Where completed phases are written.
#[derive(Debug)]
enum Sink {
    /// The switch is absent or names an off word: nothing is measured.
    Off,
    Stderr,
    /// An append handle, behind a mutex so one `write_all` is one line even
    /// when several daemon threads finish a phase at once.
    File(Mutex<File>),
}

/// What a raw switch value SELECTS, before any file is opened — the pure half
/// of [`resolve`], so the switch's semantics are testable without I/O.
#[derive(Debug, PartialEq, Eq)]
enum Chosen<'a> {
    Off,
    Stderr,
    Path(&'a OsStr),
}

static SINK: OnceLock<Sink> = OnceLock::new();
static LABEL: OnceLock<String> = OnceLock::new();

/// Classify a raw switch value. Absent and empty are off, so exporting
/// `MRD_TIMING=` disables the mode rather than naming a file with no name.
fn choose(raw: Option<&OsStr>) -> Chosen<'_> {
    let Some(raw) = raw else {
        return Chosen::Off;
    };
    if raw.is_empty() {
        return Chosen::Off;
    }
    match raw.to_str() {
        Some(text) if OFF_WORDS.contains(&text) => Chosen::Off,
        Some(text) if ON_WORDS.contains(&text) => Chosen::Stderr,
        // A non-UTF-8 value is a path like any other: paths are bytes, and the
        // on/off words are all ASCII, so nothing else can be meant by one.
        _ => Chosen::Path(raw),
    }
}

/// Read the switch and open the sink. Runs at most once per process.
fn resolve() -> Sink {
    let raw = std::env::var_os(SWITCH);
    match choose(raw.as_deref()) {
        Chosen::Off => Sink::Off,
        Chosen::Stderr => Sink::Stderr,
        // An unopenable path degrades to stderr rather than to silence: the
        // caller asked for the mode, and silence would read as "the code never
        // ran there" — the one answer this instrument must never fake.
        Chosen::Path(path) => OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(path))
            .map_or(Sink::Stderr, |file| Sink::File(Mutex::new(file))),
    }
}

fn sink() -> &'static Sink {
    SINK.get_or_init(resolve)
}

/// Is the mode on? After the first call this is one atomic load — the whole
/// cost the off path pays.
#[must_use]
pub fn on() -> bool {
    !matches!(sink(), Sink::Off)
}

/// Name the verb this process was entered with — the `cmd=` field. First
/// writer wins, and a process that never calls it reports `mrd`.
///
/// It names the PROCESS, not a request: a daemon serving many ops has one
/// label for all of them, which is why phase names below the door carry
/// enough of their own context to be read on a busy sink.
pub fn label(name: &str) {
    if on() {
        let _ = LABEL.set(name.to_owned());
    }
}

/// One line, exactly as it is written.
fn line(cmd: &str, phase: &str, us: u128) -> String {
    format!("{PREFIX} cmd={cmd} phase={phase} us={us}\n")
}

fn emit(phase: &str, elapsed: Duration) {
    let cmd = LABEL.get().map_or(DEFAULT_LABEL, String::as_str);
    let line = line(cmd, phase, elapsed.as_micros());
    match sink() {
        Sink::Off => {}
        Sink::Stderr => {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        // Poisoning is a panic in some other phase's write, not a reason to
        // lose every later measurement: take the guard back and keep writing.
        Sink::File(file) => {
            let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// One phase being measured. It ends when the value is dropped, so the phase
/// is the scope that holds it; [`Phase::stop`] ends one early.
///
/// When the mode is off it holds no [`Instant`] — nothing was read, so nothing
/// is emitted.
#[derive(Debug)]
#[must_use = "a phase measures until its span is dropped; `let _ = phase(..)` drops it at once"]
pub struct Phase {
    name: &'static str,
    started: Option<Instant>,
}

impl Phase {
    /// End this phase now instead of at the end of its scope.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Phase {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            emit(self.name, started.elapsed());
        }
    }
}

/// Start measuring `name`. Off, this reads no clock and allocates nothing.
pub fn phase(name: &'static str) -> Phase {
    Phase {
        name,
        started: on().then(Instant::now),
    }
}

#[cfg(test)]
mod tests {
    use super::{Chosen, DEFAULT_LABEL, OFF_WORDS, ON_WORDS, PREFIX, Phase, choose, line};
    use std::ffi::OsStr;

    /// The line a consumer parses: prefix, then three `key=value` fields in a
    /// fixed order, space-separated, newline-terminated.
    fn parse(raw: &str) -> Option<(String, String, u128)> {
        let body = raw.strip_suffix('\n')?;
        let mut fields = body.split(' ');
        if fields.next()? != PREFIX {
            return None;
        }
        let cmd = fields.next()?.strip_prefix("cmd=")?;
        let phase = fields.next()?.strip_prefix("phase=")?;
        let us = fields.next()?.strip_prefix("us=")?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        Some((cmd.to_owned(), phase.to_owned(), us))
    }

    #[test]
    fn the_emitted_line_parses_back() {
        let raw = line("run", "snapshot.read", 402_118);
        assert_eq!(raw, "mrd-timing cmd=run phase=snapshot.read us=402118\n");
        let (cmd, phase, us) = parse(&raw).expect("the line this crate writes parses");
        assert_eq!(cmd, "run");
        assert_eq!(phase, "snapshot.read");
        assert_eq!(us, 402_118);
    }

    /// A zero-microsecond phase is a measurement, not a missing field — the
    /// integer is always present, so a consumer summing `us` never guesses.
    #[test]
    fn a_zero_phase_still_carries_its_field() {
        let (_, _, us) = parse(&line(DEFAULT_LABEL, "cascade", 0)).expect("parses");
        assert_eq!(us, 0);
    }

    #[test]
    fn off_words_and_absence_and_empty_are_off() {
        assert_eq!(choose(None), Chosen::Off);
        assert_eq!(choose(Some(OsStr::new(""))), Chosen::Off);
        for word in OFF_WORDS {
            assert_eq!(choose(Some(OsStr::new(word))), Chosen::Off, "{word}");
        }
    }

    #[test]
    fn on_words_select_stderr() {
        for word in ON_WORDS {
            assert_eq!(choose(Some(OsStr::new(word))), Chosen::Stderr, "{word}");
        }
    }

    /// Anything that is not a word is a path — including a path that merely
    /// begins with one, which is why the match is on the whole value.
    #[test]
    fn any_other_value_is_a_path() {
        for value in ["/tmp/t.log", "timing.log", "./1", "on.log", "1x"] {
            assert_eq!(
                choose(Some(OsStr::new(value))),
                Chosen::Path(OsStr::new(value)),
                "{value}"
            );
        }
    }

    /// The off path holds no `Instant`: the switch is not read at drop time,
    /// it is read at construction, and off construction reads no clock.
    #[test]
    fn an_off_phase_holds_no_instant() {
        let phase = Phase {
            name: "probe",
            started: None,
        };
        assert!(phase.started.is_none());
        // Dropping it emits nothing — there is no elapsed time to report.
        phase.stop();
    }
}
