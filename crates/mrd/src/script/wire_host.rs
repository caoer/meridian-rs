//! The script entry in **wire-client mode**: the one door, dialled as an
//! ordinary client (`docs/run-plane.md` § The script entry, "Wire-client mode").
//!
//! Two things live here. [`Door`] is one NDJSON round trip, so the wire client
//! is testable without a daemon and the ops it puts on the socket are
//! observable. [`WireHost`] is the `effects::ScriptHost` whose `read()` lowers
//! to `toc`/`cat` — the entry's only effectful seam.
//!
//! **Zero wire delta.** Every op this module emits is an op the contract already
//! declares: `hello` (§3.2), `fingerprint` (§4.7), `toc` (§4.1), `cat` (§4.2),
//! and — from [`super::cmd`] — `splice` (§4.4). Nothing here invents a request
//! shape, and no response is re-serialized on its way into the trace.
//!
//! **A response line is carried as BYTES.** [`Door::call`] answers the raw
//! response line, not a parsed value, because the commit leg is embedded in
//! `ScriptTrace` verbatim: `serde_json::Value` sorts object keys and normalizes
//! whitespace, which would mint a second commit-fact shape (U3's law).

use std::io::{self, BufReader};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use effects::{ReadFault, ScriptHost, SecFacts, TocEntry, TocFacts};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use wire::ReadSel;

/// One NDJSON round trip on the one door: the request goes out, the whole
/// response line comes back as its own bytes.
///
/// `Send` because the kernel evaluates on its large-stack thread and the host
/// (holding a door) is moved onto it.
///
/// # Errors
/// Any transport failure — the connection closed, the write failed, the daemon
/// answered nothing.
pub trait Door: Send {
    /// Send `request`, return the response line verbatim.
    ///
    /// # Errors
    /// The transport failed; the script aborts rather than guessing.
    fn call(&mut self, request: &Value) -> io::Result<String>;
}

/// The production door: a connected daemon socket, already bound to a workspace
/// by a v3 `hello`.
pub struct SocketDoor {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl SocketDoor {
    /// Dial `socket` and bind the connection to `workspace` with the §3.2 hello
    /// frame — proto 1, contract v3, copied from the one the read verbs send so
    /// the two clients cannot negotiate differently.
    ///
    /// **The connection carries the wall clock** (§ Where the budgets bind,
    /// layer 2). A socket with no timeout parks the process forever on a daemon
    /// that accepts a frame and never answers, and a parked process never
    /// reaches the deadline check that would have refused: the check above the
    /// socket bounds the number of calls, and only the socket bounds one call.
    /// One round trip may therefore not exceed the entry's WHOLE wall clock,
    /// which is the loosest bound that is still a bound.
    ///
    /// # Errors
    /// The socket refuses the connection, the transport fails, or the daemon
    /// refuses the handshake.
    pub fn connect(socket: &Path, workspace: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(super::cmd::WALL_CLOCK))?;
        stream.set_write_timeout(Some(super::cmd::WALL_CLOCK))?;
        let writer = stream.try_clone()?;
        let reader = BufReader::new(stream);
        let mut door = Self { writer, reader };
        let hello = json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": workspace.to_string_lossy(),
        });
        let line = door.call(&hello)?;
        let frame = Frame::parse(&line)?;
        if !frame.ok {
            // The daemon names WHAT refused (code, path, cause) in its error
            // frame; collapsing it to a static string cost a dogfood session a
            // fleet-wide outage nobody could attribute. Carry the frame's error
            // verbatim — RawValue bytes, never re-serialized.
            return Err(io::Error::other(format!(
                "the daemon refused the v3 handshake for this workspace: {}",
                frame
                    .error
                    .map_or_else(|| "(no error body)".to_owned(), |e| e.get().to_owned()),
            )));
        }
        // 0025 socket law: identity equality on the hello frame already in
        // hand (one voice with the read/links lanes). Parsed once, at connect.
        let body: Option<Value> = match frame.body.as_deref() {
            Some(raw) => Some(serde_json::from_str(raw.get()).map_err(io::Error::other)?),
            None => None,
        };
        if let Err(message) = crate::engine::hello_identity_skew(body.as_ref(), socket) {
            return Err(io::Error::other(message));
        }
        Ok(door)
    }
}

impl Door for SocketDoor {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        crate::engine::call_line(&mut self.writer, &mut self.reader, request)
    }
}

/// One response frame, split without disturbing its bytes: `body` and `error`
/// stay [`RawValue`], so whichever one reaches the trace reaches it verbatim.
#[derive(Debug, Deserialize)]
pub(crate) struct Frame {
    #[serde(default)]
    pub(crate) ok: bool,
    #[serde(default)]
    pub(crate) body: Option<Box<RawValue>>,
    #[serde(default)]
    pub(crate) error: Option<Box<RawValue>>,
}

impl Frame {
    /// Split one response line.
    ///
    /// # Errors
    /// The line is not a JSON object — a transport-grade failure.
    pub(crate) fn parse(line: &str) -> io::Result<Self> {
        serde_json::from_str(line).map_err(io::Error::other)
    }

    /// The success body as a parsed value, or a transport error naming what the
    /// daemon answered instead.
    fn body_value(self, op: &str) -> io::Result<Value> {
        if !self.ok {
            // `self.error`, NOT `self.body`: the original match bound its
            // refusal arm to the body slot — always absent on a refusal — so
            // every daemon refusal rendered "(no error body)" and the error
            // frame the daemon actually sent (code, path, cause) was dropped
            // on the floor. Same incident class as the connect-time static
            // string: the daemon names the poison, the operator never sees it.
            return Err(io::Error::other(format!(
                "{op} refused: {}",
                self.error
                    .map_or_else(|| "(no error body)".to_owned(), |e| e.get().to_owned())
            )));
        }
        match self.body {
            Some(body) => serde_json::from_str(body.get()).map_err(io::Error::other),
            None => Err(io::Error::other(format!("{op}: ok frame with no body"))),
        }
    }
}

/// The wire-backed [`ScriptHost`]: `read(path)` lowers to `toc` (§4.1) plus the
/// composed `read` (§4.1, toc mode) that brackets it, and
/// `read(path, section=…)` lowers to `cat` (§4.2).
///
/// **Two trips, and why not more** *(amended 2026-08-08)*. `fm` used to cost one
/// `cat` per frontmatter key, on the stated grounds that no wire op served
/// frontmatter VALUES — the composed read's "body carries no frontmatter
/// plane". That stopped being true when the composed read grew its `props[]`
/// plane (`wire-serve::read` `read_props`): one row per top-level key in
/// document order, each value decoded by the one § A.6 codec. The comment
/// outlived the constraint and the fan-out outlived it too — an ordinary
/// session artifact carries five to eight keys, so a single script read was
/// seven to ten round trips, each paying a corpus-scoped currency pass.
///
/// The frontmatter now rides the bracket op that was already being sent, so a
/// whole-file read is TWO trips whatever its key count.
///
/// **And why not ONE.** The `toc` op's nodes carry `^anchor` rows with their own
/// `node_rev`; the composed read's anchors plane does not
/// (`wire::ReadAnchor` is `{anchor, span}`, and `wire::ReadRow` has no anchor
/// field). Dropping anchor rows from the face a script sees would be a silent
/// change to observable behaviour, so the `toc` trip stays.
pub(crate) struct WireHost<'d> {
    door: &'d mut dyn Door,
    actor: String,
    /// The entry's wall clock (§ Where the budgets bind). Checked before EVERY
    /// round trip, which is the only place a script spends unbounded time: pure
    /// evaluation is fuel-bounded, and a blocked read spends no fuel at all.
    deadline: Instant,
}

impl<'d> WireHost<'d> {
    /// A host that reads through `door` as `actor`, refusing reads past
    /// `deadline`.
    pub(crate) fn new(door: &'d mut dyn Door, actor: String, deadline: Instant) -> Self {
        Self {
            door,
            actor,
            deadline,
        }
    }

    /// One round trip, mapped into the read-fault shape the script plane speaks.
    ///
    /// **The clock is checked here, not at the script-level read**, because a
    /// script-level read is more than one round trip and an attempt is many: a
    /// check per script-level read bounds the NUMBER of checks and not the time
    /// (§ Where the budgets bind, layer 1).
    fn ask(
        &mut self,
        request: &Value,
        fault: &dyn Fn(String) -> ReadFault,
    ) -> Result<Value, ReadFault> {
        self.within_deadline(fault)?;
        let op = request["op"].as_str().unwrap_or("(op)").to_owned();
        let line = self
            .door
            .call(request)
            .map_err(|e| fault(format!("the daemon did not answer: {e}")))?;
        Frame::parse(&line)
            .and_then(|frame| frame.body_value(&op))
            .map_err(|e| fault(e.to_string()))
    }

    /// Refuse a wire call that starts past the host's wall clock.
    ///
    /// The wording claims nothing about armed state, because the host cannot
    /// see it: arms are the KERNEL's list, and this refusal renders directly
    /// below that list's rows on the face. What it does claim is what the host
    /// knows — a failed attempt sends no armed edit and commits nothing
    /// (`cmd::run` never issues a splice for a failed eval).
    fn within_deadline(&self, fault: &dyn Fn(String) -> ReadFault) -> Result<(), ReadFault> {
        if Instant::now() > self.deadline {
            return Err(fault(
                "the script entry's wall clock elapsed before this read — the attempt aborts \
                 whole: any edits the script armed are never sent, and nothing commits"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

impl ScriptHost for WireHost<'_> {
    // `_armed` ignored on purpose: this lane's reads are LIVE (run-plane § the
    // entry world names the lane split) — the wire client serves disk truth,
    // and read-your-own-writes belongs to the in-process entry world only.
    fn toc(&mut self, path: &str, _armed: &[effects::ArmedEdit]) -> Result<TocFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: None,
            reason,
        };
        let body = self.ask(&json!({"op": "toc", "path": path}), &fault)?;

        let rev = body
            .get("file_rev")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("toc answered no file_rev".to_owned()))?
            .to_owned();
        let nodes = body
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| fault("toc answered no nodes".to_owned()))?
            .clone();

        // THE BRACKET, the count, AND the frontmatter, in one op. The word
        // count is a DELIVERED fact, never one this host computes: the composed
        // `read` op (§4.1, toc mode) carries `words_total`, while the `toc`
        // op's own body is
        // `{path, file_rev, root, nodes}` and carries none. It also carries the
        // `file_rev` this composition has to agree with — so asking it LAST
        // closes the window instead of sitting inside it, and the coherence
        // check costs no round trip that was not already being made.
        //
        // Reads are LIVE. Without this the face could be assembled from a `toc`
        // at one revision and a word count at another: a state that never
        // existed, handed to a script whose whole premise is that the world
        // stood still. Zero wire delta — both ops are already declared, and a
        // toc-mode read mints no receipt (`wire-serve::read`).
        let read_body = self.ask(&json!({"op": "read", "path": path}), &fault)?;
        let closing_rev = read_body
            .get("file_rev")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("read answered no file_rev".to_owned()))?;
        if closing_rev != rev {
            return Err(fault(format!(
                "the record moved while this read was being composed — `toc` answered file_rev \
                 {rev} and the closing `read` answered {closing_rev}. A face composed from two \
                 revisions would be a state that never existed, so this read refuses whole: \
                 nothing was armed by it and nothing commits. Reads are live — re-read"
            )));
        }
        let words = usize::try_from(
            read_body
                .get("words_total")
                .and_then(Value::as_u64)
                .ok_or_else(|| fault("read answered no words_total".to_owned()))?,
        )
        .map_err(|_| fault("read answered a words_total this host cannot hold".to_owned()))?;

        // The frontmatter plane rides the SAME op as the bracket, so `fm` and the
        // `file_rev` that guards it come from one snapshot. Values arrive
        // decoded by the one § A.6 codec (`wire-serve::read` `read_props`) —
        // this host does not decode them a second time, which would strip a
        // second layer from a value that is still quote-shaped after the first.
        let mut fm = std::collections::BTreeMap::new();
        for prop in read_body
            .get("props")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(key), Some(value)) = (
                prop.get("key").and_then(Value::as_str),
                prop.get("value").and_then(Value::as_str),
            ) else {
                return Err(fault(
                    "the composed read answered a prop with no key/value".to_owned(),
                ));
            };
            fm.insert(key.to_owned(), value.to_owned());
        }

        Ok(TocFacts {
            rev,
            fm,
            toc: nodes.iter().filter_map(toc_entry).collect(),
            words,
        })
    }

    fn cat(
        &mut self,
        path: &str,
        section: &str,
        _armed: &[effects::ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(section.to_owned()),
            reason,
        };
        let sec = sec_ref(section).ok_or_else(|| {
            fault(
                "a dewey ordinal addresses a row of a table you are holding, not a document — \
                 pass the heading path or a ^anchor"
                    .to_owned(),
            )
        })?;
        let body = self.ask(&json!({"op": "cat", "path": path, "sec": sec}), &fault)?;
        let text = body
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("cat answered no content".to_owned()))?
            .to_owned();
        let rev = body
            .get("node_rev")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("cat answered no node_rev".to_owned()))?
            .to_owned();
        Ok(SecFacts { text, rev })
    }

    fn actor(&self) -> &str {
        &self.actor
    }
}

/// The `sec` selector for a `section=` string, through [`ReadSel::parse`] — the
/// one human-string→selector door in the tree, so the script face and every
/// other face parse an address the same way.
///
/// `None` for the dewey arm: `cat` has no dewey target (§4.2 takes hpath /
/// anchor / `fm_key`), and a positional ordinal is not a document address.
fn sec_ref(section: &str) -> Option<Value> {
    match ReadSel::parse(section) {
        ReadSel::Hpath { hpath } => Some(json!({ "hpath": hpath })),
        ReadSel::Anchor { anchor } => Some(json!({ "anchor": anchor })),
        ReadSel::Dewey { .. } => None,
    }
}

/// One toc row → one script-face entry, or nothing.
///
/// Two row shapes reach the face, and both are addresses a script can pass
/// straight back into `read(path, section=…)`: a heading row publishes its
/// hpath joined by `/` (what [`ReadSel::parse`] splits again), and an
/// anchor-bearing block row publishes its `^id`. The frontmatter row is not a
/// section — it reaches the face as `fm`.
fn toc_entry(node: &Value) -> Option<TocEntry> {
    let rev = node.get("node_rev").and_then(Value::as_str)?.to_owned();
    let anchor = node
        .get("anchor")
        .and_then(Value::as_str)
        .map(|id| format!("^{id}"));
    let section = match node.get("hpath").and_then(Value::as_array) {
        Some(hpath) => hpath
            .iter()
            .filter_map(|seg| seg.get("h").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("/"),
        None => anchor.clone()?,
    };
    Some(TocEntry {
        section,
        anchor,
        rev,
    })
}

#[cfg(test)]
mod tests {
    //! The two bounds a composed read has to carry, each proved by MEASUREMENT
    //! against a control run of the same door: the wall clock binds per ROUND
    //! TRIP, and the composition is bracketed by `file_rev`.
    //!
    //! These sit here rather than in `tests/` because [`WireHost`] takes its
    //! deadline as a parameter, which is the only seam that can drive the clock
    //! without a seven-second sleep. No wall-clock BUDGET is asserted anywhere
    //! below — only that a bound fired and how many calls it let through.

    use std::io::{BufRead as _, Write as _};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::{Door, Instant, ScriptHost, SocketDoor, Value, WireHost, io, json};

    /// The one page the fake door serves. Two frontmatter keys — which now cost
    /// no round trips of their own: a whole-file read is TWO trips, `toc` and
    /// the closing composed `read` that carries both the bracket and `props[]`.
    const PAGE: &str = "tasks/0011-token-audit.md";
    const REV: &str = "7c40e1a8b2f9d356";

    /// A door that spends `per_call` on every round trip and answers the
    /// composition. `moved_rev` is what the CLOSING `read` reports as its
    /// `file_rev`; `None` means the world stood still.
    struct Fake {
        calls: usize,
        /// Every op asked for, in order — so a test can assert the SHAPE of a
        /// composition and not merely how many trips it took.
        ops: Vec<String>,
        per_call: Duration,
        moved_rev: Option<&'static str>,
    }

    impl Fake {
        fn still() -> Self {
            Self {
                calls: 0,
                ops: Vec::new(),
                per_call: Duration::ZERO,
                moved_rev: None,
            }
        }

        fn slow(per_call: Duration) -> Self {
            Self {
                per_call,
                ..Self::still()
            }
        }

        fn moving_to(rev: &'static str) -> Self {
            Self {
                moved_rev: Some(rev),
                ..Self::still()
            }
        }
    }

    impl Door for Fake {
        fn call(&mut self, request: &Value) -> io::Result<String> {
            self.calls += 1;
            thread::sleep(self.per_call);
            let op = request["op"].as_str().expect("every request names an op");
            self.ops.push(op.to_owned());
            Ok(match op {
                "toc" => json!({"ok": true, "body": {
                    "path": PAGE, "file_rev": REV, "root": "b3:00",
                    "nodes": [
                        {"kind": "frontmatter", "span": [0, 32], "node_rev": "26796ebe",
                         "text_prefix_16b": "---\n", "keys": ["owner", "status"]},
                    ],
                }}),
                "cat" => json!({"ok": true, "body": {
                    "span": [4, 12], "node_rev": "33d5b0e1", "content": "owner: zt\n",
                }}),
                "read" => json!({"ok": true, "body": {
                    "path": PAGE, "file_rev": self.moved_rev.unwrap_or(REV),
                    "root": "b3:00", "words_total": 41,
                    "toc": [], "anchors": [], "rendered_text": "",
                    // The frontmatter plane, decoded, on the op that already
                    // brackets the composition — what used to be one `cat` each.
                    "props": [
                        {"key": "owner", "value": "zt", "span": [4, 13],
                         "prop_rev": "33d5b0e1"},
                        {"key": "status", "value": "open", "span": [14, 26],
                         "prop_rev": "41f643f0"},
                    ],
                }}),
                other => panic!("the composed read asked for an op it must not know: {other}"),
            }
            .to_string())
        }
    }

    /// **The bound fires mid-flight — the control run proves the door is
    /// otherwise willing.** *(rewritten 2026-08-08: a whole-file read is two
    /// round trips now, not four, so the law is driven by a program with TWO
    /// READS — four trips in total — which is what keeps "once per read" and
    /// "once per round trip" distinguishable at all.)*
    ///
    /// The arithmetic IS the distinction. The door spends 40ms a trip and the
    /// bounded deadline is 100ms, so trips are checked at 0/40/80/120: a
    /// per-TRIP clock refuses the fourth and answers a fault. A clock checked
    /// once per script-level read would check only at 0 and 80, pass both, and
    /// hand back two complete faces at 160ms — bounding the number of checks
    /// and not the time, which is the shape this test exists to forbid.
    #[test]
    fn the_wall_clock_is_checked_before_every_round_trip_not_once_per_read() {
        // Control: the same door, a deadline nothing can reach.
        let mut open = Fake::slow(Duration::from_millis(40));
        let facts = {
            let mut host = WireHost::new(
                &mut open,
                "zt".to_owned(),
                Instant::now() + Duration::from_secs(30),
            );
            host.toc(PAGE, &[])
                .expect("the control's first read answers");
            host.toc(PAGE, &[])
                .expect("the control's second read answers")
        };
        assert_eq!(facts.rev, REV);
        assert_eq!(
            open.calls, 4,
            "the control makes both compositions: two reads x (toc + the closing read)"
        );

        // Measured: a deadline the FOURTH trip crosses, which a per-read clock
        // would never test.
        let mut bounded = Fake::slow(Duration::from_millis(40));
        let fault = {
            let mut host = WireHost::new(
                &mut bounded,
                "zt".to_owned(),
                Instant::now() + Duration::from_millis(100),
            );
            host.toc(PAGE, &[])
                .expect("the first read is inside the bound");
            host.toc(PAGE, &[])
                .expect_err("the second read crosses it mid-composition")
        };
        assert!(
            fault.reason.contains("wall clock elapsed"),
            "the refusal names the clock that fired: {}",
            fault.reason
        );
        assert!(
            bounded.calls < 4,
            "the clock stopped the second composition mid-flight; it made {} of 4 calls",
            bounded.calls
        );
    }

    /// **The frontmatter plane costs no round trips of its own** *(added
    /// 2026-08-08)*. `fm` used to be one `cat` per key, so a page's read
    /// amplification scaled with its frontmatter — the dominant cost of a
    /// script read against a real corpus. It now rides the closing composed
    /// read's `props[]`, already decoded by the § A.6 codec.
    ///
    /// Measured, not asserted: two keys, two calls, and the op SEQUENCE is
    /// `toc` then `read` with no `cat` at all.
    #[test]
    fn a_whole_file_read_is_two_trips_whatever_the_frontmatter_costs() {
        let mut door = Fake::still();
        let facts = {
            let mut host = WireHost::new(
                &mut door,
                "zt".to_owned(),
                Instant::now() + Duration::from_secs(30),
            );
            host.toc(PAGE, &[]).expect("the composition answers")
        };
        assert_eq!(
            door.ops,
            vec!["toc".to_owned(), "read".to_owned()],
            "two frontmatter keys, and still exactly two trips: no `cat` is sent"
        );
        assert_eq!(
            facts.fm.get("owner").map(String::as_str),
            Some("zt"),
            "the value arrives decoded, off `props[]`"
        );
        assert_eq!(facts.fm.get("status").map(String::as_str), Some("open"));
        assert_eq!(facts.words, 41, "`words_total` is still a delivered fact");
    }

    /// **A face is never composed from two revisions.** The `toc` op and the
    /// closing `read` are separate round trips against a LIVE corpus, so a
    /// writer between them would otherwise yield a map from one revision and a
    /// word count from another — a state that never existed, handed to a script
    /// whose whole premise is that the world stood still.
    ///
    /// The control run is the same door with the same sequence and one
    /// difference: the closing `read` reports the rev the `toc` reported.
    #[test]
    fn a_composition_spanning_two_revisions_refuses_instead_of_being_assembled() {
        let mut still = Fake::still();
        let facts = {
            let mut host = WireHost::new(
                &mut still,
                "zt".to_owned(),
                Instant::now() + Duration::from_secs(30),
            );
            host.toc(PAGE, &[]).expect("a still world composes")
        };
        assert_eq!(facts.rev, REV);
        assert_eq!(facts.words, 41, "the count is the closing read's own");

        let mut moving = Fake::moving_to("ffffffffffffffff");
        let fault = {
            let mut host = WireHost::new(
                &mut moving,
                "zt".to_owned(),
                Instant::now() + Duration::from_secs(30),
            );
            host.toc(PAGE, &[])
                .expect_err("a world that moved mid-composition refuses")
        };
        assert!(
            fault.reason.contains(REV) && fault.reason.contains("ffffffffffffffff"),
            "the refusal names BOTH revisions, or the reader cannot tell what moved: {}",
            fault.reason
        );
        assert_eq!(
            moving.calls, 2,
            "the bracket is the closing op itself — it costs no extra round trip"
        );
    }

    /// **A daemon that accepts and never answers must not park the process.**
    /// The check above the socket bounds the NUMBER of calls; only the socket
    /// bounds one call, and a call that never returns is a call the clock never
    /// gets to check again — the shape that hung the tool with nothing bounding
    /// it.
    ///
    /// No wall-clock BUDGET is asserted: the lower bound only says the deadline
    /// was the thing that fired, and load can lengthen it but never shorten it.
    /// The outer channel timeout exists so a regression FAILS instead of hanging
    /// the suite forever.
    #[test]
    fn a_socket_that_never_answers_fails_the_round_trip_instead_of_parking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");

        // Control: a listener that answers the hello. The dial completes.
        let answering = UnixListener::bind(&sock).expect("bind");
        let served = thread::spawn(move || {
            let (stream, _) = answering.accept().expect("accept");
            let mut reader = io::BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the hello");
            let mut w = stream;
            // The control publishes THIS build's identity: the 0025 socket law
            // refuses an identity-less local hello, and this test measures the
            // deadline, not the law.
            let frame = format!(
                "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake\",\"caps\":[],\
                 \"identity\":{{\"build\":\"{}\"}}}}}}\n",
                env!("MRD_BUILD_SHA")
            );
            w.write_all(frame.as_bytes()).expect("answer");
            w.flush().expect("flush");
        });
        SocketDoor::connect(&sock, dir.path()).expect("an answering daemon binds the connection");
        served.join().expect("the control listener finishes");
        std::fs::remove_file(&sock).expect("unlink");

        // Measured: a listener that accepts the connection and answers nothing.
        let mute = UnixListener::bind(&sock).expect("bind");
        let held = thread::spawn(move || {
            let (stream, _) = mute.accept().expect("accept");
            thread::sleep(super::super::cmd::WALL_CLOCK * 3);
            drop(stream);
        });
        let (tx, rx) = mpsc::channel();
        let dialled = sock.clone();
        let workspace = dir.path().to_owned();
        thread::spawn(move || {
            let started = Instant::now();
            let outcome = SocketDoor::connect(&dialled, &workspace);
            let _ = tx.send((outcome.is_err(), started.elapsed()));
        });
        let (refused, elapsed) = rx
            .recv_timeout(super::super::cmd::WALL_CLOCK * 2)
            .expect("connect returned — a socket with no deadline never would");
        assert!(
            refused,
            "a daemon that answers nothing fails the round trip"
        );
        assert!(
            elapsed >= super::super::cmd::WALL_CLOCK,
            "the socket's own deadline is what fired, not something faster: {elapsed:?}"
        );
        held.join().expect("the mute listener finishes");
    }

    /// **The script host refuses a foreign-build daemon at connect** (0025
    /// socket law): the handshake succeeds, the identity does not match, and
    /// the door never opens. One voice with the read/links lanes — both
    /// builds named, the verdict, the remedy.
    #[test]
    fn a_foreign_build_daemon_is_refused_at_connect_and_both_builds_are_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let foreign = "feedfacefeedfacefeedfacefeedfacefeedface";

        let listener = UnixListener::bind(&sock).expect("bind");
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = io::BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the hello");
            let mut w = stream;
            let frame = format!(
                "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake\",\"caps\":[],\
                 \"identity\":{{\"build\":\"{foreign}\"}}}}}}\n"
            );
            w.write_all(frame.as_bytes()).expect("answer");
            w.flush().expect("flush");
        });

        let Err(refusal) = SocketDoor::connect(&sock, dir.path()) else {
            panic!("a foreign-build daemon must not bind the connection")
        };
        served.join().expect("the listener finishes");

        let message = refusal.to_string();
        assert!(
            message.contains(foreign) && message.contains(env!("MRD_BUILD_SHA")),
            "the refusal names BOTH builds: {message}"
        );
        assert!(message.contains("SKEW"), "the one skew voice: {message}");
    }

    /// **The deadline refusal is pinned, and it claims nothing the host cannot
    /// see.** Armed state is the kernel's — the host holds no armed list — so
    /// the refusal names only what the host knows: the clock fired before the
    /// trip, the attempt aborts whole, any armed edits are never sent, and
    /// nothing commits. Pinned exactly: this text renders verbatim on the face
    /// after its `SCRIPT:` opener.
    #[test]
    fn the_deadline_refusal_is_pinned_and_claims_nothing_the_host_cannot_see() {
        let mut door = Fake::still();
        let fault = {
            let mut host = WireHost::new(
                &mut door,
                "zt".to_owned(),
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .unwrap(),
            );
            host.toc(PAGE, &[])
                .expect_err("an elapsed clock refuses the read")
        };
        assert_eq!(
            fault.reason,
            "the script entry's wall clock elapsed before this read — the attempt aborts \
             whole: any edits the script armed are never sent, and nothing commits"
        );
        assert_eq!(door.calls, 0, "the refusal fires before the trip is spent");
    }

    /// **The refusal must not contradict the armed row rendered above it**
    /// (defect NEW-6). A script that arms an edit and then meets the clock on a
    /// later read used to answer a face whose armed block said `armed …` while
    /// the refusal directly below said "nothing was armed by the reads that did
    /// run". The host cannot see the kernel's armed list, so the refusal may
    /// not claim armed state either way — the armed block is the face's one
    /// truth about it.
    #[test]
    fn an_armed_run_that_meets_the_clock_answers_a_face_that_does_not_deny_the_arm() {
        use super::super::trace::{CommitLeg, ScriptTrace};

        let mut door = Fake::still();
        let mut host = WireHost::new(
            &mut door,
            "zt".to_owned(),
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .unwrap(),
        );
        let ctx = effects::ScriptCtx {
            id: "script".to_owned(),
            args: std::collections::BTreeMap::new(),
            files: vec![PAGE.to_owned()],
            effects: Vec::new(),
        };
        let eval = effects::eval_script(
            &format!("put({PAGE:?}, props={{\"status\": \"done\"}})\ncard = read({PAGE:?})\n"),
            &ctx,
            effects::ScriptLimits::default(),
            &mut host,
        );
        assert_eq!(
            eval.armed.len(),
            1,
            "the put() armed before the clock fired"
        );
        assert!(eval.outcome.is_err(), "the read met the clock and aborted");

        let trace = ScriptTrace::assemble("b3:00", &eval, CommitLeg::NotIssued);
        let armed: Vec<_> = trace.armed_entries().collect();
        assert_eq!(armed.len(), 1, "the armed row renders — the honesty law");
        assert!(!armed[0].committed, "and it renders `[not committed]`");
        let fault = trace
            .fault
            .as_ref()
            .expect("the clock's refusal is the fault");
        assert!(
            fault.reason.contains("wall clock elapsed before this read"),
            "the refusal names the clock that fired: {}",
            fault.reason
        );
        assert!(
            !fault.reason.contains("nothing was armed"),
            "the refusal directly below an armed row must not deny the arm: {}",
            fault.reason
        );
        assert!(
            fault
                .reason
                .contains("any edits the script armed are never sent"),
            "the refusal tells the armed truth the host can tell: {}",
            fault.reason
        );
    }

    /// **A runtime fault is labelled and names its faulting line** (defect
    /// DIV-1, golden scenario 4's wording). The generic face was the
    /// rules-plane framing around a bare Starlark key error — `rule 'script'
    /// evaluation error: error: Key … was not found` — with no fault label and
    /// no structured line. The script entry is not a rule: its face opens
    /// `runtime fault at line N — ` and carries the kernel's message verbatim
    /// after the label, with the same line on [`ScriptFault::line`].
    #[test]
    fn a_runtime_fault_face_is_labelled_and_names_its_line() {
        use super::super::trace::{CommitLeg, FaultClass, ScriptTrace};

        let mut door = Fake::still();
        let mut host = WireHost::new(
            &mut door,
            "zt".to_owned(),
            Instant::now() + Duration::from_secs(30),
        );
        let ctx = effects::ScriptCtx {
            id: "script".to_owned(),
            args: std::collections::BTreeMap::new(),
            files: vec![PAGE.to_owned()],
            effects: Vec::new(),
        };
        // Line 1 reads; line 2 indexes a frontmatter key the page does not
        // carry — the G7 shape.
        let eval = effects::eval_script(
            &format!("card = read({PAGE:?})\nx = card[\"fm\"][\"report_path\"]\n"),
            &ctx,
            effects::ScriptLimits::default(),
            &mut host,
        );
        assert!(eval.outcome.is_err(), "the missing key aborts the attempt");

        let trace = ScriptTrace::assemble("b3:00", &eval, CommitLeg::NotIssued);
        let fault = trace.fault.as_ref().expect("the key error is the fault");
        assert_eq!(fault.class, FaultClass::Runtime);
        assert_eq!(
            fault.line,
            Some(2),
            "the kernel attributes the faulting line from the Starlark span"
        );
        assert!(
            fault.reason.starts_with("runtime fault at line 2 — "),
            "the face opens with the fault label and line: {}",
            fault.reason
        );
        assert!(
            fault.reason.contains("report_path"),
            "the kernel's own message rides verbatim after the label: {}",
            fault.reason
        );
        assert!(
            !fault.reason.contains("rule 'script'"),
            "the script entry is not a rule; the rules-plane framing is not \
             this face's: {}",
            fault.reason
        );
    }
}
