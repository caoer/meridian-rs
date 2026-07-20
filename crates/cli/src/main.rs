//! `mrd` — one-shot in-process wire client for the meridian engine.
//!
//! # Charter
//! **Owns:** argv → one wire `Request` per invocation, driven through
//! `sidecar::serve` with an in-memory NDJSON line, the `Response` decoded and
//! rendered (human default, `--json` = the raw wire frame verbatim).
//!
//! **Never does:** a second semantic path. The CLI is a PURE wire client
//! (deps: sidecar lib + wire + clap/serde/`serde_json`/fs) — no `model`, no `syntax`;
//! the sidecar stays the only place wire and model meet (law 3 untouched).
//! Splice `edits` pass through as RAW JSON so the server's strict decode —
//! not a tolerant client re-serialization — is what judges them (v2 §3.2
//! strict-server law).
//!
//! # Exec model (locked decision 2)
//! One-shot: load the vault, answer, exit. The engine is born and dies inside
//! the invocation — no daemon, no `--connect`. Consequence for `sub`: the
//! fresh epoch's ring is empty, so `sub` can only ack at seq 0 and replay
//! nothing; live streaming needs a resident daemon (out of scope for v1, and
//! said so in `--help` rather than pretended otherwise).
//!
//! # Exit codes
//! `0` success (ok:true) · `1` wire error (ok:false frame) · `2` usage ·
//! `3` fatal (engine I/O or a frame that fails to decode).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use serde_json::{Map, Value, json};

mod render;

/// The request id every one-shot frame carries (one request per process —
/// correlation is trivial, but the id keeps the frame a Request, never a
/// Notification, under the §3.1 raw-id classification).
const REQUEST_ID: u64 = 0;

#[derive(Parser)]
#[command(
    name = "mrd",
    version,
    about = "One-shot wire client for the meridian engine (wire-contract-v2)",
    long_about = "One-shot wire client for the meridian engine (wire-contract-v2).\n\
        Each invocation loads the vault at --root, answers one wire op in-process,\n\
        and exits — no daemon. Subcommands map 1:1 onto the frozen wire ops.\n\n\
        Exit codes: 0 success · 1 wire error · 2 usage · 3 fatal."
)]
struct Cli {
    /// Workspace root (the vault). Default: the current directory.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,

    /// Emit the raw wire response frame (NDJSON) instead of human output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Handshake: server name, protocol, capability set (§3.2)
    Hello,
    /// The map of one file: nodes with hpath/anchor/keys and revs (§4.1)
    Toc {
        /// Workspace-relative md path
        path: String,
    },
    /// Read one section (full span bytes) or the whole file (§4.2)
    Cat {
        /// Workspace-relative md path
        path: String,
        #[command(flatten)]
        sec: SecFlags,
    },
    /// Full node inventory of one file (§4.3)
    Extract {
        /// Workspace-relative md path
        path: String,
        /// Filter to these node kinds (comma-separated; unknown kinds refuse loud)
        #[arg(long, value_delimiter = ',')]
        kinds: Option<Vec<String>>,
    },
    /// Walk an Obsidian ref to its location — read-only interop (§4.5)
    Resolve {
        /// Source file the ref is written in (resolution is source-relative)
        from: String,
        /// Raw linktext, no brackets: `path#sub`, `#sub`, `path#^id`
        r#ref: String,
        /// Also return the fragment bytes (still no rev — mint partition)
        #[arg(long)]
        content: bool,
    },
    /// Batch write: the only write op (§4.4)
    Splice {
        /// Workspace-relative md path
        path: String,
        /// Edits as a JSON array in the wire §4.4 shape, or @FILE to read the
        /// array from a file. Passed to the engine RAW — the server's strict
        /// decode judges unknown fields, not this client.
        #[arg(long)]
        edits: String,
        /// Opaque actor identity recorded into receipts and deltas (§9)
        #[arg(long)]
        actor: Option<String>,
        /// RFC 3339 timestamp recorded into receipts and deltas (§9; never generated)
        #[arg(long)]
        now: Option<String>,
        /// Receipt file path (requires --receipt-anchor; §6.1)
        #[arg(long, requires = "receipt_anchor")]
        receipt_path: Option<String>,
        /// Receipt block anchor (requires --receipt-path)
        #[arg(long, requires = "receipt_path")]
        receipt_anchor: Option<String>,
        /// World-grain guard: fail the whole batch unless the root matches (§5.1)
        #[arg(long)]
        if_root: Option<String>,
        /// Everything except disk: same response shape, null root-after, no receipt
        #[arg(long)]
        dry: bool,
    },
    /// Outgoing edge map: one file, or the whole corpus with no path (§4.6)
    Links {
        /// Workspace-relative md path (absent = whole corpus)
        path: Option<String>,
        /// Refuse (stale-view, retry class) unless the answer is computed at this root
        #[arg(long)]
        require_root: Option<String>,
    },
    /// Integrity: current workspace root + seq (§4.7)
    Root,
    /// Replay the deltas between two roots (§4.7/§7.3)
    Diff { from_root: String, to_root: String },
    /// Subscribe ack (§4.7). One-shot honesty: this process's engine epoch is
    /// born empty, so only `from_seq 0` acks and nothing replays; live delta
    /// streaming needs a resident daemon (not in v1)
    Sub {
        #[arg(default_value_t = 0)]
        from_seq: u64,
    },
}

/// The three §2.1 mint-plane ref forms as flags, mutually exclusive; none
/// given = whole file. Occurrence-indexed hpath segments (`{"h":…,"n":…}`)
/// have no flag spelling in v1 — use `splice --edits` JSON where occurrence
/// disambiguation matters.
#[derive(Args)]
#[group(multiple = false)]
struct SecFlags {
    /// Section by hpath: one value per heading segment (e.g. --sec Goals --sec Q3)
    #[arg(long, num_args = 1..)]
    sec: Option<Vec<String>>,
    /// Section by block anchor (block id, no `^`)
    #[arg(long)]
    anchor: Option<String>,
    /// Frontmatter node by top-level key
    #[arg(long)]
    fm_key: Option<String>,
}

impl SecFlags {
    fn to_ref(&self) -> Option<wire::SecRef> {
        if let Some(segs) = &self.sec {
            return Some(wire::SecRef::Hpath {
                hpath: segs
                    .iter()
                    .map(|h| wire::HpathSeg {
                        h: h.clone(),
                        n: None,
                    })
                    .collect(),
            });
        }
        if let Some(anchor) = &self.anchor {
            return Some(wire::SecRef::Anchor {
                anchor: anchor.clone(),
            });
        }
        self.fm_key
            .as_ref()
            .map(|k| wire::SecRef::FmKey { fm_key: k.clone() })
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let line = match request_line(&cli.cmd) {
        Ok(line) => line,
        Err(msg) => {
            eprintln!("mrd: {msg}");
            return ExitCode::from(2);
        }
    };

    // One in-memory NDJSON line through the live serve loop — the same code
    // path a daemon would run, no second dispatch.
    let root = fs::WorkspaceRoot(cli.root.clone());
    let mut out = Vec::new();
    if let Err(e) = sidecar::serve(&root, line.as_bytes(), &mut out, &[]) {
        eprintln!("mrd: fatal: {e}");
        return ExitCode::from(3);
    }
    let Ok(out) = String::from_utf8(out) else {
        eprintln!("mrd: fatal: engine emitted non-UTF-8 frames");
        return ExitCode::from(3);
    };

    // §3.1 frame classification: the `id` key marks the response; id-less
    // frames are Notifications (sub replay).
    let mut response: Option<(&str, Value)> = None;
    let mut notifications: Vec<&str> = Vec::new();
    for frame_line in out.lines() {
        match serde_json::from_str::<Value>(frame_line) {
            Ok(v) if v.get("id").is_some() => response = Some((frame_line, v)),
            Ok(_) => notifications.push(frame_line),
            Err(e) => {
                eprintln!("mrd: fatal: engine frame failed to parse: {e}");
                return ExitCode::from(3);
            }
        }
    }
    let Some((raw, _)) = response else {
        eprintln!("mrd: fatal: engine produced no response frame");
        return ExitCode::from(3);
    };
    let Ok(response) = serde_json::from_str::<wire::Response>(raw) else {
        eprintln!("mrd: fatal: response frame failed typed decode: {raw}");
        return ExitCode::from(3);
    };

    if cli.json {
        // The raw wire transcript, verbatim: response frame, then any
        // Notification frames in emission order.
        let stdout = std::io::stdout();
        let mut w = stdout.lock();
        let ok = writeln!(w, "{raw}").is_ok()
            && notifications.iter().all(|n| writeln!(w, "{n}").is_ok());
        if !ok {
            return ExitCode::from(3);
        }
    } else {
        render::human(&response, &notifications);
    }
    ExitCode::from(u8::from(!response.ok))
}

/// argv → one NDJSON request line. Everything except splice builds through
/// the typed `wire::Op` (shape guaranteed by the wire crate); splice merges
/// its raw `edits` JSON in untouched so the server's strict decode stays the
/// judge.
fn request_line(cmd: &Cmd) -> Result<String, String> {
    let value = if let Cmd::Splice {
        path,
        edits,
        actor,
        now,
        receipt_path,
        receipt_anchor,
        if_root,
        dry,
    } = cmd
    {
        splice_value(
            path,
            edits,
            actor.as_deref(),
            now.as_deref(),
            receipt_path.as_deref(),
            receipt_anchor.as_deref(),
            if_root.as_deref(),
            *dry,
        )?
    } else {
        serde_json::to_value(wire::Request {
            id: Some(REQUEST_ID),
            op: typed_op(cmd),
        })
        .map_err(|e| format!("request serialization failed: {e}"))?
    };
    serde_json::to_string(&value).map_err(|e| format!("request serialization failed: {e}"))
}

fn typed_op(cmd: &Cmd) -> wire::Op {
    match cmd {
        Cmd::Hello => wire::Op::Hello {
            proto: sidecar::PROTO,
            client: Some(format!("mrd/{}", env!("CARGO_PKG_VERSION"))),
        },
        Cmd::Toc { path } => wire::Op::Toc {
            path: wire::Path(path.clone()),
        },
        Cmd::Cat { path, sec } => wire::Op::Cat {
            path: wire::Path(path.clone()),
            sec: sec.to_ref(),
        },
        Cmd::Extract { path, kinds } => wire::Op::Extract {
            path: wire::Path(path.clone()),
            kinds: kinds.clone(),
        },
        Cmd::Resolve {
            from,
            r#ref,
            content,
        } => wire::Op::Resolve {
            from: wire::Path(from.clone()),
            r#ref: r#ref.clone(),
            content: content.then_some(true),
        },
        Cmd::Links { path, require_root } => wire::Op::Links {
            path: path.as_ref().map(|p| wire::Path(p.clone())),
            require_root: require_root.as_ref().map(|r| wire::Root(r.clone())),
        },
        Cmd::Root => wire::Op::Root,
        Cmd::Diff { from_root, to_root } => wire::Op::Diff {
            from_root: wire::Root(from_root.clone()),
            to_root: wire::Root(to_root.clone()),
        },
        Cmd::Sub { from_seq } => wire::Op::Sub {
            from_seq: *from_seq,
        },
        Cmd::Splice { .. } => unreachable!("splice builds raw JSON in splice_value"),
    }
}

/// The splice request as raw JSON: typed scalars + the user's `edits` array
/// spliced in verbatim (string or `@FILE`), so a typo'd edit field reaches
/// the server and refuses loud instead of being silently dropped by a
/// tolerant client re-serialization.
#[allow(clippy::too_many_arguments)]
fn splice_value(
    path: &str,
    edits: &str,
    actor: Option<&str>,
    now: Option<&str>,
    receipt_path: Option<&str>,
    receipt_anchor: Option<&str>,
    if_root: Option<&str>,
    dry: bool,
) -> Result<Value, String> {
    let edits_text = if let Some(file) = edits.strip_prefix('@') {
        std::fs::read_to_string(file).map_err(|e| format!("--edits {edits}: {e}"))?
    } else {
        edits.to_string()
    };
    let edits: Value = serde_json::from_str(edits_text.trim())
        .map_err(|e| format!("--edits is not valid JSON: {e}"))?;
    if !edits.is_array() {
        return Err("--edits must be a JSON array of wire §4.4 edit objects".into());
    }

    let mut obj = Map::new();
    obj.insert("id".into(), json!(REQUEST_ID));
    obj.insert("op".into(), json!("splice"));
    obj.insert("path".into(), json!(path));
    if let Some(actor) = actor {
        obj.insert("actor".into(), json!(actor));
    }
    if let Some(now) = now {
        obj.insert("now".into(), json!(now));
    }
    if let (Some(path), Some(anchor)) = (receipt_path, receipt_anchor) {
        obj.insert("receipt".into(), json!({"path": path, "anchor": anchor}));
    }
    if let Some(root) = if_root {
        obj.insert("if_root".into(), json!(root));
    }
    if dry {
        obj.insert("dry".into(), json!(true));
    }
    obj.insert("edits".into(), edits);
    Ok(Value::Object(obj))
}
