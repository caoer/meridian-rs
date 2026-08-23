//! The timing instrument — per-phase wall clock on a release binary, off by
//! default.
//!
//! **Charter.** One environment switch (`MRD_TIMING`), resolved ONCE per
//! process into a sink, and one span type that emits a single line per
//! COMPLETED phase. It writes to stderr or a file, never to stdout; it changes
//! no output and no exit code; and when the switch is off it reads no clock,
//! allocates nothing and writes nothing.
//!
//! **What it is not.** Not a log framework and not a tracer: no levels, no
//! subscriber, no spans in flight, no second time unit. `us` — microseconds,
//! integer — is the one time noun, the same one the wire frame's
//! `meta.duration_us` already uses.
//!
//! # Two line shapes, and the space that tells them apart
//!
//! A MEASUREMENT opens `mrd-timing` + a SPACE, and always carries exactly four
//! `key=value` fields in this order:
//!
//! ```text
//! mrd-timing cmd=run who=p41273.t1 phase=snapshot.read us=402118
//! ```
//!
//! A DIAGNOSTIC about the mode itself opens `mrd-timing` + a COLON:
//!
//! ```text
//! mrd-timing: cannot open `/nope/t.log` (No such file or directory) — writing to stderr instead.
//! ```
//!
//! So `grep '^mrd-timing '` is exactly the measurements and nothing else. Every
//! field is guarded to keep that true: [`label`] refuses a value carrying
//! whitespace or a control character, because one would mint a field the
//! grammar does not have.
//!
//! # A line means the phase COMPLETED
//!
//! [`Phase::stop`] is the only emitter. There is no `Drop` impl: a span that
//! goes out of scope without `stop` — the `?` on a failed load, an early
//! `return Err` — reports NOTHING. A failed page load costing 312 us and a
//! successful one costing 312 us are not the same fact, and a mode that
//! printed both identically would be inviting the reader to add them up.
//!
//! `cmd=` names the PROCESS's entry verb (set once by [`label`]), never one
//! request — a daemon serving many ops reports `cmd=daemon` for all of them.
//!
//! `who=` names the EMITTER inside that process: `p<pid>.t<n>`, where `n` is a
//! per-process thread ordinal minted on that thread's first line ([`who`]). The
//! daemon serves one connection per thread, so two ops in flight at the same
//! moment carry different `who=` and demultiplex; and one file that several
//! processes append to separates by the `p` half.
//!
//! **What `who=` is NOT: a request id.** A thread that serves two requests one
//! after the other emits both under one `who=`, so the field separates
//! CONCURRENT work, never successive work on one thread. For a single request's
//! server-side total the honest number is still the wire frame's
//! `meta.duration_us`.
//!
//! **A dotted name marks a PART of the phase it prefixes** (`snapshot.walk` is
//! inside `snapshot`). It is not the whole containment rule: `dispatch`
//! contains `snapshot`, `eval` and `apply` with no dot in sight, and `total`
//! contains everything. Do not sum the lines. Which phase contains which is a
//! table: `docs/run-plane.md` § Timing phases.
//!
//! Adding a phase is two lines at the site that owns it — the second one is
//! where the phase COMPLETED, which is the whole point:
//!
//! ```no_run
//! let read = timing::phase("snapshot.read");
//! // ... the work ...
//! read.stop();
//! ```
//!
//! Surface and lanes: `docs/status.md` § The timing mode.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// The environment switch. Its VALUE is the sink: an off word, an on word, or
/// a file path (`docs/status.md` § The timing mode).
pub const SWITCH: &str = "MRD_TIMING";

/// The fixed first token of every emitted line. A measurement follows it with a
/// SPACE, a diagnostic with a COLON — see the module docs.
pub const PREFIX: &str = "mrd-timing";

/// The `cmd=` a process that never called [`label`] reports.
const DEFAULT_LABEL: &str = "mrd";

/// Values that mean OFF, matched trimmed and case-insensitively. Spelled out
/// rather than "anything but the on words" because the fallback arm CREATES A
/// FILE: without these, `MRD_TIMING=off` would write a file named `off` into
/// the caller's working directory.
const OFF_WORDS: [&str; 4] = ["0", "off", "false", "no"];

/// Values that mean the stderr sink, matched trimmed and case-insensitively.
const ON_WORDS: [&str; 4] = ["1", "on", "true", "yes"];

/// Extensions the engine's own hash domain walks. A sink with one of these is
/// refused: the file would be a corpus member, so every line it gains changes
/// the corpus, and the next fold reports more lines to append. That is a loop
/// with an appetite, and it is easier to refuse the extension than to explain
/// the disk that filled.
const CORPUS_EXTENSIONS: [&str; 2] = ["md", "base"];

/// Where completed phases are written.
#[derive(Debug)]
enum Sink {
    /// The switch is absent or names an off word: nothing is measured.
    Off,
    /// The process's own stderr.
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

/// One diagnostic about the mode itself — the COLON shape, so it can never be
/// read as a measurement. Public because the mode's audibility is not only this
/// crate's business: the process that SPAWNS a voiceless child (`mrd`'s
/// `daemon::spawn_detached`) says so in this same shape, on the lane that is
/// heard, rather than minting a second one.
///
/// **It goes to stderr — which a detached daemon does not have.** The auto-spawn
/// nulls the child's stdio, so while the mode is ON `mrd` hands the daemon a
/// file for its stderr instead (`<socket-stem>.log` beside the socket), and
/// every diagnostic and measurement below lands there. Outside that arm — a
/// daemon someone else started with a null stderr — the complaint is still
/// inaudible, and the only confirmation of the mode is that the sink file
/// GROWS. `docs/status.md` § The timing mode.
pub fn diagnostic(text: &str) {
    // The whole message is folded to one line, not merely the paths inside it:
    // an `io::Error` renders text this crate never chose either.
    let _ = std::io::stderr().write_all(format!("{PREFIX}: {}\n", one_line(text)).as_bytes());
}

/// Text as it may appear on a diagnostic LINE: control characters — a newline
/// above all — are replaced, because a raw one would split the diagnostic in
/// two and the tail would carry no prefix at all. The same defence [`label`]
/// applies to `cmd=`, for the same reason: a value from outside must not be
/// able to mint a line.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// Classify a raw switch value.
///
/// Whitespace is trimmed and the words match case-insensitively, because the
/// value arrives from a `.env` line, a `docker -e`, or a shell export where
/// `OFF` and `"1 "` are the same intent as `off` and `1` — and where guessing
/// wrong does not merely disable the mode, it CREATES A FILE named `OFF`.
/// (Measured on the release binary, 2026-08-22: it did exactly that.)
fn choose(raw: Option<&OsStr>) -> Chosen<'_> {
    let Some(raw) = raw else {
        return Chosen::Off;
    };
    // A non-UTF-8 value can be neither trimmed nor case-folded safely, and
    // every word is ASCII, so nothing but a path can be meant by one.
    let Some(text) = raw.to_str() else {
        return Chosen::Path(raw);
    };
    let text = text.trim();
    if text.is_empty() {
        return Chosen::Off;
    }
    if OFF_WORDS.iter().any(|word| text.eq_ignore_ascii_case(word)) {
        return Chosen::Off;
    }
    if ON_WORDS.iter().any(|word| text.eq_ignore_ascii_case(word)) {
        return Chosen::Stderr;
    }
    Chosen::Path(OsStr::new(text))
}

/// Would this sink be a member of the engine's own hash domain?
fn is_corpus_extension(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
        CORPUS_EXTENSIONS
            .iter()
            .any(|c| ext.eq_ignore_ascii_case(c))
    })
}

/// Read the switch and open the sink. Runs at most once per process, so every
/// diagnostic below is said exactly once.
fn resolve() -> Sink {
    let raw = std::env::var_os(SWITCH);
    match choose(raw.as_deref()) {
        Chosen::Off => Sink::Off,
        Chosen::Stderr => Sink::Stderr,
        Chosen::Path(path) => open_file_sink(Path::new(path)),
    }
}

/// The file arm. Both refusals DEGRADE TO STDERR and say why: the caller asked
/// for the mode, and silence would read as "the code never ran there" — the one
/// answer this instrument must never fake.
fn open_file_sink(path: &Path) -> Sink {
    if is_corpus_extension(path) {
        diagnostic(&format!(
            "`{}` names a corpus extension — the engine folds those, so every \
             line the sink gained would change the corpus and the fold would \
             report more lines to append. Writing to stderr instead; name a \
             sink that is not .md or .base.",
            path.display()
        ));
        return Sink::Stderr;
    }
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => Sink::File(Mutex::new(file)),
        Err(error) => {
            diagnostic(&format!(
                "cannot open `{}` ({error}) — writing to stderr instead.",
                path.display()
            ));
            Sink::Stderr
        }
    }
}

fn sink() -> &'static Sink {
    SINK.get_or_init(resolve)
}

/// Is the mode on? After the first call this is one atomic load.
#[inline]
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
///
/// **A label carrying whitespace or a control character is REFUSED** and the
/// default stands. The label rides a space-separated line, so `mrd "run p.md"`
/// would otherwise emit a four-field `cmd=run p.md …` and a value carrying a
/// newline would inject a line into a machine-read stream. Callers hand this
/// raw argv (`args.first()`), so the grammar is defended here rather than at
/// every call site.
pub fn label(name: &str) {
    if !on() {
        return;
    }
    if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return;
    }
    let _ = LABEL.set(name.to_owned());
}

/// This process's id, read once — the `p` half of [`who`].
static PID: OnceLock<u32> = OnceLock::new();

/// The next thread ordinal to hand out. Ordinals start at 1, so `t0` is free to
/// mean "no ordinal could be minted" (see [`who`]).
static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// This thread's ordinal, minted on the thread's FIRST emitted line — so
    /// the ordinals are dense over the threads that actually measured
    /// something, and a process that never emits mints none.
    static THREAD_ORDINAL: u64 = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
}

/// The `who=` field: `p<pid>.t<n>` — which process, and which thread inside it.
///
/// This is the DISCRIMINATOR that makes one sink file demultiplexable. The
/// daemon serves one connection per thread (`registry::server` § accept loop),
/// so ops in flight at the same moment carry different `t`; several processes
/// appending to one file separate by `p`.
///
/// It names the emitter, NOT the request: a thread reused for a later request
/// keeps its ordinal. `t0` means the ordinal could not be minted — a phase
/// stopped while this thread's TLS was already being destroyed — which is a
/// value, not a panic.
///
/// Both halves are digits, so no `who=` can ever mint a field the grammar does
/// not have.
#[must_use]
pub fn who() -> String {
    let pid = *PID.get_or_init(std::process::id);
    let thread = THREAD_ORDINAL.try_with(|ordinal| *ordinal).unwrap_or(0);
    format!("p{pid}.t{thread}")
}

/// One measurement line, exactly as it is written.
fn line(cmd: &str, who: &str, phase: &str, us: u128) -> String {
    format!("{PREFIX} cmd={cmd} who={who} phase={phase} us={us}\n")
}

fn emit(phase: &str, elapsed: Duration) {
    let cmd = LABEL.get().map_or(DEFAULT_LABEL, String::as_str);
    let line = line(cmd, &who(), phase, elapsed.as_micros());
    match sink() {
        // Unreachable by construction — a `Phase` holds an `Instant` only when
        // the sink is on. Kept, and kept silent, so that a future caller who
        // reaches `emit` by another road degrades instead of writing to a sink
        // the operator switched off.
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

/// One phase being measured. Report it with [`Phase::stop`], AT THE POINT IT
/// COMPLETED — there is no `Drop` impl, so a span abandoned on an error path
/// says nothing.
///
/// When the mode is off it holds no [`Instant`] — nothing was read, so nothing
/// can be reported.
#[derive(Debug)]
#[must_use = "a phase reports only where you call .stop(); dropping it measures nothing"]
pub struct Phase {
    name: &'static str,
    started: Option<Instant>,
}

impl Phase {
    /// Report this phase as COMPLETED, here.
    #[inline]
    pub fn stop(self) {
        if let Some(started) = self.started {
            emit(self.name, started.elapsed());
        }
    }
}

/// Start measuring `name`. Off, this reads no clock and allocates nothing.
#[inline]
pub fn phase(name: &'static str) -> Phase {
    Phase {
        name,
        started: on().then(Instant::now),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Chosen, DEFAULT_LABEL, OFF_WORDS, ON_WORDS, PREFIX, Phase, choose, is_corpus_extension,
        line, one_line, who,
    };
    use std::collections::BTreeSet;
    use std::ffi::OsStr;
    use std::path::Path;

    /// The line a consumer parses: prefix, then four `key=value` fields in a
    /// fixed order, space-separated, newline-terminated.
    fn parse(raw: &str) -> Option<(String, String, String, u128)> {
        let body = raw.strip_suffix('\n')?;
        let mut fields = body.split(' ');
        if fields.next()? != PREFIX {
            return None;
        }
        let cmd = fields.next()?.strip_prefix("cmd=")?;
        let who = fields.next()?.strip_prefix("who=")?;
        let phase = fields.next()?.strip_prefix("phase=")?;
        let us = fields.next()?.strip_prefix("us=")?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        Some((cmd.to_owned(), who.to_owned(), phase.to_owned(), us))
    }

    #[test]
    fn the_emitted_line_parses_back() {
        let raw = line("run", "p41273.t1", "snapshot.read", 402_118);
        assert_eq!(
            raw,
            "mrd-timing cmd=run who=p41273.t1 phase=snapshot.read us=402118\n"
        );
        let (cmd, who, phase, us) = parse(&raw).expect("the line this crate writes parses");
        assert_eq!(cmd, "run");
        assert_eq!(who, "p41273.t1");
        assert_eq!(phase, "snapshot.read");
        assert_eq!(us, 402_118);
    }

    /// `who=` is what makes one sink file demultiplexable, so its shape is the
    /// contract: `p<digits>.t<digits>`, no whitespace — a value that could
    /// carry either would mint a field the grammar does not have.
    #[test]
    fn who_names_this_process_and_this_thread() {
        let mine = who();
        let (pid, thread) = mine
            .strip_prefix('p')
            .and_then(|rest| rest.split_once(".t"))
            .unwrap_or_else(|| panic!("who() is not p<pid>.t<n>: {mine}"));
        assert_eq!(
            pid.parse::<u32>().expect("pid is digits"),
            std::process::id()
        );
        assert!(thread.parse::<u64>().is_ok(), "thread ordinal is digits");
        assert!(!mine.chars().any(char::is_whitespace), "{mine}");
        // Stable within a thread: two phases from one thread group together.
        assert_eq!(mine, who());
    }

    /// The claim the daemon lane rests on: work in flight at the same moment
    /// carries different `who=`. One thread per connection there; N threads
    /// here, and the ordinals must be N distinct values.
    #[test]
    fn concurrent_threads_get_distinct_ordinals() {
        let seen: BTreeSet<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(who)).collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("thread"))
                .collect()
        });
        assert_eq!(seen.len(), 8, "ordinals collided: {seen:?}");
    }

    /// A diagnostic must never be mistaken for a measurement: the colon shape
    /// fails the parser, and it does not match the documented
    /// `grep '^mrd-timing '`.
    #[test]
    fn a_diagnostic_is_not_a_measurement() {
        let text = "mrd-timing: cannot open `/nope` (nope) — writing to stderr instead.\n";
        assert!(parse(text).is_none());
        assert!(!text.starts_with("mrd-timing "));
        assert!(text.starts_with(PREFIX));
    }

    /// A zero-microsecond phase is a measurement, not a missing field — the
    /// integer is always present, so a consumer summing `us` never guesses.
    #[test]
    fn a_zero_phase_still_carries_its_field() {
        let (_, _, _, us) = parse(&line(DEFAULT_LABEL, &who(), "cascade", 0)).expect("parses");
        assert_eq!(us, 0);
    }

    #[test]
    fn off_words_and_absence_and_empty_are_off() {
        assert_eq!(choose(None), Chosen::Off);
        assert_eq!(choose(Some(OsStr::new(""))), Chosen::Off);
        assert_eq!(choose(Some(OsStr::new("   "))), Chosen::Off);
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

    /// The regression that made this a rule: on the release binary,
    /// `MRD_TIMING=OFF` and `MRD_TIMING="1 "` each created a file in the
    /// caller's working directory — the exact hazard the off words exist to
    /// prevent, defeated by a shift key and a stray space.
    #[test]
    fn case_and_whitespace_do_not_turn_a_word_into_a_file() {
        for word in ["OFF", "Off", "FALSE", " off ", "\tno\n", "0 "] {
            assert_eq!(choose(Some(OsStr::new(word))), Chosen::Off, "{word:?}");
        }
        for word in ["ON", "True", " 1 ", "YES\n"] {
            assert_eq!(choose(Some(OsStr::new(word))), Chosen::Stderr, "{word:?}");
        }
    }

    /// Anything that is not a word is a path — including a path that merely
    /// begins with one, which is why the match is on the whole value. A path
    /// is served TRIMMED, the same normalization the words get.
    #[test]
    fn any_other_value_is_a_path() {
        for value in ["/tmp/t.log", "timing.log", "./1", "on.log", "1x"] {
            assert_eq!(
                choose(Some(OsStr::new(value))),
                Chosen::Path(OsStr::new(value)),
                "{value}"
            );
        }
        assert_eq!(
            choose(Some(OsStr::new("  /tmp/t.log  "))),
            Chosen::Path(OsStr::new("/tmp/t.log"))
        );
    }

    /// A sink the engine would fold is a loop, so the extension is refused
    /// before the file is ever opened.
    #[test]
    fn corpus_extensions_are_refused_as_sinks() {
        for path in ["t.md", "notes/t.md", "board.base", "T.MD", "x.Base"] {
            assert!(is_corpus_extension(Path::new(path)), "{path}");
        }
        for path in ["t.log", "t.txt", "timing", "t.md.log", "a.markdown"] {
            assert!(!is_corpus_extension(Path::new(path)), "{path}");
        }
    }

    /// A diagnostic carries text nobody in this crate chose — a sink path from
    /// argv, an `io::Error` rendered by the OS. A newline in either would split
    /// the line and leave the tail carrying no prefix at all, so the WHOLE
    /// message is folded, not just the path inside it. Same defence as
    /// `label`'s on `cmd=`.
    #[test]
    fn a_control_character_in_a_diagnostic_cannot_split_the_line() {
        let rendered = one_line(&format!(
            "cannot open `{}`",
            Path::new("a\nmrd-timing cmd=x who=p1.t1 phase=y us=1").display()
        ));
        assert!(!rendered.contains('\n'), "{rendered}");
        assert_eq!(rendered.lines().count(), 1);
        // And ordinary text is untouched.
        assert_eq!(one_line("/tmp/t.log"), "/tmp/t.log");
    }

    /// The off path holds no `Instant`: the switch is read at construction, and
    /// off construction reads no clock. Stopping it reports nothing.
    #[test]
    fn an_off_phase_holds_no_instant_and_reports_nothing() {
        let phase = Phase {
            name: "probe",
            started: None,
        };
        assert!(phase.started.is_none());
        phase.stop();
    }
}
