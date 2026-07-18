//! **Charter:** thin bin — arg parsing + transport wiring; every op is a lib
//! call (cargo doctrine). Budget: <300 lines including main, enforced at
//! review. Respawn-friendly: no state survives a restart because no state
//! exists (law 2).
//!
//! Rung 1 surface: `hello` / `toc` / `extract` over NDJSON stdin/stdout.
//! stdout carries frames only; logs go to stderr; EOF → exit 0; a bad frame
//! never kills the loop (contract §3).

use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use wire::{Codec, ErrorBody, ErrorCode, Message, NdjsonCodec, Op, RequestFrame, ResponseBody, ResponseFrame};

const SERVER: &str = concat!("meridian-rs/", env!("CARGO_PKG_VERSION"));
/// Complete op-name set this sidecar answers, `hello` included (contract §5.1).
const CAPS: [&str; 3] = ["hello", "toc", "extract"];

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: sidecar <workspace-root>");
        return ExitCode::FAILURE;
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut codec = NdjsonCodec;
    let mut out = Vec::new();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let mut buf = line.into_bytes();
        buf.push(b'\n');

        let response = match codec.decode(&buf) {
            // Blank line: ignored by contract (§3 rule 2).
            Ok(None) => continue,
            Ok(Some((Message::Request(req), _))) => handle(&root, req),
            // A non-request frame on stdin is not a parseable request.
            Ok(Some(_)) | Err(_) => bad_frame(),
        };

        out.clear();
        if codec.encode(&Message::Response(response), &mut out).is_ok() {
            if stdout.write_all(&out).and_then(|()| stdout.flush()).is_err() {
                break; // client hung up
            }
        }
    }
    ExitCode::SUCCESS // EOF: finish, flush, exit 0 (contract §3)
}

/// Dispatch: one request → one response, each op a lib call.
fn handle(root: &Path, req: RequestFrame) -> ResponseFrame {
    let id = req.id;
    let body = match req.op {
        Op::Hello(h) => op_hello(h),
        Op::Toc(t) => op_nodes(root, t.path, Some(vec!["frontmatter".into(), "heading".into()])),
        Op::Extract(e) => op_nodes(root, e.path, e.kinds),
        // Rungs 2–3 are typed but not served at rung 1: not in CAPS,
        // answered as unknown_op until their rung lands (additive rollout,
        // server first — contract §7 rule 3).
        Op::Resolve(_) | Op::Splice(_) | Op::Root | Op::Guard => err(ErrorCode::UnknownOp, None),
    };
    ResponseFrame { id, body }
}

fn op_hello(h: wire::op::HelloReq) -> ResponseBody {
    if h.proto != wire::PROTO {
        return ResponseBody::Err(ErrorBody {
            error: ErrorCode::UnsupportedProto,
            message: Some("this sidecar speaks proto 1 only".into()),
            expected: None,
            actual: None,
            supported: Some(vec![wire::PROTO]),
            path: None,
        });
    }
    ResponseBody::Ok(wire::OkBody::Hello(wire::op::HelloOk {
        proto: wire::PROTO,
        server: SERVER.into(),
        caps: CAPS.iter().map(|s| s.to_string()).collect(),
    }))
}

/// `toc` ≡ `extract` with a fixed kind filter (contract §5.3): one shared
/// wrapper, three lib calls — path law (wire), disk truth (meridian::fs),
/// parse + tree (meridian::syntax/model) — then the wire mapping.
fn op_nodes(root: &Path, path: wire::WirePath, kinds: Option<Vec<String>>) -> ResponseBody {
    if let Err(m) = path.validate() {
        return err_path(ErrorCode::BadPath, Some(m), &path);
    }
    let src = match meridian::read_file(&root.join(&path.0)) {
        Ok(s) => s,
        Err(meridian::fs::ReadError::NotFound) => return err_path(ErrorCode::NotFound, None, &path),
        Err(meridian::fs::ReadError::InvalidUtf8) => {
            return err_path(ErrorCode::InvalidUtf8, Some("file is not valid UTF-8".into()), &path)
        }
        Err(meridian::fs::ReadError::Io(e)) => {
            return err(ErrorCode::Internal, Some(format!("read failed: {e}")))
        }
    };
    let events = meridian::parse(&src);
    let tree = meridian::build(&src, events);
    let nodes = to_wire_nodes(&src, &tree, kinds.as_deref());
    ResponseBody::Ok(wire::OkBody::Nodes(wire::op::NodesOk { path, nodes }))
}

/// The model→wire mapping seam: governed tree out, contract §5.2 node
/// inventory in node order. Unknown kind names in the filter match nothing.
fn to_wire_nodes(src: &str, tree: &meridian::Tree, kinds: Option<&[String]>) -> Vec<wire::Node> {
    let _ = (src, tree, kinds);
    todo!("flatten tree → wire nodes: kinds, spans, prefix-16b, hpath arrays, node order")
}

fn err(code: ErrorCode, message: Option<String>) -> ResponseBody {
    ResponseBody::Err(ErrorBody {
        error: code,
        message,
        expected: None,
        actual: None,
        supported: None,
        path: None,
    })
}

fn err_path(code: ErrorCode, message: Option<String>, path: &wire::WirePath) -> ResponseBody {
    match err(code, message) {
        ResponseBody::Err(mut e) => {
            e.path = Some(path.0.clone());
            ResponseBody::Err(e)
        }
        ok => ok,
    }
}

fn bad_frame() -> ResponseFrame {
    ResponseFrame {
        id: None,
        body: err(ErrorCode::BadFrame, Some("line is not a JSON object".into())),
    }
}
