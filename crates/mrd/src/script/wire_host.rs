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
    /// # Errors
    /// The socket refuses the connection, the transport fails, or the daemon
    /// refuses the handshake.
    pub fn connect(socket: &Path, workspace: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
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
        if !Frame::parse(&line)?.ok {
            return Err(io::Error::other(
                "the daemon refused the v3 handshake for this workspace",
            ));
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
        match (self.ok, self.body) {
            (true, Some(body)) => serde_json::from_str(body.get()).map_err(io::Error::other),
            (true, None) => Err(io::Error::other(format!("{op}: ok frame with no body"))),
            (false, error) => Err(io::Error::other(format!(
                "{op} refused: {}",
                error.map_or_else(|| "(no error body)".to_owned(), |e| e.get().to_owned())
            ))),
        }
    }
}

/// The wire-backed [`ScriptHost`]: `read(path)` lowers to `toc` (§4.1) plus one
/// `cat` per frontmatter key, and `read(path, section=…)` lowers to `cat`
/// (§4.2).
///
/// **Why the per-key `cat`.** The script's toc face publishes `fm` as
/// key → value, and no wire op serves frontmatter VALUES: `toc` publishes the
/// frontmatter row's `keys` only, and the composed read "body carries no
/// frontmatter plane" (`wire-serve::read::ref_not_found`, verbatim). So the
/// values come the one way the wire offers them — `cat` against the
/// `{"fm_key":…}` target, whose node IS that key's line. Read amplification is
/// the honest cost of not minting a wire op for it.
pub(crate) struct WireHost<'d> {
    door: &'d mut dyn Door,
    actor: String,
    /// The host's wall clock (§ Where the budgets bind — wall time binds in the
    /// host, above the kernel). Checked at the read seam, which is the only
    /// place a script spends unbounded time: pure evaluation is fuel-bounded.
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
    fn ask(
        &mut self,
        request: &Value,
        fault: &dyn Fn(String) -> ReadFault,
    ) -> Result<Value, ReadFault> {
        let op = request["op"].as_str().unwrap_or("(op)").to_owned();
        let line = self
            .door
            .call(request)
            .map_err(|e| fault(format!("the daemon did not answer: {e}")))?;
        Frame::parse(&line)
            .and_then(|frame| frame.body_value(&op))
            .map_err(|e| fault(e.to_string()))
    }

    /// Refuse a read that starts past the host's wall clock.
    fn within_deadline(&self, fault: &dyn Fn(String) -> ReadFault) -> Result<(), ReadFault> {
        if Instant::now() > self.deadline {
            return Err(fault(
                "the script entry's wall clock elapsed before this read — nothing was armed by \
                 the reads that did run, and nothing commits"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

impl ScriptHost for WireHost<'_> {
    fn toc(&mut self, path: &str) -> Result<TocFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: None,
            reason,
        };
        self.within_deadline(&fault)?;
        let body = self.ask(&json!({"op": "toc", "path": path}), &fault)?;

        let rev = body
            .get("file_rev")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("toc answered no file_rev".to_owned()))?
            .to_owned();
        // The word count is a DELIVERED fact, never one this host computes: the
        // composed `read` op (§4.1, toc mode) carries `words_total`, while the
        // `toc` op's own body is `{path, file_rev, root, nodes}` and carries
        // none. Asking `read` costs one already-declared op and no wire schema
        // delta, and a toc-mode read mints no receipt (`wire-serve::read`), so
        // the extra ask is side-effect-free. Answering 0 instead is what renders
        // `words:0` on a live face while the goldens render the truth.
        let read_body = self.ask(&json!({"op": "read", "path": path}), &fault)?;
        let words = usize::try_from(
            read_body
                .get("words_total")
                .and_then(Value::as_u64)
                .ok_or_else(|| fault("read answered no words_total".to_owned()))?,
        )
        .map_err(|_| fault("read answered a words_total this host cannot hold".to_owned()))?;
        let nodes = body
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| fault("toc answered no nodes".to_owned()))?
            .clone();

        // Frontmatter keys first, then one `cat` each for the values.
        let keys: Vec<String> = nodes
            .iter()
            .filter(|node| node.get("kind").and_then(Value::as_str) == Some("frontmatter"))
            .filter_map(|node| node.get("keys").and_then(Value::as_array))
            .flatten()
            .filter_map(|key| key.as_str().map(str::to_owned))
            .collect();
        let mut fm = std::collections::BTreeMap::new();
        for key in keys {
            let line = self.ask(
                &json!({"op": "cat", "path": path, "sec": {"fm_key": key}}),
                &fault,
            )?;
            let content = line
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| fault(format!("cat fm_key={key} answered no content")))?;
            fm.insert(key.clone(), fm_value_of(content));
        }

        Ok(TocFacts {
            rev,
            fm,
            toc: nodes.iter().filter_map(toc_entry).collect(),
            words,
        })
    }

    fn cat(&mut self, path: &str, section: &str) -> Result<SecFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(section.to_owned()),
            reason,
        };
        self.within_deadline(&fault)?;
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

/// The value on a frontmatter key line — the inverse of the server's own
/// compose (`model::PutAt::Upsert`: "the server composes `{key}: {value}`").
///
/// The key may be quoted on disk (the resolver matches
/// `k.trim().trim_matches(['"', '\''])`), so the split is on the first colon
/// rather than on the key text. A block value spanning several lines comes back
/// whole; only the line terminator is dropped.
fn fm_value_of(line: &str) -> String {
    let rest = line
        .split_once(':')
        .map_or(line, |(_, value)| value)
        .trim_end_matches('\n');
    rest.strip_prefix(' ').unwrap_or(rest).to_owned()
}
